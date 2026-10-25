//! Headless graphics for the HP Prime emulator.
//!
//! * [`Grob`]: a pixel buffer (the Prime's graphic objects G0–G9).
//! * Integer drawing primitives with explicit clipping.
//! * [`text`]: TEXTOUT rendering with a bundled open font (approximate
//!   metrics, not the HP system font).
//! * PNG encoding and screenshot comparison.
//!
//! Nothing here opens a window, so every primitive is unit-testable.

pub mod text;

pub mod incremental;

use std::io::Write;

pub const SCREEN_W: usize = 320;
pub const SCREEN_H: usize = 240;

/// Colours are `0xTTRRGGBB`. The top byte is transparency as used by
/// `RGB(r,g,b,a)` (0 = opaque); the semantics of the alpha byte are
/// **unverified** against the G1.
pub type Color = u32;

pub const WHITE: Color = 0x00FF_FFFF;
pub const BLACK: Color = 0x0000_0000;

pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

pub fn channels(c: Color) -> (u8, u8, u8) {
    ((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Quantises to the 16-bit RGB565 format of the G1 panel, then expands back.
pub fn to_rgb565(c: Color) -> u16 {
    let (r, g, b) = channels(c);
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grob {
    w: usize,
    h: usize,
    px: Vec<Color>,
}

impl Grob {
    pub fn new(w: usize, h: usize, fill: Color) -> Grob {
        Grob {
            w,
            h,
            px: vec![fill & 0x00FF_FFFF; w * h],
        }
    }

    pub fn screen() -> Grob {
        Grob::new(SCREEN_W, SCREEN_H, WHITE)
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn pixels(&self) -> &[Color] {
        &self.px
    }

    pub fn pixels_mut(&mut self) -> &mut [Color] {
        &mut self.px
    }

    pub fn get(&self, x: i64, y: i64) -> Option<Color> {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        Some(self.px[y as usize * self.w + x as usize])
    }

    /// Writes a pixel, blending by the colour's transparency byte. Pixels
    /// outside the grob are clipped silently.
    pub fn set(&mut self, x: i64, y: i64, c: Color) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        let t = (c >> 24) & 0xFF;
        self.px[i] = match t {
            0 => c & 0x00FF_FFFF,
            255 => return,
            _ => blend(self.px[i], c & 0x00FF_FFFF, 255 - t),
        };
    }

    pub fn fill(&mut self, c: Color) {
        if (c >> 24) & 0xFF == 0 {
            self.px.fill(c);
        } else {
            for y in 0..self.h as i64 {
                for x in 0..self.w as i64 {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn hline(&mut self, x1: i64, x2: i64, y: i64, c: Color) {
        let (a, b) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
        let a = a.max(0);
        let b = b.min(self.w as i64 - 1);
        for x in a..=b {
            self.set(x, y, c);
        }
    }

    /// Filled rectangle with inclusive corners.
    pub fn fill_rect(&mut self, x1: i64, y1: i64, x2: i64, y2: i64, c: Color) {
        let (ya, yb) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
        for y in ya.max(0)..=yb.min(self.h as i64 - 1) {
            self.hline(x1, x2, y, c);
        }
    }

    /// `RECT_P` semantics: fill the interior with `fill`, then draw a 1-pixel
    /// border with `edge` (inclusive corners).
    pub fn rect(&mut self, x1: i64, y1: i64, x2: i64, y2: i64, edge: Color, fill: Color) {
        self.fill_rect(x1, y1, x2, y2, fill);
        if edge != fill {
            self.hline(x1, x2, y1, edge);
            self.hline(x1, x2, y2, edge);
            let (ya, yb) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
            for y in ya..=yb {
                self.set(x1, y, edge);
                self.set(x2, y, edge);
            }
        }
    }

    /// Bresenham line, both endpoints included.
    pub fn line(&mut self, x1: i64, y1: i64, x2: i64, y2: i64, c: Color) {
        let dx = (x2 - x1).abs();
        let dy = -(y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x1, y1);
        // Bound the walk so huge off-screen coordinates cannot hang.
        let max_steps = dx.max(-dy) + 1;
        for _ in 0..=max_steps {
            self.set(x, y, c);
            if x == x2 && y == y2 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Midpoint circle outline.
    pub fn circle(&mut self, xc: i64, yc: i64, r: i64, c: Color) {
        if r < 0 {
            return;
        }
        let (mut x, mut y, mut d) = (r, 0i64, 1 - r);
        while x >= y {
            for (px, py) in [
                (x, y),
                (y, x),
                (-y, x),
                (-x, y),
                (-x, -y),
                (-y, -x),
                (y, -x),
                (x, -y),
            ] {
                self.set(xc + px, yc + py, c);
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

    pub fn fill_circle(&mut self, xc: i64, yc: i64, r: i64, c: Color) {
        if r < 0 {
            return;
        }
        for dy in -r..=r {
            let half = (((r * r - dy * dy) as f64).sqrt()).floor() as i64;
            self.hline(xc - half, xc + half, yc + dy, c);
        }
    }

    /// Arc from `a1` to `a2` radians, counter-clockwise as seen on screen.
    pub fn arc(&mut self, xc: i64, yc: i64, r: i64, a1: f64, a2: f64, c: Color) {
        if r < 0 {
            return;
        }
        let (start, mut end) = (a1, a2);
        while end < start {
            end += std::f64::consts::TAU;
        }
        let steps = ((end - start).abs() * r as f64 * 2.0).ceil().max(1.0) as i64;
        let mut last = None;
        for i in 0..=steps {
            let a = start + (end - start) * i as f64 / steps as f64;
            let x = xc + (r as f64 * a.cos()).round() as i64;
            let y = yc - (r as f64 * a.sin()).round() as i64;
            if last != Some((x, y)) {
                self.set(x, y, c);
                last = Some((x, y));
            }
        }
    }

    /// Even-odd scanline polygon fill (pixel centres).
    pub fn fill_polygon(&mut self, pts: &[(f64, f64)], c: Color) {
        if pts.len() < 3 {
            return;
        }
        let ymin = pts
            .iter()
            .map(|p| p.1)
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as i64;
        let ymax = pts
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .min(self.h as f64 - 1.0) as i64;
        for y in ymin..=ymax {
            let yc = y as f64 + 0.5;
            let mut xs = Vec::new();
            for i in 0..pts.len() {
                let (x1, y1) = pts[i];
                let (x2, y2) = pts[(i + 1) % pts.len()];
                if (y1 <= yc && y2 > yc) || (y2 <= yc && y1 > yc) {
                    xs.push(x1 + (yc - y1) * (x2 - x1) / (y2 - y1));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.chunks(2) {
                if let [a, b] = pair {
                    let xa = (a - 0.5).ceil() as i64;
                    let xb = (b - 0.5).floor() as i64;
                    if xa <= xb {
                        self.hline(xa, xb, y, c);
                    }
                }
            }
        }
    }

    pub fn invert(&mut self, x1: i64, y1: i64, x2: i64, y2: i64) {
        let (xa, xb) = (x1.min(x2).max(0), x1.max(x2).min(self.w as i64 - 1));
        let (ya, yb) = (y1.min(y2).max(0), y1.max(y2).min(self.h as i64 - 1));
        for y in ya..=yb {
            for x in xa..=xb {
                let i = y as usize * self.w + x as usize;
                self.px[i] ^= 0x00FF_FFFF;
            }
        }
    }

    /// Copies `src` region `[sx1,sx2)×[sy1,sy2)` scaled into the destination
    /// region `[dx1,dx2)×[dy1,dy2)` with nearest-neighbour sampling. Source
    /// pixels equal to `transparent` are skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn blit(
        &mut self,
        dst: (i64, i64, i64, i64),
        src: &Grob,
        s: (i64, i64, i64, i64),
        transparent: Option<Color>,
        alpha: u8,
    ) {
        let (dx1, dy1, dx2, dy2) = dst;
        let (sx1, sy1, sx2, sy2) = s;
        let (dw, dh) = (dx2 - dx1, dy2 - dy1);
        let (sw, sh) = (sx2 - sx1, sy2 - sy1);
        if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
            return;
        }
        for y in 0..dh {
            let ty = dy1 + y;
            if ty < 0 || ty >= self.h as i64 {
                continue;
            }
            let syy = sy1 + y * sh / dh;
            for x in 0..dw {
                let tx = dx1 + x;
                if tx < 0 || tx >= self.w as i64 {
                    continue;
                }
                let sxx = sx1 + x * sw / dw;
                let Some(p) = src.get(sxx, syy) else { continue };
                if transparent.is_some_and(|t| t & 0x00FF_FFFF == p) {
                    continue;
                }
                if alpha == 255 {
                    self.px[ty as usize * self.w + tx as usize] = p;
                } else {
                    let i = ty as usize * self.w + tx as usize;
                    self.px[i] = blend(self.px[i], p, alpha as u32);
                }
            }
        }
    }

    pub fn sub(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Grob {
        let (xa, xb) = (x1.min(x2), x1.max(x2));
        let (ya, yb) = (y1.min(y2), y1.max(y2));
        let (w, h) = ((xb - xa).max(0) as usize, (yb - ya).max(0) as usize);
        let mut g = Grob::new(w, h, WHITE);
        g.blit(
            (0, 0, w as i64, h as i64),
            self,
            (xa, ya, xb, yb),
            None,
            255,
        );
        g
    }

    pub fn write_png<W: Write>(&self, w: W) -> Result<(), png::EncodingError> {
        let mut enc = png::Encoder::new(w, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header()?;
        let mut data = Vec::with_capacity(self.px.len() * 3);
        for &p in &self.px {
            let (r, g, b) = channels(p);
            data.extend_from_slice(&[r, g, b]);
        }
        writer.write_image_data(&data)
    }

    pub fn save_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        let f = std::io::BufWriter::new(std::fs::File::create(path)?);
        self.write_png(f).map_err(std::io::Error::other)
    }

    /// Decodes an 8-bit RGB/RGBA PNG (for device screenshots and fixtures).
    pub fn read_png(bytes: &[u8]) -> Result<Grob, String> {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let (w, h) = (info.width as usize, info.height as usize);
        let step = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            other => return Err(format!("unsupported PNG colour type {other:?}")),
        };
        let mut g = Grob::new(w, h, WHITE);
        for (i, p) in g.px.iter_mut().enumerate() {
            let o = i * step;
            *p = if step >= 3 {
                rgb(buf[o], buf[o + 1], buf[o + 2])
            } else {
                rgb(buf[o], buf[o], buf[o])
            };
        }
        Ok(g)
    }
}

impl Grob {
    /// Decodes an HP Prime USB screenshot (`FC 08` reply). The calculator
    /// sends a 16-bit *grayscale* PNG whose samples are really RGB565
    /// pixels, big-endian (observed on a G1, firmware build 15157).
    pub fn from_prime_screenshot(bytes: &[u8]) -> Result<Grob, String> {
        PrimeScreenshotDecoder::default()
            .decode(bytes, None)
            .map(|(g, _)| g)
    }
}

/// Reusable PNG scratch storage. Packed screenshot samples remain untouched by
/// PNG color/gamma transforms; RGB565 conversion runs exactly once.
#[derive(Default)]
pub struct PrimeScreenshotDecoder {
    buffer: Vec<u8>,
    stage: &'static str,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScreenshotDecodeTimings {
    pub decompression: std::time::Duration,
    pub conversion: std::time::Duration,
}

impl PrimeScreenshotDecoder {
    pub fn failure_stage(&self) -> &'static str {
        self.stage
    }

    pub fn decode(
        &mut self,
        bytes: &[u8],
        expected: Option<(usize, usize)>,
    ) -> Result<(Grob, ScreenshotDecodeTimings), String> {
        self.stage = "decompression";
        let start = std::time::Instant::now();
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        if reader.info().animation_control.is_some() {
            return Err("animated screenshots are unsupported".into());
        }
        let (w, h) = (reader.info().width as usize, reader.info().height as usize);
        if expected.is_some_and(|dims| dims != (w, h)) {
            return Err(format!("unexpected screenshot dimensions {w}×{h}"));
        }
        let pixels = w
            .checked_mul(h)
            .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
            .ok_or("screenshot dimensions exceed decode limit")?;
        let size = reader
            .output_buffer_size()
            .filter(|n| *n <= 64 * 1024 * 1024)
            .ok_or("screenshot decoded buffer too large")?;
        self.buffer.resize(size, 0);
        let info = reader
            .next_frame(&mut self.buffer)
            .map_err(|e| e.to_string())?;
        reader.finish().map_err(|e| e.to_string())?;
        let decompression = start.elapsed();
        self.stage = "conversion";
        let conversion = std::time::Instant::now();
        let channels = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => return Err(format!("unsupported screenshot color type {other:?}")),
        };
        let sample_bytes = match info.bit_depth {
            png::BitDepth::Eight => 1,
            png::BitDepth::Sixteen => 2,
            _ => return Err("unexpected expanded PNG bit depth".into()),
        };
        let step = channels * sample_bytes;
        let required = pixels.checked_mul(step).ok_or("PNG buffer size overflow")?;
        if info.buffer_size() != required
            || info.line_size != w * step
            || self.buffer.len() < required
        {
            return Err("invalid screenshot row stride or decoded buffer length".into());
        }
        let packed = info.color_type == png::ColorType::Grayscale && sample_bytes == 2;
        let mut g = Grob::new(w, h, BLACK);
        for (pixel, sample) in
            g.px.iter_mut()
                .zip(self.buffer[..required].chunks_exact(step))
        {
            *pixel = if packed {
                from_rgb565(u16::from_be_bytes([sample[0], sample[1]]))
            } else if channels >= 3 {
                rgb(sample[0], sample[sample_bytes], sample[2 * sample_bytes])
            } else {
                rgb(sample[0], sample[0], sample[0])
            };
        }
        Ok((
            g,
            ScreenshotDecodeTimings {
                decompression,
                conversion: conversion.elapsed(),
            },
        ))
    }
}

/// Expands RGB565 to 8-bit channels (bit replication).
pub fn from_rgb565(v: u16) -> Color {
    let r = ((v >> 11) & 31) as u32;
    let g = ((v >> 5) & 63) as u32;
    let b = (v & 31) as u32;
    let (r, g, b) = (
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    );
    r << 16 | g << 8 | b
}

fn blend(dst: Color, src: Color, src_weight: u32) -> Color {
    let mix = |shift: u32| {
        let d = (dst >> shift) & 0xFF;
        let s = (src >> shift) & 0xFF;
        (s * src_weight + d * (255 - src_weight)) / 255
    };
    mix(16) << 16 | mix(8) << 8 | mix(0)
}

/// Result of comparing two screens after RGB565 quantisation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenDiff {
    pub width: usize,
    pub height: usize,
    pub mismatched: usize,
    pub first_mismatch: Option<(usize, usize)>,
}

/// Detailed result of comparing two screens after RGB565 quantisation,
/// including match percentage, total pixels, and mismatch bounding rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenDiffDetailed {
    pub width: usize,
    pub height: usize,
    pub total_pixels: usize,
    pub matching_pixels: usize,
    pub mismatching_pixels: usize,
    pub mismatch_percentage: f64,
    pub first_mismatch: Option<(usize, usize)>,
    pub mismatch_bounds: Option<[usize; 4]>,
}

pub fn compare_rgb565(a: &Grob, b: &Grob) -> Option<ScreenDiff> {
    if a.w != b.w || a.h != b.h {
        return None;
    }
    let mut mismatched = 0;
    let mut first = None;
    for (i, (pa, pb)) in a.px.iter().zip(&b.px).enumerate() {
        if to_rgb565(*pa) != to_rgb565(*pb) {
            mismatched += 1;
            if first.is_none() {
                first = Some((i % a.w, i / a.w));
            }
        }
    }
    Some(ScreenDiff {
        width: a.w,
        height: a.h,
        mismatched,
        first_mismatch: first,
    })
}

pub fn compare_screens_detailed(a: &Grob, b: &Grob) -> Option<ScreenDiffDetailed> {
    if a.w != b.w || a.h != b.h {
        return None;
    }
    let w = a.w;
    let h = a.h;
    let total = w * h;
    let mut mismatched = 0;
    let mut first = None;
    let mut min_x = usize::MAX;
    let mut min_y = usize::MAX;
    let mut max_x = 0;
    let mut max_y = 0;

    for y in 0..h {
        for x in 0..w {
            let pa = a.px[y * w + x];
            let pb = b.px[y * w + x];
            if to_rgb565(pa) != to_rgb565(pb) {
                mismatched += 1;
                if first.is_none() {
                    first = Some((x, y));
                }
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    let matching = total - mismatched;
    let pct = if total > 0 {
        (mismatched as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    let bounds = if mismatched > 0 {
        Some([min_x, min_y, max_x, max_y])
    } else {
        None
    };

    Some(ScreenDiffDetailed {
        width: w,
        height: h,
        total_pixels: total,
        matching_pixels: matching,
        mismatching_pixels: mismatched,
        mismatch_percentage: pct,
        first_mismatch: first,
        mismatch_bounds: bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_fills_with_edge_and_fill() {
        let mut g = Grob::screen();
        g.rect(10, 10, 50, 50, rgb(0, 0, 255), rgb(0, 0, 255));
        assert_eq!(g.get(30, 30), Some(rgb(0, 0, 255)));
        assert_eq!(g.get(50, 50), Some(rgb(0, 0, 255)));
        assert_eq!(g.get(51, 51), Some(WHITE));
        g.rect(0, 0, 4, 4, BLACK, rgb(255, 0, 0));
        assert_eq!(g.get(0, 0), Some(BLACK));
        assert_eq!(g.get(2, 2), Some(rgb(255, 0, 0)));
    }

    #[test]
    fn line_includes_endpoints_and_clips() {
        let mut g = Grob::screen();
        g.line(0, 0, 100, 0, BLACK);
        assert_eq!(g.get(50, 0), Some(BLACK));
        assert_eq!(g.get(100, 0), Some(BLACK));
        assert_eq!(g.get(50, 50), Some(WHITE));
        g.line(-1000, 5, 1000, 5, BLACK); // clipped, no panic
        assert_eq!(g.get(319, 5), Some(BLACK));
    }

    #[test]
    fn circles_and_polygons() {
        let mut g = Grob::screen();
        g.fill_circle(100, 100, 20, rgb(255, 0, 0));
        assert_eq!(g.get(100, 100), Some(rgb(255, 0, 0)));
        g.circle(200, 100, 10, BLACK);
        assert_eq!(g.get(210, 100), Some(BLACK));
        assert_eq!(g.get(200, 100), Some(WHITE));
        g.fill_polygon(&[(10.0, 10.0), (20.0, 10.0), (15.0, 20.0)], rgb(0, 255, 0));
        assert_eq!(g.get(15, 15), Some(rgb(0, 255, 0)));
    }

    #[test]
    fn blit_scales_and_skips_transparent() {
        let mut src = Grob::new(20, 20, rgb(255, 0, 0));
        src.set(0, 0, BLACK);
        let mut dst = Grob::screen();
        dst.blit((10, 10, 20, 20), &src, (0, 0, 20, 20), None, 255);
        assert_eq!(dst.get(15, 15), Some(rgb(255, 0, 0)));
        assert_eq!(dst.get(20, 20), Some(WHITE));
        let mut dst2 = Grob::screen();
        dst2.blit((0, 0, 20, 20), &src, (0, 0, 20, 20), Some(BLACK), 255);
        assert_eq!(dst2.get(0, 0), Some(WHITE));
    }

    #[test]
    fn rgb565_expansion() {
        assert_eq!(from_rgb565(0xFFFF), WHITE);
        assert_eq!(from_rgb565(0x0000), BLACK);
        assert_eq!(from_rgb565(0xF800), rgb(255, 0, 0));
        assert_eq!(to_rgb565(from_rgb565(0x630C)), 0x630C);
    }

    #[test]
    fn invert_and_png_roundtrip() {
        let mut g = Grob::screen();
        g.invert(0, 0, 10, 10);
        assert_eq!(g.get(5, 5), Some(BLACK));
        let mut buf = Vec::new();
        g.write_png(&mut buf).unwrap();
        let back = Grob::read_png(&buf).unwrap();
        assert_eq!(back, g);
        assert_eq!(compare_rgb565(&g, &back).unwrap().mismatched, 0);
    }

    fn encode_16bit_gray_png(w: u32, h: u32, pixels_u16: &[u16]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Sixteen);
        let mut writer = enc.write_header().unwrap();
        let mut byte_data = Vec::with_capacity(pixels_u16.len() * 2);
        for &px in pixels_u16 {
            byte_data.extend_from_slice(&px.to_be_bytes());
        }
        writer.write_image_data(&byte_data).unwrap();
        drop(writer);
        buf
    }

    #[test]
    fn test_from_prime_screenshot_dimensions_and_colors() {
        let w = 320;
        let h = 240;
        let mut samples = vec![0u16; w * h];
        // Set specific test pixels:
        // (0,0): Black
        samples[0] = 0x0000;
        // (1,0): White
        samples[1] = 0xFFFF;
        // (2,0): Red
        samples[2] = 0xF800;
        // (3,0): Green
        samples[3] = 0x07E0;
        // (4,0): Blue
        samples[4] = 0x001F;
        // (5,0): Yellow (Red + Green)
        samples[5] = 0xFFE0;
        // (6,0): Cyan (Green + Blue)
        samples[6] = 0x07FF;
        // (7,0): Magenta (Red + Blue)
        samples[7] = 0xF81F;

        let png_bytes = encode_16bit_gray_png(w as u32, h as u32, &samples);
        let grob = Grob::from_prime_screenshot(&png_bytes).expect("decode prime screenshot");

        assert_eq!(grob.width(), 320);
        assert_eq!(grob.height(), 240);
        assert_eq!(grob.get(0, 0), Some(BLACK));
        assert_eq!(grob.get(1, 0), Some(WHITE));
        assert_eq!(grob.get(2, 0), Some(rgb(255, 0, 0)));
        assert_eq!(grob.get(3, 0), Some(rgb(0, 255, 0)));
        assert_eq!(grob.get(4, 0), Some(rgb(0, 0, 255)));
        assert_eq!(grob.get(5, 0), Some(rgb(255, 255, 0)));
        assert_eq!(grob.get(6, 0), Some(rgb(0, 255, 255)));
        assert_eq!(grob.get(7, 0), Some(rgb(255, 0, 255)));
    }

    #[test]
    fn test_from_prime_screenshot_unexpected_dimensions() {
        let w = 100;
        let h = 50;
        let samples = vec![0xF800u16; w * h]; // All red
        let png_bytes = encode_16bit_gray_png(w as u32, h as u32, &samples);
        let grob = Grob::from_prime_screenshot(&png_bytes).expect("decode custom dimension");

        assert_eq!(grob.width(), 100);
        assert_eq!(grob.height(), 50);
        assert_eq!(grob.get(50, 25), Some(rgb(255, 0, 0)));
    }

    #[test]
    fn test_from_prime_screenshot_fallback_8bit_rgb() {
        let mut original = Grob::new(40, 30, rgb(100, 150, 200));
        original.set(5, 5, rgb(255, 0, 0));
        let mut buf = Vec::new();
        original.write_png(&mut buf).unwrap();

        // 8-bit RGB PNG should be handled by the fallback branch in from_prime_screenshot
        let grob = Grob::from_prime_screenshot(&buf).expect("decode 8-bit png fallback");
        assert_eq!(grob.width(), 40);
        assert_eq!(grob.height(), 30);
        assert_eq!(grob.get(5, 5), Some(rgb(255, 0, 0)));
        assert_eq!(grob.get(0, 0), Some(rgb(100, 150, 200)));
    }

    #[test]
    fn test_from_prime_screenshot_malformed() {
        assert!(Grob::from_prime_screenshot(&[]).is_err());
        assert!(Grob::from_prime_screenshot(&[0x89, b'P', b'N', b'G']).is_err());
        assert!(Grob::from_prime_screenshot(&[1, 2, 3, 4, 5, 6, 7, 8]).is_err());
    }
    #[test]
    fn screenshot_all_rgb565_values_round_trip_without_channel_loss() {
        for v in 0..=u16::MAX {
            assert_eq!(to_rgb565(from_rgb565(v)), v);
        }
    }

    #[test]
    fn screenshot_fixture_has_known_colors_and_preserves_rows() {
        let raw = include_bytes!("../../../tests/fixtures/prime-rgb565.png");
        let image = Grob::from_prime_screenshot(raw).unwrap();
        let colors = [
            0, 0xffff, 0xf800, 0x07e0, 0x001f, 0x8410, 0xffe0, 0xf81f, 0x07ff,
        ];
        for y in 0..240 {
            for x in 0..320 {
                assert_eq!(
                    image.get(x as i64, y as i64),
                    Some(from_rgb565(colors[(x / 17 + y / 13) % 9]))
                );
            }
        }
        let mut decoder = PrimeScreenshotDecoder::default();
        assert!(decoder.decode(raw, Some((160, 120))).is_err());
        assert_eq!(decoder.decode(raw, Some((320, 240))).unwrap().0, image);
        assert!(decoder.decode(&raw[..raw.len() / 2], None).is_err());
    }

    #[test]
    fn test_compare_screens_detailed_metrics_and_bounds() {
        let mut a = Grob::new(320, 240, 0x00112233);
        let b = Grob::new(320, 240, 0x00112233);

        // Identical screens
        let diff = compare_screens_detailed(&a, &b).unwrap();
        assert_eq!(diff.total_pixels, 76800);
        assert_eq!(diff.matching_pixels, 76800);
        assert_eq!(diff.mismatching_pixels, 0);
        assert_eq!(diff.mismatch_percentage, 0.0);
        assert_eq!(diff.first_mismatch, None);
        assert_eq!(diff.mismatch_bounds, None);

        // Single pixel mismatch at (15, 25)
        a.set(15, 25, 0x00FFFFFF);
        let diff = compare_screens_detailed(&a, &b).unwrap();
        assert_eq!(diff.total_pixels, 76800);
        assert_eq!(diff.matching_pixels, 76799);
        assert_eq!(diff.mismatching_pixels, 1);
        assert!((diff.mismatch_percentage - (1.0 / 76800.0 * 100.0)).abs() < 1e-6);
        assert_eq!(diff.first_mismatch, Some((15, 25)));
        assert_eq!(diff.mismatch_bounds, Some([15, 25, 15, 25]));

        // Multiple pixel mismatches bounding box
        a.set(100, 50, 0x00FFFFFF);
        a.set(5, 80, 0x00FFFFFF);
        let diff = compare_screens_detailed(&a, &b).unwrap();
        assert_eq!(diff.mismatching_pixels, 3);
        assert_eq!(diff.first_mismatch, Some((15, 25)));
        assert_eq!(diff.mismatch_bounds, Some([5, 25, 100, 80]));

        // Dimension mismatch
        let c = Grob::new(160, 120, 0);
        assert!(compare_screens_detailed(&a, &c).is_none());
    }
}
