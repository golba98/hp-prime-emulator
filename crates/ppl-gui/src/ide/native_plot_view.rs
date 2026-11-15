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
    show_impl(ui, window, curves, styles, size, scale, trace, false, None)
}
/// Graph definitions keep their chosen colors in every plotted curve.
pub fn show_graph(
    ui: &mut egui::Ui,
    window: &mut PlotWindow,
    curves: &[CurveSamples],
    colors: &[egui::Color32],
    size: egui::Vec2,
    scale: f32,
    trace: Option<[f64; 2]>,
) -> PlotInteraction {
    show_impl(
        ui,
        window,
        curves,
        &[],
        size,
        scale,
        trace,
        false,
        Some(colors),
    )
}
/// Full-screen Statistics plot, with the Prime grid and observation markers.
pub fn show_prime(
    ui: &mut egui::Ui,
    window: &mut PlotWindow,
    curves: &[CurveSamples],
    styles: &[PlotStyle],
    size: egui::Vec2,
    scale: f32,
    trace: Option<[f64; 2]>,
) -> PlotInteraction {
    show_impl(ui, window, curves, styles, size, scale, trace, true, None)
}
#[allow(clippy::too_many_arguments)]
fn show_impl(
    ui: &mut egui::Ui,
    window: &mut PlotWindow,
    curves: &[CurveSamples],
    styles: &[PlotStyle],
    size: egui::Vec2,
    scale: f32,
    trace: Option<[f64; 2]>,
    prime: bool,
    colors: Option<&[egui::Color32]>,
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
    if prime && window.axes {
        // Bound grid work for any zoom level. Unit spacing grows by powers of ten.
        let step = |span: f64| 10_f64.powf((span / 32.).log10().ceil());
        let dx = step(window.xmax - window.xmin);
        let dy = step(window.ymax - window.ymin);
        for i in 0..=34 {
            let x = (window.xmin / dx).ceil() * dx + i as f64 * dx;
            if x <= window.xmax {
                painter.line_segment(
                    [project([x, window.ymin]), project([x, window.ymax])],
                    egui::Stroke::new(0.5_f32, egui::Color32::from_gray(192)),
                );
            }
            let y = (window.ymin / dy).ceil() * dy + i as f64 * dy;
            if y <= window.ymax {
                painter.line_segment(
                    [project([window.xmin, y]), project([window.xmax, y])],
                    egui::Stroke::new(0.5_f32, egui::Color32::from_gray(192)),
                );
            }
        }
    }
    if window.axes {
        if window.ymin <= 0. && window.ymax >= 0. {
            painter.line_segment(
                [project([window.xmin, 0.]), project([window.xmax, 0.])],
                egui::Stroke::new(
                    0.5_f32,
                    if prime {
                        egui::Color32::BLACK
                    } else {
                        egui::Color32::GRAY
                    },
                ),
            );
        }
        if window.xmin <= 0. && window.xmax >= 0. {
            painter.line_segment(
                [project([0., window.ymin]), project([0., window.ymax])],
                egui::Stroke::new(
                    0.5_f32,
                    if prime {
                        egui::Color32::BLACK
                    } else {
                        egui::Color32::GRAY
                    },
                ),
            );
        }
    }
    let mut prime_color = 0;
    let mut scatter_seen = false;
    for (index, curve) in curves.iter().enumerate() {
        if prime && styles.get(index) == Some(&PlotStyle::Scatter) {
            if scatter_seen {
                prime_color += 1;
            }
            scatter_seen = true;
        }
        let color_index = if prime && scatter_seen {
            prime_color
        } else {
            index
        };
        let default = [
            egui::Color32::BLUE,
            egui::Color32::RED,
            egui::Color32::DARK_GREEN,
            egui::Color32::MAGENTA,
        ][color_index % 4];
        let color = colors
            .and_then(|colors| colors.get(index))
            .copied()
            .unwrap_or(default);
        if styles.get(index) == Some(&PlotStyle::Scatter) {
            for point in curve.iter().flatten() {
                if prime {
                    painter.circle_stroke(
                        project(*point),
                        2.5 * scale,
                        egui::Stroke::new(scale, color),
                    );
                } else {
                    painter.circle_filled(project(*point), 2. * scale, color);
                }
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
        if !prime {
            painter.text(
                rect.left_bottom() + egui::vec2(3., -3.),
                egui::Align2::LEFT_BOTTOM,
                format!("x={:.6}  y={:.6}", point[0], point[1]),
                crate::ide::native_fonts::font_id(ui, egui::TextStyle::Monospace, 10. * scale),
                egui::Color32::BLACK,
            );
        }
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
