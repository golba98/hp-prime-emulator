//! Framebuffer comparison for runtime parity evidence.
//!
//! Exact pixel parity and visual similarity are reported separately: only an
//! exact comparison (zero changed pixels outside a declared mask) is a match.
//! The similarity figures describe a difference; they never excuse one.
use crate::display::Frame;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Comparison {
    pub width: u32,
    pub height: u32,
    /// Zero changed pixels outside the mask.
    pub exact: bool,
    pub changed: u32,
    /// Inclusive `[x0, y0, x1, y1]` of the changed pixels.
    pub bbox: Option<[u32; 4]>,
    /// Pixels excluded by the declared mask.
    pub masked: u32,
    /// Changed share of compared pixels.
    pub changed_fraction: f64,
    /// Mean absolute per-channel difference over compared pixels (0-255).
    pub mean_channel_error: f64,
    pub max_channel_delta: u8,
}

/// Decodes any 8-bit PNG (RGB, RGBA, grey, palette) into XRGB pixels.
pub fn load(path: &Path) -> Result<Frame, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("unexpanded palette PNG".into()),
    };
    let pixels = buf[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| {
            let (r, g, b) = if channels < 3 {
                (p[0], p[0], p[0])
            } else {
                (p[0], p[1], p[2])
            };
            (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
        })
        .collect();
    Ok(Frame {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// Compares `actual` with `reference`; `mask` marks pixels to ignore.
pub fn compare(
    actual: &Frame,
    reference: &Frame,
    mask: Option<&[bool]>,
) -> Result<Comparison, String> {
    if (actual.width, actual.height) != (reference.width, reference.height) {
        return Err(format!(
            "size differs: {}x{} vs {}x{}",
            actual.width, actual.height, reference.width, reference.height
        ));
    }
    let count = actual.pixels.len();
    if mask.is_some_and(|m| m.len() != count) {
        return Err("mask size differs from the frames".into());
    }
    let (mut changed, mut masked, mut error_sum, mut max_delta) = (0u32, 0u32, 0u64, 0u8);
    let mut bbox: Option<[u32; 4]> = None;
    for (i, (&a, &b)) in actual.pixels.iter().zip(&reference.pixels).enumerate() {
        if mask.is_some_and(|m| m[i]) {
            masked += 1;
            continue;
        }
        let a = a & 0xff_ffff;
        let b = b & 0xff_ffff;
        if a == b {
            continue;
        }
        changed += 1;
        for shift in [16, 8, 0] {
            let d = ((a >> shift) as u8).abs_diff((b >> shift) as u8);
            error_sum += u64::from(d);
            max_delta = max_delta.max(d);
        }
        let (x, y) = (i as u32 % actual.width, i as u32 / actual.width);
        bbox = Some(match bbox {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
    let compared = (count as u32 - masked).max(1);
    Ok(Comparison {
        width: actual.width,
        height: actual.height,
        exact: changed == 0,
        changed,
        bbox,
        masked,
        changed_fraction: f64::from(changed) / f64::from(compared),
        mean_channel_error: error_sum as f64 / (3.0 * f64::from(compared)),
        max_channel_delta: max_delta,
    })
}

/// The reference dimmed, with changed pixels in magenta.
pub fn visualize(actual: &Frame, reference: &Frame) -> Frame {
    let pixels = actual
        .pixels
        .iter()
        .zip(&reference.pixels)
        .map(|(&a, &b)| {
            if a & 0xff_ffff == b & 0xff_ffff {
                let dim = |s: u32| ((b >> s) & 255) / 3 + 120;
                (dim(16) << 16) | (dim(8) << 8) | dim(0)
            } else {
                0xff_00ff
            }
        })
        .collect();
    Frame {
        width: reference.width,
        height: reference.height,
        pixels,
    }
}

/// A mask PNG: any non-black pixel is ignored.
pub fn load_mask(path: &Path) -> Result<Vec<bool>, String> {
    Ok(load(path)?
        .pixels
        .iter()
        .map(|p| p & 0xff_ffff != 0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(pixels: Vec<u32>) -> Frame {
        Frame {
            width: 2,
            height: 2,
            pixels,
        }
    }
    #[test]
    fn exact_parity_is_separate_from_similarity() {
        let a = frame(vec![0, 0xffffff, 0x102030, 0]);
        let c = compare(&a, &a, None).unwrap();
        assert!(c.exact && c.changed == 0 && c.bbox.is_none());
        let b = frame(vec![0, 0xffffff, 0x102031, 0x000100]);
        let c = compare(&b, &a, None).unwrap();
        assert!(!c.exact);
        assert_eq!(c.changed, 2);
        assert_eq!(c.bbox, Some([0, 1, 1, 1]));
        assert_eq!(c.max_channel_delta, 1);
        assert_eq!(c.changed_fraction, 0.5);
        let c = compare(&b, &a, Some(&[false, false, true, true])).unwrap();
        assert!(c.exact && c.masked == 2);
        assert!(
            compare(
                &b,
                &Frame {
                    width: 1,
                    height: 4,
                    pixels: vec![0; 4]
                },
                None
            )
            .is_err()
        );
    }
    #[test]
    fn png_round_trip_through_generic_loader() {
        let dir = tempfile::tempdir().unwrap();
        let a = frame(vec![0x123456, 0xffffff, 0, 0xabcdef]);
        let p = dir.path().join("a.png");
        a.save_png(&p).unwrap();
        assert_eq!(load(&p).unwrap().pixels, a.pixels);
    }
}
