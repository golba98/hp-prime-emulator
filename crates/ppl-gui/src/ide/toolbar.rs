//! The main path only: file actions, Run/Stop, and Send to calculator.
//! Everything else (compile, debug, restart, engine, panels) lives in the
//! menus, the activity rail and keyboard shortcuts.
use super::{
    IdeAction, IdeAppState, IdePanel, VmExecutionState,
    icons::{self, Icon},
    layout::{CalculatorMode, WorkspaceLayout},
};
use crate::theme::{ACCENT_GREEN, TEXT_MUTED};
use egui::{RichText, Vec2};
#[derive(Debug, Default)]
pub struct ToolBar;
impl ToolBar {
    pub fn new() -> Self {
        Self
    }
}

/// Labelled button with a painted icon, for the main Send action.
fn send_button(ui: &mut egui::Ui, enabled: bool, hint: &str) -> egui::Response {
    let color = if enabled { ACCENT_GREEN } else { TEXT_MUTED };
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new(RichText::new("       Send to calculator").color(color))
                .min_size(Vec2::new(0.0, 28.0)),
        )
        .on_hover_text(hint)
        .on_disabled_hover_text(hint);
    let icon = egui::Rect::from_center_size(
        response.rect.left_center() + Vec2::new(16.0, 0.0),
        Vec2::splat(16.0),
    );
    icons::paint(ui, Icon::Send, icon, color);
    response
}

impl IdePanel for ToolBar {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (icon, hint, action) in [
                (
                    Icon::New,
                    "New program · Ctrl+N",
                    IdeAction::ShowNewProgramDialog,
                ),
                (Icon::Open, "Open · Ctrl+O", IdeAction::OpenFile(None)),
                (Icon::Save, "Save · Ctrl+S", IdeAction::SaveActiveFile),
            ] {
                if icons::button(ui, icon, hint, true, false).clicked() {
                    state.queue_action(action);
                }
            }
            ui.separator();
            let busy = matches!(
                state.vm_state,
                VmExecutionState::Running | VmExecutionState::Paused
            );
            let run_hint = if state.firmware_backend {
                "Run in the emulator (G1 firmware engine) · F5"
            } else {
                "Run in the emulator · F5"
            };
            for (icon, hint, action, enabled) in [
                (
                    Icon::Run,
                    run_hint,
                    IdeAction::RunProgram,
                    !matches!(state.vm_state, VmExecutionState::Running),
                ),
                (Icon::Stop, "Stop · Shift+F5", IdeAction::StopProgram, busy),
            ] {
                if icons::button(ui, icon, hint, enabled, false).clicked() {
                    state.queue_action(action);
                }
            }
            ui.separator();
            let send_hint = match (&state.device_busy, state.device_status.connected()) {
                (Some(purpose), _) => format!("Calculator busy: {}", purpose.busy_label()),
                (None, false) => "Connect the calculator with USB to send · Ctrl+Shift+U".into(),
                (None, true) => {
                    "Save, check and send this program to the calculator · Ctrl+Shift+U".into()
                }
            };
            if send_button(ui, state.device_busy.is_none(), &send_hint).clicked() {
                state.queue_action(IdeAction::SendToCalculator);
            }
            ui.toggle_value(&mut state.show_compare, "Compare screens")
                .on_hover_text("Keep a side-by-side emulator / physical G1 comparison panel open");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Settings, "Settings", true, false).clicked() {
                    state.queue_action(IdeAction::ShowSettingsDialog);
                }
                ui.menu_button("Layout", |ui| {
                    for (mode, title) in [
                        (WorkspaceLayout::Split, "Editor + Calculator"),
                        (WorkspaceLayout::Editor, "Editor Focus"),
                        (WorkspaceLayout::Calculator, "Expanded Calculator"),
                    ] {
                        if ui.selectable_label(state.layout == mode, title).clicked() {
                            state.queue_action(IdeAction::SetLayout(mode));
                            ui.close();
                        }
                    }
                    ui.separator();
                    for (mode, title) in [
                        (CalculatorMode::Full, "Full Calculator"),
                        (CalculatorMode::ScreenOnly, "Screen Only"),
                    ] {
                        if ui
                            .selectable_label(state.calculator_mode == mode, title)
                            .clicked()
                        {
                            state.queue_action(IdeAction::SetCalculatorMode(mode));
                            ui.close();
                        }
                    }
                    if ui.button("Reset Layout").clicked() {
                        state.queue_action(IdeAction::ResetLayout);
                        ui.close();
                    }
                });
            });
        });
    }
}
