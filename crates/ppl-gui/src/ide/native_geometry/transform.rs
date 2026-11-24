//! Euclidean similarities preserve straight objects, circles and ellipses.
use super::ellipse::Size;
use super::primitives as g;
use super::{CircularArc, Point2, Shape, point};

pub(super) enum Transform {
    Translate(Point2),
    About {
        center: Point2,
        scale: f64,
        radians: f64,
        preserve_arc_bounds: bool,
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
                ..
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
        if let Self::About { center, scale, .. } = *self
            && scale == 0.
        {
            return match shape {
                Shape::Point(_)
                | Shape::Number { .. }
                | Shape::Circle { .. }
                | Shape::Ellipse(_)
                | Shape::Parameter(_) => Ok(Shape::Point(center)),
                Shape::Polygon(vertices) => Ok(Shape::Polygon(vec![center; vertices.len()])),
                Shape::Arc(_) => Err("A circular arc cannot be scaled by zero".into()),
                Shape::Line { a, b, extent } => g::line(self.point(a)?, self.point(b)?, extent),
                other => Self::About {
                    center,
                    scale,
                    radians: 0.,
                    preserve_arc_bounds: false,
                }
                .apply_nonzero(other),
            };
        }
        self.apply_nonzero(shape)
    }

    fn apply_nonzero(&self, shape: Shape) -> Result<Shape, String> {
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
            Shape::Polygon(vertices) => Shape::Polygon(
                vertices
                    .into_iter()
                    .map(|p| self.point(p))
                    .collect::<Result<_, _>>()?,
            ),
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
            Shape::Ellipse(ellipse) => Shape::Ellipse(match *self {
                Self::Translate(_) => ellipse.translated(|p| self.point(p))?,
                Self::Mirror { .. } | Self::PointMirror(_) => {
                    ellipse.reflected(|p| self.point(p), matches!(self, Self::Mirror { .. }))?
                }
                Self::About {
                    center,
                    scale,
                    radians,
                    ..
                } => {
                    if radians == 0. {
                        ellipse.dilated(|p| self.point(p), scale)?
                    } else if scale == 1. {
                        ellipse.rotated(center, radians.to_degrees())?
                    } else {
                        ellipse.rotated(center, radians.to_degrees())?.dilated(
                            |p| {
                                point(Point2 {
                                    x: center.x + scale * (p.x - center.x),
                                    y: center.y + scale * (p.y - center.y),
                                })
                            },
                            scale,
                        )?
                    }
                }
            }),
            Shape::Arc(arc) => {
                // G1 preserves the angles for a point mirror, negates both
                // for a line mirror, and keeps source bounds for rotation/homothety.
                let (start, end) = match *self {
                    Self::Translate(_) | Self::PointMirror(_) => (arc.start, arc.end),
                    Self::About {
                        scale,
                        radians,
                        preserve_arc_bounds,
                        ..
                    } => {
                        let angle = if preserve_arc_bounds {
                            0.
                        } else if scale < 0. {
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
