//! Bottom status bar displaying cursor coordinates, VM state, and metrics.

use super::device::DeviceStatus;
use super::{AnalysisState, IdeAction, IdeAppState, IdePanel, VmExecutionState};
use crate::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_RED, TEXT_MUTED};
use egui::RichText;

/// Bottom status bar component.
#[derive(Debug, Default)]
pub struct StatusBar;

impl StatusBar {
    pub fn new() -> Self {
        Self
    }
}

impl IdePanel for StatusBar {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // Cursor position
            let cursor_str = format!("Ln {}, Col {}", state.cursor_line, state.cursor_col);
            ui.label(RichText::new(cursor_str).size(11.0).monospace());

            ui.add(egui::Separator::default().vertical());

            // Encoding & line endings
            ui.label(
                RichText::new(&state.encoding_label)
                    .size(11.0)
                    .color(TEXT_MUTED),
            );

            ui.add(egui::Separator::default().vertical());

            // Compiler verdict for the active buffer's current text.
            let analysis_color = match &state.analysis {
                AnalysisState::Valid => ACCENT_GREEN,
                AnalysisState::ValidWithWarnings(_) => ACCENT_AMBER,
                AnalysisState::Invalid { .. } | AnalysisState::AnalysisFailed(_) => ACCENT_RED,
                _ => TEXT_MUTED,
            };
            let analysis = ui.label(
                RichText::new(state.analysis.label())
                    .size(11.0)
                    .color(analysis_color),
            );
            if let AnalysisState::AnalysisFailed(reason) = &state.analysis {
                analysis.on_hover_text(reason);
            }

            ui.add(egui::Separator::default().vertical());

            // VM State
            let (vm_text, vm_color) = match &state.vm_state {
                VmExecutionState::Idle => ("VM: Idle", TEXT_MUTED),
                VmExecutionState::Running => ("VM: Running", ACCENT_GREEN),
                VmExecutionState::Paused => ("VM: Paused", ACCENT_AMBER),
                VmExecutionState::Terminated => ("VM: Finished", ACCENT_BLUE),
                VmExecutionState::Error(err) => {
                    let preview = match err.char_indices().nth(30) {
                        Some((idx, _)) => &err[..idx],
                        None => err.as_str(),
                    };
                    ui.label(
                        RichText::new(format!("VM Error: {preview}"))
                            .color(ACCENT_RED)
                            .size(11.0),
                    );
                    ("", ACCENT_RED)
                }
            };
            if !matches!(state.vm_state, VmExecutionState::Error(_)) {
                ui.label(RichText::new(vm_text).color(vm_color).size(11.0).strong());
            }

            ui.add(egui::Separator::default().vertical());

            // Metrics: steps and elapsed execution time
            let time_secs = state.elapsed_time.as_secs_f64();
            let metrics_str = format!("Steps: {} ({:.3}s)", state.step_count, time_secs);
            ui.label(
                RichText::new(metrics_str)
                    .size(11.0)
                    .color(TEXT_MUTED)
                    .monospace(),
            );

            ui.add(egui::Separator::default().vertical());

            // Engine selected in the Run menu
            let engine = if state.firmware_backend {
                "Engine: G1 Firmware"
            } else {
                "Engine: Native"
            };
            ui.label(RichText::new(engine).size(11.0).color(TEXT_MUTED));

            // Right-aligned calculator light, then the status message
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                calculator_light(ui, state);
                ui.add(egui::Separator::default().vertical());
                ui.label(
                    RichText::new(&state.status_message)
                        .size(11.0)
                        .color(TEXT_MUTED),
                );
            });
        });
    }
}

/// Coloured dot and label for the USB calculator; click for details.
fn calculator_light(ui: &mut egui::Ui, state: &mut IdeAppState) {
    let (label, color) = match (&state.device_busy, &state.device_status) {
        (Some(purpose), _) => (
            format!("Calculator: {}", purpose.busy_label()),
            ACCENT_AMBER,
        ),
        (None, status @ DeviceStatus::Connected { count: 1, .. }) => (status.label(), ACCENT_GREEN),
        (None, status @ DeviceStatus::Connected { .. }) => (status.label(), ACCENT_AMBER),
        (None, status @ DeviceStatus::DetectFailed(_)) => (status.label(), ACCENT_RED),
        (None, status) => (status.label(), TEXT_MUTED),
    };
    let response = ui
        .add(
            egui::Label::new(RichText::new(format!("● {label}")).size(11.0).color(color))
                .sense(egui::Sense::click()),
        )
        .on_hover_text(match &state.device_status {
            DeviceStatus::Connected { model, .. } => format!("{model}. Click for details."),
            DeviceStatus::DetectFailed(e) => format!("USB enumeration failed: {e}"),
            _ => "Connect an HP Prime with a USB data cable.".into(),
        });
    if response.clicked() && state.device_busy.is_none() && state.device_status.connected() {
        state.queue_action(IdeAction::ShowCalculatorStatus);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::IdeAppState;

    #[test]
    fn test_status_bar_unicode_error_truncation_no_panic() {
        let mut status = StatusBar::new();
        let mut state = IdeAppState::default();
        // 29 ASCII characters + 4-byte unicode emoji crossing byte 30 boundary
        let unicode_err = format!("{}🦀extra", "A".repeat(29));
        state.vm_state = VmExecutionState::Error(unicode_err);

        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                status.render(ui, &mut state);
            });
        });
    }
}
