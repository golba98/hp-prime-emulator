//! Left program explorer panel: the user's folder, programs on the
//! connected calculator, recent files, and the outline.

use super::{IdeAction, IdeAppState, IdePanel};
use crate::theme::{ACCENT_RED, TEXT_MUTED};
use egui::{RichText, ScrollArea};
use std::path::{Path, PathBuf};

/// How deep the folder scan descends below the workspace root.
const SCAN_DEPTH: usize = 3;
/// Upper bound on listed files, so a huge folder cannot stall the UI.
const SCAN_LIMIT: usize = 500;

/// Program Explorer panel on the left side of the IDE desktop.
#[derive(Debug, Default)]
pub struct ProgramExplorer {
    pub cached_workspace_files: Vec<PathBuf>,
    pub last_scanned_dir: Option<PathBuf>,
    pub outline_items: Vec<OutlineEntry>,
    /// Programs read from the calculator (read-only listing).
    pub calculator: CalculatorPrograms,
}

/// Result of the last "On Calculator" refresh.
#[derive(Debug, Default)]
pub struct CalculatorPrograms {
    /// `None` until listed; (name, payload bytes).
    pub programs: Option<Vec<(String, usize)>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub name: String,
    pub is_export: bool,
    pub line: usize,
}

fn is_ppl(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("hpprgm") || ext.eq_ignore_ascii_case("ppl"))
}

/// PPL files under `dir`, at most [`SCAN_DEPTH`] levels down, skipping
/// hidden folders and build output.
fn scan(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= SCAN_LIMIT {
            return;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if depth > 0 && !name.starts_with('.') && name != "target" && name != "node_modules" {
                scan(&path, depth - 1, files);
            }
        } else if is_ppl(&path) {
            files.push(path);
        }
    }
}

impl ProgramExplorer {
    pub fn new() -> Self {
        let mut explorer = Self::default();
        explorer.refresh_workspace(None);
        explorer
    }

    /// Scans `root` (default: the launch directory) for PPL source files.
    pub fn refresh_workspace(&mut self, root: Option<&Path>) {
        let dir = root
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut files = Vec::new();
        scan(&dir, SCAN_DEPTH, &mut files);
        files.sort();
        self.last_scanned_dir = Some(dir);
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

impl ProgramExplorer {
    fn render_calculator(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        let busy = state.device_busy.is_some();
        let listing = state.device_busy == Some(super::device::Purpose::ListPrograms);
        ui.horizontal(|ui| {
            let refresh = ui
                .add_enabled(
                    !busy && state.device_status.connected(),
                    egui::Button::new("Refresh").small(),
                )
                .on_hover_text("Read the program list from the calculator (read-only)")
                .on_disabled_hover_text("Connect the calculator first");
            if refresh.clicked() {
                state.queue_action(IdeAction::ListCalculatorPrograms);
            }
            if listing {
                ui.spinner();
            }
        });
        if let Some(e) = &self.calculator.error {
            ui.label(RichText::new(e.as_str()).color(ACCENT_RED).size(11.0));
        }
        let Some(programs) = &self.calculator.programs else {
            ui.label(
                RichText::new("  Click Refresh to list them")
                    .color(TEXT_MUTED)
                    .size(11.0),
            );
            return;
        };
        if programs.is_empty() {
            ui.label(
                RichText::new("  (No programs listed)")
                    .color(TEXT_MUTED)
                    .size(11.0),
            );
        }
        for (name, bytes) in programs {
            let mine = state.send_names.values().any(|n| n == name);
            let label = format!("{} {name}", if mine { "●" } else { "▫" });
            let row = ui
                .add_enabled(!busy, egui::Button::selectable(false, label))
                .on_hover_text(format!(
                    "{} bytes{}. Double-click to open a copy in a new tab.",
                    bytes,
                    if mine { " · sent from this IDE" } else { "" }
                ));
            if row.double_clicked() {
                state.queue_action(IdeAction::OpenFromCalculator(name.clone()));
            }
            row.context_menu(|ui| {
                if ui.button("Open a copy in a new tab").clicked() {
                    state.queue_action(IdeAction::OpenFromCalculator(name.clone()));
                    ui.close();
                }
            });
        }
        ui.label(
            RichText::new("The calculator's list may be incomplete.")
                .color(TEXT_MUTED)
                .size(10.5),
        );
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
                        self.refresh_workspace(state.workspace_root.as_deref());
                    }
                });
            });

            ui.add(egui::Separator::default().spacing(4.0));

            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Section: Workspace Files
                    let folder = self
                        .last_scanned_dir
                        .as_deref()
                        .and_then(Path::file_name)
                        .and_then(|n| n.to_str())
                        .unwrap_or("Workspace");
                    egui::CollapsingHeader::new(
                        RichText::new(format!("▾ {folder}")).strong().size(11.0),
                    )
                    .id_salt("workspace_programs")
                    .default_open(true)
                    .show(ui, |ui| {
                        if ui
                            .small_button("Open Folder…")
                            .on_hover_text("Choose the folder with your programs")
                            .clicked()
                        {
                            state.queue_action(IdeAction::OpenFolder);
                        }
                        if self.cached_workspace_files.is_empty() {
                            ui.label(
                                RichText::new("  (No PPL files found)")
                                    .color(TEXT_MUTED)
                                    .size(11.0),
                            );
                        } else {
                            let root = self.last_scanned_dir.clone().unwrap_or_default();
                            // One line per file; the full path is in the tooltip.
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            for path in &self.cached_workspace_files {
                                let filename = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown");
                                let is_hpprgm = filename.ends_with(".hpprgm");
                                let icon = if is_hpprgm { "📄 " } else { "📝 " };
                                // The subfolder, muted after the name, keeps rows on one line.
                                let folder = path
                                    .parent()
                                    .and_then(|p| p.strip_prefix(&root).ok())
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .filter(|p| !p.is_empty());
                                let mut text = egui::text::LayoutJob::default();
                                text.append(
                                    &format!("{icon}{filename}"),
                                    0.0,
                                    egui::TextFormat::simple(
                                        egui::FontId::proportional(13.0),
                                        ui.visuals().text_color(),
                                    ),
                                );
                                if let Some(folder) = &folder {
                                    text.append(
                                        folder,
                                        6.0,
                                        egui::TextFormat::simple(
                                            egui::FontId::proportional(11.0),
                                            TEXT_MUTED,
                                        ),
                                    );
                                }
                                if ui
                                    .selectable_label(false, text)
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    state.queue_action(IdeAction::OpenFile(Some(path.clone())));
                                }
                            }
                        }
                    });

                    ui.add_space(6.0);

                    // Section: programs on the connected calculator (read-only).
                    egui::CollapsingHeader::new(
                        RichText::new("▾ On Calculator").strong().size(11.0),
                    )
                    .id_salt("calculator_programs")
                    .default_open(false)
                    .show(ui, |ui| self.render_calculator(ui, state));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_the_chosen_folder_recursively() {
        let root = std::env::temp_dir().join(format!("ppl-explorer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["games/arcade", ".hidden", "target"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "A.hpprgm",
            "games/B.ppl",
            "games/arcade/C.hpprgm",
            "notes.txt",
            ".hidden/D.ppl",
            "target/E.ppl",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let mut explorer = ProgramExplorer::default();
        explorer.refresh_workspace(Some(&root));
        let found: Vec<_> = explorer
            .cached_workspace_files
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(found, ["A.hpprgm", "games/B.ppl", "games/arcade/C.hpprgm"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
