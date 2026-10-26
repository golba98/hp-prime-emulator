//! Modal dialog host for IDE settings, templates, go-to-line, run
//! configuration and about info. (Find/replace is the editor's find bar.)

use super::editor::EditorArea;
use super::{IdeAction, IdeAppState};
use crate::theme::TEXT_MUTED;
use egui::{RichText, Window};

/// Tracks active modal dialog windows in the desktop IDE.
#[derive(Debug, Default)]
pub struct DialogHost {
    pub show_about: bool,
    pub show_shortcuts: bool,
    pub show_settings: bool,
    pub show_new_program: bool,
    pub show_go_to_line: bool,
    /// Confirm closing a tab (or the app) with unsaved changes.
    pub show_confirm_close: bool,
    /// What the close confirmation refers to.
    pub confirm_close_all: bool,
    /// Set when the user confirmed closing ALL programs (app quit).
    pub close_all_confirmed: bool,

    // Dialog inputs
    pub new_prog_name: String,
    pub go_to_line_input: String,
    pub max_steps_input: String,
    /// Validation message shown by the go-to-line dialog.
    pub go_to_line_error: Option<String>,

    // Run configuration dialog
    pub show_run_config: bool,
    /// Exported functions of the active buffer: (name, param count).
    pub run_config_entries: Vec<(String, usize)>,
    /// Index into `run_config_entries` of the selected entry point.
    pub run_config_selected: usize,
    /// Comma-separated PPL argument expressions for the entry point.
    pub run_config_args: String,
}

impl DialogHost {
    pub fn new() -> Self {
        Self {
            new_prog_name: "MYPROG".to_string(),
            go_to_line_input: "1".to_string(),
            max_steps_input: "1000000".to_string(),
            ..Default::default()
        }
    }

    /// Renders any currently active modal dialogs. `editor` is used by the
    /// go-to-line and run-configuration dialogs.
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        state: &mut IdeAppState,
        editor: &mut EditorArea,
    ) {
        // About Dialog
        if self.show_about {
            let mut open = true;
            let mut close_clicked = false;
            Window::new("About HP Prime PPL Emulator & IDE")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("HP Prime PPL Emulator & Native IDE").strong().size(14.0));
                        ui.label(RichText::new("Version 0.2.0 (Rust Native)").color(TEXT_MUTED).size(11.0));
                    });
                    ui.add_space(8.0);
                    ui.label("A high-fidelity native Rust emulator and differential testing harness for HP Prime PPL (.hpprgm) calculator programs.");
                    ui.label("Features Copy-on-Write value semantics, 12-digit BCD decimal arithmetic, headless GROB graphics, and interactive debugging.");
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close_clicked = true;
                        }
                    });
                });
            if close_clicked {
                open = false;
            }
            self.show_about = open;
        }

        if self.show_shortcuts {
            Window::new("Keyboard Shortcuts")
                .open(&mut self.show_shortcuts)
                .show(ctx, |ui| {
                    egui::Grid::new("shortcuts")
                        .spacing([24.0, 6.0])
                        .show(ui, |ui| {
                            for (keys, action) in [
                                ("Ctrl+N / O / S", "New / Open / Save"),
                                ("F7 / F5 / Shift+F5", "Compile / Run / Stop"),
                                ("Ctrl+F5 / F9", "Restart / Native Debug"),
                                ("F10 / F11 / Shift+F11", "Step Over / Into / Out"),
                                ("Ctrl+B / K / J", "Explorer / Calculator / Bottom Panel"),
                                ("Ctrl+F / H / G", "Find / Replace / Go to line"),
                                ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / Previous file"),
                                ("Ctrl+Space", "PPL completion"),
                                ("Ctrl+= / − / 0", "Editor zoom"),
                                ("F1…F9 (calculator focused)", "Calculator function keys"),
                                ("Tab (firmware calculator)", "Alpha"),
                            ] {
                                ui.label(keys);
                                ui.label(action);
                                ui.end_row();
                            }
                        });
                });
        }

        // Settings Dialog
        if self.show_settings {
            let mut open = true;
            let mut close_clicked = false;
            Window::new("Emulator & Calculator Settings")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Execution Limits & Modes").strong());
                    ui.separator();

                    egui::Grid::new("settings_grid")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Max Instruction Steps:");
                            ui.text_edit_singleline(&mut self.max_steps_input);
                            ui.end_row();
                            ui.label("Editor Font Size:");
                            ui.add(
                                egui::Slider::new(
                                    &mut state.editor_font_size,
                                    super::EDITOR_FONT_RANGE,
                                )
                                .step_by(1.0)
                                .suffix(" pt"),
                            );
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label("Local G1 firmware");
                    ui.label(egui::RichText::new("Paths apply when the firmware session next starts. Empty paths use PPL_FIRMWARE / PPL_FACTORY_A.").small().color(crate::theme::TEXT_MUTED));
                    for (label,path) in [("Firmware ELF",&mut state.firmware.firmware),("Factory A directory",&mut state.firmware.factory_a),("Helper executable",&mut state.firmware.runner),("Calculator profile",&mut state.firmware.profile)] {
                        ui.horizontal(|ui|{ui.label(label);ui.add(egui::TextEdit::singleline(path).desired_width(320.0));});
                    }
                    ui.checkbox(&mut state.smooth_display,"Smooth LCD scaling");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("OK").clicked() {
                            close_clicked = true;
                        }
                    });
                });
            if close_clicked {
                open = false;
            }
            self.show_settings = open;
        }

        // New Program Dialog
        if self.show_new_program {
            let mut open = true;
            let mut create_clicked = false;
            let mut cancel_clicked = false;
            Window::new("New PPL Program")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Program Name (Function Identifier):");
                    ui.text_edit_singleline(&mut self.new_prog_name);
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            create_clicked = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_clicked = true;
                        }
                    });
                });
            if create_clicked {
                state.queue_action(IdeAction::NewProgram);
                open = false;
            } else if cancel_clicked {
                open = false;
            }
            self.show_new_program = open;
        }

        // Go to Line Dialog
        if self.show_go_to_line {
            let line_count = editor
                .active_tab_mut()
                .map_or(1, |t| t.document().lines().line_count());
            let mut open = true;
            let mut go_clicked = false;
            let mut cancel_clicked = false;
            Window::new("Go to Line")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(format!("Line number (1–{line_count}), optionally :column"));
                    let response = ui.text_edit_singleline(&mut self.go_to_line_input);
                    response.request_focus();
                    if response.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go_clicked = true;
                    }
                    if let Some(err) = &self.go_to_line_error {
                        ui.label(RichText::new(err).color(crate::theme::ACCENT_RED));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Go").clicked() {
                            go_clicked = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_clicked = true;
                        }
                    });
                });
            if go_clicked {
                match parse_line_col(&self.go_to_line_input, line_count) {
                    Ok((line, col)) => {
                        state.cursor_line = line;
                        state.cursor_col = col;
                        if let Some(tab) = editor.active_tab_mut() {
                            tab.goto_line(line, col);
                        }
                        state.status_message = format!("Jumped to line {line}");
                        self.go_to_line_error = None;
                        open = false;
                    }
                    Err(e) => self.go_to_line_error = Some(e),
                }
            } else if cancel_clicked {
                open = false;
            }
            if !open {
                self.go_to_line_error = None;
            }
            self.show_go_to_line = open;
        }

        // Run Configuration Dialog (entry point + arguments).
        if self.show_run_config {
            let mut open = true;
            let mut save_clicked = false;
            let mut run_clicked = false;
            let mut cancel_clicked = false;
            Window::new("Run Configuration")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    if self.run_config_entries.is_empty() {
                        ui.label(
                            RichText::new(
                                "This program has no EXPORT function. \
                                 Add an EXPORT function or pick the entry \
                                 by name once one exists.",
                            )
                            .color(TEXT_MUTED),
                        );
                    } else {
                        let selected_name = self
                            .run_config_entries
                            .get(self.run_config_selected)
                            .cloned();
                        egui::ComboBox::from_label("Entry point")
                            .selected_text(selected_name.as_ref().map_or_else(
                                || "Select a function".to_string(),
                                |(name, params)| format!("{name}({params} args)"),
                            ))
                            .show_ui(ui, |ui| {
                                for (i, (name, params)) in
                                    self.run_config_entries.iter().enumerate()
                                {
                                    ui.selectable_value(
                                        &mut self.run_config_selected,
                                        i,
                                        format!("{name}({params} args)"),
                                    );
                                }
                            });
                        ui.label("Arguments (comma-separated PPL expressions):");
                        ui.text_edit_singleline(&mut self.run_config_args);
                        ui.label(
                            RichText::new(
                                "Example: 5, {1,2}, \"text\" — evaluated by the VM before the call",
                            )
                            .color(TEXT_MUTED)
                            .size(11.0),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let can_apply = !self.run_config_entries.is_empty();
                        if ui
                            .add_enabled(can_apply, egui::Button::new("Save"))
                            .clicked()
                        {
                            save_clicked = true;
                        }
                        if ui
                            .add_enabled(can_apply, egui::Button::new("Save & Run"))
                            .clicked()
                        {
                            run_clicked = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_clicked = true;
                        }
                    });
                });
            if save_clicked || run_clicked {
                let entry = self
                    .run_config_entries
                    .get(self.run_config_selected)
                    .map(|(name, _)| name.clone());
                if let (Some(entry), Some(tab)) = (entry, editor.active_tab_mut()) {
                    tab.run_entry = Some(entry);
                    tab.run_args = self.run_config_args.clone();
                }
                if run_clicked {
                    state.queue_action(IdeAction::RunProgram);
                } else {
                    state.status_message = "Run configuration saved".to_string();
                }
                open = false;
            } else if cancel_clicked {
                open = false;
            }
            self.show_run_config = open;
        }

        // Unsaved-changes close confirmation.
        if self.show_confirm_close {
            let mut open = true;
            let mut confirm_clicked = false;
            let mut cancel_clicked = false;
            Window::new("Unsaved Changes")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("The active program has unsaved changes.");
                    ui.label("Close it without saving?");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Close Without Saving").clicked() {
                            confirm_clicked = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_clicked = true;
                        }
                    });
                });
            if confirm_clicked {
                if self.confirm_close_all {
                    self.close_all_confirmed = true;
                } else {
                    editor.close_tab(editor.active_tab_index);
                    state.status_message = "Closed unsaved program".to_string();
                }
                self.confirm_close_all = false;
                open = false;
            } else if cancel_clicked {
                self.confirm_close_all = false;
                open = false;
            }
            self.show_confirm_close = open;
        }
    }
}

/// Parses `line` or `line:col` (1-based) for a buffer of `line_count`
/// lines.
pub fn parse_line_col(input: &str, line_count: usize) -> Result<(usize, usize), String> {
    let mut parts = input.trim().splitn(2, [':', ',']);
    let line = parts
        .next()
        .unwrap_or("")
        .trim()
        .parse::<usize>()
        .map_err(|_| "enter a line number".to_string())?;
    if !(1..=line_count).contains(&line) {
        return Err(format!("line must be between 1 and {line_count}"));
    }
    let col = match parts.next() {
        Some(c) => c
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|&c| c >= 1)
            .ok_or_else(|| "column must be a positive number".to_string())?,
        None => 1,
    };
    Ok((line, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_to_line_input_is_validated() {
        assert_eq!(parse_line_col("12", 40), Ok((12, 1)));
        assert_eq!(parse_line_col(" 3:7 ", 40), Ok((3, 7)));
        assert!(parse_line_col("0", 40).is_err());
        assert!(parse_line_col("41", 40).is_err());
        assert!(parse_line_col("abc", 40).is_err());
        assert!(parse_line_col("3:0", 40).is_err());
    }
}
