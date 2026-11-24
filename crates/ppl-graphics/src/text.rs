//! Text rendering for TEXTOUT_P, PRINT and dialogs.
//!
//! A bundled DejaVu face keeps a fresh installation usable. Callers can
//! select validated Prime Sans faces from their locally installed G1 resource
//! archive. The selected renderer uses bounded, thread-local face state and
//! the independently verified G1 scan-conversion/coverage contract. Vendor
//! font bytes are never bundled; selection errors propagate to the caller.

pub mod prime;
mod raster;
pub use prime::with_prime_fonts;

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
    antialias: bool,
}

type GlyphKey = (u32, u32);

fn glyph(c: char, px: f32) -> std::sync::Arc<Glyph> {
    if let Some(glyph) = prime::active_glyph(c, px) {
        return glyph;
    }
    fallback_glyph(c as u32, px)
}

fn unit_glyph(unit: u16, px: f32) -> std::sync::Arc<Glyph> {
    if let Some(glyph) = prime::active_unit_glyph(unit, px) {
        return glyph;
    }
    fallback_glyph(u32::from(unit), px)
}

fn fallback_glyph(codepoint: u32, px: f32) -> std::sync::Arc<Glyph> {
    static CACHE: OnceLock<Mutex<HashMap<GlyphKey, std::sync::Arc<Glyph>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (codepoint, px.to_bits());
    if let Some(g) = cache.lock().unwrap().get(&key) {
        return g.clone();
    }
    let (m, bitmap) = match char::from_u32(codepoint) {
        Some(c) => font().rasterize(c, px),
        // The real face's missing glyph, without inventing a Unicode scalar
        // or converting the requested16-bit unit to a replacement character.
        None => font().rasterize_indexed(0, px),
    };
    let g = std::sync::Arc::new(Glyph {
        width: m.width,
        height: m.height,
        xmin: m.xmin,
        ymin: m.ymin,
        advance: m.advance_width,
        bitmap,
        antialias: false,
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
    if let Some((height, _)) = prime::active_metrics(px) {
        return height;
    }
    let lm = font()
        .horizontal_line_metrics(px)
        .expect("horizontal metrics");
    (lm.ascent - lm.descent).ceil() as i64
}

/// G1 TEXTOUT cell heights are logical font metrics, distinct from FreeType's
/// recommended multiline spacing. Original controls cover every font code.
pub fn calculator_cell_height(px: f32) -> i64 {
    match px.round() as u32 {
        10 => 10,
        12 => 12,
        14 => 16,
        16 => 20,
        18 => 22,
        20 => 24,
        22 => 26,
        _ => line_height(px),
    }
}

fn ascent(px: f32) -> i64 {
    if let Some((_, ascent)) = prime::active_metrics(px) {
        return ascent;
    }
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

/// Measure a calculator UTF16 buffer without replacing isolated units. HP's
/// numeric formatter measures each16-bit font index, including missing glyphs.
/// This does not turn the buffer into displayable Unicode or change its units.
pub fn measure_utf16_units(units: &[u16], px: f32) -> i64 {
    units
        .iter()
        .map(|unit| {
            prime::active_unit_advance(*unit, px).unwrap_or_else(|| {
                char::from_u32(u32::from(*unit)).map_or_else(
                    || font().metrics_indexed(0, px).advance_width,
                    |character| glyph(character, px).advance,
                )
            })
        })
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
    let mut end = x;
    for (row, line) in text.split('\n').enumerate() {
        end = draw_line(
            g,
            line.chars().map(|c| glyph(c, px)),
            x,
            y.saturating_add((row as i64).saturating_mul(line_height(px))),
            px,
            color,
            max_width,
            bg,
            false,
        );
    }
    end
}

/// Render calculator16-bit font indices directly, including isolated surrogate
/// units. A valid surrogate pair remains two indices, as in the original G1.
#[allow(clippy::too_many_arguments)]
pub fn draw_utf16_units(
    g: &mut Grob,
    units: &[u16],
    x: i64,
    y: i64,
    px: f32,
    color: Color,
    max_width: Option<i64>,
    bg: Option<Color>,
) -> i64 {
    let mut end = x;
    for (row, line) in units.split(|u| *u == 10).enumerate() {
        end = draw_line(
            g,
            line.iter().map(|u| unit_glyph(*u, px)),
            x,
            y.saturating_add((row as i64).saturating_mul(calculator_cell_height(px))),
            px,
            color,
            max_width,
            bg,
            true,
        );
    }
    end
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    g: &mut Grob,
    glyphs: impl Iterator<Item = std::sync::Arc<Glyph>> + Clone,
    x: i64,
    y: i64,
    px: f32,
    color: Color,
    max_width: Option<i64>,
    bg: Option<Color>,
    clip_pixels: bool,
) -> i64 {
    let start_x = x;
    let asc = ascent(px);
    let width = glyphs.clone().map(|gl| gl.advance).sum::<f32>().round() as i64;
    let limit = max_width.map(|m| x.saturating_add(m)).unwrap_or(i64::MAX);
    // Explicit zero creates an empty drawing region, but returns the complete
    // measured end. Omitting the argument creates an unrestricted region.
    if clip_pixels && max_width == Some(0) {
        return x.saturating_add(width);
    }
    if let Some(bg) = bg {
        if clip_pixels {
            g.fill_rect(
                x.saturating_sub(1),
                y.saturating_sub(1),
                x.saturating_add(width).min(limit),
                y.saturating_add(calculator_cell_height(px))
                    .saturating_sub(1),
                bg,
            );
        } else {
            g.fill_rect(
                x,
                y,
                x.saturating_add(width).min(limit).saturating_sub(1),
                y.saturating_add(line_height(px)).saturating_sub(1),
                bg,
            );
        }
    }
    let mut pen = x as f32;
    for gl in glyphs {
        if (!clip_pixels && (pen + gl.advance).round() as i64 > limit)
            || (clip_pixels && pen.round() as i64 >= limit)
        {
            break;
        }
        let base_x = (pen.round() as i64).saturating_add(gl.xmin as i64);
        let base_y = y
            .saturating_add(asc)
            .saturating_sub(gl.height as i64)
            .saturating_sub(gl.ymin as i64);
        // The G1 clips bitmap columns against the remaining advance width
        // before applying the glyph's left bearing. Thus ink can extend past
        // the returned end when that bearing is positive.
        let columns = if clip_pixels {
            gl.width
                .min(limit.saturating_sub(pen.round() as i64).max(0) as usize)
        } else {
            gl.width
        };
        for row in 0..gl.height {
            for col in 0..columns {
                let coverage = gl.bitmap[row * gl.width + col];
                let x = base_x.saturating_add(col as i64);
                let y = base_y.saturating_add(row as i64);
                if gl.antialias {
                    if coverage != 0 {
                        if let Some(background) = g.get(x, y) {
                            g.set(x, y, prime::blend(background, color, coverage));
                        }
                    }
                } else if coverage >= 128 {
                    g.set(x, y, color);
                }
            }
        }
        pen += gl.advance;
    }
    if clip_pixels {
        start_x.saturating_add(width).min(limit)
    } else {
        pen.round() as i64
    }
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
