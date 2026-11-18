//! Euclidean primitives shared by construction and measurement evaluation.
use super::{Extent, Shape, number, point};
use crate::ide::native_app_models::{Geometry, Point2};
pub fn delta(a: Point2, b: Point2) -> Point2 {
    Point2 {
        x: b.x - a.x,
        y: b.y - a.y,
    }
}
pub fn midpoint(a: Point2, b: Point2) -> Result<Point2, String> {
    point(Point2 {
        x: a.x / 2. + b.x / 2.,
        y: a.y / 2. + b.y / 2.,
    })
}
pub fn line(a: Point2, b: Point2, extent: Extent) -> Result<Shape, String> {
    point(a)?;
    point(b)?;
    let d = delta(a, b);
    if d.x == 0. && d.y == 0. {
        return Err("A line needs two distinct points".into());
    }
    point(d)?;
    Ok(Shape::Line { a, b, extent })
}
pub fn intersection(a: Point2, b: Point2, c: Point2, d: Point2) -> Result<Point2, String> {
    Geometry::line_intersection(a, b, c, d)
}
pub fn circle(center: Point2, radius: f64) -> Result<Shape, String> {
    point(center)?;
    number(radius)?;
    if radius <= 0. {
        return Err("A circle needs a positive radius".into());
    }
    Ok(Shape::Circle { center, radius })
}
pub fn circumcircle(a: Point2, b: Point2, c: Point2) -> Result<Shape, String> {
    let u = delta(a, b);
    let v = delta(a, c);
    let det = 2. * (u.x * v.y - u.y * v.x);
    if det.abs() <= 32. * f64::EPSILON * u.x.hypot(u.y) * v.x.hypot(v.y) {
        return Err("A circumcircle needs three non-collinear points".into());
    }
    let uu = u.x * u.x + u.y * u.y;
    let vv = v.x * v.x + v.y * v.y;
    let center = point(Point2 {
        x: a.x + (v.y * uu - u.y * vv) / det,
        y: a.y + (u.x * vv - v.x * uu) / det,
    })?;
    circle(center, center.distance(a)?)
}
pub fn project(p: Point2, a: Point2, b: Point2) -> Result<Point2, String> {
    let d = delta(a, b);
    let length = d.x.hypot(d.y);
    if length == 0. {
        return Err("A projection needs a valid line".into());
    }
    let u = Point2 {
        x: d.x / length,
        y: d.y / length,
    };
    let q = delta(a, p);
    let t = q.x * u.x + q.y * u.y;
    point(Point2 {
        x: a.x + t * u.x,
        y: a.y + t * u.y,
    })
}
pub fn reflect(p: Point2, a: Point2, b: Point2) -> Result<Point2, String> {
    let m = project(p, a, b)?;
    point(Point2 {
        x: 2. * m.x - p.x,
        y: 2. * m.y - p.y,
    })
}
pub fn polygon(points: Vec<Point2>) -> Result<Shape, String> {
    if points.len() < 3 {
        return Err("A polygon needs at least three vertices".into());
    }
    for p in &points {
        point(*p)?;
    }
    if Geometry::polygon_area(&points)? == 0. {
        return Err("A polygon needs non-collinear vertices".into());
    }
    Ok(Shape::Polygon(points))
}
pub fn angle(a: Point2, b: Point2, c: Point2) -> Result<f64, String> {
    let u = point(delta(a, b))?;
    let v = point(delta(a, c))?;
    let ul = number(u.x.hypot(u.y))?;
    let vl = number(v.x.hypot(v.y))?;
    if ul == 0. || vl == 0. {
        return Err("An angle needs distinct defining points".into());
    }
    let (u, v) = (
        Point2 {
            x: u.x / ul,
            y: u.y / ul,
        },
        Point2 {
            x: v.x / vl,
            y: v.y / vl,
        },
    );
    number((u.x * v.y - u.y * v.x).atan2(u.x * v.x + u.y * v.y))
}
pub fn closest(p: Point2, a: Point2, b: Point2, extent: Extent) -> Result<Point2, String> {
    let t = projection_parameter(p, a, b)?;
    line_element(a, b, extent, t)
}
fn projection_parameter(p: Point2, a: Point2, b: Point2) -> Result<f64, String> {
    let q = project(p, a, b)?;
    let d = delta(a, b);
    Ok(if d.x.abs() >= d.y.abs() {
        (q.x - a.x) / d.x
    } else {
        (q.y - a.y) / d.y
    })
}
fn constrain_parameter(t: f64, extent: Extent) -> f64 {
    match extent {
        Extent::Segment => t.clamp(0., 1.),
        Extent::Ray => t.max(0.),
        Extent::Line => t,
    }
}
fn line_element(a: Point2, b: Point2, extent: Extent, t: f64) -> Result<Point2, String> {
    let t = constrain_parameter(t, extent);
    let d = delta(a, b);
    point(Point2 {
        x: a.x + t * d.x,
        y: a.y + t * d.y,
    })
}
pub(super) fn circle_element(center: Point2, radius: f64, angle: f64) -> Point2 {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let (sin, cos) = match angle.rem_euclid(TAU) {
        0. => (0., 1.),
        FRAC_PI_2 => (1., 0.),
        PI => (0., -1.),
        t if t == 3. * FRAC_PI_2 => (-1., 0.),
        _ => angle.sin_cos(),
    };
    Point2 {
        x: center.x + radius * cos,
        y: center.y + radius * sin,
    }
}
/// The real parameter uses the defining points on a straight object and absolute
/// radians on a circle or arc. Only segments and rays restrict their parameters.
pub fn element(curve: &Shape, t: f64) -> Result<Point2, String> {
    number(t)?;
    match *curve {
        Shape::Line { a, b, extent } => line_element(a, b, extent, t),
        Shape::Circle { center, radius } => point(circle_element(center, radius, t)),
        Shape::Arc(arc) => point(circle_element(arc.center, arc.radius, t)),
        Shape::Ellipse(ref ellipse) => ellipse.element(t),
        Shape::Point(p) => point(p),
        _ => Err("Point On requires a line, ray, segment, circle or ellipse".into()),
    }
}
pub(super) fn tangent_at(curve: &Shape, p: Point2) -> Result<Shape, String> {
    let (center, radius) = match *curve {
        Shape::Line { .. } => return Ok(curve.clone()),
        Shape::Circle { center, radius } => (center, radius),
        _ => return Err("Tangent requires a line, ray, segment, or circle".into()),
    };
    let radial = delta(center, p);
    let distance = number(radial.x.hypot(radial.y))?;
    if distance == 0. {
        return Err("The tangent direction has no representable radius".into());
    }
    let step = radius.max(p.x.abs().max(p.y.abs()) * 4. * f64::EPSILON);
    let b = Point2 {
        x: p.x - step * (radial.y / distance),
        y: p.y + step * (radial.x / distance),
    };
    line(p, b, Extent::Line)
}
pub(super) fn tangent(curve: Shape, location: Shape) -> Result<Shape, String> {
    let p = match location {
        Shape::Point(p) => p,
        Shape::Number { re, im: 0. } => return tangent_at(&curve, element(&curve, re)?),
        Shape::Number { re, im } => Point2 { x: re, y: im },
        _ => return Err("Select a point or a real parameter for the tangent".into()),
    };
    let (center, radius) = match curve {
        Shape::Line { .. } => return Ok(curve),
        Shape::Circle { center, radius } => (center, radius),
        _ => return Err("Tangent requires a line, ray, segment, or circle".into()),
    };
    let radial = delta(center, p);
    let distance = number(radial.x.hypot(radial.y))?;
    let tolerance = 32. * f64::EPSILON * radius.max(distance);
    if distance == 0. || distance < radius - tolerance {
        return Ok(Shape::Lines(vec![]));
    }
    if (distance - radius).abs() <= tolerance {
        return tangent_at(&curve, p);
    }

    // The contact radius has projection r/d onto the outward unit radius.
    // Its other component gives the two contacts in firmware list order.
    let u = Point2 {
        x: radial.x / distance,
        y: radial.y / distance,
    };
    let q = radius / distance;
    let h = ((1. - q) * (1. + q)).sqrt();
    let contact = |side: f64| Point2 {
        x: center.x + radius * (q * u.x - side * h * u.y),
        y: center.y + radius * (q * u.y + side * h * u.x),
    };
    let mut lines = Vec::with_capacity(2);
    for side in [-1., 1.] {
        let contact = contact(side);
        line(p, contact, Extent::Line)?;
        lines.push([p, contact]);
    }
    Ok(Shape::Lines(lines))
}
pub fn element_parameter(curve: &Shape, cursor: Point2) -> Result<f64, String> {
    match *curve {
        Shape::Line { a, b, extent } => number(constrain_parameter(
            projection_parameter(cursor, a, b)?,
            extent,
        )),
        Shape::Circle { center, .. } | Shape::Arc(super::CircularArc { center, .. }) => {
            let d = point(delta(center, cursor))?;
            if d.x == 0. && d.y == 0. {
                return Err("Move the cursor away from the center to place the point".into());
            }
            Ok(d.y.atan2(d.x))
        }
        Shape::Function { ref paths, .. } => function_projection(paths, cursor).map(|p| p.x),
        Shape::Ellipse(ref ellipse) => ellipse.parameter(cursor),
        _ => {
            Err("Point On requires a line, ray, segment, circle, ellipse or function graph".into())
        }
    }
}
pub(super) fn curve_projection(curve: &Shape, cursor: Point2) -> Result<Point2, String> {
    if let Shape::Function { paths, .. } = curve {
        return function_projection(paths, cursor);
    }
    element(curve, element_parameter(curve, cursor)?)
}
fn function_projection(paths: &[Vec<Point2>], cursor: Point2) -> Result<Point2, String> {
    let distance = |p: &Point2| (p.x - cursor.x).hypot(p.y - cursor.y);
    paths
        .iter()
        .flat_map(|path| path.windows(2))
        .filter_map(|pair| closest(cursor, pair[0], pair[1], Extent::Segment).ok())
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .ok_or_else(|| "The function has no visible path for Point On".into())
}
fn within(p: Point2, a: Point2, b: Point2, extent: Extent) -> bool {
    if extent == Extent::Line {
        return true;
    }
    let d = delta(a, b);
    let t = if d.x.abs() >= d.y.abs() {
        (p.x - a.x) / d.x
    } else {
        (p.y - a.y) / d.y
    };
    t >= -1e-10 && (extent == Extent::Ray || t <= 1. + 1e-10)
}
pub fn intersections(a: &Shape, b: &Shape) -> Result<Vec<Point2>, String> {
    match (a, b) {
        (
            Shape::Line { a, b, extent: ae },
            Shape::Line {
                a: c,
                b: d,
                extent: be,
            },
        ) => {
            let p = intersection(*a, *b, *c, *d)?;
            Ok(if within(p, *a, *b, *ae) && within(p, *c, *d, *be) {
                vec![p]
            } else {
                vec![]
            })
        }
        (Shape::Line { a, b, extent }, Shape::Circle { center, radius })
        | (Shape::Circle { center, radius }, Shape::Line { a, b, extent }) => {
            line_circle_intersections(*a, *b, *extent, *center, *radius)
        }
        (Shape::Line { a, b, extent }, Shape::Arc(arc))
        | (Shape::Arc(arc), Shape::Line { a, b, extent }) => {
            let mut points = line_circle_intersections(*a, *b, *extent, arc.center, arc.radius)?;
            points.retain(|p| arc.contains_intersection(*p));
            Ok(order_curve_intersections(points, *a, *b))
        }
        (Shape::Line { a, b, extent }, Shape::Ellipse(ellipse))
        | (Shape::Ellipse(ellipse), Shape::Line { a, b, extent }) => {
            let points = ellipse.line_intersections(*a, *b, *extent)?;
            Ok(order_curve_intersections(points, *a, *b))
        }
        (
            Shape::Circle {
                center: a,
                radius: r,
            },
            Shape::Circle {
                center: b,
                radius: s,
            },
        ) => {
            let d = a.distance(*b)?;
            let tol = 64. * f64::EPSILON * r.max(*s).max(d).max(1.);
            if d <= tol {
                return if (r - s).abs() <= tol {
                    Err("Coincident circles have no unique intersections".into())
                } else {
                    Ok(vec![])
                };
            }
            if d > r + s + tol || d < (r - s).abs() - tol {
                return Ok(vec![]);
            }
            let along = (r * r - s * s + d * d) / (2. * d);
            let h = (r * r - along * along).max(0.).sqrt();
            let u = Point2 {
                x: (b.x - a.x) / d,
                y: (b.y - a.y) / d,
            };
            let m = Point2 {
                x: a.x + along * u.x,
                y: a.y + along * u.y,
            };
            let p = point(Point2 {
                x: m.x - h * u.y,
                y: m.y + h * u.x,
            })?;
            if h <= tol {
                Ok(vec![p])
            } else {
                Ok(vec![
                    p,
                    point(Point2 {
                        x: m.x + h * u.y,
                        y: m.y - h * u.x,
                    })?,
                ])
            }
        }
        _ => Err(
            "Select straight objects and circles, or a straight object with an arc or ellipse"
                .into(),
        ),
    }
}

pub(super) fn line_circle_intersections(
    a: Point2,
    b: Point2,
    extent: Extent,
    center: Point2,
    radius: f64,
) -> Result<Vec<Point2>, String> {
    let foot = project(center, a, b)?;
    let distance = foot.distance(center)?;
    let tol = 64. * f64::EPSILON * radius.max(distance).max(1.);
    if distance > radius + tol {
        return Ok(vec![]);
    }
    let half = ((radius - distance).max(0.) * (radius + distance)).sqrt();
    let d = delta(a, b);
    let len = d.x.hypot(d.y);
    let u = Point2 {
        x: d.x / len,
        y: d.y / len,
    };
    let p = point(Point2 {
        x: foot.x - half * u.x,
        y: foot.y - half * u.y,
    })?;
    let q = point(Point2 {
        x: foot.x + half * u.x,
        y: foot.y + half * u.y,
    })?;
    let mut result = Vec::new();
    if within(p, a, b, extent) {
        result.push(p);
    }
    if half > tol && within(q, a, b, extent) {
        result.push(q);
    }
    Ok(result)
}

fn order_curve_intersections(mut points: Vec<Point2>, a: Point2, b: Point2) -> Vec<Point2> {
    let coordinate = |p: &Point2| if a.x == b.x { p.y } else { p.x };
    points.sort_by(|p, q| coordinate(q).total_cmp(&coordinate(p)));
    points
}
