//! Text rendering for TEXTOUT_P, PRINT and dialogs.
//!
//! The HP system font is proprietary and is not bundled. DejaVu Sans
//! (Bitstream Vera licence, see `assets/DejaVuSans-LICENSE.txt`) is rasterised
//! without anti-aliasing at the pixel size for each TEXTOUT font code. Glyph
//! shapes and advance widths therefore differ from the calculator; the
//! compatibility matrix lists text output as APPROXIMATE.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::{Color, Grob};

static FONT_BYTES: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// The bundled DejaVu Sans font file (also used by the IDE as a glyph
/// fallback for symbols missing from its UI and code fonts).
pub fn dejavu_sans_ttf() -> &'static [u8] {
    FONT_BYTES
}

fn font() -> &'static fontdue::Font {
    static FONT: OnceLock<fontdue::Font> = OnceLock::new();
    FONT.get_or_init(|| {
        fontdue::Font::from_bytes(FONT_BYTES, fontdue::FontSettings::default())
            .expect("bundled font parses")
    })
}

struct Glyph {
    width: usize,
    height: usize,
    xmin: i32,
    ymin: i32,
    advance: f32,
    bitmap: Vec<u8>,
}

type GlyphKey = (char, u32);

fn glyph(c: char, px: f32) -> std::sync::Arc<Glyph> {
    static CACHE: OnceLock<Mutex<HashMap<GlyphKey, std::sync::Arc<Glyph>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (c, px.to_bits());
    if let Some(g) = cache.lock().unwrap().get(&key) {
        return g.clone();
    }
    let (m, bitmap) = font().rasterize(c, px);
    let g = std::sync::Arc::new(Glyph {
        width: m.width,
        height: m.height,
        xmin: m.xmin,
        ymin: m.ymin,
        advance: m.advance_width,
        bitmap,
    });
    cache.lock().unwrap().insert(key, g.clone());
    g
}

/// Pixel size for a TEXTOUT font code: 1 = 10, 2 = 12, … 7 = 22 points
/// (User Guide); 0 uses the system font size `default_px`.
pub fn font_px(code: i64, default_px: f32) -> f32 {
    match code {
        1..=7 => 8.0 + 2.0 * code as f32,
        _ => default_px,
    }
}

pub const DEFAULT_PX: f32 = 12.0;

/// Line height (ascent + descent) at `px`.
pub fn line_height(px: f32) -> i64 {
    let lm = font()
        .horizontal_line_metrics(px)
        .expect("horizontal metrics");
    (lm.ascent - lm.descent).ceil() as i64
}

fn ascent(px: f32) -> i64 {
    font()
        .horizontal_line_metrics(px)
        .expect("horizontal metrics")
        .ascent
        .round() as i64
}

/// Width in pixels of a single line of text.
pub fn measure(text: &str, px: f32) -> i64 {
    text.chars()
        .map(|c| glyph(c, px).advance)
        .sum::<f32>()
        .round() as i64
}

/// Draws `text` with its top-left corner at `(x, y)` and returns the x
/// coordinate just after the last character. `\n` starts a new line at `x`.
/// With `max_width`, characters beyond `x + max_width` are not drawn.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    g: &mut Grob,
    text: &str,
    x: i64,
    y: i64,
    px: f32,
    color: Color,
    max_width: Option<i64>,
    bg: Option<Color>,
) -> i64 {
    let lh = line_height(px);
    let asc = ascent(px);
    let mut pen_y = y;
    let mut end_x = x;
    for line in text.split('\n') {
        let w = measure(line, px);
        let limit = max_width.map(|m| x + m).unwrap_or(i64::MAX);
        if let Some(bg) = bg {
            g.fill_rect(x, pen_y, (x + w).min(limit) - 1, pen_y + lh - 1, bg);
        }
        let mut pen = x as f32;
        for c in line.chars() {
            let gl = glyph(c, px);
            if (pen + gl.advance).round() as i64 > limit {
                break;
            }
            let base_x = pen.round() as i64 + gl.xmin as i64;
            let base_y = pen_y + asc - gl.height as i64 - gl.ymin as i64;
            for row in 0..gl.height {
                for col in 0..gl.width {
                    if gl.bitmap[row * gl.width + col] >= 128 {
                        g.set(base_x + col as i64, base_y + row as i64, color);
                    }
                }
            }
            pen += gl.advance;
        }
        end_x = pen.round() as i64;
        pen_y += lh;
    }
    end_x
}

/// Renders the PRINT terminal: black text on white, newest lines kept
/// visible at the bottom when the output overflows (approximation of the
/// calculator's terminal view).
pub fn render_terminal(lines: &[String]) -> Grob {
    let mut g = Grob::screen();
    let px = DEFAULT_PX;
    let lh = line_height(px);
    let rows: Vec<&str> = lines.iter().flat_map(|l| l.split('\n')).collect();
    let visible = (crate::SCREEN_H as i64 / lh).max(1) as usize;
    let start = rows.len().saturating_sub(visible);
    for (i, line) in rows[start..].iter().enumerate() {
        draw_text(
            &mut g,
            line,
            2,
            i as i64 * lh,
            px,
            crate::BLACK,
            Some(crate::SCREEN_W as i64 - 4),
            None,
        );
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_dark_pixels_and_returns_end() {
        let mut g = Grob::screen();
        let end = draw_text(
            &mut g,
            "X",
            10,
            10,
            font_px(1, DEFAULT_PX),
            crate::BLACK,
            None,
            None,
        );
        assert!(end > 10);
        let dark = (10..26).any(|y| (10..26).any(|x| g.get(x, y) == Some(crate::BLACK)));
        assert!(dark);
        assert!(measure("Hello", 12.0) > measure("Hi", 12.0));
    }
}
