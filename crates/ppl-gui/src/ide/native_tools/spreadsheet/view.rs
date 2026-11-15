use super::*;
use crate::ide::native::chrome;
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
fn text(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 14., 0, None, None);
}
fn axes(sheet: &Spreadsheet, first: usize, column: bool) -> Vec<(usize, i64, i64)> {
    let mut result = vec![];
    let mut pos = if column { 17 } else { 37 };
    let end = if column { 320 } else { 204 };
    for index in first..=if column { MAX_COLUMNS } else { MAX_ROWS } {
        if pos >= end {
            break;
        }
        let size = if column {
            *sheet.column_widths.get(&index).unwrap_or(&65)
        } else {
            *sheet.row_heights.get(&index).unwrap_or(&17)
        };
        let next = pos + i64::from(size);
        result.push((index, pos, next));
        pos = next;
    }
    result
}
impl NativeTools {
    pub(super) fn spreadsheet_cell_text(
        &self,
        col: usize,
        row: usize,
        settings: &ppl_vm::Settings,
    ) -> (String, bool) {
        let sheet = &self.memory.models.spreadsheet;
        let style = sheet.format(col, row);
        let name = cell_name(col, row);
        let mut settings = settings.clone();
        settings.set_format(
            style.number.filter(|v| *v >= 0).unwrap_or(settings.hformat),
            style.digits.filter(|v| *v >= 0).unwrap_or(settings.hdigits),
        );
        let value = self
            .memory
            .models
            .results
            .get("Spreadsheet")
            .and_then(|v| v.get(&name));
        if let Some(n) = value.and_then(|v| v["Number"].as_f64()) {
            return (
                ppl_math::Real::from_f64(n)
                    .map(|n| n.format(settings.home_format()))
                    .unwrap_or_else(|_| n.to_string()),
                true,
            );
        }
        if let Some(s) = value.and_then(|v| v["Text"].as_str()) {
            return (
                if style.quotes == Some(1) {
                    format!("\"{s}\"")
                } else {
                    s.into()
                },
                false,
            );
        }
        if let Some(v) = value.and_then(|v| v.get("Object")) {
            let portable = crate::ide::native::home_format::recover_value(&v["typed"]);
            let text = portable
                .map(|v| v.to_value().string_form(settings.home_format()))
                .or_else(|| v["display"].as_str().map(str::to_owned))
                .unwrap_or_default();
            return (text, true);
        }
        if value.is_some_and(|v| v.get("Error").is_some()) {
            return ("#ERROR".into(), false);
        }
        if sheet.cells.contains_key(&name) {
            return ("…".into(), false);
        }
        (String::new(), false)
    }
    pub(in crate::ide::native_tools) fn show_spreadsheet(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        self.spreadsheet_refresh(ctx, snapshot);
        let settings = self.spreadsheet_panel.settings.clone();
        let sheet = &self.memory.models.spreadsheet;
        let cursor = &self.memory.spreadsheet_cursor;
        let columns = axes(sheet, cursor.first_column, true);
        let rows = axes(sheet, cursor.first_row, false);
        let block = self.spreadsheet_block();
        let mut g = Grob::new(320, 240, 0xffffff);
        chrome::header(&mut g, "Spreadsheet");
        g.line(0, 36, 319, 36, 0);
        g.line(16, 20, 16, 203, 0);
        text(&mut g, "hp", 0, 20);
        for &(col, x, end) in &columns {
            let label = cell_name(col, 1);
            text(&mut g, label.trim_end_matches('1'), x + 1, 20);
            g.line(end - 1, 20, end - 1, 203, 0);
        }
        for &(row, y, end) in &rows {
            let h = (end.min(204) - y).max(1);
            let mut label = Grob::new(16, h as usize, 0xffffff);
            text(&mut label, &row.to_string(), 0, 0);
            g.blit((0, y, 16, y + h), &label, (0, 0, 16, h), None, 255);
            g.line(0, (end - 1).min(203), 319, (end - 1).min(203), 0);
            for &(col, x, right) in &columns {
                let w = (right.min(320) - x - 1).max(1);
                let style = sheet.format(col, row);
                let selected = block.contains(col, row);
                let fill = if selected {
                    if cell_coordinates(&self.memory.models.selected_cell) == Some((col, row)) {
                        0xadccff
                    } else {
                        0xc2defe
                    }
                } else {
                    style.fill.filter(|v| *v <= 0xffffff).unwrap_or(0xffffff)
                };
                let mut cell = Grob::new(w as usize, (h - 1).max(1) as usize, fill);
                let (value, numeric) = self.spreadsheet_cell_text(col, row, &settings);
                let font = style.font.filter(|v| *v != 0).unwrap_or(14) as f32;
                let width = measure(&value, font);
                let horizontal =
                    style
                        .horizontal
                        .filter(|v| *v <= 2)
                        .unwrap_or(if numeric { 2 } else { 0 });
                let px = match horizontal {
                    1 => (w - width) / 2,
                    2 => w - width - 1,
                    _ => 1,
                };
                let py = match style.vertical.filter(|v| *v <= 2).unwrap_or(1) {
                    0 => 0,
                    2 => h - font as i64,
                    _ => (h - font as i64) / 2,
                };
                draw_text(
                    &mut cell,
                    &value,
                    px,
                    py,
                    font,
                    style.color.filter(|v| *v <= 0xffffff).unwrap_or(0),
                    None,
                    None,
                );
                g.blit((x, y, x + w, y + h - 1), &cell, (0, 0, w, h - 1), None, 255);
            }
        }
        g.fill_rect(0, 204, 319, 219, 0xffffff);
        let source = self
            .memory
            .models
            .spreadsheet
            .cells
            .get(&self.memory.models.selected_cell)
            .map_or("", String::as_str);
        text(&mut g, self.error.as_deref().unwrap_or(source), 0, 204);
        let populated = sheet.cells.contains_key(&self.memory.models.selected_cell);
        chrome::softkeys(
            &mut g,
            &[
                if populated { "Edit" } else { "" },
                "Format",
                "Go To",
                if cursor.selecting {
                    "Sel ∅"
                } else {
                    "Select"
                },
                if cursor.right { "Go →" } else { "Go ↓" },
                if populated { "Show" } else { "" },
            ],
        );
        if let Some(editor) = &self.spreadsheet_panel.editor {
            let layout = crate::ide::native::math_layout::linear(editor);
            let caret = layout.cursor(editor.cursor()).0;
            let scroll = self.spreadsheet_panel.editor_scroll;
            let form = self.spreadsheet_panel.form;
            let visible = if form.is_some() { 211 } else { 311 };
            self.spreadsheet_panel.editor_scroll = if caret < scroll {
                caret
            } else if caret > scroll + visible {
                caret - visible
            } else {
                scroll
            };
            let (width, height) = if form.is_some() { (219, 20) } else { (320, 17) };
            let mut entry = Grob::new(width, height, 0xffffff);
            layout.paint(
                &mut entry,
                i64::from(form.is_none()) - self.spreadsheet_panel.editor_scroll,
                0,
                editor.selection(),
                editor.cursor(),
            );
            if let Some(form) = form {
                let (title, label, description) = form.caption();
                g.fill(0xe8f1f6);
                chrome::header(&mut g, title);
                text(&mut g, label, 90 - measure(label, 14.), 92);
                g.rect(92, 88, 314, 111, 0, 0xffffff);
                g.blit((94, 90, 313, 110), &entry, (0, 0, 219, 20), None, 255);
                draw_text(
                    &mut g,
                    self.error.as_deref().unwrap_or(description),
                    0,
                    203,
                    14.,
                    if self.error.is_some() { 0xc00000 } else { 0 },
                    None,
                    None,
                );
                chrome::softkeys(&mut g, &["Edit", "", "", "", "Cancel", "OK"]);
            } else {
                g.blit((0, 203, 320, 220), &entry, (0, 0, 320, 17), None, 255);
                chrome::softkeys(&mut g, &["", "", "", "$", "Cancel", "OK"]);
            }
        }
        if self.spreadsheet_panel.show {
            g.fill_rect(0, 20, 319, 219, 0xffffff);
            let (c, r) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
            let (text_value, _) = self.spreadsheet_cell_text(c, r, &settings);
            let offset = self.spreadsheet_panel.show_offset;
            for (i, line) in text_value.lines().enumerate() {
                text(&mut g, line, -offset[0], 22 + i as i64 * 17 - offset[1]);
            }
            chrome::softkeys(&mut g, &["", "", "", "", "", "OK"]);
        }
        if let Some(menu) = self.spreadsheet_panel.menu {
            let items = self.spreadsheet_menu_items(None);
            let root_index = menu
                .category
                .and_then(|c| {
                    self.spreadsheet_root_items()
                        .iter()
                        .position(|(code, _)| *code == c)
                })
                .unwrap_or(menu.index);
            let first = if menu.category.is_none() {
                menu.first
            } else {
                0
            };
            g.rect(0, 21, 128, 219, 0, 0xffffff);
            g.fill_rect(1, 22, 127, 41, 0x517ebd);
            draw_text(&mut g, "Format", 41, 23, 14., 0xffffff, None, None);
            for (row, (index, label)) in items.iter().enumerate().skip(first).take(8).enumerate() {
                let y = 43 + row as i64 * 21;
                if index == root_index {
                    g.fill_rect(1, y, 123, y + 20, 0xadccff);
                }
                text(&mut g, &format!("{}{label}", index + 1), 2, y);
            }
            if items.len() > 8 {
                g.line(125, 44, 125, 215, 0x888888);
            }
            if let Some(category) = menu.category {
                let children = self.spreadsheet_menu_items(Some(category));
                let top = self.spreadsheet_submenu_top(category, children.len());
                g.rect(
                    130,
                    top,
                    229,
                    top + children.len().min(8) as i64 * 21,
                    0,
                    0xffffff,
                );
                for (row, (index, label)) in children
                    .iter()
                    .enumerate()
                    .skip(menu.first)
                    .take(8)
                    .enumerate()
                {
                    let y = top + 1 + row as i64 * 21;
                    if index == menu.index {
                        g.fill_rect(131, y, 228, y + 20, 0xadccff);
                    }
                    text(&mut g, &format!("{}{label}", index + 1), 143, y);
                }
            }
        }
        if self.spreadsheet_panel.pixels != g.pixels() {
            self.spreadsheet_panel.pixels = g.pixels().to_vec();
            let image = egui::ColorImage::new(
                [320, 240],
                g.pixels()
                    .iter()
                    .map(|&c| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8))
                    .collect(),
            );
            if let Some(t) = &mut self.spreadsheet_panel.texture {
                t.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.spreadsheet_panel.texture = Some(ctx.load_texture(
                    "native_spreadsheet",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        let (editor_x, editor_y, editor_w, editor_h) = if self.spreadsheet_panel.form.is_some() {
            (94, 90, 219, 20)
        } else {
            (1, 203, 319, 17)
        };
        egui::Area::new("native_spreadsheet".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.spreadsheet_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let response = ui.interact(
                    lcd,
                    ui.id().with("spreadsheet_surface"),
                    egui::Sense::click_and_drag(),
                );
                if response.hovered() {
                    let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
                    if wheel.abs() > 1. {
                        let root_max = self.spreadsheet_root_items().len().saturating_sub(8);
                        if let Some(menu) = &mut self.spreadsheet_panel.menu {
                            let maximum = if menu.category.is_none() { root_max } else { 0 };
                            menu.first = (menu.first as i64 + if wheel < 0. { 1 } else { -1 })
                                .clamp(0, maximum as i64)
                                as usize;
                        } else {
                            let cursor = &mut self.memory.spreadsheet_cursor;
                            cursor.first_row = (cursor.first_row as i64
                                + if wheel < 0. { 1 } else { -1 })
                            .clamp(1, MAX_ROWS as i64)
                                as usize;
                        }
                        self.dirty();
                        ctx.request_repaint();
                    }
                }
                if response.clicked()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / scale;
                    let (x, y) = (p.x.floor() as i64, p.y.floor() as i64);
                    if let Some(editor) = &self.spreadsheet_panel.editor {
                        if self.spreadsheet_panel.form.is_some() {
                            if y >= 220 {
                                match (x / 53).min(5) {
                                    4 => requests.push(Request::FinishMath(false)),
                                    5 => requests.push(Request::FinishMath(true)),
                                    _ => {}
                                }
                            } else if (92..=314).contains(&x) && (88..=111).contains(&y) {
                                let layout = crate::ide::native::math_layout::linear(editor);
                                requests.push(Request::MathCursor {
                                    position: layout.hit_test(
                                        x - 94 + self.spreadsheet_panel.editor_scroll,
                                        y - 90,
                                    ),
                                    extend: false,
                                });
                            }
                        } else if y >= 220 {
                            match (x / 53).min(5) {
                                3 => requests.push(Request::TypeText("$".into())),
                                4 => requests.push(Request::FinishMath(false)),
                                5 => requests.push(Request::FinishMath(true)),
                                _ => {}
                            }
                        } else if y >= 203 {
                            let layout = crate::ide::native::math_layout::linear(editor);
                            requests.push(Request::MathCursor {
                                position: layout.hit_test(
                                    x - 1 + self.spreadsheet_panel.editor_scroll,
                                    y - 203,
                                ),
                                extend: false,
                            });
                        } else if let (Some(col), Some(row)) = (
                            columns
                                .iter()
                                .find(|(_, a, b)| (*a..*b).contains(&x))
                                .map(|v| v.0),
                            rows.iter()
                                .find(|(_, a, b)| (*a..*b).contains(&y))
                                .map(|v| v.0),
                        ) {
                            requests.push(Request::TypeText(cell_name(col, row)));
                        }
                    } else if self.spreadsheet_panel.show {
                        if y >= 220 {
                            self.spreadsheet_panel.show = false;
                            self.dirty();
                        }
                    } else if let Some(mut menu) = self.spreadsheet_panel.menu {
                        if x <= 128 && (43..211).contains(&y) {
                            let first = if menu.category.is_none() {
                                menu.first
                            } else {
                                0
                            };
                            let index = first + (y - 43) as usize / 21;
                            if index < self.spreadsheet_menu_items(None).len() {
                                menu = Menu {
                                    category: None,
                                    index,
                                    first,
                                };
                                self.spreadsheet_panel.menu = Some(menu);
                                self.spreadsheet_menu_choose(requests);
                            }
                        } else if (130..=229).contains(&x)
                            && let Some(category) = menu.category
                        {
                            let children = self.spreadsheet_menu_items(Some(category));
                            let top = self.spreadsheet_submenu_top(category, children.len());
                            if y > top && y <= top + children.len().min(8) as i64 * 21 {
                                let index = menu.first + (y - top - 1) as usize / 21;
                                if index < children.len() {
                                    menu.index = index;
                                    self.spreadsheet_panel.menu = Some(menu);
                                    self.spreadsheet_menu_choose(requests);
                                }
                            }
                        } else {
                            self.spreadsheet_panel.menu = None;
                        }
                        self.dirty();
                    } else if y >= 220 {
                        self.spreadsheet_softkey((x / 53).min(5) as usize, requests);
                    } else if y >= 204 {
                        if !self.spreadsheet_panel.busy {
                            requests.push(self.spreadsheet_edit(false));
                        }
                    } else if !self.spreadsheet_panel.busy {
                        let col = columns
                            .iter()
                            .find(|(_, a, b)| (*a..*b).contains(&x))
                            .map(|v| v.0);
                        let row = rows
                            .iter()
                            .find(|(_, a, b)| (*a..*b).contains(&y))
                            .map(|v| v.0);
                        if let (Some(c), Some(r)) = (col, row) {
                            self.spreadsheet_select(c, r, self.memory.spreadsheet_cursor.selecting);
                            if response.double_clicked() {
                                requests.push(self.spreadsheet_edit(false));
                            }
                        } else if y < 37
                            && x >= 17
                            && let Some(c) = col
                        {
                            self.spreadsheet_select(c, 1, false);
                            self.memory.spreadsheet_cursor.anchor = Some([c, 1]);
                            self.memory.spreadsheet_cursor.whole_columns = true;
                            self.dirty();
                        } else if x < 17
                            && y >= 37
                            && let Some(r) = row
                        {
                            self.spreadsheet_select(1, r, false);
                            self.memory.spreadsheet_cursor.anchor = Some([1, r]);
                            self.memory.spreadsheet_cursor.whole_rows = true;
                            self.dirty();
                        } else if x < 17 && y < 37 {
                            self.spreadsheet_select(1, 1, false);
                            self.memory.spreadsheet_cursor.whole_rows = true;
                            self.memory.spreadsheet_cursor.whole_columns = true;
                            self.dirty();
                        }
                    }
                    ctx.request_repaint();
                }
                if response.drag_started()
                    && let Some(editor) = &self.spreadsheet_panel.editor
                    && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
                {
                    let origin = (origin - lcd.min) / scale;
                    if origin.y >= editor_y as f32
                        && origin.y < (editor_y + editor_h) as f32
                        && origin.x >= editor_x as f32
                        && origin.x < (editor_x + editor_w) as f32
                    {
                        let layout = crate::ide::native::math_layout::linear(editor);
                        requests.push(Request::MathCursor {
                            position: layout.hit_test(
                                origin.x as i64 - editor_x + self.spreadsheet_panel.editor_scroll,
                                origin.y as i64 - editor_y,
                            ),
                            extend: false,
                        });
                    }
                }
                if response.dragged()
                    && let Some(editor) = &self.spreadsheet_panel.editor
                    && let Some(origin) = ctx.input(|i| i.pointer.press_origin())
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let origin = (origin - lcd.min) / scale;
                    if origin.y >= editor_y as f32
                        && origin.y < (editor_y + editor_h) as f32
                        && origin.x >= editor_x as f32
                        && origin.x < (editor_x + editor_w) as f32
                    {
                        let p = (p - lcd.min) / scale;
                        let layout = crate::ide::native::math_layout::linear(editor);
                        requests.push(Request::MathCursor {
                            position: layout.hit_test(
                                p.x as i64 - editor_x + self.spreadsheet_panel.editor_scroll,
                                p.y as i64 - editor_y,
                            ),
                            extend: true,
                        });
                        ctx.request_repaint();
                    }
                }
                if response.drag_started()
                    && self.spreadsheet_panel.editor.is_none()
                    && self.spreadsheet_panel.menu.is_none()
                    && !self.spreadsheet_panel.busy
                {
                    if let Some(p) = ctx.input(|i| i.pointer.press_origin()) {
                        let p = (p - lcd.min) / scale;
                        if let (Some(c), Some(r)) = (
                            columns
                                .iter()
                                .find(|(_, a, b)| (*a..*b).contains(&(p.x as i64)))
                                .map(|v| v.0),
                            rows.iter()
                                .find(|(_, a, b)| (*a..*b).contains(&(p.y as i64)))
                                .map(|v| v.0),
                        ) {
                            self.spreadsheet_select(c, r, false);
                            self.memory.spreadsheet_cursor.anchor = Some([c, r]);
                            self.memory.spreadsheet_cursor.selecting = true;
                        }
                    }
                }
                if response.dragged()
                    && self.spreadsheet_panel.editor.is_none()
                    && self.spreadsheet_panel.menu.is_none()
                    && !self.spreadsheet_panel.busy
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / scale;
                    if let (Some(c), Some(r)) = (
                        columns
                            .iter()
                            .find(|(_, a, b)| (*a..*b).contains(&(p.x as i64)))
                            .map(|v| v.0),
                        rows.iter()
                            .find(|(_, a, b)| (*a..*b).contains(&(p.y as i64)))
                            .map(|v| v.0),
                    ) {
                        self.spreadsheet_select(c, r, true);
                        ctx.request_repaint();
                    }
                }
            });
    }
}
