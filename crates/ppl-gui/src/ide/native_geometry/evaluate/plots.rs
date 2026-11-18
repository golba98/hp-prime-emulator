//! Geometry plane plots share the Advanced Graphing expression evaluator and contour sampler.
use super::*;
use crate::ide::native_apps::{self, Evaluator as Plane, PlotWindow, numeric};
use ppl_syntax::ast::BinOp;

fn vector(e: &Expr) -> Result<&[Expr], String> {
    match &e.kind {
        ExprKind::Bracket(v) | ExprKind::List(v) => Ok(v),
        _ => Err("Expected a vector of plot arguments".into()),
    }
}
fn ident(e: &Expr, name: &str) -> bool {
    matches!(&e.kind,ExprKind::Ident(n) if n==name)
}
impl Evaluator<'_> {
    fn plot_window(&mut self, ranges: Option<&Expr>, source: &str) -> Result<PlotWindow, String> {
        let [mut xmin, mut xmax, mut ymin, mut ymax] = self
            .snapshot
            .geometry_plot_settings()
            .derived_bounds()
            .map(|n| n.to_f64());
        if let Some(ranges) = ranges {
            let [x, a, b, y, c, d] = vector(ranges)? else {
                return Err("Expected x and y intervals".into());
            };
            if !matches!((&x.kind,&y.kind),(ExprKind::Ident(a),ExprKind::Ident(b)) if a!=b) {
                return Err("Expected two distinct plane variables".into());
            }
            [xmin, xmax, ymin, ymax] = [
                self.real(a, source)?,
                self.real(b, source)?,
                self.real(c, source)?,
                self.real(d, source)?,
            ];
        }
        let w = PlotWindow {
            xmin,
            xmax,
            ymin,
            ymax,
            ..Default::default()
        };
        w.validate()?;
        Ok(w)
    }

    pub(super) fn plane_plot(
        &mut self,
        name: &str,
        args: &[Expr],
        source: &str,
    ) -> Result<Shape, String> {
        let (expr, rest) = args.split_first().ok_or("Missing plot expression")?;
        if name == "plotlist" {
            if !rest.is_empty() {
                return Err("List plot expects a single matrix or vector".into());
            }
            let items = vector(expr)?;
            let mut points = Vec::with_capacity(items.len());
            for (index, e) in items.iter().enumerate() {
                let (x, y) = match &e.kind {
                    ExprKind::Bracket(coords) | ExprKind::List(coords) => {
                        let [x, y] = coords.as_slice() else {
                            return Err("List plot needs one coordinate pair per row".into());
                        };
                        (self.real(x, source)?, self.real(y, source)?)
                    }
                    _ => (index as f64, self.real(e, source)?),
                };
                points.push(point(Point2 { x, y })?);
            }
            return Ok(Shape::Polyline(points));
        }
        let expression = self.expression_text(expr, source, Some("x"))?;
        if name == "plotimplicit" {
            if rest.len() > 1 {
                return Err("Implicit plot expects an equation and optional plane bounds".into());
            }
            let window = self.plot_window(rest.first(), source)?;
            if rest.is_empty()
                && let Ok(shape) = conic::build(expr, source, |e| self.real(e, source))
            {
                return Ok(shape);
            }
            if let ExprKind::Binary(BinOp::Sub | BinOp::Eq, left, right) = &expr.kind
                && ident(left, "y")
                && !function::reads_variable(right, "y")
            {
                let rhs = self.expression_text(right, source, Some("x"))?;
                let mut curve = Parametric::new(
                    format!("x+i*({rhs})"),
                    "x".into(),
                    [window.xmin, window.xmax],
                    None,
                )?;
                curve.trace(self.snapshot, self.cancel.clone())?;
                return Ok(Shape::Parametric(curve));
            }
            let samples = native_apps::geometry_contours(
                &expression,
                &window,
                self.snapshot,
                self.cancel.clone(),
            )?;
            let paths = contour_paths(&samples);
            return Ok(Shape::Objects(
                paths.into_iter().map(Shape::Polyline).collect(),
            ));
        }
        let axes = vector(rest.first().ok_or("Missing coordinate variables")?)?;
        let (x, y) = match axes {
            [x, y] | [x, _, _, y] | [x, _, _, y, _, _] => (&x.kind, &y.kind),
            _ => return Err("Invalid plot coordinates".into()),
        };
        let (ExprKind::Ident(x), ExprKind::Ident(y)) = (x, y) else {
            return Err("Expected coordinate names".into());
        };
        let mut plane =
            Plane::geometry_plane(&expression, x, y, self.snapshot, self.cancel.clone())?;
        let mut eval = |x, y| numeric(plane.at_xy_value(x, y));
        if name == "plotode" {
            let [vars, initial] = rest else {
                return Err("ODE plot expects variables and an initial point".into());
            };
            let mut window = self.plot_window(None, source)?;
            match vector(vars)? {
                [_, _] => {}
                [_, lo, hi, _] => {
                    window.xmin = self.real(lo, source)?;
                    window.xmax = self.real(hi, source)?;
                    window.validate()?;
                }
                _ => return Err("Expected [x,y] or [x=start..end,y]".into()),
            }
            let [a, b] = vector(initial)? else {
                return Err("Expected two initial coordinates".into());
            };
            let p = Point2 {
                x: self.real(a, source)?,
                y: self.real(b, source)?,
            };
            let mut left = integrate(p, window.xmin.min(p.x), &mut eval)?;
            left.reverse();
            left.extend(
                integrate(p, window.xmax.max(p.x), &mut eval)?
                    .into_iter()
                    .skip(1),
            );
            return Ok(Shape::Polyline(left));
        }
        let (range, opts) = rest
            .split_first()
            .ok_or("Slopefield requires x and y intervals")?;
        let w = self.plot_window(Some(range), source)?;
        let mut steps = vec![];
        let mut normalize = false;
        for arg in opts {
            if ident(arg, "normalize") {
                normalize = true;
            } else if let ExprKind::Bracket(items) | ExprKind::List(items) = &arg.kind {
                for item in items {
                    steps.push(self.real(item, source)?);
                }
            } else {
                steps.push(self.real(arg, source)?);
            }
        }
        let [dx, dy] = match steps.as_slice() {
            [] => [(w.xmax - w.xmin) / 14., (w.ymax - w.ymin) / 14.],
            [dx, dy] => [*dx, *dy],
            _ => return Err("Slopefield needs both x and y steps".into()),
        };
        if dx <= 0. || dy <= 0. {
            return Err("Slopefield steps must be positive".into());
        }
        let nx = ((w.xmax - w.xmin) / dx + 1e-12).floor();
        let ny = ((w.ymax - w.ymin) / dy + 1e-12).floor();
        if (nx + 1.) * (ny + 1.) > 10_000. {
            return Err("Slopefield exceeds 10000 segments".into());
        }
        let mut segments = Vec::new();
        for i in 0..=nx as usize {
            self.check()?;
            let x = w.xmin + i as f64 * dx;
            for j in 0..=ny as usize {
                let y = w.ymin + j as f64 * dy;
                let Some(m) = eval(x, y)? else { continue };
                let len = dx.min(dy) * 0.5;
                let span = if normalize {
                    0.5 * len / m.hypot(1.)
                } else {
                    len
                };
                let start = if normalize {
                    Point2 {
                        x: x - span,
                        y: y - m * span,
                    }
                } else {
                    Point2 { x, y }
                };
                segments.push(Shape::Polyline(vec![
                    point(start)?,
                    point(Point2 {
                        x: x + span,
                        y: y + m * span,
                    })?,
                ]));
            }
        }
        Ok(Shape::Objects(segments))
    }
}

// Adjacent contour cells share endpoints. Stitch their segments before exposing
// the result as geometry, including closed curves and disjoint components.
fn contour_paths(samples: &[Option<[f64; 2]>]) -> Vec<Vec<Point2>> {
    use std::collections::BTreeMap;
    let key = |p: [f64; 2]| p.map(|v| if v == 0. { 0 } else { v.to_bits() });
    let mut seen = std::collections::BTreeSet::new();
    let mut edges = vec![];
    let mut adjacency = BTreeMap::<_, Vec<usize>>::new();
    for piece in samples.split(Option::is_none) {
        if let [Some(a), Some(b)] = piece {
            let (ka, kb) = (key(*a), key(*b));
            if ka == kb || !seen.insert((ka.min(kb), ka.max(kb))) {
                continue;
            }
            let i = edges.len();
            adjacency.entry(ka).or_default().push(i);
            adjacency.entry(kb).or_default().push(i);
            edges.push((ka, kb, *a, *b));
        }
    }
    let mut used = vec![false; edges.len()];
    let mut paths = vec![];
    // Start at endpoints so that a whole open component becomes one path.
    for degree_one in [true, false] {
        for (start, adjacent) in &adjacency {
            if degree_one && adjacent.len() != 1 {
                continue;
            }
            let mut key = start;
            let mut path = vec![];
            while let Some(&i) = adjacency[key].iter().find(|&&i| !used[i]) {
                used[i] = true;
                let (a, b, p, q) = &edges[i];
                let (p, q, next) = if key == a { (p, q, b) } else { (q, p, a) };
                if path.is_empty() {
                    path.push(Point2 { x: p[0], y: p[1] });
                }
                path.push(Point2 { x: q[0], y: q[1] });
                key = next;
            }
            if !path.is_empty() {
                paths.push(path);
            }
        }
    }
    paths
}

type Slope<'a> = dyn FnMut(f64, f64) -> Result<Option<f64>, String> + 'a;
fn rk4(p: Point2, h: f64, slope: &mut Slope<'_>) -> Result<Option<Point2>, String> {
    let Some(a) = slope(p.x, p.y)? else {
        return Ok(None);
    };
    let Some(b) = slope(p.x + h / 2., p.y + h * a / 2.)? else {
        return Ok(None);
    };
    let Some(c) = slope(p.x + h / 2., p.y + h * b / 2.)? else {
        return Ok(None);
    };
    let Some(d) = slope(p.x + h, p.y + h * c)? else {
        return Ok(None);
    };
    let y = p.y + h * (a + 2. * b + 2. * c + d) / 6.;
    Ok(y.is_finite().then_some(Point2 { x: p.x + h, y }))
}
fn integrate(start: Point2, end: f64, slope: &mut Slope<'_>) -> Result<Vec<Point2>, String> {
    let mut points = vec![start];
    let mut p = start;
    let max_step = (end - start.x) / 64.;
    let mut h = max_step;
    for _ in 0..4096 {
        if p.x == end || p.x + h == p.x {
            break;
        }
        if h.abs() > (end - p.x).abs() {
            h = end - p.x;
        }
        let improved = match rk4(p, h / 2., slope)? {
            Some(half) => rk4(half, h / 2., slope)?,
            None => None,
        };
        if let (Some(full), Some(mut next)) = (rk4(p, h, slope)?, improved) {
            let error = (next.y - full.y) / 15.;
            if error.abs() <= 1e-9 * (1. + next.y.abs()) {
                next.y += error;
                next.x = p.x + h;
                points.push(next);
                p = next;
                h = (2. * h.abs()).min(max_step.abs()).copysign(h);
                continue;
            }
        }
        h /= 2.;
    }
    Ok(points)
}
