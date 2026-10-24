//! Multi-tab PPL code editor.
//!
//! * [`tab`]: one buffer (text, file identity and format, breakpoints).
//! * [`document`]: derived data cached per revision (analysis, line index,
//!   laid-out galley).
//! * [`text_ops`]: pure editing operations (indent, newline, pairs).
//! * [`input`]: routes keystrokes to `text_ops` before egui's `TextEdit`.
//! * [`view`]: the editor widget (text, bands, squiggles, gutter).
//! * [`find`] / [`find_bar`], [`completion`]: search and completion.
//!
//! Language knowledge comes from [`ppl_compiler::highlight`] (the real
//! lexer); this module never re-tokenizes PPL with its own rules.

pub mod completion;
pub mod document;
pub mod find;
pub mod find_bar;
mod gutter;
pub mod input;
pub mod tab;
pub mod text_ops;
pub mod view;
#[cfg(test)]
mod widget_tests;

use std::path::PathBuf;

use egui::{Align2, Color32, FontId, Key, RichText, Sense, Stroke, pos2, vec2};

pub use self::find_bar::{FindBar, FindCommand};
pub use self::tab::{EditorTab, FileFormat, SaveError, TabAnalysis, TabId};
pub use crate::ide::diagnostics::DiagnosticItem;

use super::syntax::CodeStyle;
use super::{IdeAction, IdeAppState, IdePanel};
use crate::theme::{
    ACCENT_BLUE, BG_PANEL, BORDER_CRISP, TEXT_MUTED, TEXT_NORMAL, editor as ecolors,
};

/// Converts a byte offset to a character offset, clamping safely to char
/// boundaries.
pub fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    let mut offset = byte_offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].chars().count()
}

/// Central editor area hosting the open program tabs.
#[derive(Debug)]
pub struct EditorArea {
    pub tabs: Vec<EditorTab>,
    pub active_tab_index: usize,
    pub find: FindBar,
    pub view: view::ViewState,
    /// Tab last scrolled into view in the tab strip.
    revealed_tab: Option<TabId>,
}

impl Default for EditorArea {
    fn default() -> Self {
        let default_source = "EXPORT DEMO()\nBEGIN\n  PRINT(\"HP Prime PPL Emulator\");\nEND;\n";
        Self {
            tabs: vec![EditorTab::new("DEMO.ppl", default_source)],
            active_tab_index: 0,
            find: FindBar::default(),
            view: view::ViewState::default(),
            revealed_tab: None,
        }
    }
}

impl EditorArea {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_tab(&self) -> Option<&EditorTab> {
        self.tabs.get(self.active_tab_index)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut EditorTab> {
        self.tabs.get_mut(self.active_tab_index)
    }

    /// Index of the tab with `id`.
    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Opens `content` as `path` (or focuses the tab already showing it).
    pub fn open_file(&mut self, path: PathBuf, content: String) {
        self.open_tab(EditorTab::from_file(path, content));
    }

    /// Adds `tab`, unless a tab for the same file is already open, in which
    /// case that one is focused.
    pub fn open_tab(&mut self, tab: EditorTab) {
        if let Some(pos) = tab
            .path
            .as_ref()
            .and_then(|p| self.tabs.iter().position(|t| t.path.as_ref() == Some(p)))
        {
            self.active_tab_index = pos;
            return;
        }
        self.tabs.push(tab);
        self.active_tab_index = self.tabs.len() - 1;
    }

    pub fn new_tab(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.tabs.push(EditorTab::new(title, content));
        self.active_tab_index = self.tabs.len() - 1;
    }

    pub fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if index < self.active_tab_index {
                self.active_tab_index -= 1;
            } else if self.active_tab_index >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab_index = self.tabs.len() - 1;
            }
        }
    }

    /// True when the active tab has unsaved changes.
    pub fn active_tab_modified(&self) -> bool {
        self.active_tab().is_some_and(|t| t.modified)
    }

    /// Opens the find bar, seeded with the active selection.
    pub fn open_find(&mut self, ctx: &egui::Context, replace: bool) {
        let seed = self.active_tab().and_then(|tab| {
            let st = egui::text_edit::TextEditState::load(ctx, view::edit_id(tab.id))?;
            let range = st.cursor.char_range()?;
            let [a, b] = range.sorted_cursors();
            Some(
                tab.content
                    .chars()
                    .skip(a.index)
                    .take(b.index - a.index)
                    .collect(),
            )
        });
        self.find.open(replace, seed);
    }

    /// Runs a find command on the active tab.
    pub fn run_find(&mut self, ctx: &egui::Context, cmd: FindCommand) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            self.find.run(ctx, tab, cmd);
        }
    }

    /// Tab strip: scrollable, active tab underlined, `●` for unsaved
    /// changes, close on hover or middle click.
    fn tab_strip(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        let height = 30.0;
        let mut to_close = None;
        let mut activate = None;
        egui::Frame::NONE.fill(BG_PANEL).show(ui, |ui| {
            ui.set_height(height);
            ui.horizontal(|ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("editor_tab_strip")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .max_width(ui.available_width() - 34.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.horizontal(|ui| {
                            let active_id = self.tabs.get(self.active_tab_index).map(|t| t.id);
                            let reveal = active_id != self.revealed_tab;
                            self.revealed_tab = active_id;
                            for (idx, tab) in self.tabs.iter().enumerate() {
                                let active = idx == self.active_tab_index;
                                let (click, rect) = tab_button(ui, tab, active, height);
                                if active && reveal {
                                    // Newly active (opened, switched): bring
                                    // it into view in the scrolled strip.
                                    ui.scroll_to_rect(rect, None);
                                }
                                match click {
                                    TabClick::Activate => activate = Some(idx),
                                    TabClick::Close => to_close = Some(idx),
                                    TabClick::None => {}
                                }
                            }
                        });
                    });
                if ui
                    .add_sized(
                        [28.0, height - 4.0],
                        egui::Button::new(RichText::new("+").size(15.0)).frame(false),
                    )
                    .on_hover_text("New Program (Ctrl+N)")
                    .clicked()
                {
                    state.queue_action(IdeAction::ShowNewProgramDialog);
                }
            });
        });
        let r = ui.min_rect();
        ui.painter().line_segment(
            [pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)],
            Stroke::new(1.0_f32, BORDER_CRISP),
        );
        if let Some(idx) = activate {
            self.active_tab_index = idx;
        }
        if let Some(idx) = to_close {
            // Unsaved changes: queue CloseActiveTab (which confirms).
            if self.tabs.get(idx).is_some_and(|t| t.modified) {
                self.active_tab_index = idx;
                state.queue_action(IdeAction::CloseActiveTab);
            } else {
                self.close_tab(idx);
            }
        }
    }
}

enum TabClick {
    None,
    Activate,
    Close,
}

fn tab_button(
    ui: &mut egui::Ui,
    tab: &EditorTab,
    active: bool,
    height: f32,
) -> (TabClick, egui::Rect) {
    let font = FontId::proportional(13.0);
    let title_w = ui.fonts(|f| {
        f.layout_no_wrap(tab.title.clone(), font.clone(), Color32::WHITE)
            .size()
            .x
    });
    let (rect, resp) = ui.allocate_exact_size(vec2(title_w + 46.0, height), Sense::click());
    let close_rect =
        egui::Rect::from_center_size(pos2(rect.max.x - 15.0, rect.center().y), vec2(16.0, 16.0));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let over_close = pointer.is_some_and(|p| close_rect.contains(p));
    let painter = ui.painter();
    let bg = if active {
        ecolors::BG
    } else if resp.hovered() {
        crate::theme::BG_WIDGET
    } else {
        BG_PANEL
    };
    painter.rect_filled(rect, 0.0, bg);
    painter.line_segment(
        [
            pos2(rect.max.x - 0.5, rect.min.y + 6.0),
            pos2(rect.max.x - 0.5, rect.max.y - 6.0),
        ],
        Stroke::new(1.0_f32, BORDER_CRISP),
    );
    if active {
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, vec2(rect.width(), 2.0)),
            0.0,
            ACCENT_BLUE,
        );
    }
    painter.text(
        pos2(rect.min.x + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        &tab.title,
        font,
        if active { TEXT_NORMAL } else { TEXT_MUTED },
    );
    // Unsaved dot, replaced by the close button on hover.
    if resp.hovered() || active {
        let color = if over_close { TEXT_NORMAL } else { TEXT_MUTED };
        if over_close {
            painter.rect_filled(close_rect, 3.0, crate::theme::BG_WIDGET);
        }
        if tab.modified && !resp.hovered() {
            painter.circle_filled(close_rect.center(), 4.0, TEXT_NORMAL);
        } else {
            painter.text(
                close_rect.center(),
                Align2::CENTER_CENTER,
                "×",
                FontId::proportional(15.0),
                color,
            );
        }
    } else if tab.modified {
        painter.circle_filled(close_rect.center(), 4.0, TEXT_MUTED);
    }
    let path = tab
        .path
        .as_ref()
        .map_or_else(|| "Not saved yet".to_string(), |p| p.display().to_string());
    let resp = resp.on_hover_text(if tab.modified {
        format!("{path}\nUnsaved changes")
    } else {
        path
    });
    let click = if resp.middle_clicked() || (resp.clicked() && over_close) {
        TabClick::Close
    } else if resp.clicked() {
        TabClick::Activate
    } else {
        TabClick::None
    };
    (click, rect)
}

impl IdePanel for EditorArea {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        self.tab_strip(ui, state);

        if self.tabs.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(
                        "No programs open.\nUse File → New Program (Ctrl+N) or Open (Ctrl+O).",
                    )
                    .color(TEXT_MUTED),
                );
            });
            return;
        }
        self.active_tab_index = self.active_tab_index.min(self.tabs.len() - 1);
        let ctx = ui.ctx().clone();

        // F3 / Shift+F3 repeat the last search from anywhere in the editor.
        if !self.find.query.needle.is_empty() {
            let (f3, shift) = ui.input(|i| (i.key_pressed(Key::F3), i.modifiers.shift));
            if f3 {
                self.find.open = true;
                self.run_find(
                    &ctx,
                    if shift {
                        FindCommand::Prev
                    } else {
                        FindCommand::Next
                    },
                );
            }
        }
        let mut find_matches = None;
        if self.find.open {
            let tab = &mut self.tabs[self.active_tab_index];
            let matches = self.find.matches(tab);
            if let Some(cmd) = self.find.show(ui, matches.len()) {
                self.run_find(&ctx, cmd);
            }
            if self.find.open {
                find_matches = Some(self.find.matches(&mut self.tabs[self.active_tab_index]));
            }
        }

        let style = CodeStyle::new(state.editor_font_size);
        let current = self.find.current.clone();
        let tab = &mut self.tabs[self.active_tab_index];
        view::show(
            ui,
            tab,
            state,
            &mut self.view,
            view::ViewInput {
                style,
                find: find_matches.as_deref().map(|m| view::FindHighlight {
                    matches: m.as_slice(),
                    current,
                }),
            },
        );
    }
}
