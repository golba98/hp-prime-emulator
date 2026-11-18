//! Follow a constrained Geometry point through the construction graph.
use super::*;

impl Evaluator<'_> {
    pub(super) fn locus(
        &mut self,
        target: &Expr,
        driver: &Expr,
        source: &str,
    ) -> Result<Shape, String> {
        let ExprKind::Ident(id) = &driver.kind else {
            return Err("Select a named Point On as the locus driver".into());
        };
        let driver = self.reference(id).ok_or("Unknown locus driver")?;
        let _ = self.resolve(&driver)?;
        let curve = self
            .point_curves
            .get(&driver)
            .or_else(|| self.book.current_point_curves.get(&driver))
            .cloned()
            .ok_or("The locus driver is not constrained to a curve")?;
        // A straight driver varies over the G1 locus parameter interval even
        // when the Point On construction itself clamps to a finite segment.
        let mut interval = [-16., 16.];
        let shape = if let Shape::Line { a, b, .. } = curve {
            let d = g::delta(a, b);
            let at = |t| {
                point(Point2 {
                    x: a.x + t * d.x,
                    y: a.y + t * d.y,
                })
            };
            g::line(at(interval[0])?, at(interval[1])?, Extent::Segment)?
        } else {
            curve.clone()
        };
        let mut with_driver = Self {
            book: self.book,
            snapshot: self.snapshot,
            cancel: self.cancel.clone(),
            cache: BTreeMap::from([(driver.clone(), Ok(shape))]),
            point_curves: BTreeMap::new(),
            visiting: self.visiting.clone(),
            started: self.started,
            current: false,
        };
        // The ordinary transformations already accept entire curves. For a
        // midpoint, scale the moving curve by one half about the fixed point.
        if let Ok(image) = with_driver.locus_image(target, source, 0) {
            return Ok(image);
        }
        let complex = |p: Point2| format!("({}+i*({}))", canonical(p.x), canonical(p.y));
        let t = "t_lieu";
        let parameter = match curve {
            Shape::Circle { center, radius } => format!(
                "({}+({})*(1-{t}^2+2*i*{t})/(1+{t}^2))",
                complex(center),
                canonical(radius)
            ),
            Shape::Line { a, b, .. } => format!("({}+{}*{t})", complex(a), complex(g::delta(a, b))),
            Shape::Arc(a) => {
                interval = [a.start, a.end];
                format!(
                    "({}+({})*(COS({t})+i*SIN({t})))",
                    complex(a.center),
                    canonical(a.radius)
                )
            }
            Shape::Ellipse(e) => e.locus_source(t),
            Shape::Parabola(p) => p.locus_source(t),
            Shape::Hyperbola(h) => h.locus_source(t),
            Shape::Function { expression, .. } => {
                format!("({t}+i*({}))", replace_variable(&expression, "x", t))
            }
            Shape::Parametric(c) => {
                interval = c.bounds();
                replace_variable(&c.expression, &c.variable, t)
            }
            _ => return Err("Select a point constrained to a curve".into()),
        };
        let expression =
            self.locus_expression(target, source, &driver, &parameter, &mut BTreeSet::new())?;
        let mut curve = Parametric::new(expression, t.into(), interval, None)?;
        if !self.current {
            curve.trace(self.snapshot, self.cancel.clone())?;
        }
        Ok(Shape::Parametric(curve))
    }
    fn locus_image(&mut self, e: &Expr, source: &str, depth: usize) -> Result<Shape, String> {
        self.check()?;
        if depth >= 64 {
            return Err("Circular or deeply nested locus definition".into());
        }
        if let ExprKind::Ident(name) = &e.kind
            && let Some(name) = self.reference(name)
        {
            if let Some(value) = self.cache.get(&name) {
                return value.clone();
            }
            let def = self
                .book
                .objects
                .iter()
                .find(|o| o.name == name)
                .ok_or("Unknown locus point")?
                .source
                .clone();
            return self.locus_image(&parse(&def)?, &def, depth + 1);
        }
        if let Some([a, b]) = call_arguments(e, "midpoint") {
            let a = self.locus_image(a, source, depth + 1)?;
            let b = self.locus_image(b, source, depth + 1)?;
            return match (a, b) {
                (Shape::Point(a), Shape::Point(b)) => Ok(Shape::Point(g::midpoint(a, b)?)),
                (Shape::Point(center), curve) | (curve, Shape::Point(center)) => {
                    super::super::transform::Transform::About {
                        center,
                        scale: 0.5,
                        radians: 0.,
                    }
                    .apply(curve)
                }
                _ => Err("Both midpoint arguments vary".into()),
            };
        }
        self.value(e, source)
    }
    fn locus_expression(
        &mut self,
        e: &Expr,
        source: &str,
        driver: &str,
        parameter: &str,
        visiting: &mut BTreeSet<String>,
    ) -> Result<String, String> {
        self.check()?;
        if visiting.len() >= 64 {
            return Err("Locus expression nesting exceeded".into());
        }
        let result = match &e.kind {
            ExprKind::Ident(id) if self.reference(id).is_some() => {
                let name = self.reference(id).unwrap();
                if name == driver {
                    return Ok(parameter.into());
                }
                if !visiting.insert(name.clone()) {
                    return Err("Circular locus construction".into());
                }
                let def = self
                    .book
                    .objects
                    .iter()
                    .find(|o| o.name == name)
                    .ok_or("Unknown locus point")?
                    .source
                    .clone();
                let result =
                    self.locus_expression(&parse(&def)?, &def, driver, parameter, visiting)?;
                visiting.remove(&name);
                result
            }
            ExprKind::Binary(op, a, b) => format!(
                "({}{}{})",
                self.locus_expression(a, source, driver, parameter, visiting)?,
                op.symbol(),
                self.locus_expression(b, source, driver, parameter, visiting)?
            ),
            ExprKind::Unary(ppl_syntax::ast::UnOp::Neg, a) => format!(
                "(-{})",
                self.locus_expression(a, source, driver, parameter, visiting)?
            ),
            ExprKind::Call(callee, args) => {
                let ExprKind::Ident(name) = &callee.kind else {
                    return Err("Invalid locus expression".into());
                };
                let values = args
                    .iter()
                    .map(|a| self.locus_expression(a, source, driver, parameter, visiting))
                    .collect::<Result<Vec<_>, _>>()?;
                match (name.to_ascii_lowercase().as_str(), values.as_slice()) {
                    ("point" | "affix", [p]) => p.clone(),
                    ("point", [x, y]) => format!("(({x})+i*({y}))"),
                    ("abscissa", [p]) => format!("RE({p})"),
                    ("ordinate", [p]) => format!("IM({p})"),
                    ("translation", [d, p]) => format!("(({d})+({p}))"),
                    ("midpoint", [p, q]) => format!("(({p})+({q}))/2"),
                    ("distance", [p, q]) => format!("ABS(({p})-({q}))"),
                    _ => format!("{name}({})", values.join(",")),
                }
            }
            _ => source[e.span.start as usize..e.span.end as usize].to_owned(),
        };
        if result.len() > 4096 {
            return Err("Expanded locus expression exceeds the Geometry size limit".into());
        }
        Ok(result)
    }
}

fn replace_variable(expression: &str, variable: &str, replacement: &str) -> String {
    let mut output = String::with_capacity(expression.len());
    let mut start = 0;
    for (i, ch) in expression.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            continue;
        }
        let word = &expression[start..i];
        output.push_str(if word == variable { replacement } else { word });
        output.push(ch);
        start = i + ch.len_utf8();
    }
    let word = &expression[start..];
    output.push_str(if word == variable { replacement } else { word });
    output
}
