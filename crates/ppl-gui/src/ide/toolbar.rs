//! Restrained action groups with vector icons and explicit execution backend.
use super::{
    IdeAction, IdeAppState, IdePanel, VmExecutionState,
    icons::{self, Icon},
    layout::{CalculatorMode, WorkspaceLayout},
};
#[derive(Debug, Default)]
pub struct ToolBar;
impl ToolBar {
    pub fn new() -> Self {
        Self
    }
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
            for (icon, hint, action, enabled) in [
                (Icon::Compile, "Compile · F7", IdeAction::CheckSyntax, !busy),
                (
                    Icon::Run,
                    "Run / Continue · F5",
                    IdeAction::RunProgram,
                    !matches!(state.vm_state, VmExecutionState::Running),
                ),
                (Icon::Stop, "Stop · Shift+F5", IdeAction::StopProgram, busy),
                (
                    Icon::Restart,
                    "Restart program · Ctrl+F5",
                    IdeAction::RestartProgram,
                    true,
                ),
                (
                    Icon::Debug,
                    if state.firmware_backend {
                        "Firmware stepping is not supported; select Native to debug"
                    } else {
                        "Debug · F9"
                    },
                    IdeAction::StartDebugging,
                    !busy && !state.firmware_backend,
                ),
            ] {
                if icons::button(ui, icon, hint, enabled, false).clicked() {
                    state.queue_action(action);
                }
            }
            ui.separator();
            let previous = state.firmware_backend;
            let mut selected_backend = previous;
            egui::ComboBox::from_id_salt("execution_backend")
                .width(100.0)
                .selected_text(if previous {
                    "G1 Firmware"
                } else {
                    "Native PPL"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected_backend, false, "Native PPL");
                    ui.selectable_value(&mut selected_backend, true, "G1 Firmware");
                });
            if previous != selected_backend {
                state.queue_action(IdeAction::SelectBackend(selected_backend));
            }
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
                if icons::button(
                    ui,
                    Icon::Calculator,
                    "Toggle calculator · Ctrl+K",
                    true,
                    state.show_calculator,
                )
                .clicked()
                {
                    state.queue_action(IdeAction::ToggleCalculator);
                }
                if ui.available_width() > 90.0
                    && icons::button(
                        ui,
                        Icon::Explorer,
                        "Toggle explorer · Ctrl+B",
                        true,
                        state.show_explorer,
                    )
                    .clicked()
                {
                    state.queue_action(IdeAction::ToggleExplorer);
                }
            });
        });
    }
}
