//! Housing, LCD and keys share a single uniform reference transform.
use super::{
    IdeAction, IdeAppState, IdePanel,
    display::CalculatorDisplay,
    keypad::VirtualKeypad,
    layout::{CalculatorMode, DEVICE_MIN_SCALE, WorkspaceLayout},
};
use egui::{Color32, Rect, Stroke, StrokeKind, Vec2};
pub const DEVICE_SIZE: Vec2 = Vec2::new(381.0, 797.0);
pub fn device_rect(available: Rect) -> Rect {
    let scale = (available.width() / DEVICE_SIZE.x)
        .min(available.height() / DEVICE_SIZE.y)
        .max(0.0);
    Rect::from_center_size(available.center(), DEVICE_SIZE * scale)
}
pub fn show(
    ui: &mut egui::Ui,
    state: &mut IdeAppState,
    display: &mut CalculatorDisplay,
    keypad: &mut VirtualKeypad,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("HP PRIME").size(12.0).strong());
        ui.label(
            egui::RichText::new("G1")
                .size(11.0)
                .color(crate::theme::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let expanded = state.layout == WorkspaceLayout::Calculator;
            if ui
                .small_button(if expanded {
                    "Return to editor"
                } else {
                    "Expand"
                })
                .clicked()
            {
                state.queue_action(IdeAction::SetLayout(if expanded {
                    state.previous_layout
                } else {
                    WorkspaceLayout::Calculator
                }));
            }
            ui.menu_button("···", |ui| {
                for (mode, name) in [
                    (CalculatorMode::Full, "Full Calculator"),
                    (CalculatorMode::ScreenOnly, "Screen Only"),
                ] {
                    if ui
                        .selectable_label(state.calculator_mode == mode, name)
                        .clicked()
                    {
                        state.queue_action(IdeAction::SetCalculatorMode(mode));
                        ui.close();
                    }
                }
                ui.separator();
                ui.checkbox(&mut state.smooth_display, "Smooth LCD scaling");
                if ui.button("Save LCD screenshot…").clicked() {
                    state.queue_action(IdeAction::TakeScreenshot);
                    ui.close();
                }
            });
        });
    });
    ui.add_space(12.0);
    let available = ui.available_rect_before_wrap().shrink(8.0);
    let rect = device_rect(available);
    let scale = rect.width() / 381.0;
    let fits = scale >= DEVICE_MIN_SCALE;
    if state.calculator_mode == CalculatorMode::ScreenOnly || !fits {
        keypad.release(state);
        if !fits && state.calculator_mode == CalculatorMode::Full {
            if ui
                .button(
                    egui::RichText::new("Expand for the full keypad")
                        .size(12.0)
                        .color(crate::theme::TEXT_MUTED),
                )
                .clicked()
            {
                state.queue_action(IdeAction::SetLayout(WorkspaceLayout::Calculator));
            }
            ui.add_space(8.0);
        }
        display.render(ui, state);
        return;
    }
    let p = ui.painter();
    // Original vector materials: restrained metal rim, black upper face, silver key bank.
    p.rect_filled(rect, 20.0 * scale, Color32::from_gray(162));
    p.rect_stroke(
        rect.shrink(scale),
        20.0 * scale,
        Stroke::new(scale, Color32::from_gray(204)),
        StrokeKind::Inside,
    );
    p.rect_filled(
        rect.shrink(5.0 * scale),
        16.0 * scale,
        Color32::from_gray(106),
    );
    let at = |x, y| rect.min + Vec2::new(x, y) * scale;
    let upper = Rect::from_min_max(at(10.0, 8.0), at(371.0, 417.0));
    p.rect_filled(upper, 8.0 * scale, Color32::from_gray(7));
    let bank = Rect::from_min_max(at(15.0, 417.0), at(366.0, 776.0));
    p.rect_filled(bank, 7.0 * scale, Color32::from_gray(166));
    p.line_segment(
        [bank.left_top(), bank.right_top()],
        Stroke::new(scale, Color32::from_gray(205)),
    );
    p.text(
        at(21.0, 27.0),
        egui::Align2::LEFT_CENTER,
        "HP Prime Graphing Calculator",
        egui::FontId::proportional(13.0 * scale),
        Color32::WHITE,
    );
    p.circle_filled(at(338.0, 27.0), 12.0 * scale, Color32::from_gray(235));
    p.text(
        at(338.0, 27.0),
        egui::Align2::CENTER_CENTER,
        "hp",
        egui::FontId::proportional(17.0 * scale),
        Color32::from_gray(12),
    );
    let lcd = Rect::from_min_size(at(28.0, 58.0), Vec2::new(320.0, 240.0) * scale);
    display.paint_at(ui, state, lcd);
    keypad.paint_at(ui, state, rect.min, scale);
    ui.allocate_space(ui.available_size());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uniform_fit_never_stretches_or_crops() {
        for (w, h) in [(400.0, 900.0), (800.0, 600.0), (2560.0, 1440.0)] {
            let available = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(w, h));
            let r = device_rect(available);
            assert!((r.width() / r.height() - 381.0 / 797.0).abs() < 0.0001);
            assert!(available.expand(0.001).contains_rect(r));
        }
    }
}
