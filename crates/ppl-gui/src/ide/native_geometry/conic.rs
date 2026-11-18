//! Reduce a quadratic in x and y to the geometric primitives used by Plot.
use super::{
    Ellipse, Extent, Hyperbola, Parabola, Point2, Shape, ellipse::Size, point, primitives,
};
use ppl_syntax::ast::{BinOp, Expr, ExprKind, PostOp, UnOp};

const POWERS: [(usize, usize); 6] = [(0, 0), (1, 0), (0, 1), (2, 0), (1, 1), (0, 2)];
type Terms = [f64; 6];

fn product(a: Terms, b: Terms) -> Result<Terms, String> {
    let mut result = [0.; 6];
    for ((x, y), v) in POWERS.into_iter().zip(a).filter(|(_, v)| *v != 0.) {
        for ((u, w), k) in POWERS.into_iter().zip(b).filter(|(_, k)| *k != 0.) {
            let i = POWERS
                .iter()
                .position(|&p| p == (x + u, y + w))
                .ok_or("A conic must have degree at most two")?;
            result[i] += v * k;
        }
    }
    Ok(result)
}

fn contains_coordinate(e: &Expr) -> bool {
    use ExprKind as E;
    match &e.kind {
        E::Ident(n) => matches!(n.as_str(), "x" | "y"),
        E::Unary(_, a) | E::Quoted(a) | E::Postfix(_, a) => contains_coordinate(a),
        E::Binary(_, a, b) | E::ComplexPair(a, b) | E::Assign(a, b) | E::Store(a, b) => {
            contains_coordinate(a) || contains_coordinate(b)
        }
        E::Index(a, args) | E::Call(a, args) => {
            contains_coordinate(a) || args.iter().any(contains_coordinate)
        }
        E::List(args) | E::Bracket(args) => args.iter().any(contains_coordinate),
        _ => false,
    }
}
fn terms(e: &Expr, real: &mut impl FnMut(&Expr) -> Result<f64, String>) -> Result<Terms, String> {
    use ExprKind as E;
    match &e.kind {
        E::Ident(n) if n == "x" => Ok([0., 1., 0., 0., 0., 0.]),
        E::Ident(n) if n == "y" => Ok([0., 0., 1., 0., 0., 0.]),
        E::Quoted(a) => terms(a, real),
        E::Unary(UnOp::Neg, a) => Ok(terms(a, real)?.map(|n| -n)),
        E::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Eq), a, b) => {
            let (a, b) = (terms(a, real)?, terms(b, real)?);
            Ok(std::array::from_fn(|i| {
                if *op == BinOp::Add {
                    a[i] + b[i]
                } else {
                    a[i] - b[i]
                }
            }))
        }
        E::Binary(BinOp::Mul, a, b) => product(terms(a, real)?, terms(b, real)?),
        E::Binary(BinOp::Div, a, b) => {
            let n = terms(b, real)?;
            if n[0] == 0. || n[1..].iter().any(|v| *v != 0.) {
                return Err("A conic requires a nonzero constant divisor".into());
            }
            Ok(terms(a, real)?.map(|v| v / n[0]))
        }
        E::Binary(BinOp::Pow, a, b) => {
            let p = terms(a, real)?;
            if p[1..].iter().all(|v| *v == 0.) {
                return Ok([real(e)?, 0., 0., 0., 0., 0.]);
            }
            if contains_coordinate(b) {
                return Err("A conic requires a constant exponent".into());
            }
            match real(b)? {
                0. => Ok([1., 0., 0., 0., 0., 0.]),
                1. => Ok(p),
                2. => product(p, p),
                _ => Err("A conic must be quadratic".into()),
            }
        }
        E::Postfix(PostOp::Square, a) => {
            let p = terms(a, real)?;
            product(p, p)
        }
        _ if contains_coordinate(e) => Err("A conic requires a polynomial in x and y".into()),
        _ => Ok([real(e)?, 0., 0., 0., 0., 0.]),
    }
}

pub(super) fn build(
    expression: &Expr,
    source: &str,
    mut real: impl FnMut(&Expr) -> Result<f64, String>,
) -> Result<Shape, String> {
    let empty = || {
        Shape::EmptyConic(format!(
            "conic({})",
            &source[expression.span.start as usize..expression.span.end as usize]
        ))
    };
    let coefficients = terms(expression, &mut real)?;
    if coefficients.iter().any(|n| !n.is_finite()) {
        return Err("Conic coefficients must be finite".into());
    }
    let [constant, dx, dy, a, b, c] = coefficients;
    let scale = a.abs().max(b.abs()).max(c.abs());
    if scale == 0. {
        if dy != 0. {
            return primitives::line(
                point(Point2 {
                    x: -24.,
                    y: (24. * dx - constant) / dy,
                })?,
                point(Point2 {
                    x: 24.,
                    y: (-24. * dx - constant) / dy,
                })?,
                Extent::Segment,
            );
        }
        let length = dx.hypot(dy);
        if length == 0. {
            return Err("The equation does not define a curve".into());
        }
        let (nx, ny) = (dx / length, dy / length);
        let p = point(Point2 {
            x: -nx * (constant / length),
            y: -ny * (constant / length),
        })?;
        return primitives::line(
            p,
            Point2 {
                x: p.x - ny,
                y: p.y + nx,
            },
            Extent::Line,
        );
    }
    let [constant, dx, dy, a, b, c] = coefficients.map(|n| n / scale);
    // Rotate the symmetric quadratic form into its principal axes. Keep the
    // nonzero eigenvalue first for a rank-one form (parabola or parallel lines).
    let mut angle = 0.5 * b.atan2(a - c);
    let half = (a + c) * 0.5;
    let split = (a - c).hypot(b) * 0.5;
    let (mut l, mut m) = (half + split, half - split);
    if m.abs() > l.abs() {
        std::mem::swap(&mut l, &mut m);
        angle += std::f64::consts::FRAC_PI_2;
    }
    let (mut sin, mut cos) = angle.sin_cos();
    if b == 0. {
        sin = sin.round();
        cos = cos.round();
    }
    let world = |u: f64, v: f64| {
        point(Point2 {
            x: cos * u - sin * v,
            y: sin * u + cos * v,
        })
    };
    let e = dx * cos + dy * sin;
    let f = dy * cos - dx * sin;
    let u = -e / (2. * l);
    if m.abs() <= 16. * f64::EPSILON * l.abs() {
        let k = constant + 0.5 * e * u;
        if f.abs() <= 16. * f64::EPSILON * dx.hypot(dy) {
            let square = -k / l;
            if square < 0. {
                return Ok(empty());
            }
            let line = |s| Ok::<_, String>([world(s, 0.)?, world(s, 1.)?]);
            let [p, q] = line(u + square.sqrt())?;
            return if square == 0. {
                primitives::line(p, q, Extent::Line)
            } else {
                Ok(Shape::Lines(vec![[p, q], line(u - square.sqrt())?]))
            };
        }
        let v = -k / f;
        let focal = -f / (4. * l);
        let sign = if cos < 0. || (cos == 0. && sin < 0.) {
            -1.
        } else {
            1.
        };
        return Ok(Shape::Parabola(Parabola::equation(
            world(u, v)?,
            Point2 {
                x: cos * sign,
                y: sin * sign,
            },
            focal * sign,
        )?));
    }
    let v = -f / (2. * m);
    let k = -(constant + 0.5 * (e * u + f * v));
    let center = world(u, v)?;
    if k == 0. {
        if l * m > 0. {
            return if a == c && b == 0. {
                Ok(Shape::Circle { center, radius: 0. })
            } else {
                Ok(Shape::Point(center))
            };
        }
        let slope = (-l / m).sqrt();
        return Ok(Shape::Lines(vec![
            [center, world(u + 1., v + slope)?],
            [center, world(u + 1., v - slope)?],
        ]));
    }
    let (r, s) = (k / l, k / m);
    let along_u = r > s;
    let (major, minor) = if along_u { (r, s) } else { (s, r) };
    if major <= 0. {
        return Ok(empty());
    }
    if (major - minor).abs() <= 16. * f64::EPSILON * major {
        return primitives::circle(center, major.sqrt());
    }
    let distance = (major - minor).sqrt();
    let equation_angle = if b == 0. {
        0.
    } else {
        0.5 * (b / (a - c)).atan()
    };
    let major_angle = angle
        + if along_u {
            0.
        } else {
            std::f64::consts::FRAC_PI_2
        };
    let phase = equation_angle - major_angle;
    let focus = |d| {
        if along_u {
            world(u + d, v)
        } else {
            world(u, v + d)
        }
    };
    let foci = [focus(-distance)?, focus(distance)?];
    let size = Size::SemiMajor(major.sqrt());
    if minor > 0. {
        let mut e = Ellipse::new(foci, size)?;
        e.equation_phase = Some(phase);
        Ok(Shape::Ellipse(e))
    } else {
        let mut h = Hyperbola::new(foci, size)?;
        h.equation = true;
        Ok(Shape::Hyperbola(h))
    }
}
