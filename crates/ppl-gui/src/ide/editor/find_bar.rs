//! The in-editor find/replace bar (Ctrl+F / Ctrl+H, F3 / Shift+F3).

use std::ops::Range;
use std::sync::Arc;

use egui::text_edit::TextEditState;
use egui::{Key, RichText};

use super::find::FindQuery;
use super::input;
use super::tab::{EditorTab, TabId};
use super::view::edit_id;
use crate::theme::{ACCENT_BLUE, TEXT_MUTED};

/// A request from the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindCommand {
    Next,
    Prev,
    Replace,
    ReplaceAll,
    Close,
}

#[derive(Debug)]
struct MatchCache {
    tab: TabId,
    revision: u64,
    query: FindQuery,
    matches: Arc<Vec<Range<usize>>>,
}

#[derive(Debug, Default)]
pub struct FindBar {
    pub open: bool,
    pub show_replace: bool,
    pub query: FindQuery,
    pub replacement: String,
    /// Byte range of the match last navigated to.
    pub current: Option<Range<usize>>,
    pub status: String,
    focus_input: bool,
    cache: Option<MatchCache>,
}

impl FindBar {
    /// Opens the bar, seeding the query with `seed` (the selection).
    pub fn open(&mut self, replace: bool, seed: Option<String>) {
        self.open = true;
        self.show_replace |= replace;
        if let Some(s) = seed.filter(|s| !s.is_empty() && !s.contains('\n')) {
            self.query.needle = s;
        }
        self.focus_input = true;
    }

    /// All matches in `tab`, cached by tab revision and query.
    pub fn matches(&mut self, tab: &mut EditorTab) -> Arc<Vec<Range<usize>>> {
        let revision = tab.revision();
        if let Some(c) = &self.cache {
            if c.tab == tab.id && c.revision == revision && c.query == self.query {
                return c.matches.clone();
            }
        }
        let matches = Arc::new(self.query.matches(&tab.content));
        self.cache = Some(MatchCache {
            tab: tab.id,
            revision,
            query: self.query.clone(),
            matches: matches.clone(),
        });
        matches
    }

    /// Draws the bar; returns the command the user issued, if any.
    pub fn show(&mut self, ui: &mut egui::Ui, match_count: usize) -> Option<FindCommand> {
        let mut cmd = None;
        egui::Frame::NONE
            .fill(crate::theme::BG_WIDGET)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Find").color(TEXT_MUTED));
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.query.needle)
                            .id_salt("ppl_find_input")
                            .desired_width(220.0)
                            .hint_text("text to find"),
                    );
                    if std::mem::take(&mut self.focus_input) {
                        r.request_focus();
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        cmd = Some(if ui.input(|i| i.modifiers.shift) {
                            FindCommand::Prev
                        } else {
                            FindCommand::Next
                        });
                        r.request_focus();
                    }
                    if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        cmd = Some(FindCommand::Close);
                    }
                    toggle(ui, &mut self.query.match_case, "Aa", "Match case");
                    toggle(ui, &mut self.query.whole_word, "W", "Whole word");
                    if ui
                        .small_button("↑")
                        .on_hover_text("Previous (Shift+F3)")
                        .clicked()
                    {
                        cmd = Some(FindCommand::Prev);
                    }
                    if ui.small_button("↓").on_hover_text("Next (F3)").clicked() {
                        cmd = Some(FindCommand::Next);
                    }
                    let summary = if self.query.needle.is_empty() {
                        String::new()
                    } else if match_count == 0 {
                        "No results".to_string()
                    } else {
                        format!(
                            "{match_count} match{}",
                            if match_count == 1 { "" } else { "es" }
                        )
                    };
                    ui.label(RichText::new(summary).color(TEXT_MUTED).size(11.0));
                    if !self.status.is_empty() {
                        ui.label(RichText::new(&self.status).color(TEXT_MUTED).size(11.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                            cmd = Some(FindCommand::Close);
                        }
                        let label = if self.show_replace {
                            "▾ Replace"
                        } else {
                            "▸ Replace"
                        };
                        if ui.small_button(label).clicked() {
                            self.show_replace = !self.show_replace;
                        }
                    });
                });
                if self.show_replace {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Replace").color(TEXT_MUTED));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.replacement)
                                .id_salt("ppl_replace_input")
                                .desired_width(220.0),
                        );
                        if ui.button("Replace").clicked() {
                            cmd = Some(FindCommand::Replace);
                        }
                        if ui.button("Replace All").clicked() {
                            cmd = Some(FindCommand::ReplaceAll);
                        }
                    });
                }
            });
        cmd
    }

    /// Executes `cmd` on `tab`.
    pub fn run(&mut self, ctx: &egui::Context, tab: &mut EditorTab, cmd: FindCommand) {
        self.status.clear();
        let id = edit_id(tab.id);
        if cmd == FindCommand::Close {
            self.open = false;
            self.current = None;
            ctx.memory_mut(|m| m.request_focus(id));
            return;
        }
        if self.query.needle.is_empty() {
            return;
        }
        let state = TextEditState::load(ctx, id).unwrap_or_default();
        let range = state.cursor.char_range().unwrap_or_default();
        let [a, b] = range.sorted_cursors();
        let lines = tab.document().lines().clone();
        let sel_bytes =
            lines.byte_of_char(&tab.content, a.index)..lines.byte_of_char(&tab.content, b.index);

        let mut from = sel_bytes.end;
        let mut backwards = false;
        match cmd {
            FindCommand::Prev => {
                from = sel_bytes.start;
                backwards = true;
            }
            FindCommand::Replace => {
                let matches = self.matches(tab);
                if matches.contains(&sel_bytes) {
                    let before = (range, tab.content.clone());
                    tab.content
                        .replace_range(sel_bytes.clone(), &self.replacement);
                    let end = sel_bytes.start + self.replacement.len();
                    let end_char = tab.content[..end].chars().count();
                    let mut st = state;
                    input::commit(&mut st, before, &tab.content, end_char..end_char);
                    st.store(ctx, id);
                    tab.sync();
                    from = end;
                }
            }
            FindCommand::ReplaceAll => {
                let (new_text, n) = self.query.replace_all(&tab.content, &self.replacement);
                if n > 0 {
                    let before = (range, tab.content.clone());
                    tab.content = new_text;
                    let mut st = state;
                    input::commit(&mut st, before, &tab.content, 0..0);
                    st.store(ctx, id);
                    tab.sync();
                }
                self.status = format!("Replaced {n}");
                self.current = None;
                return;
            }
            _ => {}
        }
        match self.query.find_from(&tab.content, from, backwards) {
            Some(m) => {
                let lines = tab.document().lines().clone();
                let s = lines.char_of_byte(&tab.content, m.start);
                let e = lines.char_of_byte(&tab.content, m.end);
                tab.select(s, e);
                self.current = Some(m);
            }
            None => {
                self.current = None;
                self.status = "No results".into();
            }
        }
    }
}

fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str, hint: &str) {
    let text = if *value {
        RichText::new(label).color(ACCENT_BLUE).strong()
    } else {
        RichText::new(label).color(TEXT_MUTED)
    };
    if ui
        .selectable_label(*value, text)
        .on_hover_text(hint)
        .clicked()
    {
        *value = !*value;
    }
}
