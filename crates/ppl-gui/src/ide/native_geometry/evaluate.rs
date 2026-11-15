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
    visiting: BTreeSet<String>,
    started: Instant,
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
    Ok(scene)
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
        let source = self
            .book
            .objects
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| format!("Unknown geometry object G{name}"))?
            .source
            .clone();
        let value = self.expression(&source);
        self.visiting.remove(name);
        self.cache.insert(name.into(), value.clone());
        value
    }
    fn expression(&mut self, source: &str) -> Result<Shape, String> {
        super::source(source)?;
        let (ast, diags) = ppl_syntax::parse_expression(source);
        if diags.has_errors() {
            return Err(diags.errors().next().unwrap().message.clone());
        }
        self.value(&ast, source)
    }
    fn value(&mut self, e: &Expr, source: &str) -> Result<Shape, String> {
        self.check()?;
        if let ExprKind::Ident(name) = &e.kind
            && let Some(name) = self.reference(name)
        {
            return self.resolve(&name);
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
        let name = name.to_ascii_lowercase();
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
                    let near = if args.len() == 3 {
                        self.pt(&args[2], source)?
                    } else {
                        Point2 { x: 0., y: 0. }
                    };
                    let p = points
                        .into_iter()
                        .min_by(|a, b| {
                            (a.x - near.x)
                                .hypot(a.y - near.y)
                                .total_cmp(&(b.x - near.x).hypot(b.y - near.y))
                        })
                        .ok_or("The geometric objects do not intersect")?;
                    Ok(Shape::Point(p))
                }
            }
            "circle" => {
                arity(2)?;
                let a = self.pt(&args[0], source)?;
                match self.value(&args[1], source)? {
                    Shape::Point(b) => {
                        let c = g::midpoint(a, b)?;
                        g::circle(c, c.distance(a)?)
                    }
                    Shape::Number { re, im } => g::circle(a, re.hypot(im)),
                    _ => Err("circle needs a point or radius after its center".into()),
                }
            }
            "circumcircle" | "incircle" => {
                arity(3)?;
                let a = self.pt(&args[0], source)?;
                let b = self.pt(&args[1], source)?;
                let c = self.pt(&args[2], source)?;
                if name == "circumcircle" {
                    g::circumcircle(a, b, c)
                } else {
                    let ab = a.distance(b)?;
                    let bc = b.distance(c)?;
                    let ca = c.distance(a)?;
                    let sum = ab + bc + ca;
                    if sum == 0. {
                        return Err("An incircle needs a nondegenerate triangle".into());
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
            "triangle" | "polygon" => {
                if name == "triangle" {
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
                    _ => Err("center needs a circle".into()),
                }
            }
            "radius" => {
                arity(1)?;
                match self.value(&args[0], source)? {
                    Shape::Circle { radius, .. } => scalar(radius),
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
                    Shape::Polygon(points) => {
                        if name == "area" {
                            crate::ide::native_app_models::Geometry::polygon_area(&points)?
                        } else {
                            let mut total = 0.;
                            for i in 0..points.len() {
                                total += points[i].distance(points[(i + 1) % points.len()])?;
                            }
                            total
                        }
                    }
                    _ => return Err(format!("{name} needs a circle or polygon")),
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
                let (a, b, _) = self.ln(&args[0], source)?;
                let p = self.pt(&args[1], source)?;
                Ok(Shape::Point(g::project(p, a, b)?))
            }
            "reflection" => {
                arity(2)?;
                let axis = self.value(&args[0], source)?;
                let p = self.pt(&args[1], source)?;
                let p = match axis {
                    Shape::Line { a, b, .. } => g::reflect(p, a, b)?,
                    center => {
                        let c = as_point(center)?;
                        point(Point2 {
                            x: 2. * c.x - p.x,
                            y: 2. * c.y - p.y,
                        })?
                    }
                };
                Ok(Shape::Point(p))
            }
            "rotation" => {
                arity(3)?;
                let center = self.pt(&args[0], source)?;
                let angle = self.real(&args[1], source)?;
                let p = self.pt(&args[2], source)?;
                Ok(Shape::Point(
                    p.rotate(center, angle * 360. / self.angle_turn())?,
                ))
            }
            "homothety" => {
                arity(3)?;
                let c = self.pt(&args[0], source)?;
                let k = self.real(&args[1], source)?;
                let p = self.pt(&args[2], source)?;
                Ok(Shape::Point(point(Point2 {
                    x: c.x + k * (p.x - c.x),
                    y: c.y + k * (p.y - c.y),
                })?))
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
    fn numeric(&mut self, e: &Expr, source: &str) -> Result<Shape, String> {
        let mut replacements = Vec::new();
        self.rewrite(e, source, &mut replacements)?;
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
    ) -> Result<(), String> {
        if let ExprKind::Assign(..) | ExprKind::Store(..) = &e.kind {
            return Err("Geometry expressions cannot assign calculator variables".into());
        }
        let replacement = match &e.kind {
            ExprKind::Ident(name) => self.reference(name).map(|name| self.resolve(&name)),
            ExprKind::Call(callee, _) if matches!(&callee.kind,ExprKind::Ident(name)if known(name)) => {
                Some(self.value(e, source))
            }
            _ => None,
        };
        if let Some(value) = replacement {
            let p = as_point(value?)?;
            let text = format!("(({})+({})*i)", canonical(p.x), canonical(p.y));
            out.push((e.span.start as usize, e.span.end as usize, text));
            return Ok(());
        }
        match &e.kind {
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => {
                self.rewrite(a, source, out)?
            }
            ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => {
                self.rewrite(a, source, out)?;
                self.rewrite(b, source, out)?;
            }
            ExprKind::Index(callee, args) => {
                self.rewrite(callee, source, out)?;
                for a in args {
                    self.rewrite(a, source, out)?;
                }
            }
            ExprKind::Call(_, args) | ExprKind::List(args) | ExprKind::Bracket(args) => {
                for a in args {
                    self.rewrite(a, source, out)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn known(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "point"
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
            | "circumcircle"
            | "incircle"
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
            | "projection"
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
fn as_point(shape: Shape) -> Result<Point2, String> {
    match shape {
        Shape::Point(p) => point(p),
        Shape::Number { re, im } => point(Point2 { x: re, y: im }),
        _ => Err("A point or numeric affix is required".into()),
    }
}
pub(super) fn free_point(source: &str) -> bool {
    fn literal(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Ident(n) => n == "i" || n == "π",
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) => literal(a),
            ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => literal(a) && literal(b),
            _ => false,
        }
    }
    let (e, d) = ppl_syntax::parse_expression(source);
    !d.has_errors()
        && matches!(&e.kind,ExprKind::Call(c,args)if matches!(&c.kind,ExprKind::Ident(n)if n.eq_ignore_ascii_case("point"))&&args.iter().all(literal))
}

fn real_number(n: ppl_math::Real) -> Result<f64, String> {
    let v = number(n.to_f64())?;
    if !n.is_zero() && v == 0. {
        Err("Geometry value exceeds numerical range".into())
    } else {
        Ok(v)
    }
}
