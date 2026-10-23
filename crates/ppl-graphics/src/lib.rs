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
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::IDENTITY);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        if info.color_type != png::ColorType::Grayscale || info.bit_depth != png::BitDepth::Sixteen
        {
            // Already a normal image (e.g. reduced formats): decode generically.
            return Grob::read_png(bytes);
        }
        let (w, h) = (info.width as usize, info.height as usize);
        let mut g = Grob::new(w, h, WHITE);
        for (i, p) in g.px.iter_mut().enumerate() {
            let v = u16::from_be_bytes([buf[i * 2], buf[i * 2 + 1]]);
            *p = from_rgb565(v);
        }
        Ok(g)
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
}
