//! Unicode pages observed in the G1 firmware's Character Pages menu.
//! Page ranges follow the corresponding BMP Unicode blocks. Missing font
//! glyphs show their code point; selecting them still inserts the real scalar.
use super::Request;
use serde::{Deserialize, Serialize};

pub const PAGES: &[(&str, u32, u32)] = &[
    ("Latin", 0x20, 0x7f),
    ("Latin Sup.", 0xa0, 0xff),
    ("Latin Ext–A", 0x100, 0x17f),
    ("Latin Ext–B", 0x180, 0x24f),
    ("IPA Extensions", 0x250, 0x2af),
    ("Space Modifiers", 0x2b0, 0x2ff),
    ("Combining Diacritics", 0x300, 0x36f),
    ("Greek and Coptic", 0x370, 0x3ff),
    ("Cyrillic", 0x400, 0x4ff),
    ("Cyrillic Sup.", 0x500, 0x52f),
    ("Hangul Jamo", 0x1100, 0x11ff),
    ("Phonetic Extns.", 0x1d00, 0x1d7f),
    ("Latin Ext. Additional", 0x1e00, 0x1eff),
    ("General Punctuation", 0x2000, 0x206f),
    ("Subscript/Superscript", 0x2070, 0x209f),
    ("Currency", 0x20a0, 0x20cf),
    ("Letterlike Symb.", 0x2100, 0x214f),
    ("Number Forms", 0x2150, 0x218f),
    ("Arrows", 0x2190, 0x21ff),
    ("Math Operators", 0x2200, 0x22ff),
    ("Misc. Technical", 0x2300, 0x23ff),
    ("Control Pictures", 0x2400, 0x243f),
    ("Encl. Alphanumerics", 0x2460, 0x24ff),
    ("Box Drawing", 0x2500, 0x257f),
    ("Block Elements", 0x2580, 0x259f),
    ("Geometric Shapes", 0x25a0, 0x25ff),
    ("Misc. Symbols", 0x2600, 0x26ff),
    ("Dingbats", 0x2700, 0x27bf),
    ("Sup. Arrows–B", 0x2900, 0x297f),
    ("CJK Symb. and Punc.", 0x3000, 0x303f),
    ("Hiragana", 0x3040, 0x309f),
    ("Katakana", 0x30a0, 0x30ff),
    ("Bopomofo", 0x3100, 0x312f),
    ("Hangul Comp. Jamo", 0x3130, 0x318f),
    ("Enc. CJK Letters & Months", 0x3200, 0x32ff),
    ("CJK Comp.", 0x3300, 0x33ff),
    ("CJK Uni. Ideographs Ext–A", 0x3400, 0x4dbf),
    ("CJK Uni. Ideographs", 0x4e00, 0x9fff),
    ("Hangul Syllables", 0xac00, 0xd7af),
    ("Private Use Area", 0xe000, 0xf8ff),
    ("CJK Comp. Ideographs", 0xf900, 0xfaff),
    ("Vertical Forms", 0xfe10, 0xfe1f),
    ("CJK Com. Forms", 0xfe30, 0xfe4f),
    ("Small Form Variants", 0xfe50, 0xfe6f),
    ("Halfwidth & Fullwidth Forms", 0xff00, 0xffef),
    ("Specials", 0xfff0, 0xfffd),
];
const COLUMNS: u32 = 16;
const CELLS: u32 = COLUMNS * 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterSelection {
    pub page: usize,
    pub code: u32,
    #[serde(skip)]
    menu_selection: Option<usize>,
}
impl Default for CharacterSelection {
    fn default() -> Self {
        Self {
            page: 0,
            code: 0x20,
            menu_selection: None,
        }
    }
}
impl CharacterSelection {
    pub fn validate(&self) -> Result<(), String> {
        if PAGES.get(self.page).is_some_and(|&(_, first, last)| {
            (first..=last).contains(&self.code) && char::from_u32(self.code).is_some()
        }) {
            Ok(())
        } else {
            Err("Invalid saved character page or selection".into())
        }
    }
    pub fn select_page(&mut self, page: usize) {
        if let Some(&(_, first, _)) = PAGES.get(page) {
            self.page = page;
            self.code = first;
        }
    }
    pub fn move_selection(&mut self, delta: i32) {
        let (_, first, last) = PAGES[self.page];
        self.code = (i64::from(self.code) + i64::from(delta))
            .clamp(i64::from(first), i64::from(last)) as u32;
    }
    pub fn selected(&self) -> String {
        char::from_u32(self.code)
            .expect("validated character selection")
            .to_string()
    }
    fn grid_start(&self) -> u32 {
        let first = PAGES[self.page].1;
        first + (self.code - first) / CELLS * CELLS
    }
    pub(super) fn show(&mut self, ui: &mut egui::Ui, scale: f32, requests: &mut Vec<Request>) {
        ui.horizontal(|ui| {
            ui.label("Page");
            let popup = egui::ComboBox::from_id_salt("character_pages")
                .selected_text(PAGES[self.page].0)
                .width(210. * scale)
                .height(160. * scale)
                .show_ui(ui, |ui| {
                    let mut cursor = self.menu_selection.unwrap_or(self.page);
                    let mut moved = self.menu_selection.is_none();
                    ui.input_mut(|input| {
                        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                            cursor = cursor.saturating_sub(1);
                            moved = true;
                        }
                        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                            cursor = (cursor + 1).min(PAGES.len() - 1);
                            moved = true;
                        }
                    });
                    self.menu_selection = Some(cursor);
                    let accept = ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    });
                    for (index, &(name, _, _)) in PAGES.iter().enumerate() {
                        let response = ui.selectable_label(cursor == index, name);
                        if moved && cursor == index {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() || (accept && cursor == index) {
                            self.select_page(index);
                            ui.close();
                            if let Some(id) = ui.memory(|memory| memory.focused()) {
                                ui.memory_mut(|memory| memory.surrender_focus(id));
                            }
                        }
                    }
                });
            if popup.inner.is_none() {
                self.menu_selection = None;
            }
        });
        let (_, first, last) = PAGES[self.page];
        let start = self.grid_start();
        let cell_width = (ui.available_width() / COLUMNS as f32).min(20. * scale);
        let font = crate::ide::native_fonts::font_id(ui, egui::TextStyle::Body, 13. * scale);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for row in 0..6 {
                ui.horizontal(|ui| {
                    for column in 0..COLUMNS {
                        let code = start + row * COLUMNS + column;
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(cell_width, 18. * scale),
                            egui::Sense::click(),
                        );
                        if code > last {
                            continue;
                        }
                        let ch = char::from_u32(code).expect("BMP page excludes surrogates");
                        let visible = !ch.is_control()
                            && !ch.is_whitespace()
                            && ui.fonts(|fonts| fonts.has_glyph(&font, ch));
                        let (label, font) = if visible {
                            (ch.to_string(), font.clone())
                        } else {
                            (
                                format!("{code:04X}"),
                                crate::ide::native_fonts::font_id(
                                    ui,
                                    egui::TextStyle::Monospace,
                                    6. * scale,
                                ),
                            )
                        };
                        let selected = self.code == code;
                        let visuals = ui.style().interact_selectable(&response, selected);
                        ui.painter()
                            .rect_filled(rect.shrink(0.5 * scale), 0., visuals.bg_fill);
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            label,
                            font,
                            visuals.text_color(),
                        );
                        if response.clicked() {
                            self.code = code;
                        }
                        if response.double_clicked() {
                            requests.push(Request::Insert(ch.to_string()));
                        }
                        response.on_hover_text(format!("U+{code:04X} · {code} decimal"));
                    }
                });
            }
        });
        ui.label(format!("U+{:04X} · {} decimal", self.code, self.code));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(start > first, egui::Button::new("Previous"))
                .clicked()
            {
                self.move_selection(-(CELLS as i32));
            }
            if ui
                .add_enabled(start + CELLS <= last, egui::Button::new("Next"))
                .clicked()
            {
                self.move_selection(CELLS as i32);
            }
            if ui.button("OK").clicked() {
                requests.push(Request::Insert(self.selected()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_cover_reference_sections_and_navigation_preserves_unicode() {
        let mut selection = CharacterSelection::default();
        for (page, &(_, first, last)) in PAGES.iter().enumerate() {
            selection.select_page(page);
            assert_eq!(selection.code, first);
            selection.move_selection(i32::MAX);
            assert_eq!(selection.code, last);
            assert_eq!(selection.selected().chars().next().unwrap() as u32, last);
            assert!(selection.validate().is_ok());
            let restored: CharacterSelection =
                serde_json::from_str(&serde_json::to_string(&selection).unwrap()).unwrap();
            assert_eq!(selection, restored);
        }
        selection.select_page(7);
        selection.move_selection(0x3c0 - 0x370);
        assert_eq!(selection.selected(), "π");
        assert_eq!(selection.grid_start(), 0x370);
        selection.select_page(19);
        selection.move_selection(0x2264 - 0x2200);
        assert_eq!(selection.selected(), "≤");
        assert_eq!(selection.grid_start(), 0x2260);
    }
    #[test]
    fn old_profiles_default_and_invalid_selections_are_rejected() {
        let old: super::super::ToolMemory = serde_json::from_str("{}").unwrap();
        assert_eq!(old.characters, CharacterSelection::default());
        for selection in [
            CharacterSelection {
                page: usize::MAX,
                code: 0x20,
                ..Default::default()
            },
            CharacterSelection {
                page: 0,
                code: 0xd800,
                ..Default::default()
            },
            CharacterSelection {
                page: 7,
                code: 0x41,
                ..Default::default()
            },
        ] {
            assert!(selection.validate().is_err());
        }
    }
}
