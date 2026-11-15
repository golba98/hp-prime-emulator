//! Optional calculator-only fonts from the user's installed firmware resources.
//! Vendor bytes remain in their installed archive; IDE font families are kept.
use super::firmware::FirmwareSettings;
use ppl_graphics::text::prime::PrimeFonts;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub const REGULAR: &str = "prime-lcd-regular";
pub const BOLD: &str = "prime-lcd-bold";
pub const MONO: &str = "prime-lcd-mono";

#[derive(Default)]
pub struct NativeFonts {
    key: Option<(String, String, Option<std::ffi::OsString>)>,
    pub selected: Option<Arc<PrimeFonts>>,
    pub source: Option<PathBuf>,
    pub error: Option<String>,
    pub generation: u64,
    render_failure_at: Option<u64>,
}
impl NativeFonts {
    /// A path change is explicit selection; an unreadable selected resource is
    /// reported, rather than silently presenting a substituted font as Prime.
    pub fn refresh(&mut self, settings: &FirmwareSettings) -> bool {
        let explicit = std::env::var_os("PPL_PRIME_FONT_RESOURCE").filter(|p| !p.is_empty());
        let key = (
            settings.factory_a.clone(),
            settings.init_data.clone(),
            explicit.clone(),
        );
        if self.key.as_ref() == Some(&key) {
            return false;
        }
        self.key = Some(key);
        let resource = if let Some(path) = explicit {
            Ok(Some(PathBuf::from(path)))
        } else {
            settings.asset_paths().map(|paths| resource_path(&paths))
        };
        match resource {
            Ok(path) => self.select(path.as_deref()),
            Err(error) => {
                self.selected = None;
                self.source = None;
                self.error = Some(error);
                self.generation = self.generation.wrapping_add(1);
            }
        }
        true
    }
    pub fn select(&mut self, source: Option<&Path>) {
        self.render_failure_at = None;
        self.source = source.map(Path::to_path_buf);
        self.error = None;
        self.selected = match source {
            Some(path) => match PrimeFonts::load_resource(path).and_then(|fonts| {
                fonts.menu_texture(false)?;
                Ok(fonts)
            }) {
                Ok(fonts) => Some(fonts),
                Err(error) => {
                    self.error = Some(format!("{}: {error}", path.display()));
                    None
                }
            },
            None => None,
        };
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn retry_after_edit(&mut self, view_generation: u64) {
        if self.selected.is_some()
            && self
                .render_failure_at
                .is_some_and(|at| at != view_generation)
        {
            self.error = None;
            self.render_failure_at = None;
        }
    }
    pub fn render_failed(&mut self, error: String, view_generation: u64) {
        self.error = Some(error);
        self.render_failure_at = Some(view_generation);
    }
    pub fn definitions(&self) -> egui::FontDefinitions {
        let mut definitions = crate::theme::font_definitions();
        for (name, base) in [
            (REGULAR, egui::FontFamily::Proportional),
            (BOLD, egui::FontFamily::Proportional),
            (MONO, egui::FontFamily::Monospace),
        ] {
            definitions.families.insert(
                egui::FontFamily::Name(name.into()),
                definitions.families[&base].clone(),
            );
        }
        if let Some(fonts) = &self.selected {
            for (name, bytes) in [
                (REGULAR, fonts.full_bytes()),
                (BOLD, fonts.bold_bytes()),
                (MONO, fonts.mono_bytes()),
            ] {
                definitions.font_data.insert(
                    name.into(),
                    Arc::new(egui::FontData::from_owned(bytes.to_vec())),
                );
                definitions
                    .families
                    .insert(egui::FontFamily::Name(name.into()), vec![name.into()]);
            }
        }
        definitions
    }
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "family": if self.error.is_some() { "unavailable" } else if self.selected.is_some() { "Prime Sans" } else { "DejaVu Sans" },
            "source": self.source.as_ref().map(|path| path.to_string_lossy().into_owned()),
            "error": self.error,
            "generation": self.generation,
        })
    }
    pub fn problem_screen(&self) -> Option<ppl_graphics::Grob> {
        self.error.as_ref()?;
        let mut screen = ppl_graphics::Grob::screen();
        for (text, y) in [
            ("Calculator font unavailable", 40),
            ("Check the firmware resource path", 70),
            ("in emulator settings.", 86),
        ] {
            ppl_graphics::text::draw_text(&mut screen, text, 8, y, 13., 0, Some(304), None);
        }
        Some(screen)
    }
}
fn resource_path(paths: &ppl_engine::firmware_options::AssetPaths) -> Option<PathBuf> {
    paths
        .factory_a
        .as_ref()
        .map(|p| p.join("programs/misc/armfir.dat"))
        .or_else(|| {
            paths
                .init_data
                .as_ref()
                .map(|p| p.join("A/programs/misc/armfir.dat"))
        })
}

/// Resolve through the LCD's local text style, preserving the IDE's global
/// monospace family for code and other panels.
pub fn font_id(ui: &egui::Ui, style: egui::TextStyle, size: f32) -> egui::FontId {
    let mut font = style.resolve(ui.style());
    font.size = size;
    font
}
pub fn monospace(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(egui::RichText::new(text).text_style(egui::TextStyle::Monospace))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_resource_precedence_does_not_depend_on_existing_files() {
        let mut paths = ppl_engine::firmware_options::AssetPaths::default();
        assert!(resource_path(&paths).is_none());
        paths.init_data = Some(PathBuf::from("initialized"));
        assert_eq!(
            resource_path(&paths).unwrap(),
            PathBuf::from("initialized/A/programs/misc/armfir.dat")
        );
        paths.factory_a = Some(PathBuf::from("factory"));
        assert_eq!(
            resource_path(&paths).unwrap(),
            PathBuf::from("factory/programs/misc/armfir.dat")
        );
    }
    #[test]
    fn missing_selection_reports_error_and_unselected_uses_the_bundled_family() {
        let directory = std::env::temp_dir().join(format!(
            "prime-font-absent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut fonts = NativeFonts::default();
        fonts.select(Some(&directory.join("absent.dat")));
        assert!(fonts.selected.is_none());
        assert!(fonts.error.is_some());
        assert_eq!(fonts.snapshot()["family"], "unavailable");
        fonts.select(None);
        assert!(fonts.error.is_none());
        assert_eq!(fonts.snapshot()["family"], "DejaVu Sans");
    }
    #[test]
    fn selected_fonts_with_a_missing_menu_texture_report_the_resource_error() {
        let Some(source) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
            return;
        };
        let mut resource = std::fs::read(source).unwrap();
        // Legacy resource directory: invalidate only the normal menu record,
        // leaving the three independently validated font faces intact.
        resource[28 + 6 * 8..28 + 6 * 8 + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let directory = std::env::temp_dir().join(format!(
            "prime-menu-invalid-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("resource.dat");
        std::fs::write(&path, resource).unwrap();
        let mut selected = NativeFonts::default();
        selected.select(Some(&path));
        assert!(selected.selected.is_none());
        assert_eq!(selected.snapshot()["family"], "unavailable");
        assert!(selected.error.as_deref().unwrap().contains("menu resource"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn installed_families_leave_every_ide_family_and_byte_source_unchanged() {
        let Some(path) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
            return;
        };
        let mut fonts = NativeFonts::default();
        fonts.select(Some(Path::new(&path)));
        assert!(fonts.error.is_none(), "{:?}", fonts.error);
        let base = crate::theme::font_definitions();
        let selected = fonts.definitions();
        for (family, chain) in &base.families {
            assert_eq!(selected.families[family], *chain);
        }
        for (name, data) in &base.font_data {
            assert_eq!(selected.font_data[name].font, data.font);
        }
        assert_eq!(
            selected.families[&egui::FontFamily::Name(REGULAR.into())],
            [REGULAR]
        );
        assert_eq!(fonts.snapshot()["family"], "Prime Sans");
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-ui-font-controls.json"
        ))
        .unwrap();
        ppl_graphics::text::with_prime_fonts(fonts.selected.clone().unwrap(), || {
            for control in controls["controls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| matches!(c["kind"].as_str(), Some("title" | "softkeys")))
            {
                let mut frame = ppl_graphics::Grob::screen();
                if control["kind"] == "title" {
                    super::super::native::chrome::header(
                        &mut frame,
                        control["title"].as_str().unwrap(),
                    );
                } else {
                    let labels = std::array::from_fn(|i| control["labels"][i].as_str().unwrap());
                    let menus = control["menu_buttons"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| i.as_u64().unwrap() as usize)
                        .collect::<Vec<_>>();
                    super::super::native::chrome::softkeys_with_menus(&mut frame, &labels, &menus);
                }
                let region = control["region"].as_array().unwrap();
                let [x, y, width, height] =
                    std::array::from_fn::<_, 4, _>(|i| region[i].as_u64().unwrap() as usize);
                let mut bytes = Vec::with_capacity(width * height * 3);
                for row in y..y + height {
                    for column in x..x + width {
                        let pixel = frame.pixels()[row * 320 + column];
                        bytes.extend([(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
                    }
                }
                assert_eq!(
                    ppl_engine::source::hash(&bytes),
                    control["rgb24_sha256"].as_str().unwrap(),
                    "{}",
                    control["id"]
                );
            }
        })
        .unwrap();
        fonts.render_failed("invalid glyph size".into(), 10);
        fonts.retry_after_edit(10);
        assert!(fonts.error.is_some());
        fonts.retry_after_edit(11);
        assert!(
            fonts.error.is_none(),
            "an edited expression can render again"
        );
    }
}
