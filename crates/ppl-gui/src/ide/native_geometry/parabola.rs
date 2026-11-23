//! Focal parabola in an orthonormal frame, with the G1 parameter interval.
use super::{Extent, Point2, Shape, canonical, number, point, primitives as g};
use serde::{Deserialize, Serialize};

fn default_limit() -> f64 {
    12.
}
const STEPS: usize = 192;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parabola {
    pub(super) vertex: Point2,
    axis: Point2,
    focal: f64,
    #[serde(default = "default_limit")]
    pub(super) limit: f64,
    #[serde(default)]
    pub(super) offset: f64,
    #[serde(default)]
    pub(super) from_equation: bool,
}

impl Parabola {
    pub(super) fn vertical(vertex: Point2, focal: f64) -> Result<Self, String> {
        if number(focal)? == 0. {
            return Err("A parabola needs nonzero focal length".into());
        }
        Ok(Self {
            offset: 0.,
            from_equation: false,
            limit: default_limit(),
            vertex,
            axis: Point2 { x: 1., y: 0. },
            focal,
        })
    }
    pub(super) fn equation(vertex: Point2, axis: Point2, focal: f64) -> Result<Self, String> {
        let mut curve = Self::vertical(vertex, focal)?;
        curve.axis = axis;
        curve.limit = 24.;
        curve.from_equation = true;
        curve.offset = vertex.x * axis.x + vertex.y * axis.y;
        Ok(curve)
    }
    pub(super) fn from_focus(focus: Point2, a: Point2, b: Point2) -> Result<Self, String> {
        let foot = g::closest(focus, a, b, Extent::Line)?;
        let distance = foot.distance(focus)?;
        if distance == 0. {
            return Err("The focus lies on the directrix".into());
        }
        let d = g::delta(foot, focus);
        Ok(Self {
            offset: 0.,
            from_equation: false,
            limit: default_limit(),
            vertex: g::midpoint(foot, focus)?,
            axis: Point2 {
                x: d.y / distance,
                y: -d.x / distance,
            },
            focal: distance / 2.,
        })
    }
    pub(super) fn equation_source(&self) -> String {
        let expr = self.locus_source("x");
        format!(
            "plotparam({expr},x,-{lim},{lim})",
            lim = canonical(self.limit)
        )
    }
    pub(super) fn locus_source(&self, variable: &str) -> String {
        let t = format!("({variable}-({}))", canonical(self.offset));
        let component = |c: f64, a: f64, b: f64| {
            format!(
                "({}+({})*{t}+({})*{t}^2)",
                canonical(c),
                canonical(a),
                canonical(b / (4. * self.focal))
            )
        };
        format!(
            "({}+i*{})",
            component(self.vertex.x, self.axis.x, -self.axis.y),
            component(self.vertex.y, self.axis.y, self.axis.x),
        )
    }
    fn world(&self, x: f64, y: f64) -> Point2 {
        Point2 {
            x: self.vertex.x + self.axis.x * x - self.axis.y * y,
            y: self.vertex.y + self.axis.y * x + self.axis.x * y,
        }
    }
    fn local(&self, p: Point2) -> Point2 {
        let d = g::delta(self.vertex, p);
        Point2 {
            x: d.x * self.axis.x + d.y * self.axis.y,
            y: d.y * self.axis.x - d.x * self.axis.y,
        }
    }
    pub(super) fn definition(&self) -> [Point2; 3] {
        [
            self.world(0., self.focal),
            self.world(-1., -self.focal),
            self.world(1., -self.focal),
        ]
    }
    pub(super) fn at(&self, t: f64) -> Point2 {
        self.at_local(t - self.offset)
    }
    fn at_local(&self, t: f64) -> Point2 {
        self.world(t, (t / self.focal) * t * 0.25)
    }
    pub(super) fn vertices(&self) -> impl Iterator<Item = Point2> + '_ {
        (0..=STEPS).map(|i| self.at(-self.limit + 2. * self.limit * (i as f64 / STEPS as f64)))
    }
    pub(crate) fn edges(&self) -> impl Iterator<Item = [Point2; 2]> + '_ {
        self.vertices()
            .zip(self.vertices().skip(1))
            .map(|(a, b)| [a, b])
    }
    /// In local coordinates the closest point minimizes the squared distance to
    /// (t, t²/4f). Its sign agrees with the cursor's local x. On that half-axis
    /// the derivative has exactly one positive crossing, even above the focus.
    pub(super) fn project(&self, cursor: Point2) -> Result<Point2, String> {
        let cursor = self.local(cursor);
        let f = self.focal;
        let linear = number(8. * f * f - 4. * f * cursor.y)?;
        let constant = number(8. * f * f * cursor.x.abs())?;
        let mut lo = (-linear).max(0.).sqrt();
        if constant == 0. {
            return point(self.at_local(lo));
        }
        let mut hi = lo + constant.cbrt();
        for _ in 0..64 {
            let mid = lo + (hi - lo) / 2.;
            if mid == lo || mid == hi {
                break;
            }
            if mid * (mid * mid + linear) < constant {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        point(self.at_local((lo + (hi - lo) / 2.).copysign(cursor.x)))
    }
    pub(super) fn parameter(&self, cursor: Point2) -> Result<f64, String> {
        let (_, near) = self
            .edges()
            .filter_map(|[a, b]| {
                let p = g::closest(cursor, a, b, Extent::Segment).ok()?;
                Some((p.distance(cursor).ok()?, p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .ok_or("No visible parabola")?;
        number(self.local(near).x + self.offset)
    }
    fn tangent_at(&self, t: f64) -> Result<Shape, String> {
        let p = point(self.at_local(t))?;
        let slope = t / (2. * self.focal);
        g::line(
            p,
            point(Point2 {
                x: p.x + self.axis.x - self.axis.y * slope,
                y: p.y + self.axis.y + self.axis.x * slope,
            })?,
            Extent::Line,
        )
    }
    pub(super) fn tangent(&self, p: Point2, constrained: bool) -> Result<Shape, String> {
        let p = self.local(p);
        let d = p.x * p.x - 4. * self.focal * p.y;
        if constrained || d.abs() <= 1e-12 * (1. + p.x * p.x + (4. * self.focal * p.y).abs()) {
            return self.tangent_at(p.x);
        }
        let mut lines = Vec::new();
        if d > 0. {
            for t in [p.x - d.sqrt(), p.x + d.sqrt()] {
                if let Shape::Line { a, b, .. } = self.tangent_at(t)? {
                    lines.push([a, b]);
                }
            }
        }
        Ok(Shape::Lines(lines))
    }
    pub(super) fn crossings(&self, a: Point2, b: Point2) -> Result<Vec<Point2>, String> {
        let p = self.local(a);
        let q = self.local(b);
        let d = g::delta(p, q);
        // Intersect the supporting line with (t, t²/(4f)).
        if d.x == 0. {
            return Ok(vec![point(self.at_local(p.x))?]);
        }
        let k = 2. * self.focal * d.y / d.x;
        let c = 2. * k * p.x - 4. * self.focal * p.y;
        let disc = k * k - c;
        let tol = 32. * f64::EPSILON * (k * k + c.abs());
        if disc < -tol {
            return Ok(vec![]);
        }
        if disc.abs() <= tol {
            return Ok(vec![point(self.at_local(k))?]);
        }
        let far = k + disc.sqrt().copysign(k);
        [far, c / far]
            .into_iter()
            .map(|t| point(self.at_local(t)))
            .collect()
    }
    pub(super) fn area(&self) -> f64 {
        // Signed -integral(y dx) along the displayed parameter interval.
        let (u, v) = (self.axis.x, self.axis.y);
        let q = 0.25 / self.focal;
        let integral = |s: f64| {
            s * (self.vertex.y * u
                + s * ((v * u - 2. * q * v * self.vertex.y) / 2.
                    + s * (q * (u * u - 2. * v * v) / 3. - s * q * q * u * v / 2.)))
        };
        integral(-self.limit - self.offset) - integral(self.limit - self.offset)
    }
    pub(super) fn perimeter(&self) -> f64 {
        let scale = 2. * self.focal.abs();
        let primitive = |s: f64| (s * (s / scale).hypot(1.) + scale * (s / scale).asinh()) / 2.;
        primitive(self.limit - self.offset) - primitive(-self.limit - self.offset)
    }
}
