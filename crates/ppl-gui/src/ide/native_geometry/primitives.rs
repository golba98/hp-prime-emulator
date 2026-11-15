//! Euclidean primitives shared by construction and measurement evaluation.
use super::{Extent, Shape, number, point};
use crate::ide::native_app_models::{Geometry, Point2};
/// Map a similarity transform over every supported construction.  Revalidate
/// each result so collapsed lines/circles and non-finite coordinates never
/// enter the dependency cache or saved geometry model.
pub fn transform(
    shape: Shape,
    radius_scale: f64,
    mut map: impl FnMut(Point2) -> Result<Point2, String>,
) -> Result<Shape, String> {
    match shape {
        Shape::Point(p) => Ok(Shape::Point(point(map(p)?)?)),
        Shape::Number { re, im } => Ok(Shape::Point(point(map(Point2 { x: re, y: im })?)?)),
        Shape::Line { a, b, extent } => line(map(a)?, map(b)?, extent),
        Shape::Circle { center, radius } => circle(map(center)?, radius * radius_scale.abs()),
        Shape::Polygon(points) => {
            polygon(points.into_iter().map(&mut map).collect::<Result<_, _>>()?)
        }
        Shape::Points(points) => Ok(Shape::Points(
            points
                .into_iter()
                .map(|p| point(map(p)?))
                .collect::<Result<_, _>>()?,
        )),
    }
}
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
    let q = project(p, a, b)?;
    let d = delta(a, b);
    let t = if d.x.abs() >= d.y.abs() {
        (q.x - a.x) / d.x
    } else {
        (q.y - a.y) / d.y
    };
    let t = match extent {
        Extent::Segment => t.clamp(0., 1.),
        Extent::Ray => t.max(0.),
        Extent::Line => t,
    };
    point(Point2 {
        x: a.x + t * d.x,
        y: a.y + t * d.y,
    })
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
            let foot = project(*center, *a, *b)?;
            let distance = foot.distance(*center)?;
            let tol = 64. * f64::EPSILON * radius.max(distance).max(1.);
            if distance > *radius + tol {
                return Ok(vec![]);
            }
            let half = ((radius - distance).max(0.) * (radius + distance)).sqrt();
            let d = delta(*a, *b);
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
            if within(p, *a, *b, *extent) {
                result.push(p);
            }
            if half > tol && within(q, *a, *b, *extent) {
                result.push(q);
            }
            Ok(result)
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
        _ => Err("Intersections currently require lines, rays, segments or circles".into()),
    }
}
