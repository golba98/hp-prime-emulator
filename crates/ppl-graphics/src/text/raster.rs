//! Safe copied-outline boundary to the standalone FreeType 2.8 gray converter.
//! The current FreeType library parses fonts and supplies hinted outlines;
//! only the scan-conversion algorithm uses the older, independently licensed C.
use super::prime::{FontError, Result};

unsafe extern "C" {
    fn ppl_prime_gray(
        xy: *const i64,
        count: usize,
        tags: *const i8,
        contours: *const i16,
        contour_count: usize,
        flags: i32,
        xmin: i64,
        ymin: i64,
        width: u32,
        height: u32,
        pixels: *mut u8,
        pixel_count: usize,
    ) -> i32;
}
#[derive(Debug)]
pub(super) struct Mask {
    pub width: usize,
    pub height: usize,
    pub xmin: i32,
    pub ymin: i32,
    pub pixels: Vec<u8>,
}
#[allow(clippy::unnecessary_cast)] // C long is32 bits on Windows and64 bits on Unix.
pub(super) fn render(outline: &freetype::Outline<'_>) -> Result<Mask> {
    let points = outline.points();
    let contours = outline.contours();
    let tags = outline.tags();
    if points.is_empty() {
        return Ok(Mask {
            width: 0,
            height: 0,
            xmin: 0,
            ymin: 0,
            pixels: Vec::new(),
        });
    }
    if points.len() > 32767
        || tags.len() != points.len()
        || contours.is_empty()
        || contours.len() > points.len()
    {
        return Err(FontError(
            "Prime glyph outline exceeds its point bound".into(),
        ));
    }
    let xy = points
        .iter()
        .flat_map(|point| [point.x as i64, point.y as i64])
        .collect::<Vec<_>>();
    let x = points.iter().map(|p| p.x as i64);
    let y = points.iter().map(|p| p.y as i64);
    let xmin = x.clone().min().unwrap().div_euclid(64);
    let ymin = y.clone().min().unwrap().div_euclid(64);
    let xmax = (x.max().unwrap() + 63).div_euclid(64);
    let ymax = (y.max().unwrap() + 63).div_euclid(64);
    let width =
        usize::try_from(xmax - xmin).map_err(|_| FontError("Invalid glyph width".into()))?;
    let height =
        usize::try_from(ymax - ymin).map_err(|_| FontError("Invalid glyph height".into()))?;
    if width > 1024
        || height > 1024
        || !(-16384..=16384).contains(&xmin)
        || !(-16384..=16384).contains(&ymin)
    {
        return Err(FontError("Prime glyph exceeds raster bounds".into()));
    }
    let mut pixels = vec![0; width * height];
    // SAFETY: all immutable slices remain alive throughout the call; points
    // have two fixed-width coordinates, tags/counts match, output has exactly
    // width*height bytes. C revalidates contours/coordinates before copying;
    // its raster and work area belong to this call and never retain pointers.
    let result = unsafe {
        ppl_prime_gray(
            xy.as_ptr(),
            points.len(),
            tags.as_ptr().cast(),
            contours.as_ptr(),
            contours.len(),
            outline.flags(),
            xmin,
            ymin,
            width as u32,
            height as u32,
            pixels.as_mut_ptr(),
            pixels.len(),
        )
    };
    if result != 0 {
        return Err(FontError(format!(
            "Prime glyph scan conversion failed ({result})"
        )));
    }
    Ok(Mask {
        width,
        height,
        xmin: xmin as i32,
        ymin: ymin as i32,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn square(offset: i64) -> Result<Mask> {
        let mut points = [
            (offset, offset),
            (offset + 2, offset),
            (offset + 2, offset + 2),
            (offset, offset + 2),
        ]
        .map(|(x, y)| freetype::Vector {
            x: (x * 64) as _,
            y: (y * 64) as _,
        });
        let mut tags = [1; 4];
        let mut contours = [3];
        let raw = freetype::ffi::FT_Outline {
            n_points: 4,
            n_contours: 1,
            points: points.as_mut_ptr(),
            tags: tags.as_mut_ptr(),
            contours: contours.as_mut_ptr(),
            flags: 0,
        };
        // SAFETY: the outline borrows the live, correctly counted local arrays;
        // no reference escapes this call and rasterization copies the points.
        let outline = unsafe { freetype::Outline::from_raw(&raw) };
        render(&outline)
    }
    #[test]
    fn translated_squares_keep_exact_coverage_and_independent_parallel_work_areas() {
        let workers = (0..8)
            .map(|index| {
                std::thread::spawn(move || {
                    for offset in (0..32).map(|i| i - 16 + index) {
                        let mask = square(offset).unwrap();
                        assert_eq!((mask.width, mask.height), (2, 2));
                        assert_eq!((mask.xmin, mask.ymin), (offset as i32, offset as i32));
                        assert_eq!(mask.pixels, [255; 4]);
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
    }
    #[test]
    fn coordinate_bound_is_refused_before_the_native_scan_converter() {
        assert!(square(32768).unwrap_err().0.contains("bounds"));
    }
}
