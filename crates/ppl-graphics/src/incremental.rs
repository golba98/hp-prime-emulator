//! Transactional RGB555 screenshot composition. No transport access.
use crate::{Grob, PrimeScreenshotDecoder, ScreenshotDecodeTimings};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameState {
    NeedsBaseline,
    Synchronized,
    Stale,
}

pub struct IncrementalFramebuffer {
    image: Option<Arc<Grob>>,
    state: FrameState,
    decoder: PrimeScreenshotDecoder,
    next: u64,
}
impl Default for IncrementalFramebuffer {
    fn default() -> Self {
        Self {
            image: None,
            state: FrameState::NeedsBaseline,
            decoder: Default::default(),
            next: 0,
        }
    }
}
pub struct AppliedUpdate {
    pub image: Arc<Grob>,
    pub patch: Arc<Grob>,
    pub changed: bool,
    pub decode: ScreenshotDecodeTimings,
    pub composition: std::time::Duration,
}
impl IncrementalFramebuffer {
    pub fn state(&self) -> FrameState {
        self.state
    }
    pub fn image(&self) -> Option<&Arc<Grob>> {
        self.image.as_ref()
    }
    pub fn invalidate(&mut self) {
        self.state = FrameState::Stale;
    }
    /// New connection/generation. Keep the stale image until a new baseline validates.
    pub fn restart(&mut self) {
        self.invalidate();
        self.next = 0;
    }
    /// Ordinal is host-local, not a recovered wire field.
    pub fn apply(
        &mut self,
        ordinal: u64,
        origin: Option<(u16, u16)>,
        png: &[u8],
    ) -> Result<AppliedUpdate, String> {
        let result = self.apply_inner(ordinal, origin, png);
        if result.is_err() {
            self.invalidate();
        }
        result
    }
    fn apply_inner(
        &mut self,
        ordinal: u64,
        origin: Option<(u16, u16)>,
        png: &[u8],
    ) -> Result<AppliedUpdate, String> {
        if ordinal != self.next {
            return Err("local update ordering mismatch".into());
        }
        // Validate the original IHDR before the general decoder expands palettes.
        if png.len() > 16 * 1024 * 1024
            || !png.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
            || png.len() < 33
            || &png[..8] != b"\x89PNG\r\n\x1a\n"
            || &png[8..16] != b"\0\0\0\rIHDR"
            || png[24..26] != [16, 0]
        {
            return Err("incremental RGB555 requires grayscale16 PNG".into());
        }
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap()) as usize;
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap()) as usize;
        let (x, y) = origin
            .map(|(x, y)| (usize::from(x), usize::from(y)))
            .unwrap_or((0, 0));
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|v| v > 320)
            || y.checked_add(h).is_none_or(|v| v > 240)
        {
            return Err("patch outside framebuffer".into());
        }
        let full = x == 0 && y == 0 && w == 320 && h == 240;
        if origin.is_none() && !full {
            return Err("incomplete full frame".into());
        }
        if self.state != FrameState::Synchronized && !full {
            return Err("full baseline required".into());
        }
        let (patch, decode) = self.decoder.decode(png, Some((w, h)))?;
        let patch_arc = Arc::new(patch);
        let next = self.next.checked_add(1).ok_or("update ordinal overflow")?;
        let composition = std::time::Instant::now();
        let changed = self.image.as_ref().is_none_or(|image| {
            (0..h).any(|row| {
                image.pixels()[(y + row) * 320 + x..(y + row) * 320 + x + w]
                    != patch_arc.pixels()[row * w..(row + 1) * w]
            })
        });
        if changed {
            let image = self
                .image
                .get_or_insert_with(|| Arc::new(Grob::new(320, 240, 0)));
            let image = Arc::make_mut(image);
            for row in 0..h {
                image.pixels_mut()[(y + row) * 320 + x..(y + row) * 320 + x + w]
                    .copy_from_slice(&patch_arc.pixels()[row * w..(row + 1) * w]);
            }
        }
        self.state = FrameState::Synchronized;
        self.next = next;
        Ok(AppliedUpdate {
            image: self.image.as_ref().unwrap().clone(),
            patch: patch_arc,
            changed,
            decode,
            composition: composition.elapsed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded(w: usize, h: usize, pixels: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, w as u32, h as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(
                &pixels
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        bytes
    }
    fn color(v: u16) -> u32 {
        let r = u32::from((v >> 10) & 31);
        let g = u32::from((v >> 5) & 31);
        let b = u32::from(v & 31);
        (((r << 3) | (r >> 2)) << 16) | (((g << 3) | (g >> 2)) << 8) | ((b << 3) | (b >> 2))
    }
    #[test]
    fn every_pixel_matches_independent_expected_canvas_after_each_patch() {
        let mut framebuffer = IncrementalFramebuffer::default();
        let initial: Vec<u16> = (0..76800).map(|v| v as u16).collect();
        let mut expected: Vec<_> = initial.iter().map(|&v| color(v)).collect();
        let first = framebuffer
            .apply(0, None, &encoded(320, 240, &initial))
            .unwrap();
        assert_eq!(first.image.pixels(), expected);
        let original = first.image.clone();
        for (i, (x, y, w, h, v)) in [
            (10, 20, 13, 11, 0xf800),
            (40, 90, 21, 3, 0x07e0),
            (12, 21, 6, 7, 0x001f),
            (319, 239, 1, 1, 0xffff),
            (0, 0, 1, 1, 0),
            (0, 0, 320, 240, 0x8410),
        ]
        .into_iter()
        .enumerate()
        {
            let patch = encoded(w, h, &vec![v; w * h]);
            let applied = framebuffer
                .apply(i as u64 + 1, Some((x as u16, y as u16)), &patch)
                .unwrap();
            for row in y..y + h {
                for col in x..x + w {
                    expected[row * 320 + col] = color(v);
                }
            }
            assert_eq!(applied.image.pixels(), expected, "patch {i}");
        }
        assert_eq!(
            original.pixels(),
            initial.iter().map(|&v| color(v)).collect::<Vec<_>>()
        );
        let previous = framebuffer.image().unwrap().clone();
        let unchanged = framebuffer
            .apply(7, Some((319, 239)), &encoded(1, 1, &[0x8410]))
            .unwrap();
        assert!(!unchanged.changed);
        assert!(Arc::ptr_eq(&previous, &unchanged.image));
    }
    #[test]
    fn invalid_updates_are_atomic_and_require_new_baseline() {
        let full = encoded(320, 240, &vec![0x1234; 76800]);
        let patch = encoded(2, 2, &[0xffff; 4]);
        let mut bad_crc = patch.clone();
        let n = bad_crc.len();
        bad_crc[n - 1] ^= 1;
        let mut bad_idat = patch.clone();
        bad_idat[43] ^= 1;
        let mut palette = patch.clone();
        palette[25] = 3;
        let mut bad_dimensions = patch.clone();
        bad_dimensions[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        let truncated = patch[..patch.len() - 8].to_vec();
        for (ordinal, origin, png) in [
            (1, Some((319, 239)), patch.clone()),
            (1, None, patch.clone()),
            (1, Some((0, 0)), bad_crc),
            (1, Some((0, 0)), bad_idat),
            (1, Some((0, 0)), palette),
            (1, Some((0, 0)), bad_dimensions),
            (1, Some((0, 0)), truncated),
            (9, Some((0, 0)), patch.clone()),
        ] {
            let mut f = IncrementalFramebuffer::default();
            f.apply(0, None, &full).unwrap();
            let saved = f.image().unwrap().clone();
            assert!(f.apply(ordinal, origin, &png).is_err());
            assert_eq!(f.state(), FrameState::Stale);
            assert_eq!(f.image().unwrap().pixels(), saved.pixels());
            assert!(f.apply(1, Some((0, 0)), &patch).is_err());
            f.restart();
            assert!(f.apply(0, Some((0, 0)), &patch).is_err());
            assert!(f.apply(0, None, &full).is_ok());
            assert_eq!(f.state(), FrameState::Synchronized);
        }
    }
    #[test]
    fn patch_without_baseline_is_refused() {
        assert!(
            IncrementalFramebuffer::default()
                .apply(0, Some((0, 0)), &encoded(1, 1, &[0]))
                .is_err()
        );
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;
    #[test]
    fn general_palette_decode_preserves_palette_and_incremental_rgb555_rejects_it() {
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, 2, 1);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(vec![0x12, 0x34, 0x56, 0x9a, 0xbc, 0xde]);
        enc.write_header()
            .unwrap()
            .write_image_data(&[0, 1])
            .unwrap();
        let (decoded, _) = PrimeScreenshotDecoder::default()
            .decode(&bytes, Some((2, 1)))
            .unwrap();
        assert_eq!(decoded.pixels(), &[0x123456, 0x9abcde]);
        assert!(
            IncrementalFramebuffer::default()
                .apply(0, Some((0, 0)), &bytes)
                .is_err()
        );
    }
}
