use super::*;
use crate::ide::native::{chrome, math_layout};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
const PAPER: u32 = 0xe8f1f6;
const BLUE: u32 = 0xadccff;
fn text(g: &mut Grob, s: &str, x: i64, y: i64, pixels: i64, width: Option<i64>) {
    draw_text(g, s, x, y, pixels as f32, 0, width, None);
}
fn numeric_display(value: &PortableValue, settings: &ppl_vm::Settings) -> String {
    if matches!(value,PortableValue::Complex(c) if !c.im.is_zero()) {
        return "NaN".into();
    }
    let mut format = settings.home_format();
    format.exp_char = 'E';
    match value {
        PortableValue::Integer(word) => word.to_real().format(format),
        _ => value.to_value().display(format),
    }
}
fn plot_numeric_values(
    window: &crate::ide::native_apps::PlotWindow,
) -> Result<(Real, Real), String> {
    window.validate()?;
    // Subtract decimal bounds so finite f64 endpoints may span beyond f64.
    let min = Real::from_f64(window.xmin).map_err(|e| e.to_string())?;
    let max = Real::from_f64(window.xmax).map_err(|e| e.to_string())?;
    let step = max
        .sub(min)
        .and_then(|span| span.div(Real::from_i64(318)))
        .map_err(|e| e.to_string())?;
    Ok((min, step))
}
impl NativeTools {
    pub(super) fn numeric_accept_plot(&mut self, snapshot: &SessionSnapshot) {
        self.numeric_panel.plot_confirmation = false;
        if self.numeric_panel.busy {
            self.error = Some("Calculator is busy".into());
            return;
        }
        let graph = &self.memory.graphs["Function"];
        let settings = (|| {
            let mut next = self.memory.function_numeric.settings.clone();
            let (start, step) = plot_numeric_values(&graph.window)?;
            next.start = start;
            next.step = step;
            next.validate()?;
            Ok::<_, String>(next)
        })();
        match settings {
            Ok(settings) => {
                self.memory.function_numeric.settings = settings;
                self.memory.function_numeric.row = 0;
            }
            Err(error) => {
                self.error = Some(error);
                return;
            }
        }
        self.function_numeric_changed(snapshot);
    }
    fn numeric_swap_accept(&mut self, snapshot: &SessionSnapshot) {
        let Some([a, b]) = self.numeric_panel.swap_rows else {
            return;
        };
        let state = &mut self.memory.function_numeric;
        if a == 0
            || b == 0
            || a > state.settings.independent.len()
            || b > state.settings.independent.len()
        {
            self.error = Some(format!(
                "Choose row1 through{}",
                state.settings.independent.len()
            ));
            return;
        }
        state.settings.independent.swap(a - 1, b - 1);
        self.numeric_panel.swap_rows = None;
        self.function_numeric_changed(snapshot);
    }
    fn numeric_swap_paint(&self, g: &mut Grob) {
        let Some(rows) = self.numeric_panel.swap_rows else {
            return;
        };
        g.fill(PAPER);
        chrome::header(g, "Swap Rows");
        for (i, y) in [49, 75].into_iter().enumerate() {
            let label = if i == 0 { "Row 1:" } else { "Row 2:" };
            text(g, label, 90 - measure(label, 14.), y + 3, 14, None);
            g.rect(
                92,
                y,
                314,
                y + 22,
                if self.numeric_panel.swap_focus == i {
                    0
                } else {
                    0xe1e1e1
                },
                0xffffff,
            );
            text(g, &rows[i].to_string(), 94, y + 3, 14, None);
        }
        text(
            g,
            self.error.as_deref().unwrap_or("Enter row number"),
            0,
            203,
            14,
            Some(319),
        );
        chrome::softkeys(g, &["Edit", "", "", "", "Cancel", "OK"]);
    }
    fn numeric_columns(&self) -> Vec<(usize, i64, i64)> {
        let count = self.function_numeric_count();
        let mut columns = vec![];
        if count > 0 {
            let minimum = 256usize.div_ceil(count).max(59);
            let scroll = self.memory.function_numeric.horizontal_scroll as i64;
            for i in 0..count {
                let left = 65
                    + if minimum == 59 {
                        59 * i
                    } else {
                        (256 * i).div_ceil(count)
                    };
                let right = 64
                    + if minimum == 59 {
                        59 * (i + 1)
                    } else {
                        (256 * (i + 1)).div_ceil(count)
                    };
                let (x, r) = (left as i64 - scroll, right as i64 - scroll);
                if r >= 65 && x < 320 {
                    columns.push((i + 1, x, r));
                }
            }
        }
        columns.push((0, 0, 64));
        columns
    }
    fn numeric_menu_rect(&self, kind: MenuKind) -> (i64, i64, i64, i64) {
        let count = self.function_numeric_items(kind).len() as i64;
        match kind {
            MenuKind::Type => (92, 153, 222, 0),
            MenuKind::Select => (105, 154.min(219 - count * 21), 120, 0),
            MenuKind::Font => (105, 219 - count * 21, 111, 0),
            MenuKind::Delete => (105, 112, 96, 0),
            MenuKind::More => (
                if self.memory.function_numeric.settings.kind == 1 {
                    24
                } else {
                    27
                },
                219 - (count + 1) * 21,
                78,
                21,
            ),
            MenuKind::Sort => (159, 219 - (count + 1) * 21, 130, 21),
            MenuKind::Zoom => (
                0,
                219 - (count + 1) * 21,
                if self.numeric_panel.undo.is_some() {
                    77
                } else {
                    65
                },
                21,
            ),
        }
    }
    fn numeric_menu_paint(&self, g: &mut Grob) {
        let Some(menu) = &self.numeric_panel.menu else {
            return;
        };
        let items = self.function_numeric_items(menu.kind);
        let (x, y, w, head) = self.numeric_menu_rect(menu.kind);
        g.rect(x, y, x + w, y + head + items.len() as i64 * 21, 0, 0xffffff);
        if head > 0 {
            let mut header = Grob::new((w - 1) as usize, 20, 0xffffff);
            chrome::header(&mut header, "");
            let label = match menu.kind {
                MenuKind::Zoom => "Zoom",
                MenuKind::Sort => "Sort",
                _ => "More",
            };
            draw_text(
                &mut header,
                label,
                ((w - 1 - measure(label, 14.)) / 2).max(1),
                1,
                14.,
                0xebebeb,
                None,
                None,
            );
            g.blit(
                (x + 1, y + 1, x + w, y + 21),
                &header,
                (0, 0, w - 1, 20),
                None,
                255,
            );
        }
        for (i, s) in items.iter().enumerate() {
            let top = y + head + i as i64 * 21;
            if i == menu.index {
                g.fill_rect(x + 1, top + 1, x + w - 1, top + 20, BLUE);
            }
            let checked = match menu.kind {
                MenuKind::Font => self.memory.function_numeric.font == i,
                MenuKind::Type => usize::from(self.memory.function_numeric.settings.kind == 1) == i,
                _ => false,
            };
            if checked {
                text(g, "•", x + 2, top + 2, 14, None);
            } else if menu.kind != MenuKind::Type {
                text(g, &(i + 1).to_string(), x + 2, top + 3, 10, None);
            }
            if menu.kind == MenuKind::Zoom && i < 2 {
                text(
                    g,
                    if i == 0 { "In" } else { "Out" },
                    x + 12,
                    top + 2,
                    14,
                    None,
                );
                let factor = format!("×{}", self.memory.function_numeric.settings.zoom);
                text(
                    g,
                    &factor,
                    x + w - measure(&factor, 14.) - 2,
                    top + 2,
                    14,
                    None,
                );
            } else {
                text(g, s, x + 12, top + 2, 14, Some(w - 14));
            }
        }
    }
    fn numeric_setup_paint(&self, g: &mut Grob) {
        let state = &self.memory.function_numeric;
        chrome::header(g, "Function Num Setup");
        for (i, (label, y, field)) in [
            ("Num Start:", 24, Field::Start),
            ("Num Step:", 50, Field::Step),
            ("Num Zoom:", 102, Field::Zoom),
            ("Num Type:", 154, Field::Value),
        ]
        .into_iter()
        .enumerate()
        {
            text(g, label, 90 - measure(label, 14.), y + 3, 14, None);
            g.rect(
                92,
                y,
                314,
                y + 21,
                if state.focus == i { 0 } else { 0xe1e1e1 },
                0xffffff,
            );
            let source = if i == 3 {
                if state.settings.kind == 1 {
                    "BuildYourOwn".into()
                } else {
                    "Automatic".into()
                }
            } else {
                self.function_numeric_source(field)
            };
            text(g, &source, 94, y + 3, 14, Some(218));
            if i == 3 {
                g.fill_rect(299, y + 1, 313, y + 20, 0xc5dcfc);
                for row in 0..4 {
                    g.line(303 + row, y + 8 + row, 309 - row, y + 8 + row, 0x888888);
                }
            }
        }
        text(
            g,
            self.error.as_deref().unwrap_or(
                [
                    "Enter table start value",
                    "Enter table step value",
                    "Enter zoom factor",
                    "Choose table type",
                ][state.focus],
            ),
            0,
            203,
            14,
            Some(319),
        );
        chrome::softkeys(
            g,
            &[
                if state.focus == 3 { "Choose" } else { "Edit" },
                "",
                "",
                "",
                "Plot→",
                "",
            ],
        );
    }
    fn numeric_grid_paint(&self, g: &mut Grob, snapshot: &SessionSnapshot) {
        let state = &self.memory.function_numeric;
        let labels = self.function_numeric_labels();
        let columns = self.numeric_columns();
        let pixels = state.pixels();
        let height = state.line() as i64;
        let start = state.header() as i64;
        let settings = snapshot
            .app_settings
            .get("Function")
            .cloned()
            .unwrap_or_default()
            .effective(&snapshot.settings);
        chrome::header(g, "Function Numeric View");
        if self.function_numeric_count() == 0 {
            for y in 20..203 {
                for x in 65..320 {
                    g.set(x, y, if (x + y) % 3 == 0 { 0xc5dcfc } else { BLUE });
                }
            }
        }
        for &(col, x, r) in &columns {
            g.fill_rect(x, 20, r, start - 2, PAPER);
            g.line(r, 20, r, start - 1, 0);
            g.line(x, start - 1, r, start - 1, 0);
            let label = &labels[col];
            text(
                g,
                label,
                (x + r - measure(label, pixels as f32)) / 2,
                21,
                pixels,
                None,
            );
        }
        let selected = state.selected_row();
        let anchor = state.anchor.unwrap_or([state.column, selected]);
        let (left, right) = (anchor[0].min(state.column), anchor[0].max(state.column));
        let (top, bottom) = (anchor[1].min(selected), anchor[1].max(selected));
        for row in 0..state.visible() {
            let index = if state.settings.kind == 1 {
                state.first + row
            } else {
                row
            };
            let y = start + row as i64 * height;
            for &(col, x, r) in &columns {
                let available =
                    state.settings.kind != 1 || index < state.settings.independent.len();
                let selected_cell =
                    (left..=right).contains(&col) && (top..=bottom).contains(&index);
                let color = if selected_cell {
                    BLUE
                } else if self.function_numeric_count() == 0 && col == 0 {
                    PAPER
                } else {
                    0xffffff
                };
                let mut cell = Grob::new((r - x + 1) as usize, height as usize, color);
                if !available {
                    cell.fill(BLUE);
                    for py in 0..height {
                        for px in 0..=r - x {
                            if (px + py) % 3 == 0 {
                                cell.set(px, py, 0xc5dcfc);
                            }
                        }
                    }
                    if selected_cell {
                        cell.fill_rect(1, 2, r - x - 1, height - 3, BLUE);
                    }
                } else {
                    let value = self
                        .numeric_panel
                        .table_values
                        .get(index)
                        .and_then(|row| row.get(col))
                        .and_then(Option::as_ref);
                    let s = value.map_or_else(
                        || {
                            if self.numeric_panel.table_job.is_some() {
                                String::new()
                            } else {
                                "NaN".into()
                            }
                        },
                        |value| numeric_display(value, &settings),
                    );
                    text(&mut cell, &s, 1, 0, pixels, Some(r - x - 1));
                }
                cell.line(0, height - 1, r - x, height - 1, 0xe1e1e1);
                cell.line(r - x, 0, r - x, height - 1, 0);
                if selected_cell && state.selecting {
                    cell.line(0, 0, r - x, 0, 0);
                    cell.line(0, 0, 0, height - 1, 0);
                    cell.line(0, height - 1, r - x, height - 1, 0);
                    cell.line(r - x, 0, r - x, height - 1, 0);
                }
                g.blit(
                    (x, y, r + 1, y + height),
                    &cell,
                    (0, 0, r - x + 1, height),
                    None,
                    255,
                );
            }
        }
        g.fill_rect(0, 203, 319, 219, 0xffffff);
        let caption = if state.definition {
            if state.column == 0 {
                "Independent Variable X".into()
            } else {
                let index = self.memory.graphs["Function"]
                    .curves
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.enabled && !c.expression.trim().is_empty())
                    .nth(state.column - 1)
                    .map(|(i, _)| i);
                index.map_or_else(String::new, |i| {
                    format!(
                        "F{}(X)={}",
                        if i == 9 { 0 } else { i + 1 },
                        self.memory.graphs["Function"].curves[i].expression
                    )
                })
            }
        } else {
            self.numeric_panel
                .table_values
                .get(selected)
                .and_then(|row| row.get(state.column))
                .map_or_else(String::new, |value| {
                    value
                        .as_ref()
                        .map_or_else(|| "NaN".into(), |v| numeric_display(v, &settings))
                })
        };
        text(
            g,
            self.error.as_deref().unwrap_or(&caption),
            0,
            203,
            14,
            Some(319),
        );
        chrome::softkeys_with_menus(
            g,
            &[
                if state.settings.kind == 1 {
                    "Edit"
                } else {
                    "Zoom"
                },
                "More",
                if state.settings.kind == 1 {
                    ""
                } else {
                    "Go To"
                },
                if state.settings.kind == 1 { "Sort" } else { "" },
                if state.definition { "Defn•" } else { "Defn" },
                "",
            ],
            if state.settings.kind == 1 {
                &[1]
            } else {
                &[0, 1]
            },
        );
    }
    fn numeric_editor_paint(&mut self, g: &mut Grob) {
        let Some(editor) = &self.numeric_panel.editor else {
            return;
        };
        let Some(field) = self.numeric_panel.field else {
            return;
        };
        let layout = math_layout::linear(editor);
        let caret = layout.cursor(editor.cursor()).0;
        let swap = matches!(field, Field::SwapFirst | Field::SwapSecond);
        let form = field == Field::Goto;
        let setup = self.page == Some(Page::NumericSetup);
        let width = if form || setup || swap { 218 } else { 320 };
        let scroll = self.numeric_panel.scroll;
        self.numeric_panel.scroll = if caret < scroll {
            caret
        } else if caret > scroll + width - 9 {
            caret - width + 9
        } else {
            scroll
        };
        let mut entry = Grob::new(width as usize, 20, 0xffffff);
        layout.paint(
            &mut entry,
            -self.numeric_panel.scroll,
            0,
            editor.selection(),
            editor.cursor(),
        );
        if swap {
            let y = if field == Field::SwapFirst { 49 } else { 75 };
            g.rect(92, y, 314, y + 22, 0, 0xffffff);
            g.blit((94, y + 1, 312, y + 21), &entry, (0, 0, 218, 20), None, 255);
        } else if form {
            g.fill(PAPER);
            chrome::header(g, "Go To");
            text(g, "X:", 90 - measure("X:", 14.), 52, 14, None);
            g.rect(92, 49, 314, 71, 0, 0xffffff);
            g.blit((94, 50, 312, 70), &entry, (0, 0, 218, 20), None, 255);
            text(
                g,
                self.error
                    .as_deref()
                    .unwrap_or("Enter the value to which to jump"),
                0,
                203,
                14,
                Some(319),
            );
        } else if setup {
            let y = match field {
                Field::Start => 24,
                Field::Step => 50,
                _ => 102,
            };
            g.rect(92, y, 314, y + 21, 0, 0xffffff);
            g.blit((94, y + 1, 312, y + 21), &entry, (0, 0, 218, 20), None, 255);
        } else {
            g.fill_rect(0, 203, 319, 219, 0xffffff);
            g.blit((0, 203, 320, 220), &entry, (0, 0, 320, 17), None, 255);
        }
        chrome::softkeys(
            g,
            &[
                if form || setup || swap { "Edit" } else { "" },
                "",
                "",
                "",
                "Cancel",
                "OK",
            ],
        );
        if self.numeric_panel.job.is_some() {
            text(g, "Calculating…", 0, 203, 14, Some(319));
        }
    }
    fn numeric_click(
        &mut self,
        x: i64,
        y: i64,
        snapshot: &SessionSnapshot,
        out: &mut Vec<Request>,
    ) {
        if self.numeric_panel.plot_confirmation {
            if y >= 220 && x >= 212 {
                self.numeric_panel.plot_confirmation = false;
                if x >= 266 {
                    self.numeric_accept_plot(snapshot);
                }
                self.dirty();
            }
            return;
        }
        if let Some(editor) = &self.numeric_panel.editor {
            if y >= 220 {
                if x >= 212 {
                    out.push(Request::FinishMath(x >= 266));
                }
                return;
            }
            let field = self.numeric_panel.field.unwrap();
            let (left, top) = if matches!(field, Field::SwapFirst | Field::SwapSecond) {
                (94, if field == Field::SwapFirst { 50 } else { 76 })
            } else if field == Field::Goto {
                (94, 50)
            } else if self.page == Some(Page::NumericSetup) {
                (
                    94,
                    match field {
                        Field::Start => 25,
                        Field::Step => 51,
                        _ => 103,
                    },
                )
            } else {
                (0, 203)
            };
            if y >= top && y < top + 21 && x >= left {
                let layout = math_layout::linear(editor);
                out.push(Request::MathCursor {
                    position: layout.hit_test(x - left + self.numeric_panel.scroll, y - top),
                    extend: false,
                });
            }
            return;
        }
        if self.numeric_panel.swap_rows.is_some() {
            if y >= 220 {
                match x / 53 {
                    0 => out.push(Request::EditMath(MathTarget::FunctionNumeric {
                        field: if self.numeric_panel.swap_focus == 0 {
                            Field::SwapFirst
                        } else {
                            Field::SwapSecond
                        },
                        clear: false,
                    })),
                    4 => self.numeric_panel.swap_rows = None,
                    5 => self.numeric_swap_accept(snapshot),
                    _ => {}
                }
            } else if (92..=314).contains(&x) {
                if (49..=71).contains(&y) {
                    self.numeric_panel.swap_focus = 0;
                } else if (75..=97).contains(&y) {
                    self.numeric_panel.swap_focus = 1;
                }
            }
            self.dirty();
            return;
        }
        if let Some(menu) = &self.numeric_panel.menu {
            let kind = menu.kind;
            let (mx, my, w, head) = self.numeric_menu_rect(kind);
            let count = self.function_numeric_items(kind).len();
            if x >= mx && x <= mx + w && y >= my + head && y < my + head + count as i64 * 21 {
                self.function_numeric_choose(kind, ((y - my - head) / 21) as usize, snapshot, out);
            } else {
                self.numeric_panel.menu = None;
            }
            self.dirty();
            return;
        }
        if self.page == Some(Page::NumericSetup) {
            if y >= 220 {
                match x / 53 {
                    0 => {
                        let _ = self
                            .function_numeric_action(A::Evaluate, snapshot)
                            .map(|v| out.extend(v));
                    }
                    4 => {
                        self.numeric_panel.plot_confirmation = true;
                        self.dirty();
                    }
                    _ => {}
                }
                return;
            }
            for (i, top) in [24, 50, 102, 154].into_iter().enumerate() {
                if (92..=314).contains(&x) && y >= top && y <= top + 21 {
                    let previous = self.memory.function_numeric.focus;
                    self.memory.function_numeric.focus = i;
                    if i == 3 {
                        self.function_numeric_menu(MenuKind::Type);
                    } else if previous == i {
                        out.push(Request::EditMath(MathTarget::FunctionNumeric {
                            field: [Field::Start, Field::Step, Field::Zoom][i],
                            clear: false,
                        }));
                    }
                    self.dirty();
                    break;
                }
            }
            return;
        }
        if y >= 220 {
            match x / 53 {
                0 => {
                    if self.memory.function_numeric.settings.kind == 1 {
                        out.push(Request::EditMath(MathTarget::FunctionNumeric {
                            field: Field::Value,
                            clear: false,
                        }));
                    } else {
                        self.function_numeric_menu(MenuKind::Zoom);
                    }
                }
                1 => self.function_numeric_menu(MenuKind::More),
                2 if self.memory.function_numeric.settings.kind != 1 => {
                    out.push(Request::EditMath(MathTarget::FunctionNumeric {
                        field: Field::Goto,
                        clear: false,
                    }))
                }
                3 if self.memory.function_numeric.settings.kind == 1 => {
                    self.function_numeric_menu(MenuKind::Sort)
                }
                4 => {
                    self.memory.function_numeric.definition =
                        !self.memory.function_numeric.definition;
                    self.dirty();
                }
                _ => {}
            }
            return;
        }
        self.numeric_point(x, y, false);
    }
    fn numeric_point(&mut self, x: i64, y: i64, drag: bool) {
        let state = &self.memory.function_numeric;
        if y < state.header() as i64 || y >= 203 {
            return;
        }
        let col = if x < 65 {
            0
        } else {
            self.numeric_columns()
                .iter()
                .find(|(_, left, right)| x >= *left && x <= *right)
                .map(|(c, _, _)| *c)
                .unwrap_or(0)
        };
        let state = &mut self.memory.function_numeric;
        if (state.selecting || drag) && state.anchor.is_none() {
            state.anchor = Some([state.column, state.selected_row()]);
        }
        if !state.selecting && !drag {
            state.anchor = None;
            state.headers = false;
        }
        state.column = col;
        state.row = ((y - state.header() as i64) / state.line() as i64) as usize;
        self.dirty();
    }
    pub(in crate::ide::native_tools) fn show_function_numeric(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        snapshot: &SessionSnapshot,
        out: &mut Vec<Request>,
    ) {
        let mut g = Grob::new(320, 240, PAPER);
        if self.page == Some(Page::NumericSetup) {
            self.numeric_setup_paint(&mut g);
        } else {
            self.numeric_grid_paint(&mut g, snapshot);
        }
        self.numeric_swap_paint(&mut g);
        self.numeric_editor_paint(&mut g);
        self.numeric_menu_paint(&mut g);
        if self.numeric_panel.plot_confirmation {
            let graph = &self.memory.graphs["Function"];
            g.rect(78, 85, 240, 133, 0, 0xffffff);
            text(&mut g, "i", 87, 96, 18, None);
            text(
                &mut g,
                &format!("Num Start: {}", graph.window.xmin),
                112,
                92,
                14,
                None,
            );
            text(
                &mut g,
                &format!(
                    "Num Step: {}",
                    plot_numeric_values(&graph.window)
                        .map_or_else(|_| "NaN".into(), |(_, step)| step.to_string())
                ),
                112,
                110,
                14,
                Some(125),
            );
            chrome::softkeys(&mut g, &["", "", "", "", "Cancel", "OK"]);
        }
        if self.numeric_panel.pixels != g.pixels() || self.numeric_panel.texture.is_none() {
            self.numeric_panel.pixels = g.pixels().to_vec();
            let image = egui::ColorImage::new(
                [320, 240],
                g.pixels()
                    .iter()
                    .map(|c| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, *c as u8))
                    .collect(),
            );
            if let Some(texture) = &mut self.numeric_panel.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.numeric_panel.texture = Some(ctx.load_texture(
                    "native_function_numeric",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        egui::Area::new("native_function_numeric".into())
            .fixed_pos(lcd.min)
            .fade_in(false)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.numeric_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let response = ui.interact(
                    lcd,
                    ui.id().with("numeric_lcd"),
                    egui::Sense::click_and_drag(),
                );
                if let Some(p) = response.interact_pointer_pos() {
                    let x = ((p.x - lcd.left()) * 320. / lcd.width()) as i64;
                    let y = ((p.y - lcd.top()) * 240. / lcd.height()) as i64;
                    if response.clicked() {
                        self.numeric_click(x, y, snapshot, out);
                    } else if response.dragged()
                        && self.numeric_panel.editor.is_none()
                        && self.numeric_panel.menu.is_none()
                        && self.page == Some(Page::Numeric)
                    {
                        self.numeric_point(x, y, true);
                    }
                }
                if response.hovered()
                    && self.numeric_panel.editor.is_none()
                    && self.numeric_panel.menu.is_none()
                    && self.page == Some(Page::Numeric)
                {
                    let wheel = ui.input(|i| i.raw_scroll_delta.y);
                    if wheel.abs() > 0.1 {
                        self.function_numeric_move(0, if wheel > 0. { -1 } else { 1 }, snapshot);
                    }
                }
            });
        if self.numeric_panel.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod literal_display_tests {
    use super::*;
    #[test]
    fn numeric_word_cells_use_decimal_numbers_and_keep_the_original_word_typed() {
        let word = ppl_math::HpInt::parse_literal("FF:-8h", ppl_math::IntBase::Hex).unwrap();
        let value = PortableValue::Integer(word);
        let mut settings = ppl_vm::Settings::default();
        settings.set_integer_base(0).unwrap();
        assert_eq!(numeric_display(&value, &settings), "255");
        assert_eq!(value, PortableValue::Integer(word));
    }
}
