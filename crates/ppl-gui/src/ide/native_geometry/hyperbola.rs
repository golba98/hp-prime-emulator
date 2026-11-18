//! A G1 two-focus hyperbola draws both parametric branches for -3 ≤ t ≤ 3.
//! Point On uses the slope of a secant through the original defining point.
use super::{Extent, Point2, canonical, ellipse::Size, number, point, primitives as g};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hyperbola {
    pub(super) foci: [Point2; 2],
    pub(super) size: Size,
    center: Point2,
    axis: Point2,
    major: f64,
    minor: f64,
    #[serde(default)]
    pub(super) equation: bool,
}

impl Hyperbola {
    pub(super) fn new(foci: [Point2; 2], size: Size) -> Result<Self, String> {
        let [a, b] = foci;
        let span = a.distance(b)?;
        if span == 0. {
            return Err("A hyperbola needs distinct foci".into());
        }
        let d = g::delta(a, b);
        let focal = span / 2.;
        let major = match size {
            Size::SemiMajor(x) => x,
            Size::Witness(p) => (a.distance(p)? - b.distance(p)?) / 2.,
        };
        if major.abs() > focal {
            return Err("The transverse radius exceeds the focal distance".into());
        }
        let minor =
            number(((focal - major.abs()) / focal * (1. + major.abs() / focal)).sqrt() * focal)?;
        Ok(Self {
            equation: false,
            foci,
            size,
            center: g::midpoint(a, b)?,
            axis: Point2 {
                x: d.x / span,
                y: d.y / span,
            },
            major,
            minor,
        })
    }
    fn world(&self, x: f64, y: f64) -> Point2 {
        Point2 {
            x: self.center.x + self.axis.x * x - self.axis.y * y,
            y: self.center.y + self.axis.y * x + self.axis.x * y,
        }
    }
    fn local_vector(&self, p: Point2) -> Point2 {
        Point2 {
            x: self.axis.x * p.x + self.axis.y * p.y,
            y: self.axis.x * p.y - self.axis.y * p.x,
        }
    }
    pub(super) fn branch(&self, sign: f64) -> impl Iterator<Item = Point2> + '_ {
        (0..=120).map(move |i| {
            let limit = self.limit(sign);
            let t = limit * (i as f64 / 60. - 1.);
            self.world(
                sign * self.major * t.cosh(),
                self.branch_y(sign) * self.minor * t.sinh(),
            )
        })
    }
    pub(crate) fn edges(&self) -> impl Iterator<Item = [Point2; 2]> + '_ {
        [1., -1.].into_iter().flat_map(|sign| {
            self.branch(sign)
                .zip(self.branch(sign).skip(1))
                .map(|(a, b)| [a, b])
        })
    }
    fn limit(&self, sign: f64) -> f64 {
        // Equation-defined G1 curves use the decimal endpoint 3.14 on the positive branch.
        if self.equation && sign > 0. {
            314. / 100.
        } else {
            3.
        }
    }
    fn branch_y(&self, sign: f64) -> f64 {
        if self.equation { 1. } else { sign }
    }
    fn anchor(&self) -> Point2 {
        match self.size {
            Size::Witness(p) => p,
            Size::SemiMajor(a) => self.world(a, 0.),
        }
    }
    pub(super) fn element(&self, t: f64) -> Result<Point2, String> {
        if self.equation {
            if t == 0. {
                return Err("The hyperbola point at zero is at infinity".into());
            }
            let inverse = 1. / t;
            return point(self.world(
                self.major * (t * 0.5 + inverse * 0.5),
                self.minor * (t * 0.5 - inverse * 0.5),
            ));
        }
        let anchor = self.anchor();
        if self.major == 0. || self.minor == 0. {
            return point(anchor);
        }
        let p = self.local_vector(g::delta(self.center, anchor));
        let h = t.hypot(1.);
        let d = self.local_vector(Point2 {
            x: 1. / h,
            y: t / h,
        });
        let dx = d.x / self.major;
        let dy = d.y / self.minor;
        let denominator = dx * dx - dy * dy;
        if denominator.abs() <= 32. * f64::EPSILON * (dx * dx + dy * dy) {
            return Err("The secant is parallel to an asymptote".into());
        }
        let k = -2. * (p.x / self.major * dx - p.y / self.minor * dy) / denominator;
        point(self.world(p.x + k * d.x, p.y + k * d.y))
    }
    pub(super) fn parameter(&self, cursor: Point2) -> Result<f64, String> {
        let (_, near) = self
            .edges()
            .filter_map(|[a, b]| {
                let p = g::closest(cursor, a, b, Extent::Segment).ok()?;
                Some((p.distance(cursor).ok()?, p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .ok_or("No visible hyperbola")?;
        if self.equation {
            let p = self.local_vector(g::delta(self.center, near));
            let (x, y) = (p.x / self.major, p.y / self.minor);
            return number(if x * y < 0. { 1. / (x - y) } else { x + y });
        }
        let d = g::delta(self.anchor(), near);
        if d.x == 0. && d.y == 0. {
            let p = self.local_vector(g::delta(self.center, near));
            let tangent = Point2 {
                x: self.major * self.major * p.y,
                y: self.minor * self.minor * p.x,
            };
            let dx = self.axis.x * tangent.x - self.axis.y * tangent.y;
            let dy = self.axis.y * tangent.x + self.axis.x * tangent.y;
            return number(dy / dx);
        }
        number(d.y / d.x)
    }
    pub(super) fn tangent_at(&self, p: Point2) -> Result<super::Shape, String> {
        let local = self.local_vector(g::delta(self.center, p));
        let (dx, dy) = (
            self.major * self.major * local.y,
            self.minor * self.minor * local.x,
        );
        let d = dx.hypot(dy);
        if d == 0. {
            return Err("The tangent is undefined at this point".into());
        }
        g::line(
            p,
            point(Point2 {
                x: p.x + (self.axis.x * dx - self.axis.y * dy) / d,
                y: p.y + (self.axis.y * dx + self.axis.x * dy) / d,
            })?,
            Extent::Line,
        )
    }
    pub(super) fn area(&self) -> f64 {
        if self.equation {
            return [1., -1.]
                .into_iter()
                .map(|sign| {
                    let t = self.limit(sign);
                    let s = (2. * t).sinh() * 0.5;
                    -sign
                        * self.major
                        * self.minor
                        * (self.axis.x.powi(2) * (s - t) - self.axis.y.powi(2) * (s + t))
                        + 2. * self.center.y * self.axis.y * self.minor * t.sinh()
                })
                .sum();
        }
        let s = 6_f64.sinh();
        self.major * self.minor * (self.axis.y.powi(2) * (s + 6.) - self.axis.x.powi(2) * (s - 6.))
    }
    pub(super) fn branch_source(&self, sign: f64) -> String {
        let u = self.axis;
        let coordinate = |c: f64, a: f64, b: f64| {
            format!(
                "({}+({})*COSH(t)+({})*SINH(t))",
                canonical(c),
                canonical(a * sign),
                canonical(b * self.branch_y(sign))
            )
        };
        format!(
            "plotparam({}+i*{},t,-{lim},{lim})",
            coordinate(self.center.x, self.major * u.x, -self.minor * u.y),
            coordinate(self.center.y, self.major * u.y, self.minor * u.x),
            lim = canonical(self.limit(sign)),
        )
    }
}
