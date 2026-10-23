//! Bottom status bar displaying cursor coordinates, VM state, and metrics.

use super::{IdeAppState, IdePanel, VmExecutionState};
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
            ui.label(RichText::new("UTF-8").size(11.0).color(TEXT_MUTED));

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

            // Display resolution and scale
            let disp_str = format!("Disp: 320×240 ({:.0}x)", state.display_scale);
            ui.label(RichText::new(disp_str).size(11.0).color(TEXT_MUTED));

            // Right-aligned status message
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(&state.status_message)
                        .size(11.0)
                        .color(TEXT_MUTED),
                );
            });
        });
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
