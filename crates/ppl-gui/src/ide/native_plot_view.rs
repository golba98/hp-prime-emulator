//! Shared LCD plot geometry and interaction for function and model apps.
use super::{
    native_apps::{CurveSamples, PlotWindow},
    native_model_plots::PlotStyle,
};
#[derive(Default)]
pub struct PlotInteraction {
    pub drag_stopped: bool,
    pub clicked: Option<[f64; 2]>,
}
pub fn show(
    ui: &mut egui::Ui,
    window: &mut PlotWindow,
    curves: &[CurveSamples],
    styles: &[PlotStyle],
    size: egui::Vec2,
    scale: f32,
    trace: Option<[f64; 2]>,
) -> PlotInteraction {
    if let Err(error) = window.validate() {
        ui.colored_label(egui::Color32::DARK_RED, error);
        return PlotInteraction::default();
    }
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let project = |p: [f64; 2]| {
        egui::pos2(
            rect.left()
                + ((p[0] - window.xmin) / (window.xmax - window.xmin)) as f32 * rect.width(),
            rect.bottom()
                - ((p[1] - window.ymin) / (window.ymax - window.ymin)) as f32 * rect.height(),
        )
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0., egui::Color32::WHITE);
    if window.axes {
        if window.ymin <= 0. && window.ymax >= 0. {
            painter.line_segment(
                [project([window.xmin, 0.]), project([window.xmax, 0.])],
                egui::Stroke::new(0.5, egui::Color32::GRAY),
            );
        }
        if window.xmin <= 0. && window.xmax >= 0. {
            painter.line_segment(
                [project([0., window.ymin]), project([0., window.ymax])],
                egui::Stroke::new(0.5, egui::Color32::GRAY),
            );
        }
    }
    for (index, curve) in curves.iter().enumerate() {
        let color = [
            egui::Color32::BLUE,
            egui::Color32::RED,
            egui::Color32::DARK_GREEN,
            egui::Color32::MAGENTA,
        ][index % 4];
        if styles.get(index) == Some(&PlotStyle::Scatter) {
            for point in curve.iter().flatten() {
                painter.circle_filled(project(*point), 2. * scale, color);
            }
        } else {
            for pair in curve.windows(2) {
                if let [Some(a), Some(b)] = pair
                    && (!styles.is_empty()
                        || (a[1] - b[1]).abs() < (window.ymax - window.ymin) * 0.75)
                {
                    painter.line_segment(
                        [project(*a), project(*b)],
                        egui::Stroke::new(scale.max(0.5), color),
                    );
                }
            }
        }
    }
    if let Some(point) = trace {
        painter.circle_stroke(
            project(point),
            3. * scale,
            egui::Stroke::new(scale.max(0.5), egui::Color32::BLACK),
        );
        painter.text(
            rect.left_bottom() + egui::vec2(3., -3.),
            egui::Align2::LEFT_BOTTOM,
            format!("x={:.6}  y={:.6}", point[0], point[1]),
            egui::FontId::monospace(10. * scale),
            egui::Color32::BLACK,
        );
    }
    let clicked = if response.clicked() {
        response.interact_pointer_pos().map(|point| {
            [
                window.xmin
                    + f64::from((point.x - rect.left()) / rect.width())
                        * (window.xmax - window.xmin),
                window.ymax
                    - f64::from((point.y - rect.top()) / rect.height())
                        * (window.ymax - window.ymin),
            ]
        })
    } else {
        None
    };
    if response.dragged() || response.drag_stopped() {
        // egui stops reporting dragged/drag_delta on the release frame. Its
        // final pointer movement still belongs to this drag and must be applied
        // before the caller resamples the new window.
        let delta = if response.dragged() {
            response.drag_delta()
        } else {
            let mut delta = ui.input(|input| input.pointer.delta());
            if let Some(transform) = ui.ctx().layer_transform_from_global(response.layer_id) {
                delta *= transform.scaling;
            }
            delta
        };
        window.pan(
            -f64::from(delta.x / rect.width()),
            f64::from(delta.y / rect.height()),
        );
    }
    PlotInteraction {
        drag_stopped: response.drag_stopped(),
        clicked,
    }
}
