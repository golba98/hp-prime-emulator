//! The data and Symbolic screens share the G1's 320×240 grid.
use super::*;
use crate::ide::native_tools::Page;
use crate::ide::{
    native::chrome,
    native_regression::{FitKind, Regression},
};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
const PAPER: u32 = 0xe8f1f6;
const INK: u32 = 0;
const COLORS: [u32; 5] = [0x0000ff, 0xff0000, 0x00a000, 0xff00ff, 0x00bfff];
pub(super) fn text(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 14., INK, None, None);
}
fn outline(g: &mut Grob, [x, y, r, b]: [i64; 4]) {
    g.line(x, y, r, y, INK);
    g.line(x, b, r, b, INK);
    g.line(x, y, x, b, INK);
    g.line(r, y, r, b, INK);
}
fn rect(analysis: usize, field: Field, scroll: i64, two: bool) -> [i64; 4] {
    let top = 22 + analysis as i64 * 79 - scroll;
    match field {
        Field::Enabled => [1, top + 3, 14, top + 16],
        Field::X => [
            if two { 35 } else { 40 },
            top,
            if two { 130 } else { 178 },
            top + 21,
        ],
        Field::Y => [131, top, 225, top + 21],
        Field::Frequency => [if two { 226 } else { 179 }, top, 313, top + 21],
        Field::Fit => [43, top + 27, 267, top + 47],
        Field::Expression => [35, top + 54, 313, top + 75],
    }
}
fn contains([x, y, r, b]: [i64; 4], point: (i64, i64)) -> bool {
    point.0 >= x && point.0 <= r && point.1 >= y && point.1 <= b
}
fn tick(g: &mut Grob, x: i64, y: i64) {
    g.line(x + 2, y + 7, x + 5, y + 11, 0);
    g.line(x + 5, y + 11, x + 11, y + 1, 0);
}
fn clipped(g: &mut Grob, s: &str, [x, y, r, b]: [i64; 4], right: bool) {
    let mut cell = Grob::new((r - x + 1) as usize, (b - y + 1) as usize, 0xffffff);
    let at = if right {
        r - x - measure(s, 14.) - 1
    } else {
        1
    };
    text(&mut cell, s, at, 1);
    g.blit(
        (x, y, r + 1, b + 1),
        &cell,
        (0, 0, r - x + 1, b - y + 1),
        None,
        255,
    );
}
pub(super) fn display(n: f64, s: &ppl_vm::Settings) -> String {
    ppl_math::Real::from_f64(n)
        .map(|n| n.format(s.home_format()))
        .unwrap_or_else(|_| "Out of range".into())
}
fn default_equation(kind: FitKind) -> &'static str {
    match kind {
        FitKind::Linear => "M*X+B",
        FitKind::Logarithmic => "M*LN(X)+B",
        FitKind::Exponential => "B*EXP(M*X)",
        FitKind::Power => "B*X^M",
        FitKind::Exponent => "B*M^X",
        FitKind::Inverse => "M/X+B",
        FitKind::Logistic => "L/(1+A*EXP(-B*X))",
        FitKind::Quadratic => "A*X^2+B*X+C",
        FitKind::Cubic => "A*X^3+B*X^2+C*X+D",
        FitKind::Quartic => "A*X^4+B*X^3+C*X^2+D*X+E",
        FitKind::Trigonometric => "A*SIN(B*X+C)+D",
        FitKind::MedianMedian => "M*X+B",
        FitKind::UserDefined => "",
    }
}
fn equation(fit: &Regression) -> String {
    let c = fit
        .coefficients
        .iter()
        .map(|&n| format!("({})", canonical(n)))
        .collect::<Vec<_>>();
    match fit.kind {
        FitKind::UserDefined => fit.user_expression.clone().unwrap_or_default(),
        FitKind::Logistic if c.len() == 3 => format!("{}/(1+{}*EXP(-{}*X))", c[0], c[1], c[2]),
        FitKind::Trigonometric if c.len() == 4 => {
            let angle = format!("{}*X+{}", c[1], c[2]);
            let angle = match fit.angle_code {
                1 => format!("({angle})*180/π"),
                2 => format!("({angle})*200/π"),
                _ => angle,
            };
            format!("{}*SIN({angle})+{}", c[0], c[3])
        }
        FitKind::Logarithmic if c.len() == 2 => format!("{}*LN(X)+{}", c[1], c[0]),
        FitKind::Power if c.len() == 2 => format!("{}*X^{}", c[0], c[1]),
        FitKind::Exponential if c.len() == 2 => format!("{}*EXP({}*X)", c[0], c[1]),
        FitKind::Exponent if c.len() == 2 => format!("{}*{}^X", c[0], c[1]),
        FitKind::Inverse if c.len() == 2 => format!("{}/X+{}", c[1], c[0]),
        _ => c
            .iter()
            .enumerate()
            .rev()
            .map(|(power, c)| match power {
                0 => c.clone(),
                1 => format!("{c}*X"),
                _ => format!("{c}*X^{power}"),
            })
            .collect::<Vec<_>>()
            .join("+"),
    }
}
impl NativeTools {
    pub(super) fn statistics_fit_equation(&self, index: usize) -> String {
        let analysis = &self
            .memory
            .models
            .statistics2_book
            .as_ref()
            .unwrap()
            .analyses[index];
        if analysis.fit == FitKind::UserDefined {
            return analysis.user_expression.clone();
        }
        self.memory
            .models
            .results
            .get(app(true))
            .and_then(|value| value.get(format!("S{}", index + 1)))
            .and_then(|value| {
                serde_json::from_value::<Regression>(value["regression"].clone()).ok()
            })
            .filter(|fit| fit.kind == analysis.fit)
            .map(|fit| equation(&fit))
            .unwrap_or_else(|| default_equation(analysis.fit).into())
    }
    fn paint_statistics_numeric(
        &self,
        g: &mut Grob,
        two: bool,
        cursor: Cursor,
        settings: &ppl_vm::Settings,
    ) {
        for y in 20..203 {
            for x in 0..320 {
                g.set(x, y, if (x + y) % 2 == 0 { 0xb8d5ff } else { 0xffffff });
            }
        }
        g.fill_rect(0, 20, 319, 36, PAPER);
        g.line(0, 36, 319, 36, 0);
        let Some(columns) = self.memory.models.statistics_columns(two) else {
            return;
        };
        let length = columns.0.iter().map(Vec::len).max().unwrap_or(0);
        for visible in 0..4 {
            let col = cursor.first_column + visible;
            if col >= 10 {
                break;
            }
            let left = 32 + visible as i64 * 72;
            let right = (left + 72).min(319);
            let label = Columns::label(col, two);
            text(
                g,
                &label,
                left + (right - left - measure(&label, 14.)) / 2,
                20,
            );
            g.line(left, 20, left, 202, 0);
            for row in cursor.first_row
                ..=(columns.0[col].len().min(model::MAX_ROWS - 1)).min(cursor.first_row + 9)
            {
                let top = 37 + (row - cursor.first_row) as i64 * 17;
                let bottom = top + 16;
                if let Some(&n) = columns.0[col].get(row) {
                    clipped(
                        g,
                        &display(n, settings),
                        [left + 1, top, right - 1, bottom],
                        true,
                    );
                } else {
                    g.fill_rect(left + 1, top, right - 1, bottom, 0xffffff);
                    g.fill_rect(left + 4, top + 3, right - 3, bottom - 3, 0xc2defe);
                }
                g.line(left, bottom, right, bottom, 0x929292);
                if (row, col) == (cursor.row, cursor.column) {
                    outline(g, [left, top - 1, right, bottom]);
                }
            }
        }
        g.line(32, 20, 32, 202, 0);
        for row in cursor.first_row..=(length.min(model::MAX_ROWS - 1)).min(cursor.first_row + 9) {
            let top = 37 + (row - cursor.first_row) as i64 * 17;
            g.fill_rect(0, top, 31, top + 16, PAPER);
            text(g, &(row + 1).to_string(), 0, top);
            g.line(0, top + 16, 31, top + 16, 0x929292);
        }
    }
    fn paint_statistics_symbolic(&self, g: &mut Grob, two: bool, cursor: Cursor) {
        g.fill_rect(0, 20, 319, 219, PAPER);
        for (index, &color) in COLORS.iter().enumerate() {
            let top = 22 + index as i64 * 79 - cursor.symbolic_scroll;
            if top > 202 || top + 75 < 20 {
                continue;
            }
            let enabled = if two {
                self.memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .unwrap()
                    .analyses[index]
                    .enabled
            } else {
                self.memory
                    .models
                    .statistics1_book
                    .as_ref()
                    .unwrap()
                    .analyses[index]
                    .enabled
            };
            let selected = index == cursor.analysis;
            g.fill_rect(
                0,
                top - 1,
                313,
                top + 76,
                if selected { 0xadccff } else { PAPER },
            );
            g.rect(1, top + 3, 14, top + 16, 0xdcdcdc, 0xffffff);
            if enabled {
                tick(g, 1, top + 3);
            }
            text(
                g,
                &format!("{}{}:", if two { 'S' } else { 'H' }, index + 1),
                16,
                top + 1,
            );
            for field in if two {
                &[Field::X, Field::Y, Field::Frequency][..]
            } else {
                &[Field::X, Field::Frequency][..]
            } {
                let value = if two {
                    let a = &self
                        .memory
                        .models
                        .statistics2_book
                        .as_ref()
                        .unwrap()
                        .analyses[index];
                    match field {
                        Field::X => a.x,
                        Field::Y => a.y,
                        _ => a.frequency,
                    }
                } else {
                    let a = &self
                        .memory
                        .models
                        .statistics1_book
                        .as_ref()
                        .unwrap()
                        .analyses[index];
                    if *field == Field::X {
                        a.data
                    } else {
                        a.frequency
                    }
                };
                let bounds = rect(index, *field, cursor.symbolic_scroll, two);
                g.rect(
                    bounds[0], bounds[1], bounds[2], bounds[3], 0xdcdcdc, 0xffffff,
                );
                clipped(
                    g,
                    &value.map_or_else(String::new, |i| Columns::label(i, two)),
                    [bounds[0] + 1, bounds[1] + 1, bounds[2] - 1, bounds[3] - 1],
                    false,
                );
            }
            if two {
                text(g, &format!("Type{}:", index + 1), 0, top + 28);
                let bounds = rect(index, Field::Fit, cursor.symbolic_scroll, two);
                g.rect(
                    bounds[0], bounds[1], bounds[2], bounds[3], 0xdcdcdc, 0xffffff,
                );
                let a = &self
                    .memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .unwrap()
                    .analyses[index];
                text(g, a.fit.label(), 45, top + 28);
                g.fill_rect(253, top + 28, 266, top + 46, 0xc2defe);
                for y in 0..5 {
                    g.line(
                        256 + y / 2,
                        top + 35 + y,
                        262 - y / 2,
                        top + 35 + y,
                        0x8b9ba5,
                    );
                }
                text(g, &format!("Fit{}:", index + 1), 23, top + 55);
                let bounds = rect(index, Field::Expression, cursor.symbolic_scroll, two);
                g.rect(
                    bounds[0], bounds[1], bounds[2], bounds[3], 0xdcdcdc, 0xffffff,
                );
                clipped(
                    g,
                    &self.statistics_fit_equation(index),
                    [bounds[0] + 1, bounds[1] + 1, bounds[2] - 1, bounds[3] - 1],
                    false,
                );
                g.rect(1, top + 57, 19, top + 74, 0xffffff, color);
            } else {
                text(g, &format!("Plot{}:", index + 1), 0, top + 28);
                g.rect(40, top + 27, 313, top + 47, 0xdcdcdc, 0xffffff);
                text(g, "Histogram", 42, top + 28);
                text(g, &format!("Option{}:", index + 1), 23, top + 55);
                g.fill_rect(80, top + 54, 313, top + 75, 0xdfdfdf);
                g.rect(1, top + 57, 19, top + 74, 0xffffff, color);
            }
            if selected {
                let bounds = rect(index, cursor.field, cursor.symbolic_scroll, two);
                outline(
                    g,
                    [bounds[0] - 1, bounds[1] - 1, bounds[2] + 1, bounds[3] + 1],
                );
            }
        }
        // The form scrolls under the fixed header and entry strip.
        chrome::header(g, &format!("{} Symbolic View", app(two)));
    }
    fn paint_statistics_report(&self, g: &mut Grob, two: bool, settings: &ppl_vm::Settings) {
        g.fill_rect(0, 20, 319, 219, 0xffffff);
        let Some(result) = self
            .memory
            .models
            .results
            .get(app(two))
            .and_then(|v| v.as_object())
        else {
            text(
                g,
                if self.statistics_panel.busy {
                    "Calculating…"
                } else {
                    "Press Stats to calculate selected analyses"
                },
                0,
                40,
            );
            return;
        };
        let columns = result
            .iter()
            .filter(|(name, _)| name.starts_with(if two { 'S' } else { 'H' }))
            .skip(self.statistics_panel.report_first)
            .take(3)
            .collect::<Vec<_>>();
        if columns.is_empty() {
            text(g, "No selected analysis in this view", 0, 40);
            return;
        }
        let rows: &[(&str, &str)] = if two {
            &[
                ("n", "summary.count"),
                ("r", "summary.correlation"),
                ("ΣXY", "summary.sum_products"),
                ("covXY", "summary.population_covariance"),
                ("sXY", "summary.sample_covariance"),
                ("x̄", "summary.x.mean"),
                ("ȳ", "summary.y.mean"),
                ("ΣX", "summary.x.sum"),
                ("ΣY", "summary.y.sum"),
                ("ΣX²", "summary.x.sum_squares"),
                ("ΣY²", "summary.y.sum_squares"),
                ("SSE", "regression.residual_sum_squares"),
            ]
        } else {
            &[
                ("n", "count"),
                ("Min", "minimum"),
                ("Q1", "quartile1"),
                ("Med", "median"),
                ("Q3", "quartile3"),
                ("Max", "maximum"),
                ("ΣX", "sum"),
                ("ΣX²", "sum_squares"),
                ("x̄", "mean"),
                ("sX", "sample_standard_deviation"),
                ("σX", "population_standard_deviation"),
                ("serrX", "standard_error"),
                ("ssX", "sum_squared_deviations"),
            ]
        };
        g.fill_rect(0, 20, 319, 36, PAPER);
        g.line(0, 36, 319, 36, 0);
        for (column, (label, value)) in columns.iter().enumerate() {
            let left = 65 + (254 * column / columns.len()) as i64;
            let right = 65 + (254 * (column + 1) / columns.len()) as i64;
            text(
                g,
                label,
                left + (right - left - measure(label, 14.)) / 2,
                20,
            );
            g.line(left, 20, left, 202, 0);
            for (row, (label, path)) in rows
                .iter()
                .skip(self.statistics_panel.report_row.min(rows.len() - 1))
                .take(10)
                .enumerate()
            {
                let top = 37 + row as i64 * 17;
                g.fill_rect(0, top, 64, top + 16, PAPER);
                text(g, label, 0, top);
                let v = path.split('.').try_fold(*value, |v, key| v.get(key));
                let label = v.map_or_else(
                    || "—".into(),
                    |v| {
                        if let Some(n) = v.as_u64() {
                            n.to_string()
                        } else if let Some(n) = v.as_f64() {
                            display(n, settings)
                        } else {
                            "—".into()
                        }
                    },
                );
                clipped(g, &label, [left + 1, top, right - 1, top + 15], false);
                g.line(0, top + 16, 319, top + 16, 0xdcdcdc);
            }
        }
    }
    pub(crate) fn show_statistics(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        let Some(two) = mode(&self.memory.selected_app) else {
            return;
        };
        if let Err(error) = self.memory.models.ensure_statistics_book(two) {
            self.statistics_sync_error(Some(error));
        }
        let index = usize::from(two);
        let cursor = self.memory.statistics_cursor[index];
        let numeric = self.page == Some(Page::Numeric);
        let mut settings = snapshot.settings.clone();
        if let Some(app) = snapshot.app_settings.get(app(two)) {
            settings = app.effective(&settings);
        }
        let mut image = Grob::new(320, 240, PAPER);
        chrome::header(
            &mut image,
            &format!(
                "{} {} View",
                app(two),
                if numeric { "Numeric" } else { "Symbolic" }
            ),
        );
        if self.statistics_panel.sync_error.is_some() {
            text(&mut image, "Statistics data unavailable", 5, 35);
        } else if let Some(source) = &self.statistics_panel.equation {
            image.fill_rect(0, 20, 319, 219, 0xffffff);
            let layout = crate::ide::native::math_layout::expression(source, 17.);
            let mut body = Grob::new(320, 183, 0xffffff);
            layout.paint_static(
                &mut body,
                10 - self.statistics_panel.equation_x,
                6 - self.statistics_panel.equation_y,
                0,
            );
            image.blit((0, 20, 320, 203), &body, (0, 0, 320, 183), None, 255);
        } else if self.statistics_panel.report {
            self.paint_statistics_report(&mut image, two, &settings);
        } else if numeric {
            self.paint_statistics_numeric(&mut image, two, cursor, &settings);
        } else {
            self.paint_statistics_symbolic(&mut image, two, cursor);
        }
        image.fill_rect(0, 203, 319, 219, if numeric { 0xffffff } else { PAPER });
        let cell_source = self.statistics_cell_source(two, cursor.column, cursor.row, false);
        let help = if self.statistics_panel.busy {
            "Calculating…"
        } else if self.statistics_panel.report {
            "Computed statistics"
        } else if numeric {
            &cell_source
        } else {
            match cursor.field {
                Field::X => "Enter independent column",
                Field::Y => "Enter dependent column",
                Field::Frequency => "Enter frequency column",
                Field::Fit => "Choose fit type",
                Field::Expression => "Enter fit expression",
                Field::Enabled => "Select statistical analysis",
            }
        };
        text(
            &mut image,
            self.statistics_panel
                .sync_error
                .as_deref()
                .or(self.error.as_deref())
                .unwrap_or(help),
            0,
            203,
        );
        let labels = if self.statistics_panel.report
            || self.statistics_panel.equation.is_some()
            || self.statistics_panel.sync_error.is_some()
        {
            ["", "", "", "", "", "OK"]
        } else if numeric {
            ["Edit", "More", "Go To", "Sort", "Make", "Stats"]
        } else {
            [
                if cursor.field == Field::Fit {
                    "Choose"
                } else {
                    "Edit"
                },
                "✓",
                "Column",
                if two { "Fit" } else { "" },
                "Show",
                "Eval",
            ]
        };
        chrome::softkeys_with_menus(
            &mut image,
            &labels,
            if labels[1] == "More" {
                &[1]
            } else if labels[2] == "Column" {
                &[2]
            } else {
                &[]
            },
        );
        if let Some(menu) = self.statistics_panel.menu {
            let items = self.statistics_menu_items(two, menu.kind);
            image.rect(44, 23, 275, 214, 0, 0xffffff);
            for (row, (item, label)) in items
                .iter()
                .enumerate()
                .skip(menu.first)
                .take(9)
                .enumerate()
            {
                let top = 24 + row as i64 * 21;
                if item == menu.index {
                    image.fill_rect(45, top, 260, top + 20, 0xadccff);
                }
                text(&mut image, label, 49, top + 1);
                image.line(45, top + 20, 260, top + 20, 0xe0e0e0);
            }
            image.fill_rect(261, 24, 274, 213, PAPER);
            text(&mut image, "↑", 262, 24);
            text(&mut image, "↓", 262, 195);
        }
        let changed = self.statistics_panel.pixels != image.pixels();
        if changed {
            self.statistics_panel.pixels = image.pixels().to_vec();
            let pixels = image
                .pixels()
                .iter()
                .map(|&v| egui::Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
                .collect();
            let image = egui::ColorImage::new([320, 240], pixels);
            if let Some(texture) = &mut self.statistics_panel.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.statistics_panel.texture = Some(ctx.load_texture(
                    "native_statistics",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        egui::Area::new("native_statistics_panel".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                if let Some(texture) = &self.statistics_panel.texture {
                    ui.painter().image(
                        texture.id(),
                        lcd,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                }
                let response = ui.interact(
                    lcd,
                    ui.id().with("statistics_surface"),
                    egui::Sense::click_and_drag(),
                );
                if response.hovered() {
                    let scroll = ctx.input(|input| input.smooth_scroll_delta);
                    if scroll.y.abs() > 1. {
                        if let Some(mut menu) = self.statistics_panel.menu {
                            let max = self
                                .statistics_menu_items(two, menu.kind)
                                .len()
                                .saturating_sub(9);
                            menu.first = (menu.first as i64 + if scroll.y < 0. { 1 } else { -1 })
                                .clamp(0, max as i64)
                                as usize;
                            self.statistics_panel.menu = Some(menu);
                        } else if self.statistics_panel.report {
                            self.statistics_panel.report_row =
                                (self.statistics_panel.report_row as i64
                                    + if scroll.y < 0. { 1 } else { -1 })
                                .clamp(0, 30) as usize;
                        } else if numeric {
                            let max = self
                                .memory
                                .models
                                .statistics_columns(two)
                                .map_or(0, |cols| cols.0.iter().map(Vec::len).max().unwrap_or(0));
                            let c = &mut self.memory.statistics_cursor[index];
                            c.first_row = (c.first_row as i64 + if scroll.y < 0. { 1 } else { -1 })
                                .clamp(0, max.min(model::MAX_ROWS - 1) as i64)
                                as usize;
                        } else {
                            let c = &mut self.memory.statistics_cursor[index];
                            c.symbolic_scroll = (c.symbolic_scroll
                                + if scroll.y < 0. { 17 } else { -17 })
                            .clamp(0, 210);
                        }
                        self.dirty();
                        ctx.request_repaint();
                    }
                }
                if response.clicked()
                    && let Some(position) = response.interact_pointer_pos()
                {
                    let point = (position - lcd.min) / scale;
                    let p = (point.x as i64, point.y as i64);
                    self.statistics_pointer(two, p, numeric, requests);
                    ctx.request_repaint();
                }
            });
        if changed {
            self.dirty();
        }
    }
    fn statistics_pointer(
        &mut self,
        two: bool,
        p: (i64, i64),
        numeric: bool,
        requests: &mut Vec<Request>,
    ) {
        let index = usize::from(two);
        let mut cursor = self.memory.statistics_cursor[index];
        if self.statistics_panel.sync_error.is_some() {
            if p.1 >= 220 && chrome::softkey_index(p.0) == 5 {
                requests.push(Request::Close);
            }
            return;
        }
        if let Some(mut menu) = self.statistics_panel.menu {
            let count = self.statistics_menu_items(two, menu.kind).len();
            if contains([261, 24, 274, 213], p) {
                menu.first = (menu.first as i64 + if p.1 < 100 { -1 } else { 1 })
                    .clamp(0, count.saturating_sub(9) as i64) as usize;
                self.statistics_panel.menu = Some(menu);
            } else {
                if contains([45, 24, 260, 213], p) {
                    menu.index = menu.first + ((p.1 - 24) / 21) as usize;
                    if menu.index < count {
                        requests.extend(self.statistics_choose(two, cursor, menu));
                    }
                }
                self.statistics_panel.menu = None;
            }
            self.dirty();
            return;
        }
        if p.1 >= 220 {
            let key = chrome::softkey_index(p.0);
            if self.statistics_panel.report || self.statistics_panel.equation.is_some() {
                if key == 5 {
                    self.statistics_panel.report = false;
                    self.statistics_panel.equation = None;
                }
                self.dirty();
                return;
            }
            if self.statistics_panel.busy {
                return;
            }
            if numeric {
                match key {
                    0 => requests.extend(self.statistics_activate(two, cursor)),
                    1 => {
                        self.statistics_panel.menu = Some(Menu {
                            kind: MenuKind::More,
                            index: 0,
                            first: 0,
                        })
                    }
                    2 => requests.push(Request::EditMath(MathTarget::StatisticsGoto { two })),
                    3 => {
                        self.statistics_panel.menu = Some(Menu {
                            kind: MenuKind::Sort,
                            index: 0,
                            first: 0,
                        })
                    }
                    4 => requests.push(Request::EditMath(MathTarget::StatisticsColumn {
                        two,
                        column: cursor.column,
                    })),
                    5 => {
                        self.statistics_panel.report = true;
                        self.statistics_panel.report_row = 0;
                        self.statistics_panel.report_first = 0;
                        if let Some(calculation) =
                            crate::ide::native_app_ui::numeric(app(two), &self.memory.models)
                        {
                            requests.push(Request::Calculate(calculation));
                        }
                    }
                    _ => {}
                }
            } else {
                match key {
                    0 => requests.extend(self.statistics_activate(two, cursor)),
                    2 => {
                        if !matches!(cursor.field, Field::X | Field::Y | Field::Frequency) {
                            cursor.field = Field::X;
                        }
                        requests.extend(self.statistics_activate(two, cursor));
                    }
                    1 => {
                        let previous = cursor.field;
                        cursor.field = Field::Enabled;
                        requests.extend(self.statistics_activate(two, cursor));
                        cursor.field = previous;
                    }
                    3 if two => {
                        if let Some(calculation) =
                            crate::ide::native_app_ui::numeric(app(two), &self.memory.models)
                        {
                            requests.push(Request::Calculate(calculation));
                        }
                    }
                    4 => self.statistics_show_field(two, cursor, false),
                    5 => self.statistics_show_field(two, cursor, true),
                    _ => {}
                }
            }
        } else if !self.statistics_panel.report
            && self.statistics_panel.equation.is_none()
            && self.statistics_panel.sync_error.is_none()
            && !self.statistics_panel.busy
        {
            if numeric && contains([32, 37, 319, 202], p) {
                let col = cursor.first_column + ((p.0 - 32) / 72) as usize;
                let row = cursor.first_row + ((p.1 - 37) / 17) as usize;
                let max = self.memory.models.statistics_columns(two).unwrap().0[col.min(9)].len();
                if col < 10 && row <= max && row < model::MAX_ROWS {
                    let same = (cursor.column, cursor.row) == (col, row);
                    cursor.column = col;
                    cursor.row = row;
                    self.memory.statistics_cursor[index] = cursor;
                    if same {
                        requests.push(Request::EditMath(self.statistics_entry_target(two, false)));
                    }
                }
            } else if !numeric {
                for analysis in 0..5 {
                    for &field in Self::statistics_fields(two) {
                        if contains(rect(analysis, field, cursor.symbolic_scroll, two), p) {
                            let same = (cursor.analysis, cursor.field) == (analysis, field);
                            cursor.analysis = analysis;
                            cursor.field = field;
                            if same
                                || field == Field::Enabled
                                || (field == Field::Fit && p.0 >= 253)
                            {
                                requests.extend(self.statistics_activate(two, cursor));
                            }
                        }
                    }
                }
            }
        }
        self.memory.statistics_cursor[index] = cursor;
        self.dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shown_trigonometric_fit_reevaluates_with_the_app_angle() {
        for code in 0..=2 {
            let fit = Regression {
                kind: FitKind::Trigonometric,
                coefficients: vec![2., 0.7, 0.3, 1.],
                count: 8,
                residual_sum_squares: 0.,
                user_expression: None,
                angle_code: code,
            };
            let source = equation(&fit).replace("X", "(30)");
            let mut context = ppl_vm::SessionContext::default();
            context.settings.angle = ppl_math::AngleMode::from_code(code).unwrap();
            let out = context
                .evaluate_home(
                    &source,
                    Box::new(ppl_vm::HeadlessHost::default()),
                    ppl_vm::Limits::default(),
                )
                .result
                .unwrap();
            let actual = out.as_real().unwrap().to_f64();
            assert!(
                (actual - fit.predict(30.).unwrap()).abs() < 1e-9,
                "{source}: {actual}"
            );
        }
    }
}
