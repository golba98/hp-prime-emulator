//! Shared LCD plot geometry and interaction for function and model apps.
use ppl_vm::AppPlotSettings;

use super::{
    native_apps::{CurveSamples, PlotWindow},
    native_model_plots::{DOT_FILL_ALPHA, PlotStyle},
    native_plot_controls::{axis_pixel, ticks},
};
#[derive(Default)]
pub struct PlotInteraction {
    pub drag_stopped: bool,
    pub clicked: Option<[f64; 2]>,
    pub trace_cursor: Option<(egui::Rect, egui::Pos2)>,
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
    show_impl(
        ui, window, curves, styles, size, scale, trace, false, None, None, true, false, None,
    )
}
pub struct GraphStyle<'a> {
    pub colors: &'a [egui::Color32],
    pub function_setup: Option<&'a AppPlotSettings>,
}

/// Graph definitions keep their chosen colors in every plotted curve.
pub fn show_graph(
    ui: &mut egui::Ui,
    window: &mut PlotWindow,
    curves: &[CurveSamples],
    style: GraphStyle<'_>,
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
        Some(style.colors),
        None,
        true,
        false,
        style.function_setup,
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
    show_impl(
        ui, window, curves, styles, size, scale, trace, true, None, None, true, false, None,
    )
}
/// Statistics chart geometry includes chosen colors, filled sectors and labels.
#[allow(clippy::too_many_arguments)]
pub fn show_prime_data(
    ui: &mut egui::Ui,
    window: &mut PlotWindow,
    curves: &[CurveSamples],
    data: &super::native_model_plots::PlotData,
    size: egui::Vec2,
    scale: f32,
    trace: Option<[f64; 2]>,
    interactive: bool,
    calculator_backdrop: bool,
) -> PlotInteraction {
    let colors = data.colors.iter().copied().map(rgb).collect::<Vec<_>>();
    show_impl(
        ui,
        window,
        curves,
        &data.styles,
        size,
        scale,
        trace,
        true,
        Some(&colors),
        Some(data),
        interactive,
        calculator_backdrop,
        None,
    )
}
// The Function plot uses the G1 visibility flags and tick geometry within
// the current desktop viewport. Curves and the trace are painted over it.
fn paint_function_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    scale: f32,
    window: &PlotWindow,
    setup: &AppPlotSettings,
) {
    const LINE: egui::Color32 = egui::Color32::from_gray(233);
    const DOT: egui::Color32 = egui::Color32::from_gray(148);
    const AXIS: egui::Color32 = egui::Color32::BLACK;

    if setup.axes && setup.grid_dots && setup.grid_lines {
        return;
    }
    let width = (rect.width() / scale).floor() as i64;
    let height = (rect.height() / scale).floor() as i64;
    if width < 3 || height < 2 {
        return;
    }
    let x_extent = (width - 2) as f64;
    let y_extent = (height - 1) as f64;
    let xs = ticks(window.xmin, window.xmax, setup.xtick.to_f64(), x_extent)
        .into_iter()
        .map(|x| x + 1)
        .collect::<Vec<_>>();
    let ys = ticks(-window.ymax, -window.ymin, setup.ytick.to_f64(), y_extent);
    let ink = |x: i64, y: i64, columns: i64, rows: i64, color| {
        painter.rect_filled(
            egui::Rect::from_min_size(
                rect.min + scale * egui::vec2(x as f32, y as f32),
                scale * egui::vec2(columns as f32, rows as f32),
            ),
            0.,
            color,
        );
    };
    if !setup.grid_lines {
        for &x in &xs {
            ink(x, 0, 1, height, LINE);
        }
        for &y in &ys {
            ink(0, y, width, 1, LINE);
        }
    }
    if !setup.grid_dots {
        if xs.len() == width as usize - 1 {
            for &y in &ys {
                ink(1, y, width - 1, 1, DOT);
            }
        } else if ys.len() == height as usize {
            for &x in &xs {
                ink(x, 0, 1, height, DOT);
            }
        } else {
            for &x in &xs {
                for &y in &ys {
                    ink(x, y, 1, 1, DOT);
                }
            }
        }
    }
    if !setup.axes && window.xmin <= 0. && window.xmax >= 0. {
        let x = 1 + axis_pixel(-window.xmin / (window.xmax - window.xmin) * x_extent);
        ink(x, 0, 1, height, AXIS);
        for &y in &ys {
            ink(x - 2, y, 5, 1, AXIS);
        }
    }
    if !setup.axes && window.ymin <= 0. && window.ymax >= 0. {
        let y = axis_pixel(window.ymax / (window.ymax - window.ymin) * y_extent);
        ink(0, y, width, 1, AXIS);
        for &x in &xs {
            ink(x, y - 2, 1, 5, AXIS);
        }
    }
}

fn rgb(color: u32) -> egui::Color32 {
    egui::Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
}
fn paint_dots(
    painter: &egui::Painter,
    window: &PlotWindow,
    rect: egui::Rect,
    columns: &[super::native_model_plots::DotColumn],
    scale: f32,
) {
    let diameter = egui::vec2(
        (f64::from(rect.width()) / (window.xmax - window.xmin)) as f32,
        (f64::from(rect.height()) / (window.ymax - window.ymin)) as f32,
    );
    // The original rasterizer rounds these subpixel disks away. Keep their
    // full counts and exact grouped trace even when the overview has no ink.
    if diameter.x < scale || diameter.y < scale {
        return;
    }
    for column in columns {
        let rows = column
            .count
            .saturating_add(u64::from(column.fraction.is_some()));
        if rows == 0
            || column.x + 0.5 < window.xmin
            || column.x - 0.5 > window.xmax
            || window.ymax < 0.
            || window.ymin > rows as f64
        {
            continue;
        }
        let first = (window.ymin - 1.).ceil().max(0.) as u64;
        let last = (window.ymax.floor().max(0.) as u64).min(rows - 1);
        if first > last {
            continue;
        }
        let x = rect.left()
            + ((column.x - window.xmin) / (window.xmax - window.xmin)) as f32 * rect.width();
        // The stored shade includes the contribution from a white background.
        // Remove it so the mesh also blends with the grid and earlier stacks.
        let background = 255 - DOT_FILL_ALPHA;
        let color = rgb(column.color);
        let fill = egui::Color32::from_rgba_premultiplied(
            color.r().saturating_sub(background),
            color.g().saturating_sub(background),
            color.b().saturating_sub(background),
            DOT_FILL_ALPHA,
        );
        // At least one pixel per disk bounds this loop by the visible LCD
        // height, independently of the frequency total stored in memory.
        for row in first..=last {
            let y = rect.bottom()
                - ((row as f64 + 0.5 - window.ymin) / (window.ymax - window.ymin)) as f32
                    * rect.height();
            let center = egui::pos2(x, y);
            let radius = diameter * 0.5;
            let segments = (radius.x.max(radius.y) * std::f32::consts::PI / 4.)
                .ceil()
                .clamp(16., 96.) as usize;
            let fraction = column.fraction.filter(|_| row == column.count);
            let sweep = fraction.unwrap_or(1.) as f32;
            let ring = (0..segments + usize::from(fraction.is_some()))
                .map(|index| {
                    let angle = std::f32::consts::TAU * sweep * index as f32 / segments as f32;
                    center + egui::vec2(radius.x * angle.cos(), radius.y * angle.sin())
                })
                .collect::<Vec<_>>();
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(center, fill);
            for &point in &ring {
                mesh.colored_vertex(point, fill);
            }
            for index in 0..segments {
                mesh.add_triangle(0, index as u32 + 1, ((index + 1) % ring.len()) as u32 + 1);
            }
            painter.add(egui::Shape::mesh(mesh));
            let mut outline = ring;
            if fraction.is_some() {
                outline.push(center);
            }
            painter.add(egui::Shape::closed_line(
                outline,
                egui::Stroke::new(scale.max(0.5), egui::Color32::BLACK),
            ));
        }
    }
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
    data: Option<&super::native_model_plots::PlotData>,
    interactive: bool,
    calculator_backdrop: bool,
    function_setup: Option<&AppPlotSettings>,
) -> PlotInteraction {
    if let Err(error) = window.validate() {
        ui.colored_label(egui::Color32::DARK_RED, error);
        return PlotInteraction::default();
    }
    let sense = if interactive {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let project = |p: [f64; 2]| {
        egui::pos2(
            rect.left()
                + ((p[0] - window.xmin) / (window.xmax - window.xmin)) as f32 * rect.width(),
            rect.bottom()
                - ((p[1] - window.ymin) / (window.ymax - window.ymin)) as f32 * rect.height(),
        )
    };
    let painter = ui.painter().with_clip_rect(rect);
    if !calculator_backdrop {
        painter.rect_filled(rect, 0., egui::Color32::WHITE);
    }
    if !calculator_backdrop && prime && window.axes {
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
    if let Some(setup) = function_setup {
        paint_function_grid(&painter, rect, scale, window, setup);
    } else if !calculator_backdrop && window.axes {
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
    if let Some(data) = data {
        paint_dots(&painter, window, rect, &data.dots, scale);
        for fill in &data.fills {
            if fill.points.len() >= 3 {
                // Tessellate the whole fan as one mesh: separately antialiased
                // triangles leave visible seams inside disks and pie sectors.
                let mut mesh = egui::Mesh::default();
                for &point in &fill.points {
                    mesh.colored_vertex(project(point), rgb(fill.color));
                }
                for index in 1..fill.points.len() - 1 {
                    mesh.add_triangle(0, index as u32, (index + 1) as u32);
                }
                painter.add(egui::Shape::mesh(mesh));
            }
        }
    }
    let mut prime_color = 0;
    let mut scatter_seen = false;
    for (index, curve) in curves.iter().enumerate() {
        if styles.get(index) == Some(&PlotStyle::TraceOnly) {
            continue;
        }
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
            if matches!(
                styles.get(index),
                Some(PlotStyle::PointLine | PlotStyle::CrossLine)
            ) {
                for point in curve.iter().flatten() {
                    let point = project(*point);
                    if styles.get(index) == Some(&PlotStyle::PointLine) {
                        painter.circle_stroke(point, 2.5 * scale, egui::Stroke::new(scale, color));
                    } else {
                        let delta = egui::vec2(2.5 * scale, 2.5 * scale);
                        painter.line_segment(
                            [point - delta, point + delta],
                            egui::Stroke::new(scale, color),
                        );
                        let delta = egui::vec2(delta.x, -delta.y);
                        painter.line_segment(
                            [point - delta, point + delta],
                            egui::Stroke::new(scale, color),
                        );
                    }
                }
            }
        }
    }
    if let Some(data) = data {
        for annotation in &data.annotations {
            painter.text(
                project(annotation.point),
                if annotation.right_aligned {
                    egui::Align2::RIGHT_CENTER
                } else {
                    egui::Align2::LEFT_CENTER
                },
                &annotation.text,
                crate::ide::native_fonts::font_id(
                    ui,
                    egui::TextStyle::Monospace,
                    annotation.font_size * scale,
                ),
                rgb(annotation.color),
            );
        }
    }
    if let Some(point) = trace {
        if prime {
            let center = project(point);
            let radius = 3. * scale;
            painter.add(egui::Shape::closed_line(
                vec![
                    center + egui::vec2(0., -radius),
                    center + egui::vec2(radius, 0.),
                    center + egui::vec2(0., radius),
                    center + egui::vec2(-radius, 0.),
                ],
                egui::Stroke::new(scale.max(0.5), egui::Color32::BLUE),
            ));
        } else if function_setup.is_none() {
            painter.circle_stroke(
                project(point),
                3. * scale,
                egui::Stroke::new(scale.max(0.5), egui::Color32::BLACK),
            );
        }
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
    if !interactive {
        return PlotInteraction::default();
    }
    let trace_cursor = trace
        .filter(|_| function_setup.is_some())
        .map(|point| (rect, project(point)));
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
        trace_cursor,
    }
}
