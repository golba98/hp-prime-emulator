//! Evaluated function graphs use the same compiled numeric engine as Function.
use super::{Extent, Point2, Shape, number, point, primitives, source};
use crate::ide::native_apps::Evaluator;
use ppl_syntax::ast::{BinOp, Expr, ExprKind, PostOp, UnOp};
use ppl_vm::SessionSnapshot;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) fn reads_variable(expr: &Expr, variable: &str) -> bool {
    match &expr.kind {
        ExprKind::Ident(name) => name == variable,
        ExprKind::Binary(_, a, b)
        | ExprKind::ComplexPair(a, b)
        | ExprKind::Assign(a, b)
        | ExprKind::Store(a, b) => reads_variable(a, variable) || reads_variable(b, variable),
        ExprKind::Unary(_, arg) | ExprKind::Postfix(_, arg) | ExprKind::Quoted(arg) => {
            reads_variable(arg, variable)
        }
        ExprKind::Call(base, args) | ExprKind::Index(base, args) => {
            reads_variable(base, variable) || args.iter().any(|a| reads_variable(a, variable))
        }
        ExprKind::List(args) | ExprKind::Bracket(args) => {
            args.iter().any(|a| reads_variable(a, variable))
        }
        _ => false,
    }
}

// The original evaluates affine plotfunc definitions to segments. Only
// simplify expressions whose syntax establishes that property.
pub(super) fn degree_in(expr: &Expr, variable: &str) -> Option<u8> {
    if !reads_variable(expr, variable) {
        return Some(0);
    }
    match &expr.kind {
        ExprKind::Ident(name) if name == variable => Some(1),
        ExprKind::Unary(UnOp::Neg, arg) => degree_in(arg, variable),
        ExprKind::Binary(BinOp::Add | BinOp::Sub, a, b) => {
            Some(degree_in(a, variable)?.max(degree_in(b, variable)?))
        }
        ExprKind::Binary(BinOp::Mul, a, b) => {
            degree_in(a, variable)?.checked_add(degree_in(b, variable)?)
        }
        ExprKind::Binary(BinOp::Div, a, b) if degree_in(b, variable)? == 0 => {
            degree_in(a, variable)
        }
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
    if degree_in(&expr, "x").is_some_and(|n| n <= 1) {
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

pub(super) fn tangent_at(
    expression: &str,
    p: Point2,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Shape, String> {
    let (expr, diagnostics) = ppl_syntax::parse_expression(expression);
    if let Some(error) = diagnostics.errors().next() {
        return Err(error.message.clone());
    }
    let constant = |expr: &Expr| {
        let source = expression
            .get(expr.span.start as usize..expr.span.end as usize)
            .ok_or("Invalid Function Graph constant")?;
        Ok::<_, String>(element(source, p.x, snapshot, cancel.clone())?.y)
    };
    let (_, slope) = algebraic_value_and_slope(&expr, p.x, &constant)?;
    let length = slope.hypot(1.);
    let step = 1_f64.max(p.x.abs().max(p.y.abs()) * 4. * f64::EPSILON);
    primitives::line(
        p,
        Point2 {
            x: p.x + step / length,
            y: p.y + step * (slope / length),
        },
        Extent::Line,
    )
}

fn value_and_slope_of_power(base: f64, slope: f64, exponent: f64) -> (f64, f64) {
    if exponent == 0. {
        (1., 0.)
    } else {
        (
            base.powf(exponent),
            exponent * base.powf(exponent - 1.) * slope,
        )
    }
}

// Evaluate the derivative along with the function value. Graph samples are
// rounded calculator results and do not define the tangent direction.
fn algebraic_value_and_slope(
    expr: &Expr,
    x: f64,
    constant: &impl Fn(&Expr) -> Result<f64, String>,
) -> Result<(f64, f64), String> {
    if !reads_variable(expr, "x") {
        return Ok((constant(expr)?, 0.));
    }
    let unsupported = || "Tangent needs a polynomial or rational Function Graph".to_owned();
    let (value, slope) = match &expr.kind {
        ExprKind::Ident(name) if name == "x" => (x, 1.),
        ExprKind::Unary(UnOp::Neg, arg) => {
            let (value, slope) = algebraic_value_and_slope(arg, x, constant)?;
            (-value, -slope)
        }
        ExprKind::Postfix(operation @ (PostOp::Square | PostOp::Cube), arg) => {
            let (value, slope) = algebraic_value_and_slope(arg, x, constant)?;
            let exponent = if *operation == PostOp::Square { 2. } else { 3. };
            value_and_slope_of_power(value, slope, exponent)
        }
        ExprKind::Binary(operation, left, right) => {
            let (u, du) = algebraic_value_and_slope(left, x, constant)?;
            let (v, dv) = algebraic_value_and_slope(right, x, constant)?;
            match operation {
                BinOp::Add => (u + v, du + dv),
                BinOp::Sub => (u - v, du - dv),
                BinOp::Mul => (u * v, du * v + u * dv),
                BinOp::Div => {
                    let value = u / v;
                    (value, (du - value * dv) / v)
                }
                BinOp::Pow if !reads_variable(right, "x") && v.fract() == 0. => {
                    value_and_slope_of_power(u, du, v)
                }
                _ => return Err(unsupported()),
            }
        }
        _ => return Err(unsupported()),
    };
    Ok((number(value)?, number(slope)?))
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
