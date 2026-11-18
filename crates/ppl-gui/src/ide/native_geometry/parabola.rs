//! Focal parabola in an orthonormal frame, with the G1 parameter interval.
use super::{Extent, Point2, Shape, number, point, primitives as g};
use serde::{Deserialize, Serialize};

const LIMIT: f64 = 12.;
const STEPS: usize = 192;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parabola {
    pub(super) vertex: Point2,
    axis: Point2,
    focal: f64,
}

impl Parabola {
    pub(super) fn vertical(vertex: Point2, focal: f64) -> Result<Self, String> {
        if number(focal)? == 0. {
            return Err("A parabola needs nonzero focal length".into());
        }
        Ok(Self {
            vertex,
            axis: Point2 { x: 1., y: 0. },
            focal,
        })
    }
    pub(super) fn from_focus(focus: Point2, a: Point2, b: Point2) -> Result<Self, String> {
        let foot = g::closest(focus, a, b, Extent::Line)?;
        let distance = foot.distance(focus)?;
        if distance == 0. {
            return Err("The focus lies on the directrix".into());
        }
        let d = g::delta(foot, focus);
        Ok(Self {
            vertex: g::midpoint(foot, focus)?,
            axis: Point2 {
                x: d.y / distance,
                y: -d.x / distance,
            },
            focal: distance / 2.,
        })
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
        self.world(t, (t / self.focal) * t * 0.25)
    }
    pub(super) fn vertices(&self) -> impl Iterator<Item = Point2> + '_ {
        (0..=STEPS).map(|i| self.at(-LIMIT + 2. * LIMIT * (i as f64 / STEPS as f64)))
    }
    pub(crate) fn edges(&self) -> impl Iterator<Item = [Point2; 2]> + '_ {
        self.vertices()
            .zip(self.vertices().skip(1))
            .map(|(a, b)| [a, b])
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
        number(self.local(near).x)
    }
    fn tangent_at(&self, t: f64) -> Result<Shape, String> {
        let p = point(self.at(t))?;
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
            return Ok(vec![point(self.at(p.x))?]);
        }
        let k = 2. * self.focal * d.y / d.x;
        let c = 2. * k * p.x - 4. * self.focal * p.y;
        let disc = k * k - c;
        let tol = 32. * f64::EPSILON * (k * k + c.abs());
        if disc < -tol {
            return Ok(vec![]);
        }
        if disc.abs() <= tol {
            return Ok(vec![point(self.at(k))?]);
        }
        let far = k + disc.sqrt().copysign(k);
        [far, c / far]
            .into_iter()
            .map(|t| point(self.at(t)))
            .collect()
    }
    pub(super) fn area(&self) -> f64 {
        // Signed -integral(y dx) along the displayed parameter interval.
        let (u, v) = (self.axis.x, self.axis.y);
        -2. * LIMIT * self.vertex.y * u - (LIMIT.powi(3) / (6. * self.focal)) * (u * u - 2. * v * v)
    }
    pub(super) fn perimeter(&self) -> f64 {
        let scale = 2. * self.focal.abs();
        let t = LIMIT / scale;
        LIMIT * t.hypot(1.) + scale * t.asinh()
    }
}
