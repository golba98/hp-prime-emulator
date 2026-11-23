//! Statistics 1Var's LCD cursor and Cartesian coordinate conversion.
use super::cursor_overlay::CursorOverlay;
use crate::ide::{
    native::{expression::EditCommand, keys::KeyAction},
    native_apps::PlotWindow,
    native_plot_controls::{X_PIXEL_SPAN, Y_PIXEL_SPAN},
};

use ppl_vm::PlotCursorMode;
use std::time::Duration;

#[derive(Clone, Copy)]
pub(super) struct PlotCursor(pub(super) [i64; 2]);

impl PlotCursor {
    pub(super) const CENTER: Self = Self([160, 109]);

    fn from_pixels([x, y]: [i64; 2]) -> Self {
        Self([x.clamp(1, 319), y.clamp(0, 218)])
    }

    pub(super) fn from_point(window: &PlotWindow, [x, y]: [f64; 2]) -> Self {
        Self::from_pixels([
            (1.5 + (x - window.xmin) / (window.xmax - window.xmin) * X_PIXEL_SPAN).floor() as i64,
            (0.5 + (window.ymax - y) / (window.ymax - window.ymin) * Y_PIXEL_SPAN).floor() as i64,
        ])
    }

    pub(super) fn from_pointer(pointer: egui::Pos2, origin: egui::Pos2, scale: f32) -> Self {
        let point = (pointer - origin) / scale;
        Self::from_pixels([point.x.floor() as i64, point.y.floor() as i64])
    }

    pub(super) fn point(self, window: &PlotWindow) -> [f64; 2] {
        let [x, y] = self.0;
        [
            window.xmin + (x - 1) as f64 * (window.xmax - window.xmin) / X_PIXEL_SPAN,
            window.ymax - y as f64 * (window.ymax - window.ymin) / Y_PIXEL_SPAN,
        ]
    }

    pub(super) fn step(self, action: KeyAction) -> Option<Self> {
        let [x, y] = self.0;
        let point = match action {
            KeyAction::Edit(EditCommand::Left(false)) => [x - 1, y],
            KeyAction::Edit(EditCommand::Right(false)) => [x + 1, y],
            KeyAction::History(delta) => [x, y + i64::from(delta)],
            _ => return None,
        };
        Some(Self::from_pixels(point))
    }

    pub(super) fn screen(self, origin: egui::Pos2, scale: f32) -> egui::Pos2 {
        let [x, y] = self.0;
        origin + egui::vec2((x as f32 + 0.5) * scale, (y as f32 + 0.5) * scale)
    }

    pub(super) fn paint_mode(
        self,
        painter: &egui::Painter,
        origin: egui::Pos2,
        scale: f32,
        mode: PlotCursorMode,
        overlay: &CursorOverlay,
    ) {
        paint_at(painter, self.screen(origin, scale), scale, mode, overlay);
    }

    pub(super) fn paint(self, painter: &egui::Painter, origin: egui::Pos2, scale: f32) {
        let center = self.screen(origin, scale);
        let ink = egui::Stroke::new(scale, egui::Color32::BLACK);
        for pixel in [egui::vec2(scale, 0.), egui::vec2(0., scale)] {
            painter.line_segment([center - pixel * 4., center - pixel], ink);
            painter.line_segment([center + pixel, center + pixel * 4.], ink);
        }
    }
}

pub(super) fn paint_at(
    painter: &egui::Painter,
    center: egui::Pos2,
    scale: f32,
    mode: PlotCursorMode,
    overlay: &CursorOverlay,
) {
    let arms = [
        [-4.5, -0.5, -0.5, 0.5],
        [0.5, -0.5, 4.5, 0.5],
        [-0.5, -4.5, 0.5, -0.5],
        [-0.5, 0.5, 0.5, 4.5],
    ]
    .map(|[left, top, right, bottom]| {
        egui::Rect::from_min_max(
            center + scale * egui::vec2(left, top),
            center + scale * egui::vec2(right, bottom),
        )
    });
    if mode == PlotCursorMode::Inverting {
        overlay.paint(painter, arms);
        return;
    }
    let ink = if mode == PlotCursorMode::Blinking {
        let time = painter.ctx().input(|input| input.time).rem_euclid(1.0);
        let black = time < 0.5;
        let next_edge = if black { 0.5 } else { 1.0 };
        painter
            .ctx()
            .request_repaint_after(Duration::from_secs_f64(next_edge - time));
        // The original draws white arms on the other phase, including
        // where the cursor crosses a dark axis or colored trace sample.
        if black {
            egui::Color32::BLACK
        } else {
            egui::Color32::WHITE
        }
    } else {
        egui::Color32::BLACK
    };
    for arm in arms {
        painter.rect_filled(arm, 0.0, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_math::Real;

    #[test]
    fn free_cursor_moves_by_one_lcd_pixel_and_reports_typed_cartesian_values() {
        let window = PlotWindow {
            xmin: 0.,
            xmax: 12.,
            ymin: -4.,
            ymax: 8.,
            ..Default::default()
        };
        let cursor = PlotCursor::from_pointer(egui::pos2(60.5, 52.5), egui::Pos2::ZERO, 1.);
        assert_eq!(cursor.0, [60, 52]);
        let cursor = cursor.step(KeyAction::History(1)).unwrap();
        assert_eq!(cursor.0, [60, 53]);
        let cursor = cursor
            .step(KeyAction::Edit(EditCommand::Right(false)))
            .unwrap();
        assert_eq!(cursor.0, [61, 53]);
        let actual = cursor
            .point(&window)
            .map(|coordinate| Real::from_f64(coordinate).unwrap());
        let expected = ["2.2641509434", "5.08256880734"].map(|number| Real::parse(number).unwrap());
        assert_eq!(actual, expected);
        assert_eq!(
            PlotCursor([319, 218])
                .step(KeyAction::History(1))
                .unwrap()
                .0,
            [319, 218]
        );
    }
}
