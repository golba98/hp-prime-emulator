//! Main menu bar component and global keyboard shortcut handling.

use super::{DockTab, IdeAction, IdeAppState, IdePanel, VmExecutionState};
use egui::Key;

/// Top menu bar for the IDE desktop shell.
#[derive(Debug, Default)]
pub struct MenuBar;

impl MenuBar {
    pub fn new() -> Self {
        Self
    }

    /// Checks global keyboard shortcuts and queues corresponding actions.
    pub fn handle_shortcuts(&self, ctx: &egui::Context, state: &mut IdeAppState) {
        ctx.input(|i| {
            // Hotkeys with Ctrl / Command
            if i.modifiers.command {
                if i.key_pressed(Key::N) {
                    state.queue_action(IdeAction::ShowNewProgramDialog);
                } else if i.key_pressed(Key::O) {
                    state.queue_action(IdeAction::OpenFile(None));
                } else if i.key_pressed(Key::S) {
                    if i.modifiers.shift {
                        state.queue_action(IdeAction::SaveActiveFileAs);
                    } else {
                        state.queue_action(IdeAction::SaveActiveFile);
                    }
                } else if i.key_pressed(Key::W) {
                    state.queue_action(IdeAction::CloseActiveTab);
                } else if i.key_pressed(Key::B) {
                    state.queue_action(IdeAction::ToggleExplorer);
                } else if i.key_pressed(Key::K) {
                    state.queue_action(IdeAction::ToggleCalculator);
                } else if i.key_pressed(Key::J) {
                    state.queue_action(IdeAction::ToggleBottomDock);
                } else if i.key_pressed(Key::F) {
                    state.queue_action(IdeAction::ShowFind);
                } else if i.key_pressed(Key::H) {
                    state.queue_action(IdeAction::ShowFindReplaceDialog);
                } else if i.key_pressed(Key::G) {
                    state.queue_action(IdeAction::ShowGoToLineDialog);
                } else if i.key_pressed(Key::Equals) || i.key_pressed(Key::Plus) {
                    state.queue_action(IdeAction::ZoomEditorFont(1));
                } else if i.key_pressed(Key::Minus) {
                    state.queue_action(IdeAction::ZoomEditorFont(-1));
                } else if i.key_pressed(Key::Num0) {
                    state.queue_action(IdeAction::ZoomEditorFont(0));
                }
            }

            // Function keys
            if i.key_pressed(Key::F5) {
                if i.modifiers.shift {
                    state.queue_action(IdeAction::StopProgram);
                } else if i.modifiers.command {
                    state.queue_action(IdeAction::RestartProgram);
                } else if state.vm_state == VmExecutionState::Paused {
                    state.queue_action(IdeAction::ContinueExecution);
                } else {
                    state.queue_action(IdeAction::RunProgram);
                }
            } else if i.key_pressed(Key::F7) {
                state.queue_action(IdeAction::CheckSyntax);
            } else if i.key_pressed(Key::F9) {
                state.queue_action(IdeAction::StartDebugging);
            } else if i.key_pressed(Key::F10) {
                state.queue_action(IdeAction::StepOver);
            } else if i.key_pressed(Key::F11) {
                if i.modifiers.shift {
                    state.queue_action(IdeAction::StepOut);
                } else {
                    state.queue_action(IdeAction::StepInto);
                }
            } else if i.key_pressed(Key::F12) {
                state.queue_action(IdeAction::ToggleBreakpoint(state.cursor_line));
            }
        });
    }
}

impl IdePanel for MenuBar {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        egui::MenuBar::new().ui(ui, |ui| {
            // File Menu
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("New Program").shortcut_text("Ctrl+N"))
                    .clicked()
                {
                    state.queue_action(IdeAction::ShowNewProgramDialog);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    state.queue_action(IdeAction::OpenFile(None));
                    ui.close();
                }
                if !state.recent_files.is_empty() {
                    ui.menu_button("Open Recent", |ui| {
                        let recents = state.recent_files.clone();
                        for path in recents {
                            let label = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("Unknown");
                            if ui.button(label).clicked() {
                                state.queue_action(IdeAction::OpenFile(Some(path)));
                                ui.close();
                            }
                        }
                    });
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                    .clicked()
                {
                    state.queue_action(IdeAction::SaveActiveFile);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
                    .clicked()
                {
                    state.queue_action(IdeAction::SaveActiveFileAs);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Close Tab").shortcut_text("Ctrl+W"))
                    .clicked()
                {
                    state.queue_action(IdeAction::CloseActiveTab);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Exit").shortcut_text("Ctrl+Q"))
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close();
                }
            });

            // Edit Menu: every item replays the matching keyboard input
            // into the editor (see `EditCommand`).
            ui.menu_button("Edit", |ui| {
                use super::EditCommand as E;
                let items: [(&str, &str, E); 10] = [
                    ("Undo", "Ctrl+Z", E::Undo),
                    ("Redo", "Ctrl+Y", E::Redo),
                    ("Cut", "Ctrl+X", E::Cut),
                    ("Copy", "Ctrl+C", E::Copy),
                    ("Paste", "Ctrl+V", E::Paste),
                    ("Select All", "Ctrl+A", E::SelectAll),
                    ("Indent", "Tab", E::Indent),
                    ("Outdent", "Shift+Tab", E::Outdent),
                    ("Toggle Line Comment", "Ctrl+/", E::ToggleComment),
                    ("Complete Word", "Ctrl+Space", E::TriggerCompletion),
                ];
                for (i, (label, keys, cmd)) in items.into_iter().enumerate() {
                    if matches!(i, 2 | 6) {
                        ui.separator();
                    }
                    if ui
                        .add(egui::Button::new(label).shortcut_text(keys))
                        .clicked()
                    {
                        state.queue_action(IdeAction::Edit(cmd));
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Find").shortcut_text("Ctrl+F"))
                    .clicked()
                {
                    state.queue_action(IdeAction::ShowFind);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Replace").shortcut_text("Ctrl+H"))
                    .clicked()
                {
                    state.queue_action(IdeAction::ShowFindReplaceDialog);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Go to Line...").shortcut_text("Ctrl+G"))
                    .clicked()
                {
                    state.queue_action(IdeAction::ShowGoToLineDialog);
                    ui.close();
                }
            });

            // Program Menu
            ui.menu_button("Program", |ui| {
                if ui
                    .add(egui::Button::new("Check Syntax").shortcut_text("F7"))
                    .clicked()
                {
                    state.queue_action(IdeAction::CheckSyntax);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Format Source").shortcut_text("Ctrl+Shift+F"))
                    .clicked()
                {
                    state.queue_action(IdeAction::FormatSource);
                    ui.close();
                }
                ui.separator();
                if ui.button("Program Properties...").clicked() {
                    ui.close();
                }
            });

            // Run Menu
            ui.menu_button("Run", |ui| {
                let is_running = matches!(state.vm_state, VmExecutionState::Running);
                let is_paused = matches!(state.vm_state, VmExecutionState::Paused);
                let is_active = is_running || is_paused;

                let run_label = if is_paused { "Continue" } else { "Start" };
                if ui
                    .add_enabled(
                        !is_running,
                        egui::Button::new(run_label).shortcut_text("F5"),
                    )
                    .clicked()
                {
                    if is_paused {
                        state.queue_action(IdeAction::ContinueExecution);
                    } else {
                        state.queue_action(IdeAction::RunProgram);
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        is_active,
                        egui::Button::new("Stop").shortcut_text("Shift+F5"),
                    )
                    .clicked()
                {
                    state.queue_action(IdeAction::StopProgram);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Restart").shortcut_text("Ctrl+F5"))
                    .clicked()
                {
                    state.queue_action(IdeAction::RestartProgram);
                    ui.close();
                }
                ui.separator();
                if ui.add(egui::Button::new("Configure Run...")).clicked() {
                    state.queue_action(IdeAction::ShowRunConfigDialog);
                    ui.close();
                }
            });

            // Debug Menu
            ui.menu_button("Debug", |ui| {
                if ui
                    .add(egui::Button::new("Start Debugging").shortcut_text("F9"))
                    .clicked()
                {
                    state.queue_action(IdeAction::StartDebugging);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Continue").shortcut_text("F5"))
                    .clicked()
                {
                    state.queue_action(IdeAction::ContinueExecution);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Step Into").shortcut_text("F11"))
                    .clicked()
                {
                    state.queue_action(IdeAction::StepInto);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Step Over").shortcut_text("F10"))
                    .clicked()
                {
                    state.queue_action(IdeAction::StepOver);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Step Out").shortcut_text("Shift+F11"))
                    .clicked()
                {
                    state.queue_action(IdeAction::StepOut);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Toggle Breakpoint").shortcut_text("F12"))
                    .clicked()
                {
                    state.queue_action(IdeAction::ToggleBreakpoint(state.cursor_line));
                    ui.close();
                }
            });

            // View Menu
            ui.menu_button("View", |ui| {
                if ui
                    .checkbox(&mut state.show_explorer, "Program Explorer (Ctrl+B)")
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .checkbox(&mut state.show_calculator, "Calculator & Keypad (Ctrl+K)")
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .checkbox(&mut state.show_bottom_dock, "Bottom Dock (Ctrl+J)")
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .checkbox(&mut state.show_status_bar, "Status Bar")
                    .clicked()
                {
                    ui.close();
                }
                ui.separator();
                if ui.button("Dock: Output (Alt+1)").clicked() {
                    state.show_bottom_dock = true;
                    state.active_dock_tab = DockTab::Output;
                    ui.close();
                }
                if ui.button("Dock: Problems (Alt+2)").clicked() {
                    state.show_bottom_dock = true;
                    state.active_dock_tab = DockTab::Problems;
                    ui.close();
                }
                if ui.button("Dock: Variables (Alt+3)").clicked() {
                    state.show_bottom_dock = true;
                    state.active_dock_tab = DockTab::Variables;
                    ui.close();
                }
                if ui.button("Dock: Call Stack (Alt+4)").clicked() {
                    state.show_bottom_dock = true;
                    state.active_dock_tab = DockTab::CallStack;
                    ui.close();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Zoom In").clicked() {
                        state.queue_action(IdeAction::ZoomEditorFont(1));
                    }
                    if ui.button("Zoom Out").clicked() {
                        state.queue_action(IdeAction::ZoomEditorFont(-1));
                    }
                    if ui.button("Reset Font").clicked() {
                        state.queue_action(IdeAction::ZoomEditorFont(0));
                    }
                });
                ui.separator();
                if ui.button("Reset Layout").clicked() {
                    state.reset_layout();
                    ui.close();
                }
            });

            // Tools Menu
            ui.menu_button("Tools", |ui| {
                if ui.button("Detect USB Hardware").clicked() {
                    ui.close();
                }
                if ui.button("Take Screenshot").clicked() {
                    state.queue_action(IdeAction::TakeScreenshot);
                    ui.close();
                }
                ui.separator();
                if ui.button("Calculator Settings...").clicked() {
                    state.queue_action(IdeAction::ShowSettingsDialog);
                    ui.close();
                }
            });

            // Help Menu
            ui.menu_button("Help", |ui| {
                if ui.button("Built-in PPL Reference").clicked() {
                    ui.close();
                }
                if ui.button("Keyboard Shortcuts").clicked() {
                    ui.close();
                }
                ui.separator();
                if ui.button("About HP Prime Emulator").clicked() {
                    state.queue_action(IdeAction::ShowAboutDialog);
                    ui.close();
                }
            });
        });
    }
}
