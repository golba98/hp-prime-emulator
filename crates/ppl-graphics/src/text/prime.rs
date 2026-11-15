//! Prime Sans from a user's installed G1 resource archive. No vendor font data
//! is bundled. Legacy v5 resource records and font IDs follow the actual G1
//! loader; original font names are checked before admitting the resource.
//!
//! A scope selects these metrics/rasterization on its own thread. Open-source
//! default rendering and other threads are unaffected. Every selected-font
//! failure propagates from the scope; it cannot silently render a substitute.
use super::Glyph;
use freetype::{Face, Library, face::LoadFlag};
use std::{
    cell::RefCell,
    collections::HashMap,
    fmt,
    fs::File,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_RESOURCE: usize = 32 * 1024 * 1024;
const MAX_FONT: usize = 8 * 1024 * 1024;
const BASE: usize = 28;
const FONT_IDS: [usize; 3] = [0x50, 0x4f, 0x51];
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontError(pub String);
impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for FontError {}
pub(super) type Result<T> = std::result::Result<T, FontError>;
fn error(message: impl fmt::Display) -> FontError {
    FontError(message.to_string())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontStyle {
    #[default]
    Regular,
    Bold,
    Mono,
}
impl FontStyle {
    fn index(self) -> usize {
        match self {
            Self::Regular => 0,
            Self::Bold => 1,
            Self::Mono => 2,
        }
    }
}
#[derive(Debug)]
pub struct PrimeFonts {
    id: u64,
    bytes: [Arc<[u8]>; 3],
    menus: [Result<Arc<crate::Grob>>; 2],
}
fn word(data: &[u8], at: usize) -> Result<u32> {
    let bytes = data
        .get(
            at..at
                .checked_add(4)
                .ok_or_else(|| error("Resource offset overflow"))?,
        )
        .ok_or_else(|| error("Truncated resource record"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}
impl PrimeFonts {
    pub fn load_resource(path: &Path) -> Result<Arc<Self>> {
        let file = File::open(path).map_err(error)?;
        let mut data = Vec::new();
        file.take((MAX_RESOURCE + 1) as u64)
            .read_to_end(&mut data)
            .map_err(error)?;
        Self::from_resource(&data)
    }
    /// Also accepts owned/in-memory resource data; useful for callers that
    /// already resolved and authenticated their installed firmware resources.
    pub fn from_resource(data: &[u8]) -> Result<Arc<Self>> {
        if data.len() > MAX_RESOURCE || data.len() < BASE {
            return Err(error("G1 font resource size is outside its bound"));
        }
        let header = [u32::MAX, 5, 1, 0, 3, 2, 6];
        for (i, expected) in header.into_iter().enumerate() {
            if word(data, i * 4)? != expected {
                return Err(error("Unsupported G1 font resource format"));
            }
        }
        let read_font = |id: usize| -> Result<Arc<[u8]>> {
            let entry = BASE + id * 8;
            let at = BASE
                .checked_add(word(data, entry)? as usize)
                .ok_or_else(|| error("Font resource offset overflow"))?;
            let length = word(data, entry + 4)? as usize;
            if !(12..=MAX_FONT).contains(&length)
                || at < BASE + (FONT_IDS.iter().max().unwrap() + 1) * 8
            {
                return Err(error("Invalid font resource size or overlapping directory"));
            }
            let end = at
                .checked_add(length)
                .ok_or_else(|| error("Font resource end overflow"))?;
            Ok(data
                .get(at..end)
                .ok_or_else(|| error("Truncated G1 font resource"))?
                .into())
        };
        let fonts = Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            bytes: [
                read_font(FONT_IDS[0])?,
                read_font(FONT_IDS[1])?,
                read_font(FONT_IDS[2])?,
            ],
            menus: [read_menu(data, 6), read_menu(data, 0)],
        });
        // Validate all three faces now, not half way through painting a frame.
        Renderer::new(&fonts)?;
        Ok(fonts)
    }
    pub fn full_bytes(&self) -> Arc<[u8]> {
        self.bytes[0].clone()
    }
    pub fn bold_bytes(&self) -> Arc<[u8]> {
        self.bytes[1].clone()
    }
    pub fn mono_bytes(&self) -> Arc<[u8]> {
        self.bytes[2].clone()
    }
    pub fn menu_texture(&self, dark: bool) -> Result<Arc<crate::Grob>> {
        self.menus[usize::from(dark)].clone()
    }
}

fn read_menu(data: &[u8], id: usize) -> Result<Arc<crate::Grob>> {
    let record = BASE + id * 8;
    let at = BASE
        .checked_add(word(data, record)? as usize)
        .ok_or_else(|| error("Menu offset overflow"))?;
    let length = word(data, record + 4)? as usize;
    if !(33..=65536).contains(&length) || at < BASE + (FONT_IDS[2] + 1) * 8 {
        return Err(error("Invalid installed menu resource bounds"));
    }
    let end = at
        .checked_add(length)
        .ok_or_else(|| error("Menu end overflow"))?;
    let png = data
        .get(at..end)
        .ok_or_else(|| error("Truncated installed menu resource"))?;
    // Admit dimensions before invoking the PNG decoder or allocating pixels.
    if &png[..8] != b"\x89PNG\r\n\x1a\n"
        || &png[12..16] != b"IHDR"
        || u32::from_be_bytes(png[16..20].try_into().unwrap()) != 60
        || u32::from_be_bytes(png[20..24].try_into().unwrap()) != 18
    {
        return Err(error("Installed menu texture must be a60x18 PNG"));
    }
    crate::Grob::read_png(png).map(Arc::new).map_err(error)
}

/// A selected archive must supply its own validated menu texture. Absence of
/// selection is distinct from an invalid explicitly selected resource.
pub fn active_menu_texture(dark: bool) -> Option<Result<Arc<crate::Grob>>> {
    ACTIVE.with(|slot| slot.borrow().as_ref().map(|s| s.fonts.menu_texture(dark)))
}
struct Renderer {
    id: u64,
    faces: [Face<Arc<[u8]>>; 3],
    glyphs: HashMap<(FontStyle, u32, u32), Arc<Glyph>>,
}
impl Renderer {
    fn new(fonts: &PrimeFonts) -> Result<Self> {
        let library = Library::init().map_err(error)?;
        let faces = [
            library
                .new_memory_face2(fonts.full_bytes(), 0)
                .map_err(error)?,
            library
                .new_memory_face2(fonts.bold_bytes(), 0)
                .map_err(error)?,
            library
                .new_memory_face2(fonts.mono_bytes(), 0)
                .map_err(error)?,
        ];
        for (face, expected) in
            faces
                .iter()
                .zip(["Prime Sans", "Prime Sans Bold", "Prime Sans Mono"])
        {
            if face.family_name().as_deref() != Some(expected) {
                return Err(error(
                    "Resource does not contain the expected Prime Sans family",
                ));
            }
        }
        Ok(Self {
            id: fonts.id,
            faces,
            glyphs: HashMap::new(),
        })
    }
    fn face(&self, style: FontStyle, px: f32) -> Result<&Face<Arc<[u8]>>> {
        if !px.is_finite() || !(1.0..=256.0).contains(&px) {
            return Err(error("Prime font pixel size must be finite and1..256"));
        }
        let face = &self.faces[style.index()];
        face.set_pixel_sizes(0, px.round() as u32).map_err(error)?;
        Ok(face)
    }
    fn metrics(&self, style: FontStyle, px: f32) -> Result<(i64, i64)> {
        let metrics = self
            .face(style, px)?
            .size_metrics()
            .ok_or_else(|| error("Prime font has no size metrics"))?;
        let ascent = if style == FontStyle::Regular {
            match px.round() as u32 {
                // G1 TEXTOUT font-code baselines, measured independently for
                // all seven sizes. Line spacing in general UI text still
                // uses the face metrics; TEXTOUT ignores LF at its boundary.
                10 => 8,
                12 => 10,
                14 => 13,
                16 => 16,
                18 => 18,
                20 => 20,
                22 => 22,
                _ => metrics.ascender / 64,
            }
        } else {
            metrics.ascender / 64
        };
        Ok(((metrics.height / 64).max(1), ascent))
    }
    fn glyph(&mut self, style: FontStyle, ch: char, px: f32) -> Result<Arc<Glyph>> {
        self.glyph_codepoint(style, ch as u32, px)
    }
    fn glyph_codepoint(&mut self, style: FontStyle, codepoint: u32, px: f32) -> Result<Arc<Glyph>> {
        let key = (style, codepoint, px.to_bits());
        if let Some(glyph) = self.glyphs.get(&key) {
            return Ok(glyph.clone());
        }
        let face = self.face(style, px)?;
        face.load_char(
            codepoint as usize,
            LoadFlag::TARGET_LIGHT | LoadFlag::NO_BITMAP,
        )
        .map_err(error)?;
        let slot = face.glyph();
        let outline = slot
            .outline()
            .ok_or_else(|| error("Prime glyph has no outline"))?;
        let mask = super::raster::render(&outline)?;
        let glyph = Arc::new(Glyph {
            width: mask.width,
            height: mask.height,
            xmin: mask.xmin,
            ymin: mask.ymin,
            advance: slot.advance().x as f32 / 64.,
            bitmap: mask.pixels,
            antialias: true,
        });
        let cached_bytes = self.glyphs.values().map(|g| g.bitmap.len()).sum::<usize>();
        if self.glyphs.len() >= 1024 || cached_bytes + glyph.bitmap.len() > 8 * 1024 * 1024 {
            self.glyphs.clear();
        }
        self.glyphs.insert(key, glyph.clone());
        Ok(glyph)
    }
}
#[derive(Clone)]
struct Selection {
    fonts: Arc<PrimeFonts>,
    style: FontStyle,
}
thread_local! {
    static ACTIVE: RefCell<Option<Selection>> = const { RefCell::new(None) };
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
    static FAILURE: RefCell<Option<FontError>> = const { RefCell::new(None) };
}
fn renderer<T>(selection: &Selection, call: impl FnOnce(&mut Renderer) -> Result<T>) -> Result<T> {
    RENDERER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_none_or(|r| r.id != selection.fonts.id) {
            *slot = Some(Renderer::new(&selection.fonts)?);
        }
        call(slot.as_mut().unwrap())
    })
}
struct Scope {
    previous: Option<Selection>,
    previous_error: Option<FontError>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|slot| *slot.borrow_mut() = self.previous.take());
        FAILURE.with(|slot| *slot.borrow_mut() = self.previous_error.take());
    }
}
/// Selected rendering is thread-scoped and nested scopes restore their caller.
/// A failure invalidates the entire produced frame/result instead of silently
/// substituting the open-source font for a requested Prime glyph.
pub fn with_prime_fonts<T>(fonts: Arc<PrimeFonts>, draw: impl FnOnce() -> T) -> Result<T> {
    let selection = Selection {
        fonts,
        style: FontStyle::Regular,
    };
    renderer(&selection, |_| Ok(()))?;
    let _scope = Scope {
        previous: ACTIVE.with(|slot| slot.replace(Some(selection))),
        previous_error: FAILURE.with(|slot| slot.replace(None)),
    };
    let value = draw();
    let failure = FAILURE.with(|slot| slot.borrow_mut().take());
    match failure {
        Some(error) => Err(error),
        None => Ok(value),
    }
}
pub fn with_style<T>(style: FontStyle, draw: impl FnOnce() -> T) -> T {
    let previous = ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let previous = slot.clone();
        if let Some(s) = slot.as_mut() {
            s.style = style;
        }
        previous
    });
    struct Restore(Option<Selection>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(previous);
    draw()
}
/// Whether the current thread explicitly selected admitted installed fonts.
pub fn is_selected() -> bool {
    ACTIVE.with(|slot| slot.borrow().is_some())
}
fn selected<T>(call: impl FnOnce(&Selection, &mut Renderer) -> Result<T>) -> Option<Result<T>> {
    let selection = ACTIVE.with(|slot| slot.borrow().clone())?;
    Some(renderer(&selection, |r| call(&selection, r)))
}
fn remember(error: FontError) {
    FAILURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    });
}
pub(super) fn active_glyph(ch: char, px: f32) -> Option<Arc<Glyph>> {
    selected(|s, r| r.glyph(s.style, ch, px)).map(admit_glyph)
}
pub(super) fn active_unit_glyph(unit: u16, px: f32) -> Option<Arc<Glyph>> {
    selected(|s, r| r.glyph_codepoint(s.style, u32::from(unit), px)).map(admit_glyph)
}
fn admit_glyph(result: Result<Arc<Glyph>>) -> Arc<Glyph> {
    result.unwrap_or_else(|error| {
        remember(error);
        Arc::new(Glyph {
            width: 0,
            height: 0,
            xmin: 0,
            ymin: 0,
            advance: 0.,
            bitmap: Vec::new(),
            antialias: true,
        })
    })
}
pub(super) fn active_unit_advance(unit: u16, px: f32) -> Option<f32> {
    selected(|s, r| {
        r.glyph_codepoint(s.style, u32::from(unit), px)
            .map(|glyph| glyph.advance)
    })
    .map(|result| {
        result.unwrap_or_else(|error| {
            remember(error);
            0.
        })
    })
}
pub(super) fn active_metrics(px: f32) -> Option<(i64, i64)> {
    selected(|s, r| r.metrics(s.style, px)).map(|result| {
        result.unwrap_or_else(|error| {
            remember(error);
            (1, 0)
        })
    })
}

/// The G1 glyph compositor keeps seven coverage bits. Its channel products
/// use an eight-bit shift with weights summing to255 (original306221AC).
/// Unlike an opaque primitive, a fully covered font pixel can therefore be253.
pub(super) fn blend(background: u32, foreground: u32, coverage: u8) -> u32 {
    let alpha = u32::from(coverage & 254);
    if alpha == 0 {
        return background;
    }
    let mut result = 0;
    for shift in [0, 8, 16] {
        let channel = (((foreground >> shift) & 255) * alpha
            + ((background >> shift) & 255) * (255 - alpha))
            >> 8;
        result |= channel << shift;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Grob, text};
    fn installed() -> Option<Arc<PrimeFonts>> {
        let path = std::env::var_os("PRIME_TEST_FONT_RESOURCE")?;
        Some(PrimeFonts::load_resource(Path::new(&path)).unwrap())
    }
    #[test]
    fn g1_channel_blending_matches_independently_observed_color_controls() {
        assert_eq!(blend(0xffffff, 0, 0), 0xffffff);
        assert_eq!(blend(0, 0xffffff, 255), 0xfdfdfd);
        assert_eq!(blend(0xffffff, 0, 16), 0xeeeeee);
        assert_eq!(blend(0xef8d2f, 0x115dc9, 16), 0xe08938);
        assert_eq!(blend(0xef8d2f, 0x115dc9, 248), 0x175dc4);
        assert_eq!(blend(0xef8d2f, 0x115dc9, 254), 0x115cc7);
    }

    #[test]
    fn selected_g1_fonts_match_complete_independent_firmware_rasters() {
        let Some(fonts) = installed() else {
            return;
        };
        use sha2::{Digest, Sha256};
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-font-controls.json"
        ))
        .unwrap();
        for case in controls["cases"].as_array().unwrap() {
            let mut screen = Grob::new(320, 240, 0);
            with_prime_fonts(fonts.clone(), || match case["name"].as_str().unwrap() {
                "joined" | "isolated" => {
                    for code in 1..=7 {
                        let px = text::font_px(code, 12.);
                        let y = 20 + code * 26;
                        if case["name"] == "joined" {
                            text::draw_text(
                                &mut screen,
                                "AbgXπ√∑∫\u{e003}",
                                10,
                                y,
                                px,
                                0xffffff,
                                None,
                                None,
                            );
                        } else {
                            for (i, ch) in "AbgXπ√∑∫\u{e003}".chars().enumerate() {
                                text::draw_text(
                                    &mut screen,
                                    &ch.to_string(),
                                    10 + i as i64 * 32,
                                    y,
                                    px,
                                    0xffffff,
                                    None,
                                    None,
                                );
                            }
                        }
                    }
                }
                "layout_color" => {
                    for k in 0..4 {
                        text::draw_text(
                            &mut screen,
                            "AA",
                            10 + 80 * k,
                            40,
                            text::font_px(1 + 2 * k, 12.),
                            0xffffff,
                            None,
                            None,
                        );
                    }
                    screen.fill_rect(0, 130, 319, 239, 0xef8d2f);
                    text::draw_text(&mut screen, "A", 10, 150, 12., 0x115dc9, None, None);
                }
                _ => panic!("unknown G1 font control"),
            })
            .unwrap();
            let rgb = screen
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect::<Vec<_>>();
            assert_eq!(
                format!("{:x}", Sha256::digest(&rgb)),
                case["rgb_sha256"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn resource_admission_rejects_format_bounds_directory_aliases_and_wrong_families() {
        assert!(PrimeFonts::from_resource(&[]).is_err());
        let bytes = text::dejavu_sans_ttf();
        let start = BASE + 0x52 * 8;
        let mut data = vec![0; start + bytes.len()];
        for (i, value) in [u32::MAX, 5, 1, 0, 3, 2, 6].into_iter().enumerate() {
            data[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[start..].copy_from_slice(bytes);
        for id in FONT_IDS {
            data[BASE + id * 8..BASE + id * 8 + 4]
                .copy_from_slice(&((start - BASE) as u32).to_le_bytes());
            data[BASE + id * 8 + 4..BASE + id * 8 + 8]
                .copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        }
        assert!(
            PrimeFonts::from_resource(&data)
                .unwrap_err()
                .0
                .contains("Prime Sans family")
        );
        data[BASE + 0x50 * 8..BASE + 0x50 * 8 + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(PrimeFonts::from_resource(&data).is_err());
        data[BASE + 0x50 * 8..BASE + 0x50 * 8 + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(PrimeFonts::from_resource(&data).is_err());
    }
    #[test]
    fn installed_faces_render_real_antialiased_glyphs_in_scopes_and_keep_threads_independent() {
        let Some(fonts) = installed() else {
            return;
        };
        fn send_sync<T: Send + Sync>() {}
        send_sync::<PrimeFonts>();
        let before = text::measure("AbgXπ√∑∫\u{e003}", 12.);
        let evidence = std::env::var_os("PRIME_TEST_FONT_EVIDENCE").map(std::path::PathBuf::from);
        for style in [FontStyle::Regular, FontStyle::Bold, FontStyle::Mono] {
            let mut screen = Grob::new(320, 240, 0);
            with_prime_fonts(fonts.clone(), || {
                with_style(style, || {
                    for code in 1..=7 {
                        let px = text::font_px(code, 12.);
                        text::draw_text(
                            &mut screen,
                            "AbgXπ√∑∫\u{e003}",
                            10,
                            20 + code * 26,
                            px,
                            0xffffff,
                            None,
                            None,
                        );
                    }
                    let other = std::thread::spawn(|| text::measure("AbgXπ√∑∫\u{e003}", 12.))
                        .join()
                        .unwrap();
                    assert_eq!(
                        other, before,
                        "font selection must not leak into another worker"
                    );
                })
            })
            .unwrap();
            assert!(
                screen.pixels().iter().any(|p| *p != 0 && *p != 0xffffff),
                "genuine font coverage must not be thresholded"
            );
            if let Some(out) = &evidence {
                std::fs::create_dir_all(out).unwrap();
                screen
                    .save_png(&out.join(format!("{style:?}.png")))
                    .unwrap();
            }
            assert_eq!(text::measure("AbgXπ√∑∫\u{e003}", 12.), before);
        }
        let mono = with_prime_fonts(fonts.clone(), || {
            with_style(FontStyle::Mono, || {
                [text::measure("iii", 12.), text::measure("WWW", 12.)]
            })
        })
        .unwrap();
        assert_eq!(mono[0], mono[1]);
        assert!(with_prime_fonts(fonts.clone(), || text::measure("x", f32::NAN)).is_err());
        assert_eq!(text::measure("AbgXπ√∑∫\u{e003}", 12.), before);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = with_prime_fonts(fonts, || panic!("scope unwind"));
        }));
        assert!(panic.is_err());
        assert_eq!(text::measure("AbgXπ√∑∫\u{e003}", 12.), before);
    }
}
