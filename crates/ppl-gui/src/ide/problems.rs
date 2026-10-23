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

            if self.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("No problems detected.").color(TEXT_MUTED));
                });
                return;
            }

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

                        for item in self.visible() {
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
                            let msg = ui.selectable_label(false, RichText::new(message).size(12.0));
                            let msg = match &item.hint {
                                Some(h) => msg.on_hover_text(format!("hint: {h}")),
                                None => msg,
                            };
                            if loc.clicked() || code.clicked() || msg.clicked() {
                                navigate = Some((item.tab, item.line, item.col));
                            }
                            ui.end_row();
                        }
                    });
            });
            if let Some((tab, line, col)) = navigate {
                state.queue_action(super::IdeAction::NavigateToLocation { tab, line, col });
            }
        });
    }
}
