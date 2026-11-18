use super::{
    Point2, number, point,
    primitives::{circle_element, delta},
};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

const ANGLE_TOLERANCE: f64 = 1e-11;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CircularArc {
    pub center: Point2,
    pub radius: f64,
    pub start: f64,
    pub end: f64,
}

impl CircularArc {
    pub(super) fn new(center: Point2, radius: f64, start: f64, end: f64) -> Result<Self, String> {
        point(center)?;
        number(radius)?;
        number(end - start)?;
        Ok(Self {
            center,
            radius,
            start,
            end,
        })
    }

    pub(super) fn perimeter(self) -> f64 {
        self.radius * (self.end - self.start)
    }

    pub(super) fn area(self) -> f64 {
        self.perimeter() * (self.radius / 2.)
    }

    fn sweep(self) -> f64 {
        let difference = self.end - self.start;
        // Measurements retain the signed difference. The displayed arc runs
        // counterclockwise, and either sign of a full turn closes the curve.
        // Allow for the rounding of angle expressions in the 12-digit evaluator.
        if difference.abs() >= TAU - ANGLE_TOLERANCE {
            TAU
        } else if difference < 0. {
            difference + TAU
        } else {
            difference
        }
    }

    fn start_angle(self) -> f64 {
        let (sin, cos) = self.start.sin_cos();
        sin.atan2(cos)
    }

    pub(super) fn contains_intersection(self, p: Point2) -> bool {
        if self.radius == 0. {
            return false;
        }
        let sweep = self.sweep();
        if sweep == TAU {
            return true;
        }
        let start = self.start_angle().rem_euclid(TAU);
        // The firmware tests positive angles against the interval without
        // moving candidates across zero into the wrapped part. It treats
        // equal endpoints as an interval from the start through one turn.
        let end = if sweep == 0. { TAU } else { start + sweep };
        let d = delta(self.center, p);
        let mut angle = d.y.atan2(d.x).rem_euclid(TAU);
        if start <= ANGLE_TOLERANCE && angle >= TAU - ANGLE_TOLERANCE {
            angle = 0.;
        }
        angle >= start - ANGLE_TOLERANCE && angle <= end + ANGLE_TOLERANCE
    }

    pub fn vertices(self) -> impl Iterator<Item = Point2> {
        let start = self.start_angle();
        let sweep = self.sweep();
        let steps = ((sweep / TAU * 192.).ceil() as u32).max(1);
        (0..=steps).map(move |i| {
            circle_element(
                self.center,
                self.radius,
                start + sweep * f64::from(i) / f64::from(steps),
            )
        })
    }

    pub fn edges(self) -> impl Iterator<Item = [Point2; 2]> {
        let mut previous = circle_element(self.center, self.radius, self.start);
        self.vertices()
            .skip(1)
            .map(move |p| [std::mem::replace(&mut previous, p), p])
    }
}
