//! Bounded midpoint circles. Reconstruct only octants that can touch the GROB.
use crate::{Color, Grob};
use std::collections::BTreeSet;

pub(crate) fn outline(grob: &mut Grob, xc: i64, yc: i64, radius: i64, color: Color) {
    if radius < 0 {
        return;
    }
    let radius = radius as u128;
    // Each visible point has one minor offset in a visible row or column.
    // Deduplicate and preserve the original increasing-minor drawing order,
    // including repeated axis/diagonal writes for transparency compatibility.
    let mut minors = BTreeSet::new();
    for x in 0..grob.width() {
        minors.insert((x as i128 - i128::from(xc)).unsigned_abs());
    }
    for y in 0..grob.height() {
        minors.insert((y as i128 - i128::from(yc)).unsigned_abs());
    }
    for minor in minors.into_iter().take_while(|minor| *minor <= radius) {
        let square = radius * radius - minor * minor;
        let floor = square.isqrt();
        // Round sqrt to nearest integer without floating-point loss. The
        // midpoint recurrence chooses precisely this major-axis coordinate.
        let major = floor + u128::from(square - floor * floor > floor);
        if major < minor {
            continue;
        }
        let (x, y) = (major as i128, minor as i128);
        for (dx, dy) in [
            (x, y),
            (y, x),
            (-y, x),
            (-x, y),
            (-x, -y),
            (-y, -x),
            (y, -x),
            (x, -y),
        ] {
            let (px, py) = (i128::from(xc) + dx, i128::from(yc) + dy);
            if px >= 0 && py >= 0 && px < grob.width() as i128 && py < grob.height() as i128 {
                grob.set(px as i64, py as i64, color);
            }
        }
    }
}

pub(crate) fn fill(grob: &mut Grob, xc: i64, yc: i64, radius: i64, color: Color) {
    if radius < 0 {
        return;
    }
    let radius = radius as u128;
    for y in 0..grob.height() {
        let offset = (y as i128 - i128::from(yc)).unsigned_abs();
        if offset > radius {
            continue;
        }
        let half = (radius * radius - offset * offset).isqrt() as i128;
        let first = (i128::from(xc) - half).max(0);
        let last = (i128::from(xc) + half).min(grob.width() as i128 - 1);
        for x in first..=last {
            grob.set(x as i64, y as i64, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BLACK, WHITE};

    fn reference_outline(g: &mut Grob, xc: i64, yc: i64, r: i64, color: Color) {
        let (mut x, mut y, mut d) = (r, 0, 1 - r);
        while x >= y {
            for (dx, dy) in [
                (x, y),
                (y, x),
                (-y, x),
                (-x, y),
                (-x, -y),
                (-y, -x),
                (y, -x),
                (x, -y),
            ] {
                g.set(xc + dx, yc + dy, color);
            }
            y += 1;
            if d < 0 {
                d += 2 * y + 1;
            } else {
                x -= 1;
                d += 2 * (y - x) + 1;
            }
        }
    }

    #[test]
    fn clipped_octants_match_midpoint_pixels_and_transparency() {
        for r in 0..80 {
            for (xc, yc) in [(-30, 17), (0, 0), (17, 15), (50, 48)] {
                for color in [BLACK, 0x800000ff] {
                    let mut expected = Grob::new(37, 31, WHITE);
                    reference_outline(&mut expected, xc, yc, r, color);
                    let mut actual = Grob::new(37, 31, WHITE);
                    actual.circle(xc, yc, r, color);
                    assert_eq!(actual, expected, "r={r}, center=({xc},{yc}), color={color}");
                }
            }
        }
    }

    #[test]
    fn clipped_fill_matches_integer_disk() {
        for r in 0..40 {
            for (xc, yc) in [(-10, 17), (0, 0), (17, 15), (40, 39)] {
                let mut expected = Grob::new(37, 31, WHITE);
                for y in 0..31 {
                    for x in 0..37 {
                        if (x - xc) * (x - xc) + (y - yc) * (y - yc) <= r * r {
                            expected.set(x, y, BLACK);
                        }
                    }
                }
                let mut actual = Grob::new(37, 31, WHITE);
                actual.fill_circle(xc, yc, r, BLACK);
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn extreme_centers_and_radii_are_clipped_without_overflow_or_unbounded_work() {
        let mut g = Grob::new(8, 8, WHITE);
        g.fill_circle(0, 0, i64::MAX, BLACK);
        assert!(g.pixels().iter().all(|p| *p == BLACK));
        g.circle(0, 0, i64::MAX, WHITE);
        assert!(g.pixels().iter().all(|p| *p == BLACK));
        g.circle(i64::MAX, 0, i64::MAX, WHITE);
        assert_eq!(g.get(0, 0), Some(WHITE));
        let mut off = Grob::new(8, 8, WHITE);
        off.circle(i64::MIN, i64::MIN, i64::MAX, BLACK);
        off.fill_circle(i64::MIN, i64::MIN, i64::MAX, BLACK);
        assert!(off.pixels().iter().all(|p| *p == WHITE));
        off.circle(0, 0, -1, BLACK);
        off.fill_circle(0, 0, -1, BLACK);
        assert!(off.pixels().iter().all(|p| *p == WHITE));
        Grob::new(0, 0, WHITE).circle(0, 0, 0, BLACK);
    }
}
