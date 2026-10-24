//! Compact action toolbar for common IDE operations.

use super::{IdeAction, IdeAppState, IdePanel, VmExecutionState};
use crate::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_RED, TEXT_MUTED};
use egui::{Color32, RichText, Stroke, StrokeKind};

/// Top action toolbar widget.
#[derive(Debug, Default)]
pub struct ToolBar;

impl ToolBar {
    pub fn new() -> Self {
        Self
    }
}

impl IdePanel for ToolBar {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        // Narrow windows get icon-only labels for secondary buttons, so the
        // left and right groups never overlap.
        let compact = ui.available_width() < 1100.0;
        let label = |icon: &str, text: &str| {
            if compact {
                icon.to_string()
            } else {
                format!("{icon} {text}")
            }
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;

            // File operations
            if ui
                .button(RichText::new(label("📄", "New")).size(12.0))
                .on_hover_text("New Program (Ctrl+N)")
                .clicked()
            {
                state.queue_action(IdeAction::ShowNewProgramDialog);
            }
            if ui
                .button(RichText::new(label("📂", "Open")).size(12.0))
                .on_hover_text("Open File (Ctrl+O)")
                .clicked()
            {
                state.queue_action(IdeAction::OpenFile(None));
            }
            if ui
                .button(RichText::new(label("💾", "Save")).size(12.0))
                .on_hover_text("Save File (Ctrl+S)")
                .clicked()
            {
                state.queue_action(IdeAction::SaveActiveFile);
            }

            ui.add(egui::Separator::default().vertical());

            // Compilation & Execution operations
            if ui
                .button(RichText::new("⚙ Compile").size(12.0))
                .on_hover_text("Check Syntax & Compile (F7)")
                .clicked()
            {
                state.queue_action(IdeAction::CheckSyntax);
            }

            let is_running = matches!(state.vm_state, VmExecutionState::Running);
            let is_paused = matches!(state.vm_state, VmExecutionState::Paused);
            let is_active = is_running || is_paused;

            if is_paused {
                let cont_btn = ui.add(egui::Button::new(
                    RichText::new("▶ Continue").color(ACCENT_GREEN).size(12.0),
                ));
                if cont_btn.on_hover_text("Continue Execution (F5)").clicked() {
                    state.queue_action(IdeAction::ContinueExecution);
                }
            } else {
                let run_color = if is_running { TEXT_MUTED } else { ACCENT_GREEN };
                let run_btn = ui.add_enabled(
                    !is_running,
                    egui::Button::new(RichText::new("▶ Run").color(run_color).size(12.0)),
                );
                if run_btn.on_hover_text("Run Program (F5)").clicked() {
                    state.queue_action(IdeAction::RunProgram);
                }
            }

            let stop_color = if is_active { ACCENT_RED } else { TEXT_MUTED };
            let stop_btn = ui.add_enabled(
                is_active,
                egui::Button::new(RichText::new("⏹ Stop").color(stop_color).size(12.0)),
            );
            if stop_btn
                .on_hover_text("Stop / Interrupt Program (Shift+F5)")
                .clicked()
            {
                state.queue_action(IdeAction::StopProgram);
            }

            let restart_color = ACCENT_BLUE;
            let restart_btn = ui.add(egui::Button::new(
                RichText::new(label("🔄", "Restart"))
                    .color(restart_color)
                    .size(12.0),
            ));
            if restart_btn
                .on_hover_text("Restart Program (Ctrl+F5)")
                .clicked()
            {
                state.queue_action(IdeAction::RestartProgram);
            }

            let debug_color = if is_active { TEXT_MUTED } else { ACCENT_AMBER };
            let debug_btn = ui.add_enabled(
                !is_active,
                egui::Button::new(RichText::new("🐞 Debug").color(debug_color).size(12.0)),
            );
            if debug_btn.on_hover_text("Start Debugging (F9)").clicked() {
                state.queue_action(IdeAction::StartDebugging);
            }

            ui.add(egui::Separator::default().vertical());

            // Panel Toggles
            let exp_label = label("📁", "Explorer");
            if ui
                .selectable_label(state.show_explorer, exp_label)
                .on_hover_text("Toggle Explorer (Ctrl+B)")
                .clicked()
            {
                state.show_explorer = !state.show_explorer;
            }

            let calc_label = label("🖩", "Calculator");
            if ui
                .selectable_label(state.show_calculator, calc_label)
                .on_hover_text("Toggle Calculator (Ctrl+K)")
                .clicked()
            {
                state.show_calculator = !state.show_calculator;
            }

            let dock_label = label("▼", "Dock");
            if ui
                .selectable_label(state.show_bottom_dock, dock_label)
                .on_hover_text("Toggle Bottom Dock (Ctrl+J)")
                .clicked()
            {
                state.show_bottom_dock = !state.show_bottom_dock;
            }

            // Right-aligned settings & VM status badge
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new(label("⚙", "Settings")).size(12.0))
                    .on_hover_text("Calculator & Emulator Settings")
                    .clicked()
                {
                    state.queue_action(IdeAction::ShowSettingsDialog);
                }

                ui.add(egui::Separator::default().vertical());

                // VM Status Badge
                let (status_text, status_color) = match &state.vm_state {
                    VmExecutionState::Idle => ("VM: IDLE", TEXT_MUTED),
                    VmExecutionState::Running => ("VM: RUNNING", ACCENT_GREEN),
                    VmExecutionState::Paused => ("VM: PAUSED", ACCENT_AMBER),
                    VmExecutionState::Terminated => ("VM: DONE", ACCENT_BLUE),
                    VmExecutionState::Error(_) => ("VM: ERROR", ACCENT_RED),
                };

                let badge = egui::Label::new(
                    RichText::new(format!(" {status_text} "))
                        .size(11.0)
                        .color(Color32::WHITE)
                        .monospace(),
                );
                let rect = ui.add(badge).rect;
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(1.0_f32, status_color),
                    StrokeKind::Inside,
                );
            });
        });
    }
}
