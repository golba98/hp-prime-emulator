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
    PointMirror(Point2),
    Mirror {
        a: Point2,
        b: Point2,
    },
}

impl Transform {
    fn point(&self, p: Point2) -> Result<Point2, String> {
        match *self {
            Self::PointMirror(c) => point(Point2 {
                x: c.x + (c.x - p.x),
                y: c.y + (c.y - p.y),
            }),
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
            Shape::Polyline(vertices) => Shape::Polyline(
                vertices
                    .into_iter()
                    .map(|p| self.point(p))
                    .collect::<Result<_, _>>()?,
            ),
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
            Shape::Objects(objects) => Shape::Objects(
                objects
                    .into_iter()
                    .map(|s| self.apply(s))
                    .collect::<Result<_, _>>()?,
            ),
            Shape::Lines(lines) => Shape::Lines(
                lines
                    .into_iter()
                    .map(|[a, b]| Ok([self.point(a)?, self.point(b)?]))
                    .collect::<Result<_, String>>()?,
            ),
            Shape::Parabola(curve) => {
                let [f, a, b] = curve.definition();
                let mut transformed =
                    super::Parabola::from_focus(self.point(f)?, self.point(a)?, self.point(b)?)?;
                transformed.limit = curve.limit * scale;
                transformed.offset = curve.offset * scale;
                transformed.from_equation = curve.from_equation;
                Shape::Parabola(transformed)
            }
            Shape::Hyperbola(curve) => {
                let [a, b] = curve.foci;
                let size = match curve.size {
                    Size::Witness(w) => Size::Witness(self.point(w)?),
                    Size::SemiMajor(r) => Size::SemiMajor(r * scale),
                };
                let mut transformed =
                    super::Hyperbola::new([self.point(a)?, self.point(b)?], size)?;
                transformed.equation = curve.equation;
                Shape::Hyperbola(transformed)
            }
            Shape::Ellipse(ellipse)
                if matches!(self, Self::Mirror { .. } | Self::PointMirror(_)) =>
            {
                Shape::Ellipse(
                    ellipse.reflected(|p| self.point(p), matches!(self, Self::Mirror { .. }))?,
                )
            }
            Shape::Ellipse(ellipse) => {
                let [a, b] = ellipse.foci;
                let size = match ellipse.size {
                    Size::Witness(w) => Size::Witness(self.point(w)?),
                    Size::SemiMajor(r) => Size::SemiMajor(r * scale),
                };
                let mut transformed = Ellipse::new([self.point(a)?, self.point(b)?], size)?;
                transformed.equation_phase = ellipse.equation_phase;
                Shape::Ellipse(transformed)
            }
            Shape::Arc(arc) => {
                // G1 preserves the angles for a point mirror and negates
                // both for a line mirror, including oblique lines.
                let (start, end) = match *self {
                    Self::Translate(_) | Self::PointMirror(_) => (arc.start, arc.end),
                    Self::About { scale, radians, .. } => {
                        let angle = if scale < 0. {
                            radians + std::f64::consts::PI
                        } else {
                            radians
                        };
                        (arc.start + angle, arc.end + angle)
                    }
                    Self::Mirror { .. } => (-arc.start, -arc.end),
                };
                Shape::Arc(CircularArc::new(
                    self.point(arc.center)?,
                    arc.radius * scale,
                    start,
                    end,
                )?)
            }
            Shape::Undefined => Shape::Undefined,
            Shape::Parameter(p) => Shape::Point(self.point(Point2 { x: p.value, y: 0. })?),
            Shape::EmptyConic(_) => shape,
            Shape::Sequence(_) | Shape::Parametric(_) | Shape::Function { .. } => {
                return Err("Transform requires a point, straight object, polygon or conic".into());
            }
        })
    }
}
