//! Logical 320×240 framebuffer scaling, aspect-ratio preservation,
//! and coordinate transformations for presentation via `softbuffer`.

pub const LOGICAL_WIDTH: u32 = 320;
pub const LOGICAL_HEIGHT: u32 = 240;
pub const ASPECT_RATIO: f64 = 320.0 / 240.0; // 4:3

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScaleFilter {
    #[default]
    Nearest,
    Linear,
}

impl std::str::FromStr for ScaleFilter {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nearest" => Ok(Self::Nearest),
            "linear" => Ok(Self::Linear),
            _ => Err("filter must be nearest or linear".into()),
        }
    }
}

/// Viewport destination within the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub is_integer: bool,
}

impl Viewport {
    /// Calculate integer-scaled viewport preserving 4:3 aspect ratio.
    ///
    /// If the window is smaller than 320×240, falls back to fractional fit.
    pub fn calculate_integer(win_w: u32, win_h: u32) -> Self {
        if win_w == 0 || win_h == 0 {
            return Self {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scale: 1,
                is_integer: true,
            };
        }

        let max_scale_x = win_w / LOGICAL_WIDTH;
        let max_scale_y = win_h / LOGICAL_HEIGHT;
        let scale = max_scale_x.min(max_scale_y);

        if scale >= 1 {
            let width = scale * LOGICAL_WIDTH;
            let height = scale * LOGICAL_HEIGHT;
            let x = (win_w.saturating_sub(width)) / 2;
            let y = (win_h.saturating_sub(height)) / 2;
            Self {
                x,
                y,
                width,
                height,
                scale,
                is_integer: true,
            }
        } else {
            Self::calculate_aspect_fit(win_w, win_h)
        }
    }

    /// Calculate smooth aspect-preserving fit (can scale up or down smoothly).
    pub fn calculate_aspect_fit(win_w: u32, win_h: u32) -> Self {
        if win_w == 0 || win_h == 0 {
            return Self {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scale: 1,
                is_integer: false,
            };
        }

        let win_aspect = win_w as f64 / win_h as f64;
        let (width, height) = if win_aspect > ASPECT_RATIO {
            // Window is wider than 4:3 -> pillarbox (black bars on left/right)
            let h = win_h;
            let w = ((h as f64 * ASPECT_RATIO).round() as u32).min(win_w);
            (w, h)
        } else {
            // Window is taller than 4:3 -> letterbox (black bars on top/bottom)
            let w = win_w;
            let h = ((w as f64 / ASPECT_RATIO).round() as u32).min(win_h);
            (w, h)
        };

        let x = (win_w.saturating_sub(width)) / 2;
        let y = (win_h.saturating_sub(height)) / 2;
        let scale = (width / LOGICAL_WIDTH).max(1);

        Self {
            x,
            y,
            width,
            height,
            scale,
            is_integer: false,
        }
    }

    /// Calculate viewport based on user configuration.
    pub fn calculate(win_w: u32, win_h: u32, integer_scaling: bool) -> Self {
        if integer_scaling {
            Self::calculate_integer(win_w, win_h)
        } else {
            Self::calculate_aspect_fit(win_w, win_h)
        }
    }
}

/// Blits and scales a source framebuffer (320×240) into destination window surface.
///
/// Clears letterbox borders with `border_color` and scales source pixels into `vp`.
#[allow(clippy::too_many_arguments)]
pub fn blit_scaled(
    src: &[u32],
    src_w: usize,
    src_h: usize,
    dst: &mut [u32],
    dst_w: usize,
    dst_h: usize,
    vp: &Viewport,
    border_color: u32,
) {
    if dst_w == 0 || dst_h == 0 || vp.width == 0 || vp.height == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    if !valid_buffers(src, src_w, src_h, dst, dst_w, dst_h, vp) {
        return;
    }

    let vp_x_end = (vp.x + vp.width).min(dst_w as u32) as usize;
    let vp_y_end = (vp.y + vp.height).min(dst_h as u32) as usize;
    let vp_x_start = vp.x as usize;
    let vp_y_start = vp.y as usize;

    // Fast path: 1x unscaled copy
    if vp.is_integer
        && vp.scale == 1
        && src_w == LOGICAL_WIDTH as usize
        && src_h == LOGICAL_HEIGHT as usize
        && vp.width == LOGICAL_WIDTH
        && vp.height == LOGICAL_HEIGHT
    {
        for (y, row) in dst.chunks_exact_mut(dst_w).enumerate().take(dst_h) {
            if y < vp_y_start || y >= vp_y_end {
                row.fill(border_color);
            } else {
                let src_y = y - vp_y_start;
                let src_row = &src[src_y * src_w..(src_y + 1) * src_w];
                row[..vp_x_start].fill(border_color);
                row[vp_x_start..vp_x_end].copy_from_slice(src_row);
                row[vp_x_end..].fill(border_color);
            }
        }
        return;
    }

    // General nearest-neighbor scaler
    for (y, row) in dst.chunks_exact_mut(dst_w).enumerate().take(dst_h) {
        if y < vp_y_start || y >= vp_y_end {
            row.fill(border_color);
        } else {
            row[..vp_x_start].fill(border_color);
            row[vp_x_end..].fill(border_color);

            let vy = (y - vp_y_start) as u32;
            let sy = ((vy as usize * src_h) / vp.height as usize).min(src_h - 1);
            let src_row_start = sy * src_w;

            for (x_idx, dst_px) in row[vp_x_start..vp_x_end].iter_mut().enumerate() {
                let vx = x_idx as u32;
                let sx = ((vx as usize * src_w) / vp.width as usize).min(src_w - 1);
                *dst_px = src[src_row_start + sx];
            }
        }
    }
}

fn valid_buffers(
    src: &[u32],
    sw: usize,
    sh: usize,
    dst: &[u32],
    dw: usize,
    dh: usize,
    vp: &Viewport,
) -> bool {
    sw.checked_mul(sh).is_some_and(|n| n <= src.len())
        && dw.checked_mul(dh).is_some_and(|n| n <= dst.len())
        && vp.x.checked_add(vp.width).is_some_and(|n| n as usize <= dw)
        && vp
            .y
            .checked_add(vp.height)
            .is_some_and(|n| n as usize <= dh)
}

/// Aspect fitting and filtering are independent: nearest can fill a fractional
/// viewport without creating new colors. Linear uses center-aligned sampling.
#[allow(clippy::too_many_arguments)]
pub fn blit_filtered(
    src: &[u32],
    sw: usize,
    sh: usize,
    dst: &mut [u32],
    dw: usize,
    dh: usize,
    vp: &Viewport,
    border: u32,
    filter: ScaleFilter,
) {
    if filter == ScaleFilter::Nearest {
        blit_scaled(src, sw, sh, dst, dw, dh, vp, border);
        return;
    }
    if sw == 0
        || sh == 0
        || dw == 0
        || dh == 0
        || vp.width == 0
        || vp.height == 0
        || !valid_buffers(src, sw, sh, dst, dw, dh, vp)
    {
        return;
    }
    dst[..dw * dh].fill(border);
    // Reuse horizontal coordinates for every row. This removes per-pixel
    // divisions/floors without changing the center-aligned interpolation.
    let horizontal: Vec<_> = (0..vp.width as usize)
        .map(|x| {
            let sx =
                ((x as f64 + 0.5) * sw as f64 / vp.width as f64 - 0.5).clamp(0.0, (sw - 1) as f64);
            let x0 = sx.floor() as usize;
            (x0, (x0 + 1).min(sw - 1), sx - x0 as f64)
        })
        .collect();
    for y in 0..vp.height as usize {
        let sy =
            ((y as f64 + 0.5) * sh as f64 / vp.height as f64 - 0.5).clamp(0.0, (sh - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f64;
        for (x, &(x0, x1, fx)) in horizontal.iter().enumerate() {
            let samples = [
                src[y0 * sw + x0],
                src[y0 * sw + x1],
                src[y1 * sw + x0],
                src[y1 * sw + x1],
            ];
            let mut color = 0;
            for shift in [0, 8, 16] {
                let c = samples.map(|p| ((p >> shift) & 255) as f64);
                let a = c[0] + (c[1] - c[0]) * fx;
                let b = c[2] + (c[3] - c[2]) * fx;
                // Channels are nonnegative: add-half/truncate is the same
                // rounding rule, without a floating-point round per channel.
                color |= ((a + (b - a) * fy + 0.5) as u32) << shift;
            }
            dst[(vp.y as usize + y) * dw + vp.x as usize + x] = color;
        }
    }
}

/// Translates window cursor coordinates to framebuffer coordinates (0..320, 0..240).
///
/// Returns `None` if the coordinates lie inside the letterbox/pillarbox borders.
pub fn window_to_framebuffer(win_x: f64, win_y: f64, vp: &Viewport) -> Option<(i64, i64)> {
    if vp.width == 0 || vp.height == 0 {
        return None;
    }

    let left = vp.x as f64;
    let right = (vp.x + vp.width) as f64;
    let top = vp.y as f64;
    let bottom = (vp.y + vp.height) as f64;

    if win_x < left || win_x >= right || win_y < top || win_y >= bottom {
        return None;
    }

    let rel_x = (win_x - left) / vp.width as f64;
    let rel_y = (win_y - top) / vp.height as f64;

    let fb_x = (rel_x * LOGICAL_WIDTH as f64).floor() as i64;
    let fb_y = (rel_y * LOGICAL_HEIGHT as f64).floor() as i64;

    let clamped_x = fb_x.clamp(0, LOGICAL_WIDTH as i64 - 1);
    let clamped_y = fb_y.clamp(0, LOGICAL_HEIGHT as i64 - 1);

    Some((clamped_x, clamped_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewport_integer_scaling() {
        // Exact 1x
        let vp1 = Viewport::calculate_integer(320, 240);
        assert_eq!(vp1.scale, 1);
        assert_eq!(vp1.x, 0);
        assert_eq!(vp1.y, 0);
        assert_eq!(vp1.width, 320);
        assert_eq!(vp1.height, 240);
        assert!(vp1.is_integer);

        // Exact 2x
        let vp2 = Viewport::calculate_integer(640, 480);
        assert_eq!(vp2.scale, 2);
        assert_eq!(vp2.x, 0);
        assert_eq!(vp2.y, 0);
        assert_eq!(vp2.width, 640);
        assert_eq!(vp2.height, 480);

        // 800x600 (scale 2 with pillarbox/letterbox)
        // 800/320 = 2, 600/240 = 2 -> scale 2: 640x480 inside 800x600
        let vp_mid = Viewport::calculate_integer(800, 600);
        assert_eq!(vp_mid.scale, 2);
        assert_eq!(vp_mid.width, 640);
        assert_eq!(vp_mid.height, 480);
        assert_eq!(vp_mid.x, (800 - 640) / 2); // 80
        assert_eq!(vp_mid.y, (600 - 480) / 2); // 60

        // 1920x1080 (1080/240 = 4, 1920/320 = 6 -> scale = 4: 1280x960)
        let vp_hd = Viewport::calculate_integer(1920, 1080);
        assert_eq!(vp_hd.scale, 4);
        assert_eq!(vp_hd.width, 1280);
        assert_eq!(vp_hd.height, 960);
        assert_eq!(vp_hd.x, (1920 - 1280) / 2); // 320
        assert_eq!(vp_hd.y, (1080 - 960) / 2); // 60
    }

    #[test]
    fn test_viewport_aspect_fit() {
        // Wide window
        let vp = Viewport::calculate_aspect_fit(1000, 500);
        // height = 500, width = round(500 * 4/3) = 667
        assert_eq!(vp.height, 500);
        assert_eq!(vp.width, 667);
        assert_eq!(vp.x, (1000 - 667) / 2);
        assert_eq!(vp.y, 0);

        // Tall window
        let vp_tall = Viewport::calculate_aspect_fit(400, 600);
        // width = 400, height = round(400 / (4/3)) = 300
        assert_eq!(vp_tall.width, 400);
        assert_eq!(vp_tall.height, 300);
        assert_eq!(vp_tall.x, 0);
        assert_eq!(vp_tall.y, (600 - 300) / 2);
    }

    #[test]
    fn test_window_to_framebuffer_mapping() {
        let vp = Viewport::calculate_integer(640, 480);
        // Center of window
        assert_eq!(window_to_framebuffer(320.0, 240.0, &vp), Some((160, 120)));
        // Top-left corner
        assert_eq!(window_to_framebuffer(0.0, 0.0, &vp), Some((0, 0)));
        // Bottom-right corner inside
        assert_eq!(window_to_framebuffer(639.0, 479.0, &vp), Some((319, 239)));

        // Window with borders
        let vp_borders = Viewport::calculate_integer(800, 600); // x=80..720, y=60..540
        // Outside in left border
        assert_eq!(window_to_framebuffer(50.0, 300.0, &vp_borders), None);
        // Outside in top border
        assert_eq!(window_to_framebuffer(400.0, 30.0, &vp_borders), None);
        // Inside
        assert_eq!(window_to_framebuffer(80.0, 60.0, &vp_borders), Some((0, 0)));
        assert_eq!(
            window_to_framebuffer(400.0, 300.0, &vp_borders),
            Some((160, 120))
        );
    }

    #[test]
    fn test_blit_scaled_1x() {
        let mut src = vec![0x00FF0000; 320 * 240];
        src[0] = 0x0000FF00;
        src[320 * 240 - 1] = 0x000000FF;

        let mut dst = vec![0u32; 320 * 240];
        let vp = Viewport::calculate_integer(320, 240);
        blit_scaled(&src, 320, 240, &mut dst, 320, 240, &vp, 0);

        assert_eq!(dst[0], 0x0000FF00);
        assert_eq!(dst[320 * 240 - 1], 0x000000FF);
        assert_eq!(dst[1], 0x00FF0000);
    }

    #[test]
    fn test_blit_scaled_letterbox_borders() {
        let src = vec![0x00FFFFFF; 320 * 240];
        let mut dst = vec![0u32; 800 * 600];
        let vp = Viewport::calculate_integer(800, 600);
        let border_color = 0x00123456;

        blit_scaled(&src, 320, 240, &mut dst, 800, 600, &vp, border_color);

        // Check top-left corner (in border)
        assert_eq!(dst[0], border_color);
        // Check inside viewport (x=80, y=60 -> idx = 60 * 800 + 80)
        assert_eq!(dst[60 * 800 + 80], 0x00FFFFFF);
        // Check right border (x=750, y=60)
        assert_eq!(dst[60 * 800 + 750], border_color);
    }
}

#[cfg(test)]
mod mirror_filter_tests {
    use super::*;

    #[test]
    fn logical_scales_fill_fractional_physical_surfaces_without_margins() {
        for scale in 1..=4 {
            for dpi in [1.0, 1.25, 4.0 / 3.0, 1.5, 2.0] {
                let w = (320.0 * scale as f64 * dpi).round() as u32;
                let h = (240.0 * scale as f64 * dpi).round() as u32;
                let vp = Viewport::calculate_aspect_fit(w, h);
                assert!(
                    w - vp.width <= 1 && h - vp.height <= 1,
                    "{scale}× {dpi}: {vp:?}"
                );
                assert!(vp.x <= 1 && vp.y <= 1);
                let src = vec![0x00123456; 320 * 240];
                let mut dst = vec![0; w as usize * h as usize];
                blit_filtered(
                    &src,
                    320,
                    240,
                    &mut dst,
                    w as usize,
                    h as usize,
                    &vp,
                    0,
                    ScaleFilter::Nearest,
                );
                assert_eq!(dst[(vp.y * w + vp.x) as usize], src[0]);
                assert_eq!(
                    dst[((vp.y + vp.height - 1) * w + vp.x + vp.width - 1) as usize],
                    src[0]
                );
            }
        }
    }

    #[test]
    fn nearest_preserves_each_source_pixel_and_does_not_create_colors() {
        let src: Vec<_> = (0..320 * 240)
            .map(|i| if i % 2 == 0 { 0xffffff } else { 0 })
            .collect();
        let before = src.clone();
        for (w, h) in [
            (320, 240),
            (640, 480),
            (853, 640),
            (960, 720),
            (1280, 960),
            (200, 150),
        ] {
            let vp = Viewport::calculate_aspect_fit(w, h);
            let mut dst = vec![0; (w * h) as usize];
            blit_filtered(
                &src,
                320,
                240,
                &mut dst,
                w as usize,
                h as usize,
                &vp,
                0,
                ScaleFilter::Nearest,
            );
            for y in 0..h as usize {
                for x in 0..w as usize {
                    assert_eq!(
                        dst[y * w as usize + x],
                        src[(y * 240 / h as usize) * 320 + x * 320 / w as usize]
                    );
                }
            }
        }
        assert_eq!(src, before);
    }

    #[test]
    fn bilinear_uses_independent_channels_and_clamps_edges() {
        let src = [0, 0xff0000, 0x00ff00, 0x0000ff];
        let mut dst = [0; 9];
        let vp = Viewport {
            x: 0,
            y: 0,
            width: 3,
            height: 3,
            scale: 1,
            is_integer: false,
        };
        blit_filtered(&src, 2, 2, &mut dst, 3, 3, &vp, 0, ScaleFilter::Linear);
        assert_eq!(dst[0], src[0]);
        assert_eq!(dst[2], src[1]);
        assert_eq!(dst[6], src[2]);
        assert_eq!(dst[8], src[3]);
        assert_eq!(dst[4], 0x404040);
        assert!(dst.iter().all(|x| x >> 24 == 0));
        assert_eq!(src, [0, 0xff0000, 0x00ff00, 0x0000ff]);
    }

    #[test]
    fn invalid_buffers_and_viewports_do_not_index_outside_slices() {
        let vp = Viewport::calculate_aspect_fit(320, 240);
        for filter in [ScaleFilter::Nearest, ScaleFilter::Linear] {
            let mut dst = [0x123; 10];
            blit_filtered(&[0; 1], 320, 240, &mut dst, 320, 240, &vp, 0, filter);
            assert_eq!(dst, [0x123; 10]);
        }
    }
}

#[cfg(test)]
mod presentation_acceptance {
    use super::*;

    #[test]
    fn native_and_fast_pixels_are_exact_after_fractional_dpi_and_resize() {
        for (sw, sh) in [(320usize, 240usize), (160, 120)] {
            let source: Vec<u32> = (0..sw * sh).map(|i| (i as u32 * 137) & 0xffffff).collect();
            // Physical client sizes, including 125%, 133%, 150% desktop scales.
            for (w, h) in [
                (320, 240),
                (800, 600),
                (853, 640),
                (960, 720),
                (1000, 600),
                (640, 480),
            ] {
                let vp = Viewport::calculate_integer(w, h);
                let factor = vp.width as usize / sw;
                let mut actual = vec![0xdeadbeef; (w * h) as usize];
                for replacement in [false, true] {
                    let pixels: Vec<_> = source
                        .iter()
                        .map(|c| if replacement { c ^ 0xffffff } else { *c })
                        .collect();
                    blit_filtered(
                        &pixels,
                        sw,
                        sh,
                        &mut actual,
                        w as usize,
                        h as usize,
                        &vp,
                        0x101014,
                        ScaleFilter::Nearest,
                    );
                    // Independently expand each source pixel into a square block.
                    let mut expected = vec![0x101014; actual.len()];
                    for sy in 0..sh {
                        for sx in 0..sw {
                            for dy in 0..factor {
                                for dx in 0..factor {
                                    expected[(vp.y as usize + sy * factor + dy) * w as usize
                                        + vp.x as usize
                                        + sx * factor
                                        + dx] = pixels[sy * sw + sx];
                                }
                            }
                        }
                    }
                    assert_eq!(
                        actual, expected,
                        "{sw}×{sh} on {w}×{h}, replacement={replacement}"
                    );
                }
            }
        }
    }
}
