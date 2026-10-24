//! Differential verification of a Home-screen result (M4-C), from firmware
//! frames only: no OCR, no host arithmetic, no drawing.
//!
//! When ENTER evaluates an expression, the firmware writes a history line
//! with the expression on the left and the result right-aligned. The result
//! glyphs are the right-most cluster of ink in the region ENTER changed.
//! Running `2+2` and `4` and comparing those clusters pixel for pixel shows
//! whether the firmware displayed the same value for both; `2+3` is the
//! control that must differ.

use std::path::Path;

use crate::display::Frame;

/// Columns without ink that end a glyph cluster (glyphs of one number are
/// closer together than this).
const CLUSTER_GAP: u32 = 4;

/// A cropped run of glyphs from a frame.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Glyphs {
    /// Inclusive `[x0, y0, x1, y1]` in the frame.
    pub bbox: [u32; 4],
    #[serde(skip)]
    pub pixels: Vec<u32>,
}

impl Glyphs {
    pub fn width(&self) -> u32 {
        self.bbox[2] - self.bbox[0] + 1
    }

    pub fn height(&self) -> u32 {
        self.bbox[3] - self.bbox[1] + 1
    }

    /// Same size and identical pixels.
    pub fn same_as(&self, other: &Glyphs) -> bool {
        self.width() == other.width()
            && self.height() == other.height()
            && self.pixels == other.pixels
    }
}

/// Unchanged rows that separate two changed bands. The history line and
/// the cleared edit line below it are 4 rows apart on the G1 Home view
/// (measured, M4); the status bar is ~170 rows above them.
const BAND_GAP: u32 = 8;

/// The lowest band of contiguous changed rows, as `[x0, y0, x1, y1]`: the
/// history and edit lines at the bottom of the Home view. Changes higher
/// up (the status bar clock ticking over a minute) are separate bands and
/// are ignored.
fn lowest_changed_band(before: &Frame, after: &Frame) -> Option<[u32; 4]> {
    let row_changed = |y: u32| (0..after.width).any(|x| after.pixel(x, y) != before.pixel(x, y));
    let y1 = (0..after.height).rev().find(|&y| row_changed(y))?;
    let mut y0 = y1;
    let mut gap = 0;
    for y in (0..y1).rev() {
        if row_changed(y) {
            y0 = y;
            gap = 0;
        } else {
            gap += 1;
            if gap > BAND_GAP {
                break;
            }
        }
    }
    let cols: Vec<u32> = (0..after.width)
        .filter(|&x| (y0..=y1).any(|y| after.pixel(x, y) != before.pixel(x, y)))
        .collect();
    Some([*cols.first()?, y0, *cols.last()?, y1])
}

/// The glyph clusters of the line ENTER produced: `(expression, result)`
/// (left-most and right-most clusters of the lowest changed band).
pub fn history_line(before: &Frame, after: &Frame) -> Option<(Glyphs, Glyphs)> {
    let [x0, y0, x1, y1] = lowest_changed_band(before, after)?;
    // "Ink" is what ENTER changed: static decorations (the rule above the
    // edit line, the background) are identical in both frames.
    let changed = |x: u32, y: u32| after.pixel(x, y) != before.pixel(x, y);
    let column_has_ink = |x: u32| (y0..=y1).any(|y| changed(x, y));
    let crop = |xa: u32, xb: u32| -> Option<Glyphs> {
        let rows: Vec<u32> = (y0..=y1)
            .filter(|&y| (xa..=xb).any(|x| changed(x, y)))
            .collect();
        let (ya, yb) = (*rows.first()?, *rows.last()?);
        let mut pixels = Vec::new();
        for y in ya..=yb {
            for x in xa..=xb {
                pixels.push(after.pixel(x, y));
            }
        }
        Some(Glyphs {
            bbox: [xa, ya, xb, yb],
            pixels,
        })
    };
    // Right-most cluster: from the last inked column leftwards until a gap.
    let right_end = (x0..=x1).rev().find(|&x| column_has_ink(x))?;
    let mut right_start = right_end;
    let mut gap = 0;
    for x in (x0..right_end).rev() {
        if column_has_ink(x) {
            right_start = x;
            gap = 0;
        } else {
            gap += 1;
            if gap > CLUSTER_GAP {
                break;
            }
        }
    }
    // Left-most cluster, mirrored.
    let left_start = (x0..=x1).find(|&x| column_has_ink(x))?;
    let mut left_end = left_start;
    gap = 0;
    for x in left_start + 1..=x1 {
        if column_has_ink(x) {
            left_end = x;
            gap = 0;
        } else {
            gap += 1;
            if gap > CLUSTER_GAP {
                break;
            }
        }
    }
    if left_end >= right_start {
        return None; // one cluster only: not an expression/result line
    }
    Some((crop(left_start, left_end)?, crop(right_start, right_end)?))
}

/// Verdict of comparing `2+2` (A) with `4` (B) and a control (C).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CalcVerdict {
    pub a_result: Option<Glyphs>,
    pub b_result: Option<Glyphs>,
    pub c_result: Option<Glyphs>,
    /// A's result is pixel-identical to B's.
    pub result_matches_reference: bool,
    /// A's result differs from the control's.
    pub result_differs_from_control: bool,
    /// A's expression differs from B's (the typed `2+2` is not `4`).
    pub expressions_differ: bool,
    pub verified: bool,
}

/// `runs` are `(before ENTER, after ENTER)` frame pairs for A, B and C.
pub fn verify_calculation(
    a: (&Frame, &Frame),
    b: (&Frame, &Frame),
    c: (&Frame, &Frame),
) -> CalcVerdict {
    let la = history_line(a.0, a.1);
    let lb = history_line(b.0, b.1);
    let lc = history_line(c.0, c.1);
    let same = |x: &Option<(Glyphs, Glyphs)>,
                y: &Option<(Glyphs, Glyphs)>,
                f: fn(&(Glyphs, Glyphs)) -> &Glyphs| {
        match (x, y) {
            (Some(p), Some(q)) => Some(f(p).same_as(f(q))),
            _ => None,
        }
    };
    let result_matches_reference = same(&la, &lb, |l| &l.1) == Some(true);
    let result_differs_from_control = same(&la, &lc, |l| &l.1) == Some(false);
    let expressions_differ = same(&la, &lb, |l| &l.0) == Some(false);
    CalcVerdict {
        verified: result_matches_reference && result_differs_from_control && expressions_differ,
        a_result: la.map(|l| l.1),
        b_result: lb.map(|l| l.1),
        c_result: lc.map(|l| l.1),
        result_matches_reference,
        result_differs_from_control,
        expressions_differ,
    }
}

/// Loads a PNG written by [`Frame::save_png`].
pub fn load_png(path: &Path) -> std::io::Result<Frame> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info().map_err(std::io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(std::io::Error::other)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(std::io::Error::other("expected 8-bit RGBA"));
    }
    let pixels = buf[..info.buffer_size()]
        .chunks_exact(4)
        .map(|p| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]))
        .collect();
    Ok(Frame {
        width: info.width,
        height: info.height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 320;
    const H: u32 = 240;
    const WHITE: u32 = 0xFFFFFF;

    fn blank() -> Frame {
        Frame {
            width: W,
            height: H,
            pixels: vec![WHITE; (W * H) as usize],
        }
    }

    /// Draws a fake glyph: a `w`x`h` pattern seeded by `seed`.
    fn glyph(f: &mut Frame, x: u32, y: u32, seed: u32) {
        for dy in 0..10 {
            for dx in 0..6 {
                if (dx * 7 + dy * 3 + seed) % 4 == 0 {
                    f.pixels[((y + dy) * W + x + dx) as usize] = 0x000000;
                }
            }
        }
    }

    /// History line: expression glyphs at the left, result at the right.
    fn line(expr: &[u32], result: &[u32]) -> (Frame, Frame) {
        let before = blank();
        let mut after = blank();
        for (i, &s) in expr.iter().enumerate() {
            glyph(&mut after, 2 + 8 * i as u32, 190, s);
        }
        for (i, &s) in result.iter().rev().enumerate() {
            glyph(&mut after, 310 - 8 * i as u32, 190, s);
        }
        // ENTER also clears the edit line below (4 unchanged rows apart,
        // as on the firmware's Home view).
        let mut before = before;
        for (i, &s) in expr.iter().enumerate() {
            glyph(&mut before, 2 + 8 * i as u32, 204, s);
        }
        (before, after)
    }

    #[test]
    fn extracts_expression_and_right_aligned_result() {
        let (b, a) = line(&[2, 10, 2], &[4]);
        let (expr, result) = history_line(&b, &a).unwrap();
        assert_eq!(expr.bbox[0], 2);
        assert!(expr.bbox[2] >= 2 + 16, "three glyphs");
        assert!(result.bbox[0] >= 300 && result.bbox[2] <= 315);
    }

    #[test]
    fn static_decorations_inside_the_band_are_ignored() {
        // A full-width rule present in both frames (the Home view's line
        // above the edit line) must not merge everything into one cluster.
        let (mut b, mut a) = line(&[2, 10, 2], &[4]);
        for x in 0..W {
            b.pixels[(195 * W + x) as usize] = 0x808080;
            a.pixels[(195 * W + x) as usize] = 0x808080;
        }
        let (expr, result) = history_line(&b, &a).expect("two clusters");
        assert_eq!(expr.bbox[0], 2);
        assert!(result.bbox[0] >= 300);
    }

    #[test]
    fn a_status_bar_change_is_not_mistaken_for_the_result() {
        // The clock (top right) changes in the same frame pair.
        let (b, mut a) = line(&[2, 10, 2], &[4]);
        glyph(&mut a, 300, 4, 7);
        let (_, result) = history_line(&b, &a).unwrap();
        assert!(result.bbox[1] >= 190, "{:?}", result.bbox);
        let (b2, a2) = line(&[2, 10, 2], &[4]);
        assert!(result.same_as(&history_line(&b2, &a2).unwrap().1));
    }

    #[test]
    fn verdict_requires_match_control_and_distinct_expression() {
        let a = line(&[2, 10, 2], &[4]);
        let b = line(&[4], &[4]);
        let c = line(&[2, 10, 3], &[5]);
        let v = verify_calculation((&a.0, &a.1), (&b.0, &b.1), (&c.0, &c.1));
        assert!(v.verified, "{v:?}");
        // A wrong result (5) is caught.
        let wrong = line(&[2, 10, 2], &[5]);
        let v = verify_calculation((&wrong.0, &wrong.1), (&b.0, &b.1), (&c.0, &c.1));
        assert!(!v.verified && !v.result_matches_reference);
        // A control that equals the result is not a valid control.
        let v = verify_calculation((&a.0, &a.1), (&b.0, &b.1), (&b.0, &b.1));
        assert!(!v.verified && !v.result_differs_from_control);
        // No change at all: nothing to verify.
        let none = (blank(), blank());
        let v = verify_calculation((&none.0, &none.1), (&b.0, &b.1), (&c.0, &c.1));
        assert!(!v.verified && v.a_result.is_none());
    }

    #[test]
    fn png_round_trip_preserves_pixels() {
        let (_, a) = line(&[2, 10, 2], &[4]);
        let dir = std::env::temp_dir().join(format!("prime-verify-{}", std::process::id()));
        let path = dir.join("f.png");
        a.save_png(&path).unwrap();
        let back = load_png(&path).unwrap();
        assert_eq!(back, a);
        let _ = std::fs::remove_dir_all(dir);
    }
}
