//! Bottom dock Problems tab: every diagnostic of every open program, with
//! its class (syntax, semantic, compatibility, runtime), filters per
//! class, and click-to-navigate.

use std::collections::BTreeSet;

use egui::{RichText, ScrollArea};
use ppl_syntax::{DiagnosticClass, Severity};

use super::diagnostics::DiagnosticItem;
use super::{IdeAppState, IdePanel};
use crate::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_RED, TEXT_MUTED};

/// Problems panel displaying diagnostics with click-to-navigate.
#[derive(Debug, Default)]
pub struct ProblemsPanel {
    pub items: Vec<DiagnosticItem>,
    /// Kept for callers that track which tab the list was built for.
    pub active_tab_index: usize,
    /// Classes currently hidden by the filter chips.
    pub hidden: BTreeSet<DiagnosticClass>,
    /// Open files whose current text has no compiler verdict yet: the list
    /// is incomplete while this is non-empty.
    pub pending: Vec<String>,
    /// Open files the compiler failed on internally: `(file, reason)`.
    pub failures: Vec<(String, String)>,
    /// Screen rect of each visible row's message, in display order (for
    /// pointer-driven tests).
    pub(crate) row_rects: Vec<egui::Rect>,
}

fn severity_color(s: Severity) -> egui::Color32 {
    match s {
        Severity::Error => ACCENT_RED,
        Severity::Warning => ACCENT_AMBER,
        Severity::Note => ACCENT_BLUE,
    }
}

impl ProblemsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the list; items are ordered by file, line and column.
    pub fn set_diagnostics(&mut self, mut items: Vec<DiagnosticItem>) {
        items.sort_by(|a, b| {
            (&a.file, a.line, a.col, a.severity).cmp(&(&b.file, b.line, b.col, b.severity))
        });
        self.items = items;
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Every open file has a compiler verdict for its current text, so an
    /// empty list really means "no problems".
    pub fn is_settled(&self) -> bool {
        self.pending.is_empty() && self.failures.is_empty()
    }

    /// The dock tab title. The count is only claimed once settled.
    pub fn title(&self) -> String {
        let total = self.items.len();
        let errors = self.error_count();
        let count = if self.is_settled() {
            total.to_string()
        } else {
            format!("{total}…")
        };
        if errors > 0 {
            format!("Problems ({count}, {errors} errors)")
        } else {
            format!("Problems ({count})")
        }
    }

    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count()
    }

    pub fn count_of(&self, class: DiagnosticClass) -> usize {
        self.items.iter().filter(|i| i.class == class).count()
    }

    /// Items not hidden by the class filters.
    pub fn visible(&self) -> impl Iterator<Item = &DiagnosticItem> {
        self.items
            .iter()
            .filter(|i| !self.hidden.contains(&i.class))
    }
}

impl IdePanel for ProblemsPanel {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let errs = self.error_count();
                let warns = self.warning_count();
                ui.label(RichText::new(format!("{errs} errors")).color(if errs > 0 {
                    ACCENT_RED
                } else {
                    TEXT_MUTED
                }));
                ui.label(
                    RichText::new(format!("{warns} warnings")).color(if warns > 0 {
                        ACCENT_AMBER
                    } else {
                        TEXT_MUTED
                    }),
                );
                ui.separator();
                for class in DiagnosticClass::ALL {
                    let shown = !self.hidden.contains(&class);
                    let label = format!("{} {}", class.label(), self.count_of(class));
                    if ui
                        .selectable_label(shown, RichText::new(label).size(12.0))
                        .on_hover_text(match class {
                            DiagnosticClass::Syntax => {
                                "Malformed source: invalid tokens, statements, delimiters"
                            }
                            DiagnosticClass::Semantic => {
                                "Language rule violations: undeclared names, wrong argument counts"
                            }
                            DiagnosticClass::Compatibility => {
                                "Valid HP Prime code this emulator cannot (fully) run"
                            }
                            DiagnosticClass::Runtime => "Errors raised while running the program",
                        })
                        .clicked()
                    {
                        if shown {
                            self.hidden.insert(class);
                        } else {
                            self.hidden.remove(&class);
                        }
                    }
                }
            });
            ui.add(egui::Separator::default().spacing(4.0));

            for (file, reason) in &self.failures {
                ui.label(
                    RichText::new(format!(
                        "{file}: analysis failed ({reason}); the source was not checked"
                    ))
                    .color(ACCENT_RED)
                    .size(12.0),
                );
            }
            if !self.pending.is_empty() {
                ui.label(
                    RichText::new(format!("Analysing {}…", self.pending.join(", ")))
                        .color(TEXT_MUTED)
                        .size(12.0),
                );
            }
            self.row_rects.clear();
            if self.items.is_empty() {
                if self.is_settled() {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("No problems detected.").color(TEXT_MUTED));
                    });
                }
                return;
            }
            let grouped = self.items.iter().any(|i| i.file != self.items[0].file);

            let mut navigate = None;
            ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
                egui::Grid::new("problems_grid")
                    .striped(true)
                    .num_columns(6)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        for h in ["Class", "Severity", "File", "Location", "Code", "Message"] {
                            ui.label(RichText::new(h).strong().size(12.0));
                        }
                        ui.end_row();

                        let mut file: Option<&str> = None;
                        let mut rects = Vec::new();
                        for item in self.visible() {
                            if grouped && file != Some(item.file.as_str()) {
                                file = Some(item.file.as_str());
                                let n = self.items.iter().filter(|i| i.file == item.file).count();
                                ui.label(
                                    RichText::new(format!("{} ({n})", item.file))
                                        .strong()
                                        .size(12.0),
                                );
                                ui.end_row();
                            }
                            let color = severity_color(item.severity);
                            ui.label(RichText::new(item.class.label()).size(12.0));
                            ui.label(RichText::new(item.severity_label()).color(color).size(12.0));
                            ui.label(RichText::new(&item.file).size(12.0).color(TEXT_MUTED));
                            let loc = ui.link(
                                RichText::new(format!("Ln {}, Col {}", item.line, item.col))
                                    .size(12.0),
                            );
                            let code = ui.selectable_label(
                                false,
                                RichText::new(&item.code).monospace().size(12.0),
                            );
                            let mut message = item.message.clone();
                            if item.unverified {
                                message.push_str("  (unverified rule)");
                            }
                            if let Some(h) = &item.hint {
                                message.push_str(&format!("  — {h}"));
                            }
                            let msg = ui.selectable_label(false, RichText::new(message).size(12.0));
                            rects.push(msg.rect);
                            if loc.clicked() || code.clicked() || msg.clicked() {
                                navigate = Some((item.tab, item.line, item.col));
                            }
                            ui.end_row();
                        }
                        self.row_rects = rects;
                    });
            });
            if let Some((tab, line, col)) = navigate {
                state.queue_action(super::IdeAction::NavigateToLocation { tab, line, col });
            }
        });
    }
}
