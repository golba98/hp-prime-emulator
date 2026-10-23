//! Left program explorer panel with workspace tree, recent files, and outline.

use super::{IdeAction, IdeAppState, IdePanel};
use crate::theme::TEXT_MUTED;
use egui::{RichText, ScrollArea};
use std::path::PathBuf;

/// Program Explorer panel on the left side of the IDE desktop.
#[derive(Debug, Default)]
pub struct ProgramExplorer {
    pub cached_workspace_files: Vec<PathBuf>,
    pub last_scanned_dir: Option<PathBuf>,
    pub outline_items: Vec<OutlineEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub name: String,
    pub is_export: bool,
    pub line: usize,
}

impl ProgramExplorer {
    pub fn new() -> Self {
        let mut explorer = Self::default();
        explorer.refresh_workspace();
        explorer
    }

    /// Scans the current workspace directory for PPL source files.
    pub fn refresh_workspace(&mut self) {
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.last_scanned_dir = Some(dir.clone());
        let mut files = Vec::new();

        // Scan examples/ and tests/ and current dir for .hpprgm / .ppl files
        let search_dirs = [dir.join("examples"), dir.join("tests"), dir.clone()];

        for search_dir in &search_dirs {
            if let Ok(entries) = std::fs::read_dir(search_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let is_ppl = path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| {
                                ext.eq_ignore_ascii_case("hpprgm")
                                    || ext.eq_ignore_ascii_case("ppl")
                            })
                            .unwrap_or(false);
                        if is_ppl && !files.contains(&path) {
                            files.push(path);
                        }
                    }
                }
            }
        }

        files.sort();
        self.cached_workspace_files = files;
    }

    /// Rebuilds the outline from the program's function declarations, as
    /// found by the language analyzer (the same header rules as the parser).
    pub fn update_outline(&mut self, source: &str) {
        let analysis = ppl_compiler::highlight::analyze(source);
        self.update_outline_from(source, &analysis.symbols);
    }

    /// [`Self::update_outline`] from an existing analysis.
    pub fn update_outline_from(
        &mut self,
        source: &str,
        symbols: &ppl_compiler::highlight::DocumentSymbols,
    ) {
        self.outline_items = symbols
            .functions
            .iter()
            .map(|f| OutlineEntry {
                name: f.name.clone(),
                is_export: f.exported,
                line: source[..f.offset.min(source.len())].matches('\n').count() + 1,
            })
            .collect();
    }
}

impl IdePanel for ProgramExplorer {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(RichText::new("📁 PROGRAM EXPLORER").strong().size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("◀")
                        .on_hover_text("Collapse Explorer (Ctrl+B)")
                        .clicked()
                    {
                        state.show_explorer = false;
                    }
                    if ui
                        .small_button("⟳")
                        .on_hover_text("Refresh Workspace")
                        .clicked()
                    {
                        self.refresh_workspace();
                    }
                });
            });

            ui.add(egui::Separator::default().spacing(4.0));

            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Section: Workspace Files
                    egui::CollapsingHeader::new(
                        RichText::new("▾ Workspace Programs").strong().size(11.0),
                    )
                    .default_open(true)
                    .show(ui, |ui| {
                        if self.cached_workspace_files.is_empty() {
                            ui.label(
                                RichText::new("  (No PPL files found)")
                                    .color(TEXT_MUTED)
                                    .size(11.0),
                            );
                        } else {
                            for path in &self.cached_workspace_files {
                                let filename = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown");
                                let is_hpprgm = filename.ends_with(".hpprgm");
                                let icon = if is_hpprgm { "📄 " } else { "📝 " };

                                if ui
                                    .selectable_label(false, format!("{icon}{filename}"))
                                    .clicked()
                                {
                                    state.queue_action(IdeAction::OpenFile(Some(path.clone())));
                                }
                            }
                        }
                    });

                    ui.add_space(6.0);

                    // Section: Recent Files
                    if !state.recent_files.is_empty() {
                        egui::CollapsingHeader::new(
                            RichText::new("▾ Recent Files").strong().size(11.0),
                        )
                        .default_open(true)
                        .show(ui, |ui| {
                            let recents = state.recent_files.clone();
                            for path in recents {
                                let filename = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown");
                                if ui
                                    .selectable_label(false, format!("🕒 {filename}"))
                                    .clicked()
                                {
                                    state.queue_action(IdeAction::OpenFile(Some(path)));
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }

                    // Section: Outline / Functions
                    egui::CollapsingHeader::new(
                        RichText::new("▾ Program Outline").strong().size(11.0),
                    )
                    .default_open(true)
                    .show(ui, |ui| {
                        if self.outline_items.is_empty() {
                            ui.label(
                                RichText::new("  (No exported functions)")
                                    .color(TEXT_MUTED)
                                    .size(11.0),
                            );
                        } else {
                            for entry in &self.outline_items {
                                let icon = if entry.is_export { "★ " } else { "• " };
                                let label = format!("{}Ln {}: {}", icon, entry.line, entry.name);
                                if ui.selectable_label(false, label).clicked() {
                                    state.queue_action(super::IdeAction::NavigateToLocation {
                                        tab: None,
                                        line: entry.line,
                                        col: 1,
                                    });
                                }
                            }
                        }
                    });
                });
        });
    }
}
