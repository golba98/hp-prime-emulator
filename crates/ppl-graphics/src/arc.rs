//! Integer circular arcs. Angles select a wedge of boundary pixels using
//! quantized endpoint rays, rather than repeatedly sampling trigonometric points.
use crate::{Color, Grob};

const TURN: i64 = 4096;

/// Mathematically generated Q15 sine, including exact quadrant endpoints.
/// This does not require a firmware lookup table.
pub(crate) fn sine(angle: i64) -> i64 {
    let angle = angle.rem_euclid(TURN);
    let (quarter, negative) = match angle {
        0..=1024 => (angle, false),
        1025..=2048 => (2048 - angle, false),
        2049..=3072 => (angle - 2048, true),
        _ => (4096 - angle, true),
    };
    let magnitude =
        ((quarter as f64 * std::f64::consts::FRAC_PI_2 / 1024.).sin() * 32768.).round() as i64 * 2;
    if negative { -magnitude } else { magnitude }
}

pub(crate) fn units(angle: f64) -> i64 {
    i64::from((angle * (4096. / std::f64::consts::TAU)) as i32)
}

struct Wedge {
    start: (i128, i128),
    end: (i128, i128),
    wide: bool,
}

impl Wedge {
    fn ray(radius: i64, angle: i64) -> (i128, i128) {
        let radius = i128::from(radius);
        // Endpoint multiplication uses an arithmetic shift. For positive
        // counterclockwise angles the screen-y result is negative, so its
        // magnitude rounds upward, unlike the x coordinate.
        (
            (radius * i128::from(sine(angle + 1024))) >> 16,
            -((radius * i128::from(sine(-angle))) >> 16),
        )
    }

    fn contains(&self, point: (i128, i128)) -> bool {
        let cross = |a: (i128, i128), b: (i128, i128)| a.0 * b.1 - a.1 * b.0;
        let after = cross(self.start, point) >= 0;
        let before = cross(point, self.end) >= 0;
        if self.wide {
            after || before
        } else {
            after && before
        }
    }
}

pub(crate) fn draw(grob: &mut Grob, xc: i64, yc: i64, radius: i64, a1: f64, a2: f64, c: Color) {
    // The calculator converts angles to signed integer units of 1/4096 turn,
    // truncating toward zero and saturating at the signed 32-bit limits.
    let (start, end) = (units(a1), units(a2));
    let difference = end - start;
    let full = difference.abs() >= TURN - 1;
    if !full && start == end {
        let (x, y) = Wedge::ray(radius, start);
        set_wide(grob, i128::from(xc) + x, i128::from(yc) - y, c);
        return;
    }
    let wedge = (!full).then(|| Wedge {
        start: Wedge::ray(radius, start),
        end: Wedge::ray(radius, end),
        wide: difference.rem_euclid(TURN) >= TURN / 2,
    });
    if radius < 0 {
        // The scalar circular path emits its initial point even when a
        // negative radius prevents traversal of the rest of the boundary.
        let point = (i128::from(radius), 0);
        if wedge.as_ref().is_none_or(|w| w.contains(point)) {
            set_wide(grob, i128::from(xc) + point.0, i128::from(yc), c);
        }
        return;
    }
    let (xc, yc, radius) = (i128::from(xc), i128::from(yc), i128::from(radius));
    let squared = radius * radius;
    // Work is bounded by the visible rectangle, even for huge, mostly clipped
    // circles. All differences and squares fit i128 after the radius check.
    let left = (xc - radius).max(0);
    let right = (xc + radius).min(grob.width() as i128 - 1);
    let top = (yc - radius).max(0);
    let bottom = (yc + radius).min(grob.height() as i128 - 1);
    for y in top..=bottom {
        let dy = (yc - y).abs();
        for x in left..=right {
            let dx = (x - xc).abs();
            let distance = dx * dx + dy * dy;
            // The outer integer boundary is the ceiling of the circle's
            // coordinate in either octant. Adjacent boundary runs meet at
            // corners without the extra pixels from dense angle sampling.
            let boundary = distance >= squared
                && (dx == 0
                    || dy == 0
                    || (dx - 1) * (dx - 1) + dy * dy < squared
                    || dx * dx + (dy - 1) * (dy - 1) < squared);
            if boundary && wedge.as_ref().is_none_or(|w| w.contains((x - xc, yc - y))) {
                set_wide(grob, x, y, c);
            }
        }
    }
    // The integer traversal closes on its initial rightmost point. This is
    // observable with transparent colors even though opaque pixels agree.
    if radius > 0 && wedge.as_ref().is_none_or(|w| w.contains((radius, 0))) {
        set_wide(grob, xc + radius, yc, c);
    }
}

fn set_wide(grob: &mut Grob, x: i128, y: i128, color: Color) {
    crate::raster::point(grob, x, y, color);
}
