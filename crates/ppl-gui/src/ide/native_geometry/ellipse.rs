use super::{
    Extent, Point2, number, point,
    primitives::{delta, line_circle_intersections, midpoint},
};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

const STEPS: usize = 192;
const STEP: f64 = TAU / STEPS as f64;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(super) enum Size {
    Witness(Point2),
    SemiMajor(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ellipse {
    pub(super) foci: [Point2; 2],
    pub(super) size: Size,
    center: Point2,
    axis: Point2,
    major: f64,
    minor: f64,
    #[serde(default)]
    orientation_reversed: bool,
    /// Point On uses the original ellipse after a transformation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) element_reference: Option<Box<Ellipse>>,
}

impl Ellipse {
    pub(super) fn new(foci: [Point2; 2], size: Size) -> Result<Self, String> {
        let [a, b] = foci;
        let d = delta(a, b);
        let span = a.distance(b)?;
        let focal_radius = span / 2.;
        let major = match size {
            Size::Witness(p) => (a.distance(p)? / 2. + b.distance(p)? / 2.).max(focal_radius),
            Size::SemiMajor(signed) => signed.abs(),
        };
        if major < focal_radius {
            return Err("An ellipse's semi-major axis must reach its foci".into());
        }
        let minor = if major == focal_radius {
            0.
        } else {
            major * (((major - focal_radius) / major) * (1. + focal_radius / major)).sqrt()
        };
        let axis = if span == 0. {
            Point2 { x: 0., y: 1. }
        } else {
            Point2 {
                x: d.x / span,
                y: d.y / span,
            }
        };
        Ok(Self {
            foci,
            size,
            center: midpoint(a, b)?,
            axis,
            major: number(major)?,
            minor: number(minor)?,
            orientation_reversed: false,
            element_reference: None,
        })
    }

    pub(super) fn reflected(
        &self,
        reflect: impl Fn(Point2) -> Result<Point2, String>,
        about_line: bool,
    ) -> Result<Self, String> {
        let [a, b] = self.foci;
        let size = match self.size {
            Size::Witness(p) => Size::Witness(reflect(p)?),
            size @ Size::SemiMajor(_) => size,
        };
        let mut result = Self::new([reflect(a)?, reflect(b)?], size)?;
        result.major = self.major;
        result.minor = self.minor;
        result.orientation_reversed = self.orientation_reversed ^ about_line;
        result.element_reference = Some(self.original_element_reference());
        Ok(result)
    }

    pub(super) fn rotated(&self, center: Point2, degrees: f64) -> Result<Self, String> {
        let rotate = |p: Point2| p.rotate(center, degrees);
        let [a, b] = self.foci;
        let size = match self.size {
            Size::Witness(p) => Size::Witness(rotate(p)?),
            size @ Size::SemiMajor(_) => size,
        };
        Ok(Self {
            foci: [rotate(a)?, rotate(b)?],
            size,
            center: rotate(self.center)?,
            axis: self.axis.rotate(Point2 { x: 0., y: 0. }, degrees)?,
            major: self.major,
            minor: self.minor,
            orientation_reversed: self.orientation_reversed,
            element_reference: Some(self.original_element_reference()),
        })
    }

    pub(super) fn translated(
        &self,
        translate: impl Fn(Point2) -> Result<Point2, String>,
    ) -> Result<Self, String> {
        let mut result = self.clone();
        let [a, b] = self.foci;
        result.foci = [translate(a)?, translate(b)?];
        result.center = translate(self.center)?;
        if let Size::Witness(p) = self.size {
            result.size = Size::Witness(translate(p)?);
        }
        result.element_reference = Some(self.original_element_reference());
        Ok(result)
    }

    pub(super) fn dilated(
        &self,
        transform: impl Fn(Point2) -> Result<Point2, String>,
        factor: f64,
    ) -> Result<Self, String> {
        let [a, b] = self.foci;
        let magnitude = factor.abs();
        let sign = factor.signum();
        let size = match self.size {
            Size::Witness(p) => Size::Witness(transform(p)?),
            Size::SemiMajor(length) => Size::SemiMajor(number(length * magnitude)?),
        };
        Ok(Self {
            foci: [transform(a)?, transform(b)?],
            size,
            center: transform(self.center)?,
            axis: Point2 {
                x: self.axis.x * sign,
                y: self.axis.y * sign,
            },
            major: number(self.major * magnitude)?,
            minor: number(self.minor * magnitude)?,
            orientation_reversed: self.orientation_reversed,
            element_reference: Some(self.original_element_reference()),
        })
    }

    fn original_element_reference(&self) -> Box<Self> {
        self.element_reference
            .clone()
            .unwrap_or_else(|| Box::new(self.clone()))
    }

    fn world(&self, x: f64, y: f64) -> Point2 {
        Point2 {
            x: self.center.x + (x * self.axis.x - y * self.axis.y),
            y: self.center.y + (x * self.axis.y + y * self.axis.x),
        }
    }

    fn local_vector(&self, p: Point2) -> Point2 {
        Point2 {
            x: self.axis.x * p.x + self.axis.y * p.y,
            y: self.axis.x * p.y - self.axis.y * p.x,
        }
    }

    fn at(&self, angle: f64) -> Point2 {
        let (sin, cos) = angle.sin_cos();
        self.world(self.major * cos, self.minor * sin)
    }

    fn anchor(&self) -> Point2 {
        match self.size {
            Size::Witness(p) => p,
            Size::SemiMajor(signed) => self.world(signed, 0.),
        }
    }

    pub fn vertices(&self) -> impl Iterator<Item = Point2> + '_ {
        (0..=STEPS).map(|i| self.at(i as f64 * STEP))
    }

    pub fn edges(&self) -> impl Iterator<Item = [Point2; 2]> + '_ {
        let mut previous = self.at(0.);
        self.vertices()
            .skip(1)
            .map(move |p| [std::mem::replace(&mut previous, p), p])
    }

    pub(super) fn area(&self) -> f64 {
        let area = PI * (self.major * self.minor);
        let signed = match self.size {
            Size::SemiMajor(signed) => area.copysign(signed),
            Size::Witness(_) => area,
        };
        if self.orientation_reversed {
            -signed
        } else {
            signed
        }
    }

    pub(super) fn perimeter(&self) -> f64 {
        if self.minor == 0. {
            return 4. * self.major;
        }
        let ratio = self.minor / self.major;
        if ratio == 0. {
            return 4. * self.major;
        }
        // The complete elliptic integral of the second kind converges through
        // arithmetic and geometric means even when the ellipse is nearly flat.
        let (mut arithmetic, mut geometric) = (1., ratio);
        let mut correction = (1. - ratio) * (1. + ratio) / 2.;
        let mut weight = 1.;
        while (arithmetic - geometric).abs() > f64::EPSILON * arithmetic {
            let difference = (arithmetic - geometric) / 2.;
            correction += weight * difference * difference;
            (arithmetic, geometric) = (
                (arithmetic + geometric) / 2.,
                (arithmetic * geometric).sqrt(),
            );
            weight *= 2.;
        }
        self.major * (TAU * (1. - correction) / arithmetic)
    }

    pub(super) fn element(&self, slope: f64) -> Result<Point2, String> {
        if let Some(reference) = &self.element_reference {
            return reference.element(slope);
        }
        let anchor = self.anchor();
        if self.minor == 0. {
            return point(anchor);
        }
        // A slope describes the line through the defining point on the ellipse.
        // In unit-circle coordinates the other intersection is a reflection
        // across the diameter perpendicular to that line.
        let length = slope.hypot(1.);
        let direction = self.local_vector(Point2 {
            x: 1. / length,
            y: slope / length,
        });
        let along_major = direction.x * (self.minor / self.major);
        let scale = along_major.hypot(direction.y);
        let (dx, dy) = (along_major / scale, direction.y / scale);
        let a = self.local_vector(delta(self.center, anchor));
        let (x, y) = (a.x / self.major, a.y / self.minor);
        let twice_projection = 2. * (x * dx + y * dy);
        point(self.world(
            self.major * (x - twice_projection * dx),
            self.minor * (y - twice_projection * dy),
        ))
    }

    pub(super) fn line_intersections(
        &self,
        a: Point2,
        b: Point2,
        extent: Extent,
    ) -> Result<Vec<Point2>, String> {
        if self.minor == 0. {
            return Err("Intersections with a collapsed ellipse are unsupported".into());
        }
        let unit = |p| {
            let p = self.local_vector(delta(self.center, p));
            point(Point2 {
                x: p.x / self.major,
                y: p.y / self.minor,
            })
        };
        // The affine change of coordinates preserves the parameter of a
        // line, segment or ray, so use the existing unit-circle calculation.
        let mut points =
            line_circle_intersections(unit(a)?, unit(b)?, extent, Point2 { x: 0., y: 0. }, 1.)?;
        // Axis normalization can leave an exact tangent a few ulps inside
        // the unit circle. Keep its two roundoff-sized roots as one point.
        if let [p, q] = points.as_slice()
            && p.distance(*q)? <= 8. * f64::EPSILON.sqrt()
        {
            points = vec![midpoint(*p, *q)?];
        }
        points
            .into_iter()
            .map(|p| point(self.world(p.x * self.major, p.y * self.minor)))
            .collect()
    }

    pub(super) fn parameter(&self, cursor: Point2) -> Result<f64, String> {
        if let Some(reference) = &self.element_reference {
            return reference.parameter(cursor);
        }
        if self.minor == 0. {
            return Ok(0.);
        }
        let distance = |p| {
            let d = delta(cursor, p);
            d.x.hypot(d.y)
        };
        let (i, _) = self
            .vertices()
            .enumerate()
            .min_by(|(_, a), (_, b)| distance(*a).total_cmp(&distance(*b)))
            .expect("ellipse has a fixed nonempty sample set");
        let angle = i as f64 * STEP;
        let (mut lo, mut hi) = (angle - STEP, angle + STEP);
        for _ in 0..28 {
            let step = (hi - lo) / 3.;
            let left = lo + step;
            let right = hi - step;
            if distance(self.at(left)) <= distance(self.at(right)) {
                hi = right;
            } else {
                lo = left;
            }
        }
        let a = self.local_vector(delta(self.center, self.anchor()));
        let anchor_angle = (a.y / self.minor).atan2(a.x / self.major);
        // Factoring the difference of two ellipse points gives the chord
        // direction even where the cursor coincides with the anchor.
        let (sin, cos) = ((anchor_angle + (lo + hi) / 2.) / 2.).sin_cos();
        let along_major = -sin;
        let along_minor = cos * (self.minor / self.major);
        let dx = along_major * self.axis.x - along_minor * self.axis.y;
        let dy = along_major * self.axis.y + along_minor * self.axis.x;
        // A vertical chord is the limit of real slopes.
        let min_dx = 1e-15 * dx.hypot(dy);
        number(dy / dx.abs().max(min_dx).copysign(dx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highly_eccentric_perimeters_match_the_original_virtual_measurements() {
        let horizontal = [Point2 { x: -2., y: 0. }, Point2 { x: 2., y: 0. }];
        let vertical = [Point2 { x: 0., y: -3. }, Point2 { x: 0., y: 3. }];
        for (foci, radius, original) in [
            (horizontal, 2.0000062, 8.00019546817),
            (horizontal, -2.00000001, 8.00000044386),
            (horizontal, 2.00000000001, 8.00000000027),
            (vertical, 3.0005, 12.0117794679),
        ] {
            let ellipse = Ellipse::new(foci, Size::SemiMajor(radius)).unwrap();
            let perimeter = ellipse.perimeter();
            assert!(
                (perimeter - original).abs() < 4e-10,
                "{radius}: {perimeter}"
            );
        }
        let segment = Ellipse::new(horizontal, Size::SemiMajor(2.)).unwrap();
        assert_eq!(segment.perimeter(), 8.);
    }
}
