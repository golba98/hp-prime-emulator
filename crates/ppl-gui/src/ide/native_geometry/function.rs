//! Evaluated function graphs use the same compiled numeric engine as Function.
use super::{Extent, Point2, Shape, number, point, primitives, source};
use crate::ide::native_apps::Evaluator;
use ppl_syntax::ast::{BinOp, Expr, ExprKind, UnOp};
use ppl_vm::SessionSnapshot;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) fn reads_x(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Ident(name) => name == "x",
        ExprKind::Binary(_, a, b)
        | ExprKind::ComplexPair(a, b)
        | ExprKind::Assign(a, b)
        | ExprKind::Store(a, b) => reads_x(a) || reads_x(b),
        ExprKind::Unary(_, arg) | ExprKind::Postfix(_, arg) | ExprKind::Quoted(arg) => reads_x(arg),
        ExprKind::Call(base, args) | ExprKind::Index(base, args) => {
            reads_x(base) || args.iter().any(reads_x)
        }
        ExprKind::List(args) | ExprKind::Bracket(args) => args.iter().any(reads_x),
        _ => false,
    }
}

// The original evaluates affine plotfunc definitions to segments. Only
// simplify expressions whose syntax establishes that property.
fn degree(expr: &Expr) -> Option<u8> {
    if !reads_x(expr) {
        return Some(0);
    }
    match &expr.kind {
        ExprKind::Ident(name) if name == "x" => Some(1),
        ExprKind::Unary(UnOp::Neg, arg) => degree(arg),
        ExprKind::Binary(BinOp::Add | BinOp::Sub, a, b) => Some(degree(a)?.max(degree(b)?)),
        ExprKind::Binary(BinOp::Mul, a, b) => degree(a)?.checked_add(degree(b)?),
        ExprKind::Binary(BinOp::Div, a, b) if degree(b)? == 0 => degree(a),
        _ => None,
    }
}

pub(super) fn build(
    expression: String,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
    sample: bool,
) -> Result<Shape, String> {
    source(&format!("plotfunc({expression})"))?;
    let (expr, diagnostics) = ppl_syntax::parse_expression(&expression);
    if let Some(error) = diagnostics.errors().next() {
        return Err(error.message.clone());
    }
    let settings = snapshot.geometry_plot_settings();
    let [xmin, xmax, ymin, ymax] = settings.derived_bounds().map(|n| n.to_f64());
    let extent = number(settings.pix_size.to_f64() * 320.)?;
    if degree(&expr).is_some_and(|n| n <= 1) {
        let mut evaluator = Evaluator::geometry_function(&expression, snapshot, cancel)?;
        let x0 = number(xmin - extent / 4.)?;
        let x1 = number(xmin + extent * 1.25)?;
        return primitives::line(
            at(&mut evaluator, x0)?,
            at(&mut evaluator, x1)?,
            Extent::Segment,
        );
    }
    if !sample {
        return Ok(Shape::Function {
            expression,
            paths: vec![],
        });
    }

    let mut evaluator = Evaluator::geometry_function(&expression, snapshot, cancel.clone())?;
    let span = number(xmax - xmin)?;
    number(ymax - ymin)?;
    let mut paths = vec![];
    let mut run = Vec::<Point2>::new();
    for index in 0..=640 {
        if cancel.load(Ordering::Acquire) {
            return Err("Geometry plot interrupted".into());
        }
        let x = xmin + span * f64::from(index) / 640.;
        let next = evaluator.at(x)?.map(|y| Point2 { x, y });
        // Separate undefined samples, and avoid a false segment across a pole
        // when adjacent values are on opposite sides of the whole viewport.
        let split = match (run.last(), next) {
            (Some(a), Some(b)) if (a.y < ymin && b.y > ymax) || (a.y > ymax && b.y < ymin) => {
                let midpoint = evaluator.at(a.x / 2. + b.x / 2.)?;
                !midpoint.is_some_and(|y| y >= ymin && y <= ymax)
            }
            (_, None) => true,
            _ => false,
        };
        if split && !run.is_empty() {
            paths.push(std::mem::take(&mut run));
        }
        if let Some(next) = next {
            run.push(next);
        }
    }
    if !run.is_empty() {
        paths.push(run);
    }
    Ok(Shape::Function { expression, paths })
}
fn at(evaluator: &mut Evaluator, x: f64) -> Result<Point2, String> {
    let y = evaluator
        .at(x)?
        .ok_or("The function is undefined at this parameter")?;
    point(Point2 { x, y })
}
pub(super) fn element(
    expression: &str,
    x: f64,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Point2, String> {
    at(
        &mut Evaluator::geometry_function(expression, snapshot, cancel)?,
        x,
    )
}

/// Use the same evaluator and angle settings as the plotted curve. Successive
/// centered differences remove their leading curvature error by extrapolation.
pub(super) fn tangent(
    expression: &str,
    x: f64,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Shape, String> {
    let mut evaluator = Evaluator::geometry_function(expression, snapshot, cancel)?;
    let p = at(&mut evaluator, x)?;
    let mut step = x.abs().max(1.) / 32.;
    let mut previous: Option<(f64, f64)> = None;
    let mut extrapolated: Option<f64> = None;
    for _ in 0..16 {
        let left = at(&mut evaluator, x - step)?;
        let right = at(&mut evaluator, x + step)?;
        let backward = (p.y - left.y) / step;
        let forward = (right.y - p.y) / step;
        let slope = (backward + forward) / 2.;
        if let Some((old_backward, old_forward)) = previous {
            let coarse = (old_backward + old_forward) / 2.;
            let refined: f64 = slope + (slope - coarse) / 3.;
            let scale = refined.abs().max(1.);
            if extrapolated.is_some_and(|old| (refined - old).abs() <= 1e-8 * scale)
                && ((2. * forward - old_forward) - (2. * backward - old_backward)).abs()
                    <= 1e-7 * scale
            {
                let refined = ppl_math::Real::from_f64(refined)
                    .map_err(|error| error.to_string())?
                    .to_f64();
                let dx = x.abs().max(1.);
                return primitives::line(
                    p,
                    Point2 {
                        x: x + dx,
                        y: p.y + dx * refined,
                    },
                    Extent::Line,
                );
            }
            extrapolated = Some(refined);
        }
        previous = Some((backward, forward));
        step /= 2.;
    }
    Err("No finite tangent could be resolved at this point".into())
}
