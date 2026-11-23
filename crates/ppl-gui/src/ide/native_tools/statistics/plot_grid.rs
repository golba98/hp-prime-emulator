//! Statistics 1Var's grid uses the same calculator pixels as the free cursor.
use crate::ide::{
    native_apps::PlotWindow,
    native_plot_controls::{X_PIXEL_SPAN, Y_PIXEL_SPAN, axis_pixel, ticks},
};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use ppl_vm::{AppPlotSettings, Settings};

use super::view::display;

const WIDTH: i64 = 320;
const HEIGHT: i64 = Y_PIXEL_SPAN as i64;
const LABEL_FONT: f32 = 12.;
const GRID_LINE: u32 = 0xe9e9e9;
const GRID_DOT: u32 = 0x949494;

pub(super) fn paint(
    grob: &mut Grob,
    window: &PlotWindow,
    settings: &AppPlotSettings,
    number_format: &Settings,
) {
    if window.validate().is_err() {
        return;
    }
    let x_ticks = ticks(
        window.xmin,
        window.xmax,
        settings.xtick.to_f64(),
        X_PIXEL_SPAN,
    )
    .into_iter()
    .map(|x| x + 1)
    .collect::<Vec<_>>();
    let y_ticks = ticks(
        -window.ymax,
        -window.ymin,
        settings.ytick.to_f64(),
        Y_PIXEL_SPAN,
    );

    // The G1 exposes the visibility masks inverted in these app variables.
    if !settings.grid_lines {
        for &x in &x_ticks {
            grob.line(x, 0, x, HEIGHT, GRID_LINE);
        }
        for &y in &y_ticks {
            grob.line(0, y, WIDTH - 1, y, GRID_LINE);
        }
    }
    if !settings.grid_dots {
        for &x in &x_ticks {
            for &y in &y_ticks {
                grob.set(x, y, GRID_DOT);
            }
        }
    }
    let vertical = (!settings.axes && window.xmin <= 0. && window.xmax >= 0.)
        .then(|| 1 + axis_pixel(-window.xmin / (window.xmax - window.xmin) * X_PIXEL_SPAN));
    let horizontal = (!settings.axes && window.ymin <= 0. && window.ymax >= 0.)
        .then(|| axis_pixel(window.ymax / (window.ymax - window.ymin) * Y_PIXEL_SPAN));
    if let Some(x) = vertical {
        grob.line(x, 0, x, HEIGHT, 0);
        for &y in &y_ticks {
            grob.line(x - 2, y, x + 2, y, 0);
        }
    }
    if let Some(y) = horizontal {
        grob.line(0, y, WIDTH - 1, y, 0);
        for &x in &x_ticks {
            grob.line(x, (y - 2).max(0), x, (y + 2).min(HEIGHT), 0);
        }
    }
    if settings.labels {
        paint_labels(grob, window, number_format, vertical, horizontal);
    }
}

fn paint_labels(
    grob: &mut Grob,
    window: &PlotWindow,
    format: &Settings,
    vertical: Option<i64>,
    horizontal: Option<i64>,
) {
    let xmin = window.xmin - (window.xmax - window.xmin) / X_PIXEL_SPAN;
    let label = |value| display(value, format).replace('-', "−");
    let xrange = [label(xmin), label(window.xmax)];
    let yrange = [label(window.ymax), label(window.ymin)];
    let draw = |grob: &mut Grob, label: &str, x, y| {
        draw_text(grob, label, x, y, LABEL_FONT, 0, None, None);
    };
    let width = |label: &str| measure(label, LABEL_FONT);

    let (x_values, x_mark) = horizontal.map_or((HEIGHT / 2 - 2, HEIGHT / 2 + 10), |at| {
        ((at + 3).min(HEIGHT - 10), (at - 13).max(0))
    });
    draw(grob, &xrange[0], 1, x_values);
    draw(grob, &xrange[1], WIDTH - width(&xrange[1]), x_values);
    draw(grob, "X", WIDTH - width("X"), x_mark);

    let label_column = |value: &str| match vertical {
        Some(x) if x <= WIDTH / 2 => x + 4,
        Some(x) => x - 3 - width(value),
        None => WIDTH / 2 + 3 - width(value) / 2,
    };
    draw(grob, &yrange[0], label_column(&yrange[0]), 0);
    draw(grob, &yrange[1], label_column(&yrange[1]), HEIGHT - 10);
    let (x, y) = match vertical {
        Some(x) if x <= WIDTH / 2 => (x - 3 - width("Y"), 0),
        Some(x) => (x + 4, 0),
        None => (WIDTH / 2 + 3 - width("Y") / 2, 12),
    };
    draw(grob, "Y", x, y);
}
