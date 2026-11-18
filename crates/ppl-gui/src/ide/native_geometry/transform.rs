//! Euclidean similarities preserve straight objects, circles and ellipses.
use super::ellipse::{Ellipse, Size};
use super::primitives as g;
use super::{CircularArc, Point2, Shape, point};

pub(super) enum Transform {
    Translate(Point2),
    About {
        center: Point2,
        scale: f64,
        radians: f64,
    },
    Mirror {
        a: Point2,
        b: Point2,
    },
}

impl Transform {
    fn point(&self, p: Point2) -> Result<Point2, String> {
        match *self {
            Self::Translate(d) => point(Point2 {
                x: p.x + d.x,
                y: p.y + d.y,
            }),
            Self::About {
                center,
                scale,
                radians,
            } => {
                let d = g::delta(center, p);
                let (s, c) = radians.sin_cos();
                point(Point2 {
                    x: center.x + scale * (d.x * c - d.y * s),
                    y: center.y + scale * (d.x * s + d.y * c),
                })
            }
            Self::Mirror { a, b } => g::reflect(p, a, b),
        }
    }

    pub(super) fn apply(&self, shape: Shape) -> Result<Shape, String> {
        let scale = match self {
            Self::About { scale, .. } => scale.abs(),
            _ => 1.,
        };
        Ok(match shape {
            Shape::Point(p) => Shape::Point(self.point(p)?),
            Shape::Number { re, im } => Shape::Point(self.point(Point2 { x: re, y: im })?),
            Shape::Line { a, b, extent } => g::line(self.point(a)?, self.point(b)?, extent)?,
            Shape::Circle { center, radius } => g::circle(self.point(center)?, radius * scale)?,
            Shape::Polygon(vertices) => g::polygon(
                vertices
                    .into_iter()
                    .map(|p| self.point(p))
                    .collect::<Result<_, _>>()?,
            )?,
            Shape::Points(points) => Shape::Points(
                points
                    .into_iter()
                    .map(|p| self.point(p))
                    .collect::<Result<_, _>>()?,
            ),
            Shape::Lines(lines) => Shape::Lines(
                lines
                    .into_iter()
                    .map(|[a, b]| Ok([self.point(a)?, self.point(b)?]))
                    .collect::<Result<_, String>>()?,
            ),
            Shape::Ellipse(ellipse) => {
                let [a, b] = ellipse.foci;
                let size = match ellipse.size {
                    Size::Witness(w) => Size::Witness(self.point(w)?),
                    Size::SemiMajor(r) => Size::SemiMajor(r * scale),
                };
                Shape::Ellipse(Ellipse::new([self.point(a)?, self.point(b)?], size)?)
            }
            Shape::Arc(arc) => {
                // A reflection reverses the boundary orientation. Swap endpoints
                // so the renderer follows the reflected portion of the circle.
                let (start, end) = match *self {
                    Self::Translate(_) => (arc.start, arc.end),
                    Self::About { scale, radians, .. } => {
                        let angle = if scale < 0. {
                            radians + std::f64::consts::PI
                        } else {
                            radians
                        };
                        (arc.start + angle, arc.end + angle)
                    }
                    Self::Mirror { a, b } => {
                        let axis = g::delta(a, b);
                        let twice_angle = 2. * axis.y.atan2(axis.x);
                        (twice_angle - arc.end, twice_angle - arc.start)
                    }
                };
                Shape::Arc(CircularArc::new(
                    self.point(arc.center)?,
                    arc.radius * scale,
                    start,
                    end,
                )?)
            }
            Shape::Undefined => Shape::Undefined,
            Shape::Function { .. } => {
                return Err("Transform requires a point, straight object, polygon or conic".into());
            }
        })
    }
}
