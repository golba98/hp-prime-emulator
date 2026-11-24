//! Plot pictures from the user's installed Prime resource archive.
use crate::{firmware_options::AssetPaths, fonts};
use ppl_graphics::Grob;
use std::{fs::File, io::Read, path::Path};
use zune_core::{colorspace::ColorSpace, options::DecoderOptions};
use zune_jpeg::JpegDecoder;

const MAX_RESOURCE_BYTES: usize = 32 * 1024 * 1024;
const DIRECTORY: usize = 28;
const MAX_WIDTH: usize = 320;
const MAX_HEIGHT: usize = 240;
const MAX_APP_IMAGE_WORDS: usize = 512 * 1024;

// Order, labels and dimensions of the built-in picker on the installed G1.
const PICTURES: [(u8, &str, [usize; 2]); 16] = [
    (69, "Tiles.jpg", [320, 180]),
    (68, "Theater.jpg", [320, 207]),
    (67, "Stream.jpg", [320, 213]),
    (66, "Stairs.jpg", [283, 219]),
    (65, "Spiral.jpg", [292, 219]),
    (64, "Slices.jpg", [320, 211]),
    (63, "Skatepark.jpg", [320, 213]),
    (62, "Saturn.jpg", [320, 164]),
    (61, "Rocket.jpg", [320, 213]),
    (59, "Nautilus.jpg", [320, 209]),
    (58, "Louvre.jpg", [320, 213]),
    (56, "Hexagon.jpg", [289, 217]),
    (55, "Garden.jpg", [320, 213]),
    (54, "Eclipse.jpg", [320, 144]),
    (12, "Coins.jpg", [320, 207]),
    (11, "Bridge.jpg", [320, 188]),
];

pub struct Picture {
    pub slot: u8,
    pub filename: &'static str,
    pub pixels: Grob,
}

pub fn configured() -> Result<Vec<Picture>, String> {
    fonts::resource(&AssetPaths::default().resolve()?)
        .map(|path| load(&path))
        .transpose()
        .map(|pictures| pictures.unwrap_or_default())
}

fn load(path: &Path) -> Result<Vec<Picture>, String> {
    let mut archive = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take((MAX_RESOURCE_BYTES + 1) as u64)
                .read_to_end(&mut archive)
        })
        .map_err(|error| format!("{}: {error}", path.display()))?;
    pictures(&archive).map_err(|error| format!("{}: {error}", path.display()))
}

fn pictures(archive: &[u8]) -> Result<Vec<Picture>, String> {
    let word = |offset: usize| -> Option<usize> {
        Some(u32::from_le_bytes(archive.get(offset..offset + 4)?.try_into().ok()?) as usize)
    };
    let directory_len = word(DIRECTORY).filter(|&len| {
        len % 8 == 0
            && len / 8 > usize::from(PICTURES[0].0)
            && archive
                .get(DIRECTORY..)
                .is_some_and(|body| len <= body.len())
    });
    let directory_len = match directory_len {
        Some(len)
            if archive.len() <= MAX_RESOURCE_BYTES
                && [u32::MAX as usize, 5, 1, 0, 3, 2, 6]
                    .into_iter()
                    .enumerate()
                    .all(|(index, value)| word(index * 4) == Some(value)) =>
        {
            len
        }
        _ => return Err("Unsupported Plot picture resource archive".into()),
    };
    PICTURES
        .iter()
        .map(|&(slot, filename, dimensions)| {
            let entry = DIRECTORY + usize::from(slot) * 8;
            let jpeg = word(entry)
                .filter(|&offset| offset >= directory_len)
                .and_then(|offset| DIRECTORY.checked_add(offset))
                .and_then(|start| {
                    word(entry + 4)
                        .and_then(|size| start.checked_add(size))
                        .and_then(|end| archive.get(start..end))
                })
                .filter(|entry| entry.starts_with(&[0xff, 0xd8, 0xff]))
                .ok_or_else(|| format!("Invalid Plot picture entry {filename}"))?;
            let pixels = decode_jpeg(jpeg).map_err(|error| format!("{filename}: {error}"))?;
            if [pixels.width(), pixels.height()] != dimensions {
                return Err(format!("Unexpected {filename} dimensions"));
            }
            Ok(Picture {
                slot,
                filename,
                pixels,
            })
        })
        .collect()
}

/// Decode the Prime app-file string carrier used for attached JPEG and PNG pictures.
/// Packed UTF-16 units are interpreted as raw little-endian words.
pub fn app_picture(value: &ppl_vm::PortableValue) -> Option<Grob> {
    use ppl_vm::PortableValue;
    let words: std::borrow::Cow<'_, [u16]> = match value {
        PortableValue::Utf16(words) if words.len() <= MAX_APP_IMAGE_WORDS => {
            words.as_slice().into()
        }
        PortableValue::String(text) if text.len() <= MAX_APP_IMAGE_WORDS * 3 => {
            std::borrow::Cow::Owned(text.encode_utf16().collect())
        }
        _ => return None,
    };
    let payload = words.strip_prefix(&[80, 78, 71, 0])?;
    if payload.len() > MAX_APP_IMAGE_WORDS {
        return None;
    }
    let bytes: Vec<u8> = payload.iter().flat_map(|word| word.to_le_bytes()).collect();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Grob::read_png_with_transparency(&bytes, [MAX_WIDTH as u32, MAX_HEIGHT as u32]).ok()
    } else {
        decode_jpeg(&bytes).ok()
    }
}

fn decode_jpeg(jpeg: &[u8]) -> Result<Grob, String> {
    if !jpeg.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err("Unsupported Plot picture encoding".into());
    }
    let options = DecoderOptions::new_safe()
        .set_max_width(MAX_WIDTH)
        .set_max_height(MAX_HEIGHT)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(jpeg, options);
    let rgb = decoder.decode().map_err(|error| error.to_string())?;
    let info = decoder.info().ok_or("Missing Plot picture dimensions")?;
    let [width, height] = [usize::from(info.width), usize::from(info.height)];
    if width == 0 || height == 0 || rgb.len() != width * height * 3 {
        return Err("Unexpected Plot picture dimensions".into());
    }
    let mut pixels = Grob::new(width, height, 0xffffff);
    for (destination, pixel) in pixels.pixels_mut().iter_mut().zip(rgb.chunks_exact(3)) {
        *destination = u32::from_be_bytes([0, pixel[0], pixel[1], pixel[2]]);
    }
    Ok(pixels)
}
