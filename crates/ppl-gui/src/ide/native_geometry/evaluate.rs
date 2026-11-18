mod locus;
use super::{primitives as g, *};
use ppl_syntax::ast::{Expr, ExprKind};
use ppl_vm::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
struct Evaluator<'a> {
    book: &'a Book,
    snapshot: &'a SessionSnapshot,
    cancel: Arc<AtomicBool>,
    cache: BTreeMap<String, Result<Shape, String>>,
    point_curves: BTreeMap<String, Shape>,
    visiting: BTreeSet<String>,
    started: Instant,
    current: bool,
}
pub(super) fn scene(
    book: &Book,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Scene, String> {
    let mut e = Evaluator {
        book,
        snapshot,
        cancel,
        cache: BTreeMap::new(),
        visiting: BTreeSet::new(),
        started: Instant::now(),
        point_curves: BTreeMap::new(),
        current: false,
    };
    let mut scene = Scene::default();
    for object in &book.objects {
        e.check()?;
        scene.objects.push(SceneObject {
            name: object.name.clone(),
            value: e.resolve(&object.name),
        });
    }
    for m in &book.measurements {
        e.check()?;
        scene.measurements.push(e.expression(&m.source));
    }
    e.check()?;
    scene.point_curves = e.point_curves;
    Ok(scene)
}
pub(super) fn current_expression(
    book: &Book,
    source: &str,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<ppl_vm::PortableValue, String> {
    super::source(source)?;
    let (ast, diagnostics) = ppl_syntax::parse_expression(&super::plot_syntax(source));
    if diagnostics.has_errors() {
        return Err(diagnostics.errors().next().unwrap().message.clone());
    }
    fn references(
        expression: &Expr,
        book: &Book,
        snapshot: &SessionSnapshot,
    ) -> Result<(), String> {
        match &expression.kind {
            ExprKind::Ident(name) => {
                let qualified = name.strip_prefix("Geometry.");
                let bare = qualified.unwrap_or(name);
                if ppl_compiler::geometry::is_object(bare)
                    && (qualified.is_some() || !snapshot.globals.contains_key(name))
                    && !book.objects.iter().any(|o| bare == format!("G{}", o.name))
                {
                    return Err(format!("Unknown Geometry object {name}"));
                }
            }
            ExprKind::Call(_, args)
            | ExprKind::Index(_, args)
            | ExprKind::List(args)
            | ExprKind::Bracket(args) => {
                for a in args {
                    references(a, book, snapshot)?;
                }
            }
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => {
                references(a, book, snapshot)?
            }
            ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => {
                references(a, book, snapshot)?;
                references(b, book, snapshot)?;
            }
            ExprKind::Assign(..) | ExprKind::Store(..) => {
                return Err("Geometry expressions cannot assign calculator variables".into());
            }
            _ => {}
        }
        Ok(())
    }
    references(&ast, book, snapshot)?;
    let mut evaluator = Evaluator {
        book,
        snapshot,
        cancel,
        cache: BTreeMap::new(),
        visiting: BTreeSet::new(),
        started: Instant::now(),
        point_curves: BTreeMap::new(),
        current: true,
    };
    evaluator.check()?;
    if let ExprKind::Call(callee, args) = &ast.kind
        && matches!(&callee.kind, ExprKind::Ident(name) if name.strip_prefix("Geometry.").unwrap_or(name).eq_ignore_ascii_case("coordinates"))
    {
        if args.len() != 1 {
            return Err("coordinates expects one point or vector".into());
        }
        if let Ok(shape) = evaluator.value(&args[0], source) {
            let points = match shape {
                Shape::Point(p) => vec![p],
                Shape::Points(p) => p,
                _ => return Err("coordinates needs a point or vector".into()),
            };
            return Ok(ppl_vm::PortableValue::Matrix {
                rows: points.len(),
                cols: 2,
                vector: points.len() == 1,
                data: points
                    .iter()
                    .flat_map(|p| [super::access::scalar(p.x), super::access::scalar(p.y)])
                    .collect::<Result<_, _>>()?,
            });
        }
    } else {
        match evaluator.value(&ast, source) {
            Ok(shape) => return super::access::shape_value(&shape),
            Err(_)
                if matches!(&ast.kind, ExprKind::Call(callee, args)
                    if matches!(&callee.kind, ExprKind::Ident(name) if name == "circle")
                        && args.len() != 2) =>
            {
                evaluator.check()?;
                let options = ppl_cas::Options {
                    timeout: Duration::from_secs(4),
                    ..Default::default()
                };
                return algebra_overload(source, snapshot, &evaluator.cancel, &options);
            }
            Err(_) if call_arguments(&ast, "tangent").is_some() => {
                evaluator.check()?;
                return Ok(ppl_vm::PortableValue::String(format!(
                    "{source} \n Error: Bad Argument Value"
                )));
            }
            Err(_) => {}
        }
    }
    evaluator.check()?;
    symbolic_value(&ast, book, snapshot)
}
fn algebra_overload(
    source: &str,
    snapshot: &SessionSnapshot,
    cancel: &AtomicBool,
    options: &ppl_cas::Options,
) -> Result<ppl_vm::PortableValue, String> {
    let home = snapshot
        .globals
        .iter()
        .filter_map(|(name, value)| {
            ppl_vm::cas::to_cas(&value.to_value()).map(|value| (name.clone(), value))
        })
        .collect();
    let mut session = snapshot.cas.clone();
    let result = session
        .evaluate_with_home(source, &home, options, cancel)
        .map_err(|e| e.to_string())?;
    Ok(ppl_vm::PortableValue::from(&ppl_vm::cas::from_cas(
        result.value,
    )))
}
// Symbolic point data is intentionally different from a live object reference.
// A copied point stays algebraic; only the named cache is a construction.
fn symbolic_value(
    expression: &Expr,
    book: &Book,
    snapshot: &SessionSnapshot,
) -> Result<ppl_vm::PortableValue, String> {
    use ExprKind as E;
    fn source(e: &Expr, book: &Book, snapshot: &SessionSnapshot) -> Result<String, String> {
        if let E::Ident(name) = &e.kind {
            let qualified = name.strip_prefix("Geometry.");
            if qualified.is_none()
                && let Some(value) = snapshot.globals.get(name)
            {
                return ppl_vm::cas::expression(&value.to_value()).map_err(|e| e.to_string());
            }
            if let Some(name) = qualified.unwrap_or(name).strip_prefix('G')
                && let Some(object) = book.objects.iter().find(|o| o.name == name)
            {
                return match book.current_values.get(name) {
                    Some(value) => {
                        ppl_vm::cas::expression(&value.to_value()).map_err(|e| e.to_string())
                    }
                    None => Ok(object.source.clone()),
                };
            }
        }
        if let E::Call(callee, args) = &e.kind
            && let E::Ident(name) = &callee.kind
        {
            let name = name.strip_prefix("Geometry.").unwrap_or(name);
            let args = args
                .iter()
                .map(|arg| source(arg, book, snapshot))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(
                match (name.to_ascii_lowercase().as_str(), args.as_slice()) {
                    ("abscissa", [a]) => format!("re({a})"),
                    ("ordinate", [a]) => format!("im({a})"),
                    ("affix", [a]) => a.clone(),
                    ("distance", [a, b]) => {
                        format!("sqrt((re({a})-re({b}))^2+(im({a})-im({b}))^2)")
                    }
                    _ => format!("{name}({})", args.join(",")),
                },
            );
        }
        ppl_compiler::ppl_expression_source(e).map_err(|e| e.to_string())
    }
    if let E::Call(callee, args) = &expression.kind
        && matches!(&callee.kind, E::Ident(name) if name.strip_prefix("Geometry.").unwrap_or(name).eq_ignore_ascii_case("coordinates"))
        && let [argument] = args.as_slice()
    {
        let data = source(argument, book, snapshot)?;
        let (data, diagnostics) = ppl_syntax::parse_expression(&data);
        if !diagnostics.has_errors()
            && let E::Call(callee, components) = &data.kind
            && matches!(&callee.kind, E::Ident(name) if name == "point")
            && let [x, y] = components.as_slice()
        {
            return Ok(ppl_vm::PortableValue::List(
                [x, y]
                    .iter()
                    .map(|e| {
                        ppl_compiler::ppl_expression_source(e)
                            .map(|s| super::access::symbolic(format!("point({s},0)")))
                            .map_err(|e| e.to_string())
                    })
                    .collect::<Result<_, _>>()?,
            ));
        }
    }
    Ok(super::access::symbolic(source(expression, book, snapshot)?))
}
impl Evaluator<'_> {
    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            Err("Geometry calculation interrupted".into())
        } else if self.started.elapsed() > Duration::from_secs(4) {
            Err("Geometry calculation time limit exceeded".into())
        } else {
            Ok(())
        }
    }
    fn reference(&self, name: &str) -> Option<String> {
        let (name, qualified) = name
            .strip_prefix("Geometry.")
            .map_or((name, false), |n| (n, true));
        if self.current && !qualified && self.snapshot.globals.contains_key(name) {
            return None;
        }
        let n = name.strip_prefix('G')?;
        self.book
            .objects
            .iter()
            .find(|o| o.name == n)
            .map(|o| o.name.clone())
    }
    fn resolve(&mut self, name: &str) -> Result<Shape, String> {
        self.check()?;
        if let Some(value) = self.cache.get(name) {
            return value.clone();
        }
        if self.visiting.len() >= 64 {
            return Err("Geometry dependency nesting exceeded".into());
        }
        if !self.visiting.insert(name.into()) {
            return Err(format!("Circular geometry definition G{name}"));
        }
        let source = if self.current {
            let value = self
                .book
                .current_values
                .get(name)
                .ok_or("Geometry object has not been evaluated")?;
            if let Some(curve) = self.book.current_curves.get(name) {
                self.visiting.remove(name);
                return Ok(curve.clone());
            }
            match value {
                ppl_vm::PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) => {
                    source.clone()
                }
                ppl_vm::PortableValue::Symbolic(ppl_cas::CasValue::Undefined) => {
                    self.visiting.remove(name);
                    return Ok(Shape::Undefined);
                }
                ppl_vm::PortableValue::Real(v) => {
                    self.visiting.remove(name);
                    return scalar(real_number(*v)?);
                }
                ppl_vm::PortableValue::Complex(v) => {
                    self.visiting.remove(name);
                    return Ok(Shape::Number {
                        re: real_number(v.re)?,
                        im: real_number(v.im)?,
                    });
                }
                _ => return Err("Geometry value is not a constructed object".into()),
            }
        } else {
            self.book
                .objects
                .iter()
                .find(|o| o.name == name)
                .ok_or_else(|| format!("Unknown geometry object G{name}"))?
                .source
                .clone()
        };
        let value = (|| {
            let ast = parse(&source)?;
            let shape = self.value(&ast, &source)?;
            if !self.current
                && matches!(shape, Shape::Point(_))
                && let Some(curve) = self.point_curve(&ast, &source)?
            {
                self.point_curves.insert(name.to_owned(), curve);
            }
            Ok(shape)
        })();
        self.visiting.remove(name);
        self.cache.insert(name.into(), value.clone());
        value
    }
    fn expression(&mut self, source: &str) -> Result<Shape, String> {
        self.value(&parse(source)?, source)
    }
    fn point_curve(&mut self, point: &Expr, source: &str) -> Result<Option<Shape>, String> {
        if let ExprKind::Ident(name) = &point.kind
            && let Some(name) = self.reference(name)
        {
            return Ok(if self.current {
                self.book.current_point_curves.get(&name)
            } else {
                self.point_curves.get(&name)
            }
            .cloned());
        }
        let Some([curve, ..]) = call_arguments(point, "element") else {
            return Ok(None);
        };
        let curve = self.value(curve, source)?;
        if let Shape::Function { expression, .. } = curve {
            return Ok(Some(Shape::Function {
                expression,
                paths: Vec::new(),
            }));
        }
        Ok(matches!(
            curve,
            Shape::Line { .. }
                | Shape::Circle { .. }
                | Shape::Arc(_)
                | Shape::Ellipse(_)
                | Shape::Parametric(_)
                | Shape::Hyperbola(_)
                | Shape::Parabola(_)
        )
        .then_some(curve))
    }
    fn value(&mut self, e: &Expr, source: &str) -> Result<Shape, String> {
        self.check()?;
        if let ExprKind::Ident(name) = &e.kind {
            if let Some(name) = self.reference(name) {
                return self.resolve(&name);
            }
            if let Some(ppl_vm::PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(arc))) =
                self.snapshot.globals.get(name)
                && let Some(ast) = replayable_arc(arc)
            {
                return self.value(&ast, arc);
            }
        }
        if let ExprKind::Assign(..) | ExprKind::Store(..) = &e.kind {
            return Err("Geometry expressions cannot assign calculator variables".into());
        }
        let ExprKind::Call(callee, args) = &e.kind else {
            return self.numeric(e, source);
        };
        let ExprKind::Ident(name) = &callee.kind else {
            return self.numeric(e, source);
        };
        let name = name
            .strip_prefix("Geometry.")
            .unwrap_or(name)
            .to_ascii_lowercase();
        let arity = |n: usize| {
            if args.len() == n {
                Ok(())
            } else {
                Err(format!("{name} expects{n}arguments"))
            }
        };
        match name.as_str() {
            "point" => match args.len() {
                1 => Ok(Shape::Point(self.pt(&args[0], source)?)),
                2 => Ok(Shape::Point(point(Point2 {
                    x: self.real(&args[0], source)?,
                    y: self.real(&args[1], source)?,
                })?)),
                _ => Err("point expects one affix or two coordinates".into()),
            },
            "locus" => {
                arity(2)?;
                self.locus(&args[0], &args[1], source)
            }
            "conic" => {
                arity(1)?;
                super::conic::build(&args[0], source, |arg| self.real(arg, source))
            }
            "plotseq" => {
                let [expr, var, range, count] = args.as_slice() else {
                    return Err("Expected expression, variable, sequence interval and count".into());
                };
                let (ExprKind::Ident(var), ExprKind::List(range)) = (&var.kind, &range.kind) else {
                    return Err("Invalid sequence variable or interval".into());
                };
                let [start, lo, hi] = range.as_slice() else {
                    return Err("Sequence requires start, minimum and maximum".into());
                };
                let range = [
                    self.real(start, source)?,
                    self.real(lo, source)?,
                    self.real(hi, source)?,
                ];
                let n = self.real(count, source)?;
                if n.fract() != 0. {
                    return Err("Sequence count must be an integer".into());
                }
                Ok(Shape::Sequence(SequencePlot::build(
                    self.expression_text(expr, source, Some(var))?,
                    var.clone(),
                    range,
                    n as i64,
                    self.snapshot,
                    self.cancel.clone(),
                )?))
            }
            "plotparam" | "plotpolar" => {
                if !matches!(args.len(), 4 | 5) {
                    return Err(
                        "A curve needs expression, variable, bounds and optional step".into(),
                    );
                }
                let ExprKind::Ident(variable) = &args[1].kind else {
                    return Err("Expected curve parameter name".into());
                };
                let mut expression = self.expression_text(&args[0], source, Some(variable))?;
                if name == "plotpolar" {
                    expression = format!("({expression})*(COS({variable})+i*SIN({variable}))");
                }
                let bounds = [self.real(&args[2], source)?, self.real(&args[3], source)?];
                let step = args.get(4).map(|x| self.real(x, source)).transpose()?;
                let mut curve = Parametric::new(expression, variable.clone(), bounds, step)?;
                if !self.current {
                    curve.trace(self.snapshot, self.cancel.clone())?;
                }
                Ok(Shape::Parametric(curve))
            }
            "plotfunc" => {
                arity(1)?;
                let expression = self.expression_text(&args[0], source, Some("x"))?;
                super::function::build(
                    expression,
                    self.snapshot,
                    self.cancel.clone(),
                    !self.current,
                )
            }
            "element" => {
                let [curve, rest @ ..] = args.as_slice() else {
                    return Err("element expects a curve and an optional real parameter".into());
                };
                let parameter = match rest {
                    [] => 0.5,
                    [t] => self.real(t, source)?,
                    _ => {
                        return Err("element expects a curve and an optional real parameter".into());
                    }
                };
                let curve = self.value(curve, source)?;
                let curve = if let Shape::Sequence(seq) = curve {
                    *seq.function
                } else {
                    curve
                };
                let p = match curve {
                    Shape::Parametric(c) => c.at(parameter, self.snapshot, self.cancel.clone())?,
                    Shape::Function { expression, .. } => super::function::element(
                        &expression,
                        parameter,
                        self.snapshot,
                        self.cancel.clone(),
                    )?,
                    _ => g::element(&curve, parameter)?,
                };
                Ok(Shape::Point(p))
            }
            "tangent" => match args.as_slice() {
                [point] => {
                    let location = self.value(point, source)?;
                    let curve = self
                        .point_curve(point, source)?
                        .ok_or("A one-argument tangent needs a point on a curve")?;
                    let p = as_point(location)?;
                    if let Shape::Parametric(c) = &curve {
                        let t = c.nearest(p)?.0;
                        return c.tangent(t, self.snapshot, self.cancel.clone());
                    }
                    if let Shape::Function { expression, .. } = &curve {
                        return super::function::tangent(
                            expression,
                            p.x,
                            self.snapshot,
                            self.cancel.clone(),
                        );
                    }
                    g::tangent_at(&curve, p)
                }
                [curve, location] => {
                    let curve = self.value(curve, source)?;
                    let at = self.value(location, source)?;
                    if let Shape::Parametric(c) = &curve {
                        let t = match at {
                            Shape::Number { re, im: 0. } => re,
                            Shape::Point(p) => c.nearest(p)?.0,
                            _ => return Err("Expected a point or parameter".into()),
                        };
                        return c.tangent(t, self.snapshot, self.cancel.clone());
                    }
                    if let Shape::Function { expression, .. } = &curve {
                        let x = match at {
                            Shape::Point(p) => p.x,
                            Shape::Number { re, im: 0. } => re,
                            _ => return Err("Select a function point or real abscissa".into()),
                        };
                        return super::function::tangent(
                            expression,
                            x,
                            self.snapshot,
                            self.cancel.clone(),
                        );
                    }
                    if let Shape::Point(p) = at
                        && let Some(constraint) = self.point_curve(location, source)?
                        && super::access::shape_value(&curve)?
                            == super::access::shape_value(&constraint)?
                    {
                        return g::tangent_at(&curve, p);
                    }
                    g::tangent(curve, at)
                }
                _ => Err(
                    "tangent expects a point on a curve, or a curve and a point or parameter"
                        .into(),
                ),
            },
            "segment" | "line" | "half_line" => {
                arity(2)?;
                let a = self.pt(&args[0], source)?;
                let b = self.pt(&args[1], source)?;
                g::line(
                    a,
                    b,
                    match name.as_str() {
                        "segment" => Extent::Segment,
                        "half_line" => Extent::Ray,
                        _ => Extent::Line,
                    },
                )
            }
            "midpoint" => {
                let (a, b) = if args.len() == 1 {
                    let (a, b, extent) = self.ln(&args[0], source)?;
                    if extent != Extent::Segment {
                        return Err("midpoint needs a segment".into());
                    }
                    (a, b)
                } else {
                    arity(2)?;
                    (self.pt(&args[0], source)?, self.pt(&args[1], source)?)
                };
                Ok(Shape::Point(g::midpoint(a, b)?))
            }
            "parallel" | "perpendicular" => {
                let (p, a, b) = if args.len() == 3 {
                    (
                        self.pt(&args[0], source)?,
                        self.pt(&args[1], source)?,
                        self.pt(&args[2], source)?,
                    )
                } else {
                    arity(2)?;
                    let p = self.pt(&args[0], source)?;
                    let (a, b, _) = self.ln(&args[1], source)?;
                    (p, a, b)
                };
                let mut d = g::delta(a, b);
                if name == "perpendicular" {
                    d = Point2 { x: -d.y, y: d.x };
                }
                g::line(
                    p,
                    point(Point2 {
                        x: p.x + d.x,
                        y: p.y + d.y,
                    })?,
                    Extent::Line,
                )
            }
            "perpen_bisector" => {
                let (a, b) = if args.len() == 1 {
                    let (a, b, _) = self.ln(&args[0], source)?;
                    (a, b)
                } else {
                    arity(2)?;
                    (self.pt(&args[0], source)?, self.pt(&args[1], source)?)
                };
                let m = g::midpoint(a, b)?;
                let d = g::delta(a, b);
                g::line(
                    m,
                    point(Point2 {
                        x: m.x - d.y,
                        y: m.y + d.x,
                    })?,
                    Extent::Line,
                )
            }
            "median_line" | "altitude" | "bisector" => {
                arity(3)?;
                let a = self.pt(&args[0], source)?;
                let b = self.pt(&args[1], source)?;
                let c = self.pt(&args[2], source)?;
                let to = if name == "median_line" {
                    g::midpoint(b, c)?
                } else if name == "altitude" {
                    g::project(a, b, c)?
                } else {
                    let u = g::delta(a, b);
                    let v = g::delta(a, c);
                    let ul = a.distance(b)?;
                    let vl = a.distance(c)?;
                    if ul == 0. || vl == 0. {
                        return Err("An angle bisector needs distinct defining points".into());
                    }
                    point(Point2 {
                        x: a.x + u.x / ul + v.x / vl,
                        y: a.y + u.y / ul + v.y / vl,
                    })?
                };
                g::line(a, to, Extent::Line)
            }
            "inter" | "single_inter" => {
                if name == "inter" {
                    arity(2)?;
                } else if !(2..=3).contains(&args.len()) {
                    return Err(
                        "single_inter expects two curves and an optional nearby point".into(),
                    );
                }
                let a = self.value(&args[0], source)?;
                let b = self.value(&args[1], source)?;
                let points = g::intersections(&a, &b)?;
                if name == "inter" {
                    Ok(Shape::Points(points))
                } else {
                    let line_on_curve = matches!(
                        (&a, &b),
                        (
                            Shape::Line { .. },
                            Shape::Arc(_) | Shape::Ellipse(_) | Shape::Parabola(_)
                        ) | (
                            Shape::Arc(_) | Shape::Ellipse(_) | Shape::Parabola(_),
                            Shape::Line { .. }
                        )
                    );
                    let p = if args.len() == 2 && line_on_curve {
                        points.first().copied()
                    } else {
                        let near = if args.len() == 3 {
                            self.pt(&args[2], source)?
                        } else {
                            Point2 { x: 0., y: 0. }
                        };
                        points.into_iter().min_by(|a, b| {
                            (a.x - near.x)
                                .hypot(a.y - near.y)
                                .total_cmp(&(b.x - near.x).hypot(b.y - near.y))
                        })
                    };
                    match p {
                        Some(p) => Ok(Shape::Point(p)),
                        None if line_on_curve => Ok(Shape::Undefined),
                        None => Err("The geometric objects do not intersect".into()),
                    }
                }
            }
            "circle" => {
                if !matches!(args.len(), 2 | 4) {
                    return Err(
                        "circle expects two arguments and optional start and end angles".into(),
                    );
                }
                let a = self.pt(&args[0], source)?;
                let (center, radius) = match self.value(&args[1], source)? {
                    Shape::Point(b) => {
                        let center = g::midpoint(a, b)?;
                        (center, center.distance(a)?)
                    }
                    Shape::Number { re, im } => (a, re.hypot(im)),
                    _ => return Err("circle needs a point or radius after its center".into()),
                };
                if let [start, end] = &args[2..] {
                    Ok(Shape::Arc(CircularArc::new(
                        center,
                        radius,
                        self.real(start, source)?,
                        self.real(end, source)?,
                    )?))
                } else {
                    g::circle(center, radius)
                }
            }
            "parabola" => {
                arity(2)?;
                let first = self.pt(&args[0], source)?;
                let parabola = match self.value(&args[1], source)? {
                    Shape::Number { re, im: 0. } => super::Parabola::vertical(first, re)?,
                    Shape::Line { a, b, .. } => super::Parabola::from_focus(first, a, b)?,
                    _ => return Err("A parabola needs a directrix or focal length".into()),
                };
                Ok(Shape::Parabola(parabola))
            }
            "ellipse" | "hyperbola" => {
                arity(3)?;
                let foci = [self.pt(&args[0], source)?, self.pt(&args[1], source)?];
                let size = match self.value(&args[2], source)? {
                    Shape::Number { re, im: 0. } => super::ellipse::Size::SemiMajor(re),
                    witness => super::ellipse::Size::Witness(as_point(witness)?),
                };
                if name == "hyperbola" {
                    Ok(Shape::Hyperbola(super::Hyperbola::new(foci, size)?))
                } else {
                    Ok(Shape::Ellipse(super::Ellipse::new(foci, size)?))
                }
            }
            "circumcircle" | "incircle" | "excircle" => {
                arity(3)?;
                let a = self.pt(&args[0], source)?;
                let b = self.pt(&args[1], source)?;
                let c = self.pt(&args[2], source)?;
                if name == "circumcircle" {
                    g::circumcircle(a, b, c)
                } else {
                    let ab = a.distance(b)?;
                    let opposite = b.distance(c)?;
                    let bc = if name == "excircle" {
                        -opposite
                    } else {
                        opposite
                    };
                    let ca = c.distance(a)?;
                    let sum = ab + bc + ca;
                    if sum <= 0. {
                        return Err(format!("{name} needs a nondegenerate triangle"));
                    }
                    let center = point(Point2 {
                        x: (bc / sum) * a.x + (ca / sum) * b.x + (ab / sum) * c.x,
                        y: (bc / sum) * a.y + (ca / sum) * b.y + (ab / sum) * c.y,
                    })?;
                    g::circle(
                        center,
                        2. * crate::ide::native_app_models::Geometry::polygon_area(&[a, b, c])?
                            / sum,
                    )
                }
            }
            "equilateral_triangle"
            | "square"
            | "rhombus"
            | "isosceles_triangle"
            | "rectangle"
            | "right_triangle"
            | "parallelogram"
            | "isopolygon" => {
                arity(
                    if matches!(name.as_str(), "equilateral_triangle" | "square") {
                        2
                    } else {
                        3
                    },
                )?;
                let a = self.pt(&args[0], source)?;
                let b = self.pt(&args[1], source)?;
                let side = g::delta(a, b);
                let offset = match name.as_str() {
                    "isopolygon" => return g::regular_polygon(a, b, self.real(&args[2], source)?),
                    "parallelogram" => g::delta(b, self.pt(&args[2], source)?),
                    "rectangle" | "right_triangle" => {
                        let scale = match self.value(&args[2], source)? {
                            Shape::Number { re, im: 0. } => re,
                            third => {
                                let p = as_point(third)?;
                                let foot = g::project(p, a, b)?;
                                let offset = g::delta(foot, p);
                                return g::side_polygon(a, b, offset, name == "rectangle");
                            }
                        };
                        Point2 {
                            x: -side.y * scale,
                            y: side.x * scale,
                        }
                    }
                    "square" => Point2 {
                        x: -side.y,
                        y: side.x,
                    },
                    _ => {
                        let angle = if name == "equilateral_triangle" {
                            std::f64::consts::FRAC_PI_3
                        } else {
                            self.real(&args[2], source)? * std::f64::consts::TAU / self.angle_turn()
                        };
                        let (sin, cos) = angle.sin_cos();
                        Point2 {
                            x: side.x * cos - side.y * sin,
                            y: side.y * cos + side.x * sin,
                        }
                    }
                };
                g::side_polygon(
                    a,
                    b,
                    offset,
                    matches!(
                        name.as_str(),
                        "parallelogram" | "rectangle" | "square" | "rhombus"
                    ),
                )
            }
            "quadrilateral" | "triangle" | "polygon" => {
                if name == "quadrilateral" {
                    arity(4)?;
                } else if name == "triangle" {
                    arity(3)?;
                }
                let points = args
                    .iter()
                    .map(|a| self.pt(a, source))
                    .collect::<Result<Vec<_>, _>>()?;
                g::polygon(points)
            }
            "center" => {
                arity(1)?;
                match self.value(&args[0], source)? {
                    Shape::Circle { center, .. } => Ok(Shape::Point(center)),
                    Shape::Arc(arc) => Ok(Shape::Point(arc.center)),
                    _ => Err("center needs a circle".into()),
                }
            }
            "radius" => {
                arity(1)?;
                match self.value(&args[0], source)? {
                    Shape::Circle { radius, .. } => scalar(radius),
                    Shape::Arc(arc) => scalar(arc.radius),
                    Shape::Ellipse(_) => scalar(0.),
                    _ => Err("radius needs a circle".into()),
                }
            }
            "abscissa" | "ordinate" | "affix" => {
                arity(1)?;
                let p = self.pt(&args[0], source)?;
                if name == "affix" {
                    Ok(Shape::Number { re: p.x, im: p.y })
                } else {
                    scalar(if name == "abscissa" { p.x } else { p.y })
                }
            }
            "distance" => {
                arity(2)?;
                let a = self.value(&args[0], source)?;
                let b = self.value(&args[1], source)?;
                let value = match (a, b) {
                    (Shape::Line { a, b, extent }, p) | (p, Shape::Line { a, b, extent }) => {
                        let p = as_point(p)?;
                        p.distance(g::closest(p, a, b, extent)?)?
                    }
                    (a, b) => as_point(a)?.distance(as_point(b)?)?,
                };
                scalar(value)
            }
            "area" | "perimeter" => {
                arity(1)?;
                let value = match self.value(&args[0], source)? {
                    Shape::Circle { radius, .. } => {
                        if name == "area" {
                            std::f64::consts::PI * radius * radius
                        } else {
                            std::f64::consts::TAU * radius
                        }
                    }
                    Shape::Parametric(curve) => {
                        curve.integral(name == "area", self.snapshot, self.cancel.clone())?
                    }
                    Shape::EmptyConic(_) if name == "area" => 0.,
                    Shape::Lines(_) | Shape::Sequence(_) => return Ok(Shape::Undefined),
                    Shape::Arc(arc) => {
                        if name == "area" {
                            arc.area()
                        } else {
                            arc.perimeter()
                        }
                    }
                    Shape::Hyperbola(curve) => {
                        if name == "perimeter" {
                            return Ok(Shape::Undefined);
                        }
                        // A stored hyperbola evaluates to a list of two plotparam objects.
                        // G1 area on that list is zero; an inline conic integrates both branches.
                        if !curve.equation
                            && matches!(&args[0].kind,ExprKind::Ident(id) if self.reference(id).is_some())
                        {
                            0.
                        } else {
                            curve.area()
                        }
                    }
                    Shape::Parabola(curve) => {
                        if name == "area" {
                            curve.area()
                        } else {
                            curve.perimeter()
                        }
                    }
                    Shape::Ellipse(ellipse) => {
                        if name == "area" {
                            ellipse.area()
                        } else {
                            ellipse.perimeter()
                        }
                    }
                    Shape::Polygon(points) => {
                        if name == "area" {
                            // Firmware preserves orientation for polygon area.
                            let mut twice_area = 0.;
                            for i in 0..points.len() {
                                let a = points[i];
                                let b = points[(i + 1) % points.len()];
                                twice_area += a.x * b.y - b.x * a.y;
                            }
                            number(twice_area / 2.)?
                        } else {
                            let mut total = 0.;
                            for i in 0..points.len() {
                                total += points[i].distance(points[(i + 1) % points.len()])?;
                            }
                            total
                        }
                    }
                    _ => return Err(format!("{name} needs a circle, ellipse or polygon")),
                };
                scalar(value)
            }
            "slope" => {
                arity(1)?;
                let (a, b, _) = self.ln(&args[0], source)?;
                if b.x == a.x {
                    return Err("Vertical line has no finite slope".into());
                }
                scalar((b.y - a.y) / (b.x - a.x))
            }
            "angle" => {
                arity(3)?;
                let a = self.pt(&args[0], source)?;
                let b = self.pt(&args[1], source)?;
                let c = self.pt(&args[2], source)?;
                scalar(g::angle(a, b, c)? * self.angle_turn() / std::f64::consts::TAU)
            }
            "projection" => {
                arity(2)?;
                let curve = self.value(&args[0], source)?;
                let p = self.pt(&args[1], source)?;
                let q = match curve {
                    Shape::Line { a, b, .. } => g::project(p, a, b)?,
                    Shape::Circle { center, radius }
                    | Shape::Arc(CircularArc { center, radius, .. }) => {
                        let d = g::delta(center, p);
                        point(g::circle_element(center, radius, d.y.atan2(d.x)))?
                    }
                    Shape::Parabola(curve) => curve.project(p)?,
                    _ => g::curve_projection(&curve, p)?,
                };
                Ok(Shape::Point(q))
            }
            "inversion" => {
                arity(3)?;
                g::inversion(
                    self.pt(&args[0], source)?,
                    self.real(&args[1], source)?,
                    self.pt(&args[2], source)?,
                )
            }
            "reciprocation" => {
                arity(2)?;
                let Shape::Circle { center, radius } = self.value(&args[0], source)? else {
                    return Err("Reciprocation needs a circle".into());
                };
                match &args[1].kind {
                    ExprKind::Bracket(items) | ExprKind::List(items) => {
                        let mut images = Vec::with_capacity(items.len());
                        for e in items {
                            images.push(g::reciprocate(center, radius, self.value(e, source)?)?);
                        }
                        Ok(Shape::Objects(images))
                    }
                    _ => g::reciprocate(center, radius, self.value(&args[1], source)?),
                }
            }
            "translation" | "reflection" | "rotation" | "homothety" | "similarity" => {
                use super::transform::Transform;
                let transform = match name.as_str() {
                    "translation" => {
                        arity(2)?;
                        let Shape::Number { re, im } = self.value(&args[0], source)? else {
                            return Err(
                                "Translation requires a vector or a point difference".into()
                            );
                        };
                        Transform::Translate(Point2 { x: re, y: im })
                    }
                    "reflection" => {
                        arity(2)?;
                        match self.value(&args[0], source)? {
                            Shape::Line { a, b, .. } => Transform::Mirror { a, b },
                            center => Transform::About {
                                center: as_point(center)?,
                                scale: -1.,
                                radians: 0.,
                            },
                        }
                    }
                    _ => {
                        arity(if name == "similarity" { 4 } else { 3 })?;
                        let center = self.pt(&args[0], source)?;
                        let scale = if name == "rotation" {
                            1.
                        } else {
                            self.real(&args[1], source)?
                        };
                        let radians = if name == "homothety" {
                            0.
                        } else {
                            let angle = &args[if name == "rotation" { 1 } else { 2 }];
                            self.real(angle, source)? * std::f64::consts::TAU / self.angle_turn()
                        };
                        Transform::About {
                            center,
                            scale,
                            radians,
                        }
                    }
                };
                transform.apply(self.value(args.last().unwrap(), source)?)
            }
            _ => self.numeric(e, source),
        }
    }
    fn angle_turn(&self) -> f64 {
        self.snapshot
            .app_settings
            .get("Geometry")
            .cloned()
            .unwrap_or_default()
            .effective(&self.snapshot.settings)
            .angle
            .full_turn()
    }
    fn pt(&mut self, e: &Expr, source: &str) -> Result<Point2, String> {
        as_point(self.value(e, source)?)
    }
    fn real(&mut self, e: &Expr, source: &str) -> Result<f64, String> {
        match self.value(e, source)? {
            Shape::Number { re, im: 0. } => number(re),
            _ => Err("A real numeric expression is required".into()),
        }
    }
    fn ln(&mut self, e: &Expr, source: &str) -> Result<(Point2, Point2, Extent), String> {
        match self.value(e, source)? {
            Shape::Line { a, b, extent } => Ok((a, b, extent)),
            _ => Err("A line, ray or segment is required".into()),
        }
    }
    fn expression_text(
        &mut self,
        e: &Expr,
        source: &str,
        function: Option<&str>,
    ) -> Result<String, String> {
        let mut replacements = Vec::new();
        self.rewrite(e, source, &mut replacements, function)?;
        let mut expression = source
            .get(e.span.start as usize..e.span.end as usize)
            .ok_or("Invalid geometry expression span")?
            .to_string();
        replacements.sort_by_key(|r: &(usize, usize, String)| std::cmp::Reverse(r.0));
        for (start, end, text) in replacements {
            expression.replace_range(
                start - e.span.start as usize..end - e.span.start as usize,
                &text,
            );
        }
        Ok(expression)
    }
    fn numeric(&mut self, e: &Expr, source: &str) -> Result<Shape, String> {
        let expression = self.expression_text(e, source, None)?;
        let result = super::super::native_readonly::evaluate(
            &expression,
            "Geometry",
            self.snapshot,
            std::iter::empty(),
            self.cancel.clone(),
        )?;
        match result {
            Value::Complex(c) => {
                let re = real_number(c.re)?;
                let im = real_number(c.im)?;
                Ok(Shape::Number { re, im })
            }
            v => scalar(real_number(
                v.expect_real("geometry coordinate")
                    .map_err(|e| e.to_string())?,
            )?),
        }
    }
    fn rewrite(
        &mut self,
        e: &Expr,
        source: &str,
        out: &mut Vec<(usize, usize, String)>,
        function: Option<&str>,
    ) -> Result<(), String> {
        if let ExprKind::Assign(..) | ExprKind::Store(..) = &e.kind {
            return Err("Geometry expressions cannot assign calculator variables".into());
        }
        let replacement = match &e.kind {
            ExprKind::Ident(name) if function == Some(name.as_str()) => None,
            ExprKind::Ident(name) => {
                if let Some(name) = self.reference(name) {
                    Some(self.resolve(&name))
                } else if function.is_some()
                    && let Some(ppl_vm::PortableValue::Real(value)) =
                        self.snapshot.globals.get(name)
                {
                    Some(real_number(*value).and_then(scalar))
                } else {
                    None
                }
            }
            ExprKind::Call(callee, _) if matches!(&callee.kind,ExprKind::Ident(name)if known(name)) =>
            {
                if function.is_some_and(|v| super::function::reads_variable(e, v)) {
                    return Err("Function graphs require fixed Geometry arguments".into());
                }
                Some(self.value(e, source))
            }
            _ => None,
        };
        if let Some(value) = replacement {
            let p = as_point(value?)?;
            let text = if function.is_some() && p.y == 0. {
                format!("({})", canonical(p.x))
            } else {
                format!("(({})+({})*i)", canonical(p.x), canonical(p.y))
            };
            out.push((e.span.start as usize, e.span.end as usize, text));
            return Ok(());
        }
        match &e.kind {
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => {
                self.rewrite(a, source, out, function)?
            }
            ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => {
                self.rewrite(a, source, out, function)?;
                self.rewrite(b, source, out, function)?;
            }
            ExprKind::Index(callee, args) => {
                self.rewrite(callee, source, out, function)?;
                for a in args {
                    self.rewrite(a, source, out, function)?;
                }
            }
            ExprKind::Call(_, args) | ExprKind::List(args) | ExprKind::Bracket(args) => {
                for a in args {
                    self.rewrite(a, source, out, function)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn parse(source: &str) -> Result<Expr, String> {
    super::source(source)?;
    let (ast, diagnostics) = ppl_syntax::parse_expression(&super::plot_syntax(source));
    if let Some(error) = diagnostics.errors().next() {
        return Err(error.message.clone());
    }
    Ok(ast)
}
fn call_arguments<'a>(e: &'a Expr, function: &str) -> Option<&'a [Expr]> {
    let ExprKind::Call(callee, args) = &e.kind else {
        return None;
    };
    let ExprKind::Ident(name) = &callee.kind else {
        return None;
    };
    name.strip_prefix("Geometry.")
        .unwrap_or(name)
        .eq_ignore_ascii_case(function)
        .then_some(args.as_slice())
}
// A stored PPL argument can replay the literal construction exported by
// shape_value. Keep arbitrary symbolic bindings out of recursive evaluation.
fn replayable_arc(source: &str) -> Option<Expr> {
    source.strip_prefix("circle(point(")?;
    let arc = parse(source).ok()?;
    let [center, radius, start, end] = call_arguments(&arc, "circle")? else {
        return None;
    };
    let [x, y] = call_arguments(center, "point")? else {
        return None;
    };
    fn literal(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Unary(_, arg) => literal(arg),
            _ => false,
        }
    }
    let literal_parameters = [x, y, radius, start, end].into_iter().all(literal);
    literal_parameters.then_some(arc)
}
fn known(name: &str) -> bool {
    let name = name.strip_prefix("Geometry.").unwrap_or(name);
    matches!(
        name.to_ascii_lowercase().as_str(),
        "point"
            | "element"
            | "locus"
            | "conic"
            | "plotseq"
            | "plotparam"
            | "plotpolar"
            | "plotfunc"
            | "tangent"
            | "segment"
            | "line"
            | "half_line"
            | "midpoint"
            | "parallel"
            | "perpendicular"
            | "perpen_bisector"
            | "median_line"
            | "altitude"
            | "bisector"
            | "inter"
            | "single_inter"
            | "circle"
            | "parabola"
            | "hyperbola"
            | "ellipse"
            | "circumcircle"
            | "incircle"
            | "excircle"
            | "quadrilateral"
            | "equilateral_triangle"
            | "square"
            | "rectangle"
            | "parallelogram"
            | "right_triangle"
            | "rhombus"
            | "isosceles_triangle"
            | "isopolygon"
            | "triangle"
            | "polygon"
            | "center"
            | "radius"
            | "abscissa"
            | "ordinate"
            | "affix"
            | "distance"
            | "area"
            | "perimeter"
            | "slope"
            | "angle"
            | "inversion"
            | "reciprocation"
            | "projection"
            | "translation"
            | "similarity"
            | "reflection"
            | "rotation"
            | "homothety"
    )
}
fn scalar(n: f64) -> Result<Shape, String> {
    Ok(Shape::Number {
        re: number(n)?,
        im: 0.,
    })
}
pub(super) fn as_point(shape: Shape) -> Result<Point2, String> {
    match shape {
        Shape::Point(p) => point(p),
        Shape::Number { re, im } => point(Point2 { x: re, y: im }),
        _ => Err("A point or numeric affix is required".into()),
    }
}
pub(super) enum PointMovement {
    Free,
    OnCurve(String),
}
pub(super) fn point_movement(source: &str) -> Option<PointMovement> {
    fn literal(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Ident(n) => n == "i" || n == "π",
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) => literal(a),
            ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => literal(a) && literal(b),
            _ => false,
        }
    }
    let (e, diagnostics) = ppl_syntax::parse_expression(&super::plot_syntax(source));
    if diagnostics.has_errors() {
        return None;
    }
    let ExprKind::Call(callee, args) = e.kind else {
        return None;
    };
    let ExprKind::Ident(function) = callee.kind else {
        return None;
    };
    let name = function.strip_prefix("Geometry.").unwrap_or(&function);
    if name.eq_ignore_ascii_case("point") && args.iter().all(literal) {
        return Some(PointMovement::Free);
    }
    if name.eq_ignore_ascii_case("element")
        && let [curve, rest @ ..] = args.as_slice()
        && (rest.is_empty() || matches!(rest, [parameter] if literal(parameter)))
        && let ExprKind::Ident(reference) = &curve.kind
        && ppl_compiler::geometry::is_object(
            reference.strip_prefix("Geometry.").unwrap_or(reference),
        )
    {
        return Some(PointMovement::OnCurve(reference.clone()));
    }
    None
}

fn real_number(n: ppl_math::Real) -> Result<f64, String> {
    let v = number(n.to_f64())?;
    if !n.is_zero() && v == 0. {
        Err("Geometry value exceeds numerical range".into())
    } else {
        Ok(v)
    }
}
#[cfg(test)]
mod algebra_tests {
    use super::*;
    #[test]
    fn circle_overloads_fail_explicitly_without_the_algebra_helper() {
        let options = ppl_cas::Options {
            helper: std::env::temp_dir()
                .join("prime-nonexistent-geometry-helper")
                .join("ppl-giac-helper"),
            ..Default::default()
        };
        let result = algebra_overload(
            "circle(0,0,1)",
            &ppl_vm::SessionContext::default().snapshot(),
            &AtomicBool::new(false),
            &options,
        );
        assert!(result.unwrap_err().contains("CAS helper unavailable"));
    }
    #[test]
    fn installed_algebra_helper_computes_circle_parameter_overloads() {
        let Some(helper) = std::env::var_os("PPL_CAS_HELPER") else {
            eprintln!("No explicit CAS helper; installed-resource control omitted");
            return;
        };
        let options = ppl_cas::Options {
            helper: helper.into(),
            ..Default::default()
        };
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let value = algebra_overload(
            "circle(0,0,1)",
            &snapshot,
            &AtomicBool::new(false),
            &options,
        )
        .unwrap();
        assert_eq!(
            value,
            ppl_vm::PortableValue::Symbolic(ppl_cas::CasValue::Undefined)
        );
        let value = algebra_overload(
            "circle(0,1,0,1)",
            &snapshot,
            &AtomicBool::new(false),
            &options,
        )
        .unwrap();
        assert!(
            matches!(value,ppl_vm::PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) if source.contains("circle")),
            "Circle overload must return actual symbolic geometry"
        );
    }
}
