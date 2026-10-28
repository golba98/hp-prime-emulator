//! Workspace composition: side docks surround the editor/bottom-panel column.
use super::*;
use crate::ide::{
    calculator,
    icons::{self, Icon},
    layout::{ACTIVITY_WIDTH, PanelLimits, WorkspaceLayout},
};

impl IdeDesktopApp {
    fn calculator_content(&mut self, ui: &mut egui::Ui) {
        self.update_display_texture(ui.ctx());
        let status = if self.state.firmware_backend {
            self.firmware_status.as_str()
        } else {
            "Native PPL · keypad enabled while running"
        };
        ui.label(
            egui::RichText::new(status)
                .size(11.0)
                .color(theme::TEXT_MUTED),
        );
        calculator::show(ui, &mut self.state, &mut self.display, &mut self.keypad);
    }
    pub(super) fn render_workspace(&mut self, ctx: &egui::Context) {
        let mode = self.state.layout;
        let limits = PanelLimits::for_width(ctx.screen_rect().width(), self.state.show_explorer);
        if mode != WorkspaceLayout::Calculator {
            egui::SidePanel::left("activity_rail")
                .exact_width(ACTIVITY_WIDTH)
                .frame(egui::Frame::NONE.fill(theme::BG_EXTREME).inner_margin(8.0))
                .show(ctx, |ui| {
                    for (icon, hint, action, selected) in [
                        (
                            Icon::Explorer,
                            "Explorer · Ctrl+B",
                            IdeAction::ToggleExplorer,
                            self.state.show_explorer,
                        ),
                        (Icon::Debug, "Debugger", IdeAction::StartDebugging, false),
                        (
                            Icon::Compile,
                            "Problems",
                            IdeAction::ToggleBottomDock,
                            self.state.show_bottom_dock,
                        ),
                        (
                            Icon::Calculator,
                            "Calculator · Ctrl+K",
                            IdeAction::ToggleCalculator,
                            self.state.show_calculator,
                        ),
                    ] {
                        if icons::button(ui, icon, hint, true, selected).clicked() {
                            self.state.queue_action(action);
                        }
                        ui.add_space(8.0);
                    }
                });
        }
        if mode == WorkspaceLayout::Split && limits.explorer_visible {
            let width = self.state.explorer_width.min(limits.explorer_max);
            reset_panel_width(ctx, "left_explorer", width);
            let panel = egui::SidePanel::left("left_explorer")
                .resizable(true)
                .default_width(width)
                .width_range(160.0..=limits.explorer_max)
                .show(ctx, |ui| self.explorer.render(ui, &mut self.state));
            if resizing(ctx, "left_explorer") {
                self.state.explorer_width = panel.response.rect.width();
            }
        }
        if mode == WorkspaceLayout::Split && self.state.show_calculator {
            let width = self.state.calculator_width.min(limits.calculator_max);
            reset_panel_width(ctx, "right_calculator", width);
            let panel = egui::SidePanel::right("right_calculator")
                .resizable(true)
                .default_width(width)
                .width_range(240.0..=limits.calculator_max)
                .frame(egui::Frame::NONE.fill(theme::BG_EXTREME).inner_margin(12.0))
                .show(ctx, |ui| self.calculator_content(ui));
            if resizing(ctx, "right_calculator") {
                self.state.calculator_width = panel.response.rect.width();
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::editor::BG))
            .show(ctx, |ui| {
                if mode == WorkspaceLayout::Calculator {
                    self.calculator_content(ui);
                } else {
                    if self.state.show_bottom_dock && mode != WorkspaceLayout::Editor {
                        self.render_bottom_dock(ui);
                    }
                    self.editor.render(ui, &mut self.state);
                }
            });
        if !self.state.show_calculator || mode == WorkspaceLayout::Editor {
            self.keypad.release(&mut self.state);
            self.display.last_display_rect = egui::Rect::NOTHING;
        }
        // Transitions and typed key events intentionally have separate meanings:
        // ISKEYDOWN observes the former, GETKEY consumes the latter exactly once.
        let transitions = std::mem::take(&mut self.state.key_transitions);
        for (code, down) in transitions {
            self.send_calc_transition(code, down);
        }
        let events = std::mem::take(&mut self.state.key_events);
        for code in events {
            if self.state.firmware_backend {
                continue;
            }
            if code == 46 {
                self.controller.stop();
            } else {
                self.controller.shared.push_key(code);
            }
        }
    }
    fn render_bottom_dock(&mut self, ui: &mut egui::Ui) {
        let max = (ui.available_height() * 0.6).max(100.0);
        let height = self.state.bottom_height.min(max);
        let id = ui.make_persistent_id("bottom_dock");
        if !ui
            .ctx()
            .read_response(id.with("__resize"))
            .is_some_and(|r| r.dragged())
        {
            if let Some(mut p) = egui::containers::panel::PanelState::load(ui.ctx(), id) {
                p.rect.min.y = p.rect.max.y - height;
                ui.ctx().data_mut(|d| d.insert_persisted(id, p));
            }
        }
        let dock = egui::TopBottomPanel::bottom("bottom_dock")
            .resizable(true)
            .default_height(height)
            .height_range(100.0..=max)
            .frame(egui::Frame::NONE.fill(theme::BG_PANEL).inner_margin(8.0))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for tab in [
                        DockTab::Output,
                        DockTab::Problems,
                        DockTab::Variables,
                        DockTab::CallStack,
                    ] {
                        let title = if tab == DockTab::Problems {
                            self.problems.title()
                        } else {
                            tab.title().to_string()
                        };
                        if dock_tab_button(ui, &title, self.state.active_dock_tab == tab) {
                            self.state.active_dock_tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Close, "Hide panel · Ctrl+J", true, false)
                            .clicked()
                        {
                            self.state.show_bottom_dock = false;
                        }
                    });
                });
                ui.separator();
                match self.state.active_dock_tab {
                    DockTab::Output => self.console.render(ui, &mut self.state),
                    DockTab::Problems => self.problems.render(ui, &mut self.state),
                    DockTab::Variables => self.debugger.render_variables(ui, &mut self.state),
                    DockTab::CallStack => self.debugger.render_call_stack(ui, &mut self.state),
                }
            });
        if ui
            .ctx()
            .read_response(id.with("__resize"))
            .is_some_and(|r| r.dragged())
        {
            self.state.bottom_height = dock.response.rect.height();
        }
    }
}
fn resizing(ctx: &egui::Context, id: &str) -> bool {
    ctx.read_response(egui::Id::new(id).with("__resize"))
        .is_some_and(|r| r.dragged())
}
fn reset_panel_width(ctx: &egui::Context, id: &str, width: f32) {
    if !resizing(ctx, id) {
        if let Some(mut p) = egui::containers::panel::PanelState::load(ctx, egui::Id::new(id)) {
            p.rect.max.x = p.rect.min.x + width;
            ctx.data_mut(|d| d.insert_persisted(egui::Id::new(id), p));
        }
    }
}
