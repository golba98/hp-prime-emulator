//! Debugger panel hosting variable inspector and call stack frames.

use super::execution::{DebugPauseSnapshot, VariableSnapshot};
use super::{IdeAppState, IdePanel};
use crate::theme::{ACCENT_BLUE, TEXT_MUTED};
use egui::{FontId, RichText, ScrollArea};

/// Debugger inspector panel widget.
#[derive(Debug, Default)]
pub struct DebuggerPanel {
    pub snapshot: Option<DebugPauseSnapshot>,
    pub selected_frame_index: usize,
}

impl DebuggerPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_snapshot(&mut self, snapshot: Option<DebugPauseSnapshot>) {
        self.snapshot = snapshot;
        self.selected_frame_index = 0;
    }

    /// Renders the Variables inspection tree tab.
    pub fn render_variables(&mut self, ui: &mut egui::Ui, _state: &mut IdeAppState) {
        let Some(snapshot) = &self.snapshot else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Debugger paused: No active debug session.").color(TEXT_MUTED),
                );
            });
            return;
        };

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                // Local variables in selected frame
                if let Some(frame) = snapshot.frames.get(self.selected_frame_index) {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("▾ Locals ({})", frame.function))
                            .strong()
                            .size(11.0),
                    )
                    .default_open(true)
                    .show(ui, |ui| {
                        if frame.locals.is_empty() {
                            ui.label(
                                RichText::new("  (No local variables)")
                                    .color(TEXT_MUTED)
                                    .size(11.0),
                            );
                        } else {
                            render_variable_table(ui, &frame.locals);
                        }
                    });
                }

                ui.add_space(4.0);

                // Global variables
                egui::CollapsingHeader::new(RichText::new("▾ Globals").strong().size(11.0))
                    .default_open(true)
                    .show(ui, |ui| {
                        if snapshot.globals.is_empty() {
                            ui.label(
                                RichText::new("  (No global variables)")
                                    .color(TEXT_MUTED)
                                    .size(11.0),
                            );
                        } else {
                            render_variable_table(ui, &snapshot.globals);
                        }
                    });
            });
    }

    /// Renders the Call Stack tab.
    pub fn render_call_stack(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        let Some(snapshot) = &self.snapshot else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No active call stack.").color(TEXT_MUTED));
            });
            return;
        };

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("call_stack_grid")
                    .striped(true)
                    .num_columns(3)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("#").strong().size(11.0));
                        ui.label(RichText::new("Function").strong().size(11.0));
                        ui.label(RichText::new("Line:Col").strong().size(11.0));
                        ui.end_row();

                        for (idx, frame) in snapshot.frames.iter().enumerate() {
                            let is_selected = idx == self.selected_frame_index;
                            let prefix = if is_selected { "▶ " } else { "  " };

                            ui.label(RichText::new(format!("{prefix}{idx}")).size(11.0));

                            if ui.selectable_label(is_selected, &frame.function).clicked() {
                                self.selected_frame_index = idx;
                                state.cursor_line = frame.line;
                                state.cursor_col = frame.col;
                            }

                            ui.label(
                                RichText::new(format!("{}:{}", frame.line, frame.col))
                                    .monospace()
                                    .size(11.0),
                            );
                            ui.end_row();
                        }
                    });
            });
    }
}

fn render_variable_table(ui: &mut egui::Ui, vars: &[VariableSnapshot]) {
    egui::Grid::new("vars_table")
        .striped(true)
        .num_columns(3)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Name").strong().size(11.0));
            ui.label(RichText::new("Type").strong().size(11.0));
            ui.label(RichText::new("Value").strong().size(11.0));
            ui.end_row();

            for var in vars {
                ui.label(RichText::new(&var.name).strong().size(11.0));
                ui.label(RichText::new(var.type_name).color(ACCENT_BLUE).size(11.0));
                ui.label(RichText::new(&var.preview).font(FontId::monospace(11.0)));
                ui.end_row();
            }
        });
}

impl IdePanel for DebuggerPanel {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        self.render_variables(ui, state);
    }
}
