//! Headless capture of the firmware's LCD framebuffer.
//!
//! The OS-side LCD object returned by `GetActiveLCD` (SVC 0x1008D,
//! [`crate::services::system2`]) carries an `LCD_MAGIC` block whose
//! `window1_bufferstart` points at the panel buffer. The firmware draws
//! into that buffer itself: M4 traced 32-bit stores from its blit routine
//! (`0x3061FBE8`, `str r5, [r1], #4`) writing `0x00RRGGBB` words there.
//!
//! Capture is a read of guest memory between execution slices: nothing is
//! written to the guest, nothing is drawn by the host, and the brightness
//! scaling PrimU2 applies when *displaying* the buffer is not applied.

use std::path::Path;

use crate::memory::GuestMemory;
use crate::services::system2::{LCD_BUFFER_OFF, LCD_MAGIC_OFF};

/// Panel geometry of the G1 (and of the LCD object's `LCD_MAGIC`).
pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 240;
const BYTES_PER_PIXEL: u32 = 4;

/// Byte offsets inside the LCD object (PrimU2 `LCD.h`, "BLIGLCD").
mod off {
    pub const MAGIC_PTR: u32 = 0;
    pub const BUFFER_SIZE: u32 = 8;
    pub const ITSELF: u32 = 12;
    pub const X_RES: u32 = 468;
    pub const Y_RES: u32 = 470;
    /// Inside `LCD_MAGIC`.
    pub const MAGIC_X_RES: u32 = 2;
    pub const MAGIC_Y_RES: u32 = 4;
    pub const MAGIC_PIXEL_BITS: u32 = 6;
    pub const MAGIC_WINDOW1: u32 = 16;
}

/// Pixel encoding of the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PixelFormat {
    /// Little-endian 32-bit words `0x00RRGGBB` (bytes B, G, R, unused).
    /// Verified from guest stores (e.g. `0x00F2F2F2`); the top byte is not
    /// alpha (the firmware leaves it 0) and is ignored.
    Xrgb8888,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayError {
    /// `GetActiveLCD` has not been called yet: there is no LCD object.
    NotInitialized,
    /// The object does not have the verified layout.
    InvalidLayout(String),
    /// The buffer is not inside one mapped guest block.
    OutOfBounds { va: u32, len: u32 },
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayError::NotInitialized => {
                write!(f, "display not initialized (no GetActiveLCD call yet)")
            }
            DisplayError::InvalidLayout(m) => write!(f, "invalid LCD object: {m}"),
            DisplayError::OutOfBounds { va, len } => {
                write!(
                    f,
                    "framebuffer {va:#010X}+{len:#X} is not in mapped guest memory"
                )
            }
        }
    }
}

impl std::error::Error for DisplayError {}

/// A validated view of the active LCD's framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LcdView {
    pub lcd_va: u32,
    pub fb_va: u32,
    pub width: u32,
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    pub format: PixelFormat,
}

impl LcdView {
    /// Validates the LCD object at `lcd_va` and locates its framebuffer.
    pub fn locate(mem: &GuestMemory, lcd_va: Option<u32>) -> Result<LcdView, DisplayError> {
        let lcd_va = lcd_va.ok_or(DisplayError::NotInitialized)?;
        let bad = |m: String| DisplayError::InvalidLayout(m);
        let rd32 = |va: u32| {
            mem.read_u32(va)
                .map_err(|e| bad(format!("unreadable field at {va:#010X}: {e}")))
        };
        let rd16 = |va: u32| {
            mem.read_u16(va)
                .map_err(|e| bad(format!("unreadable field at {va:#010X}: {e}")))
        };
        let magic = lcd_va + LCD_MAGIC_OFF;
        let magic_ptr = rd32(lcd_va + off::MAGIC_PTR)?;
        if magic_ptr != magic {
            return Err(bad(format!(
                "LCDMagicPtr {magic_ptr:#010X} != {magic:#010X}"
            )));
        }
        let itself = rd32(lcd_va + off::ITSELF)?;
        if itself != lcd_va {
            return Err(bad(format!("itself {itself:#010X} != {lcd_va:#010X}")));
        }
        let dims = [
            ("xRes", rd16(lcd_va + off::X_RES)?, WIDTH),
            ("yRes", rd16(lcd_va + off::Y_RES)?, HEIGHT),
            ("LCD_MAGIC.x_res", rd16(magic + off::MAGIC_X_RES)?, WIDTH),
            ("LCD_MAGIC.y_res", rd16(magic + off::MAGIC_Y_RES)?, HEIGHT),
            (
                "LCD_MAGIC.pixel_bits",
                rd16(magic + off::MAGIC_PIXEL_BITS)?,
                32,
            ),
        ];
        for (name, got, want) in dims {
            if u32::from(got) != want {
                return Err(bad(format!("{name} = {got}, expected {want}")));
            }
        }
        let len = WIDTH * HEIGHT * BYTES_PER_PIXEL;
        let size = rd32(lcd_va + off::BUFFER_SIZE)?;
        if size < len {
            return Err(bad(format!("bufferSize {size:#X} < {len:#X}")));
        }
        let fb_va = rd32(magic + off::MAGIC_WINDOW1)?;
        if fb_va != lcd_va + LCD_BUFFER_OFF {
            return Err(bad(format!(
                "window1_bufferstart {fb_va:#010X} is not the object's buffer"
            )));
        }
        if mem.covering_block(fb_va, len as usize).is_none() {
            return Err(DisplayError::OutOfBounds { va: fb_va, len });
        }
        Ok(LcdView {
            lcd_va,
            fb_va,
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH * BYTES_PER_PIXEL,
            format: PixelFormat::Xrgb8888,
        })
    }

    /// Reads the framebuffer (one guest-memory read; the guest is not
    /// modified).
    pub fn capture(&self, mem: &GuestMemory) -> Result<Frame, DisplayError> {
        let len = self.stride * self.height;
        let bytes = mem
            .read_bytes_alloc(self.fb_va, len as usize)
            .map_err(|_| DisplayError::OutOfBounds {
                va: self.fb_va,
                len,
            })?;
        Ok(Frame::from_xrgb_bytes(self.width, self.height, &bytes))
    }
}

/// One captured screen: raw guest pixel words, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `0x00RRGGBB` per pixel (top byte cleared).
    pub pixels: Vec<u32>,
}

/// Pixels that differ between two frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct FrameDiff {
    pub changed: u32,
    /// Inclusive bounding box `[x0, y0, x1, y1]` of the changes.
    pub bbox: Option<[u32; 4]>,
}

impl Frame {
    /// Decodes little-endian XRGB8888 rows (`stride == width * 4`).
    pub fn from_xrgb_bytes(width: u32, height: u32, bytes: &[u8]) -> Frame {
        let pixels = bytes
            .chunks_exact(4)
            .take((width * height) as usize)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]))
            .collect();
        Frame {
            width,
            height,
            pixels,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// RGBA8 bytes (opaque) for image encoders.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &p in &self.pixels {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 0xFF]);
        }
        out
    }

    /// SHA-256 of the RGBA rendering (stable across runs for equal screens).
    pub fn sha256(&self) -> String {
        crate::sha256::hex(&self.to_rgba())
    }

    /// Cheap content fingerprint for change tracking (not a security
    /// hash; [`Frame::sha256`] is the reported identity).
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.pixels.hash(&mut h);
        h.finish()
    }

    pub fn unique_colors(&self) -> usize {
        let mut v = self.pixels.clone();
        v.sort_unstable();
        v.dedup();
        v.len()
    }

    /// Changed pixels versus `before`, ignoring pixels set in `ignore`
    /// (one flag per pixel, e.g. a blinking-cursor mask).
    pub fn diff(&self, before: &Frame, ignore: Option<&[bool]>) -> FrameDiff {
        let mut changed = 0;
        let mut bbox: Option<[u32; 4]> = None;
        for (i, (a, b)) in self.pixels.iter().zip(&before.pixels).enumerate() {
            if a == b || ignore.is_some_and(|m| m[i]) {
                continue;
            }
            changed += 1;
            let (x, y) = (i as u32 % self.width, i as u32 / self.width);
            bbox = Some(match bbox {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            });
        }
        FrameDiff { changed, bbox }
    }

    /// Marks every pixel that differs from `other` in `mask`.
    pub fn accumulate_changes(&self, other: &Frame, mask: &mut [bool]) {
        for (i, (a, b)) in self.pixels.iter().zip(&other.pixels).enumerate() {
            if a != b {
                mask[i] = true;
            }
        }
    }

    /// PNG bytes (8-bit RGBA).
    pub fn encode_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header()?;
            w.write_image_data(&self.to_rgba())?;
        }
        Ok(out)
    }

    pub fn save_png(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = self.encode_png().map_err(std::io::Error::other)?;
        std::fs::write(path, bytes)
    }
}
