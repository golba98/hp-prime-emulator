use super::*;
use crate::ide::native::chrome;
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
fn text(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 14., 0, None, None);
}
const SORT_LIST_TOP: i64 = 75;
const SORT_ROW_HEIGHT: i64 = 22;
const CHOICE_BUTTON_WIDTH: i64 = 12;
pub(super) fn axes(
    sheet: &Spreadsheet,
    first: usize,
    column: bool,
    first_row: usize,
) -> Vec<(usize, i64, i64)> {
    let mut result = vec![];
    let mut pos = if column {
        row_gutter(sheet, first_row)
    } else {
        37
    };
    let end = if column { 320 } else { 204 };
    for index in first..=if column { MAX_COLUMNS } else { MAX_ROWS } {
        if pos >= end {
            break;
        }
        let size = grid_step(sheet, index, column);
        let next = pos + i64::from(size);
        result.push((index, pos, next));
        pos = next;
    }
    result
}
// Persisted dimensions are the content size; the grid line adds one pixel.
// Defaults already include that line (64+1 and16+1).
pub(super) fn grid_step(sheet: &Spreadsheet, index: usize, column: bool) -> u16 {
    if column {
        sheet
            .column_widths
            .get(&index)
            .map_or(65, |n| n.saturating_add(1))
    } else {
        sheet
            .row_heights
            .get(&index)
            .map_or(17, |n| n.saturating_add(1))
    }
}
pub(super) fn row_gutter(sheet: &Spreadsheet, first_row: usize) -> i64 {
    let mut row = first_row;
    let mut y = 37;
    while y + i64::from(grid_step(sheet, row, false)) < 204 && row < MAX_ROWS {
        y += i64::from(grid_step(sheet, row, false));
        row += 1;
    }
    row.to_string().len() as i64 * 8 + 2
}
pub(super) fn composited_color(color: u32, background: u32) -> u32 {
    let mut sample = Grob::new(1, 1, background);
    sample.set(0, 0, color);
    sample.get(0, 0).unwrap_or(background)
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
        if value.is_some_and(|value| value["UnselectedChoice"].as_bool() == Some(true)) {
            return (String::new(), false);
        }
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
            let numeric = !matches!(
                portable.as_ref(),
                Some(
                    ppl_vm::PortableValue::Matrix { .. }
                        | ppl_vm::PortableValue::List(_)
                        | ppl_vm::PortableValue::String(_)
                        | ppl_vm::PortableValue::Utf16(_)
                )
            );
            let text = portable
                .as_ref()
                .map(|v| v.to_value().string_form(settings.home_format()))
                .or_else(|| v["display"].as_str().map(str::to_owned))
                .unwrap_or_default();
            return (text, numeric);
        }
        if value.is_some_and(|v| v.get("Error").is_some()) {
            return ("#ERROR".into(), false);
        }
        if sheet.has_value(col, row) {
            return ("…".into(), false);
        }
        (String::new(), false)
    }
    pub(super) fn spreadsheet_cell_layout(
        &self,
        col: usize,
        row: usize,
        settings: &ppl_vm::Settings,
    ) -> (crate::ide::native::math_layout::MathLayout, bool) {
        let style = self.memory.models.spreadsheet.format(col, row);
        let (text, numeric) = self.spreadsheet_cell_text(col, row, settings);
        let font = style.font.filter(|v| *v != 0).unwrap_or(14) as f32;
        if text.is_empty() {
            return (
                crate::ide::native::math_layout::MathLayout::default(),
                numeric,
            );
        }
        if style.textbook == Some(1) {
            let portable = self
                .memory
                .models
                .results
                .get("Spreadsheet")
                .and_then(|values| values.get(cell_name(col, row)))
                .and_then(|value| value.get("Object"))
                .and_then(|value| crate::ide::native::home_format::recover_value(&value["typed"]));
            if let Some(value @ ppl_vm::PortableValue::Matrix { .. }) = portable {
                let mut settings = settings.clone();
                settings.set_format(
                    style.number.filter(|v| *v >= 0).unwrap_or(settings.hformat),
                    style.digits.filter(|v| *v >= 0).unwrap_or(settings.hdigits),
                );
                let typed = crate::ide::native::portable_display_json(&value, &settings);
                return (
                    crate::ide::native::math_layout::result(&text, Some(&typed), font),
                    numeric,
                );
            }
        }
        // In particular, Textbook does not turn a rational into exact fraction
        // notation: ordinary worksheet numbers retain their decimal format.
        (crate::ide::native::math_layout::text(&text, font), numeric)
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
        let columns = axes(sheet, cursor.first_column, true, cursor.first_row);
        let rows = axes(sheet, cursor.first_row, false, cursor.first_row);
        let gutter = row_gutter(sheet, cursor.first_row);
        let block = self.spreadsheet_block();
        let mut g = Grob::new(320, 240, 0xffffff);
        chrome::header(&mut g, "Spreadsheet");
        g.line(0, 36, 319, 36, 0);
        g.line(gutter - 1, 20, gutter - 1, 203, 0);
        let mut corner = Grob::new((gutter - 1) as usize, 16, 0xffffff);
        text(&mut corner, "hp", 0, 0);
        g.blit(
            (0, 20, gutter - 1, 36),
            &corner,
            (0, 0, gutter - 1, 16),
            None,
            255,
        );
        for &(col, x, end) in &columns {
            let label = cell_name(col, 1);
            text(&mut g, label.trim_end_matches('1'), x + 1, 20);
            g.line(end - 1, 20, end - 1, 203, 0);
        }
        for &(row, y, end) in &rows {
            let h = (end.min(204) - y).max(1);
            let mut label = Grob::new((gutter - 1) as usize, h as usize, 0xffffff);
            text(&mut label, &row.to_string(), 0, 0);
            g.blit(
                (0, y, gutter - 1, y + h),
                &label,
                (0, 0, gutter - 1, h),
                None,
                255,
            );
            g.line(0, (end - 1).min(203), 319, (end - 1).min(203), 0);
            for &(col, x, right) in &columns {
                let w = (right.min(320) - x - 1).max(1);
                let choice = right <= 320 && sheet.has_choice(col, row);
                let content_width = if choice { w - CHOICE_BUTTON_WIDTH } else { w };
                let style = sheet.format(col, row);
                let selected = block.contains(col, row);
                let background = if selected {
                    if cell_coordinates(&self.memory.models.selected_cell) == Some((col, row)) {
                        0xadccff
                    } else {
                        0xc2defe
                    }
                } else {
                    0xffffff
                };
                let cell_height = (h - 1).max(1);
                let mut cell = Grob::new(w as usize, cell_height as usize, background);
                if !selected {
                    cell.fill_rect(0, 0, w - 1, cell_height - 1, style.fill.unwrap_or(0xffffff));
                }
                let (layout, numeric) = self.spreadsheet_cell_layout(col, row, &settings);
                let horizontal =
                    style
                        .horizontal
                        .filter(|v| *v <= 2)
                        .unwrap_or(if numeric { 2 } else { 0 });
                let px = match horizontal {
                    1 => (content_width - layout.width) / 2,
                    2 => content_width - layout.width - 1,
                    _ => 1,
                };
                let py = match style.vertical.filter(|v| *v <= 2).unwrap_or(1) {
                    0 => 0,
                    2 => cell_height - layout.height,
                    _ => (cell_height - layout.height) / 2,
                };
                // Compositing ink first preserves the PPL transparency byte
                // even when the font renderer applies antialiased coverage.
                let ink = composited_color(
                    style.color.unwrap_or(0),
                    cell.get(0, 0).unwrap_or(background),
                );
                layout.paint_static(&mut cell, px, py, ink);
                if choice {
                    cell.fill_rect(content_width, 0, w - 1, cell_height - 1, 0xe4eef7);
                    let x = content_width + 3;
                    let y = ((cell_height - 3) / 2).max(0);
                    cell.line(x, y, x + 6, y, 0x3b5266);
                    cell.line(x + 1, y + 1, x + 5, y + 1, 0x3b5266);
                    cell.line(x + 2, y + 2, x + 4, y + 2, 0x3b5266);
                }
                g.blit((x, y, x + w, y + h - 1), &cell, (0, 0, w, h - 1), None, 255);
            }
        }
        g.fill_rect(0, 204, 319, 219, 0xffffff);
        let target = self.spreadsheet_edit_address();
        let source = sheet.editor_source(&target);
        if let Some(error) = &self.error {
            text(&mut g, error, 0, 204);
        } else if let Some(source) = source {
            use crate::ide::native_app_models::spreadsheet::worksheet_cas;
            if let Some(body) = source
                .trim_start()
                .strip_prefix('=')
                .and_then(worksheet_cas::body)
            {
                draw_text(&mut g, "CAS", 0, 204, 14., 0xb02020, None, None);
                text(&mut g, &format!("={body}"), 30, 204);
            } else {
                text(&mut g, source, 0, 204);
            }
        }
        let (col, row) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
        let header = self.spreadsheet_header().is_some();
        let editable = header || source.is_some();
        let populated = !header && sheet.has_value(col, row);
        chrome::softkeys(
            &mut g,
            &[
                if editable { "Edit" } else { "" },
                "Format",
                "Go To",
                if cursor.selecting {
                    "Sel ∅"
                } else {
                    "Select"
                },
                if cursor.right { "Go →" } else { "Go ↓" },
                if self.spreadsheet_can_sort() {
                    "Sort"
                } else if populated {
                    "Show"
                } else {
                    ""
                },
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
                let cas = match self.spreadsheet_panel.cas_mode {
                    Some(true) => "CAS•",
                    Some(false) => "CAS",
                    None => "",
                };
                chrome::softkeys(&mut g, &["", cas, "", "$", "Cancel", "OK"]);
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
                root_index.saturating_sub(7)
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
            if menu.is_palette() {
                palette::paint(&mut g, menu.index, menu.category == Some(4));
            } else if let Some(category) = menu.category {
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
        if let Some(selected) = self.spreadsheet_panel.paste_choice {
            g.rect(39, 51, 282, 191, 0x4b6070, 0xf7fafc);
            g.fill_rect(40, 52, 281, 72, 0x3b5266);
            draw_text(&mut g, "Paste cells", 48, 54, 14., 0xffffff, None, None);
            for (index, (_, label)) in PasteMode::CHOICES.iter().enumerate() {
                let y = 75 + index as i64 * 22;
                if index == selected {
                    g.fill_rect(43, y, 278, y + 21, 0xd2e6f6);
                }
                text(&mut g, label, 50, y + 2);
            }
            chrome::softkeys(&mut g, &["", "", "", "", "Cancel", "Paste"]);
        }
        if let Some(menu) = self.spreadsheet_panel.sort_menu {
            let directions = menu.descending.is_some();
            let title = if directions {
                "Sort order"
            } else {
                "Sort rows by"
            };
            let submit = if directions { "Sort" } else { "Next" };
            g.rect(39, 51, 282, 191, 0x4b6070, 0xf7fafc);
            g.fill_rect(40, 52, 281, 72, 0x3b5266);
            draw_text(&mut g, title, 48, 54, 14., 0xffffff, None, None);
            let first = menu.first_visible();
            for index in first..(first + SortMenu::VISIBLE).min(menu.count()) {
                let y = SORT_LIST_TOP + (index - first) as i64 * SORT_ROW_HEIGHT;
                if index == menu.selected() {
                    g.fill_rect(43, y, 278, y + SORT_ROW_HEIGHT - 1, 0xd2e6f6);
                }
                let label = if directions {
                    SortMenu::DIRECTIONS[index].to_owned()
                } else {
                    let column = cell_name(menu.block.left + index, 1);
                    format!("Column {}", column.trim_end_matches('1'))
                };
                text(&mut g, &label, 50, y + 2);
            }
            chrome::softkeys(&mut g, &["", "", "", "", "Cancel", submit]);
        }
        if let Some(choice) = &self.spreadsheet_panel.cell_choice {
            g = choice.widget.render(&g);
        }
        if let Some(picker) = &self.spreadsheet_panel.tool_picker {
            g = picker.widget().render(&g);
            chrome::softkeys(&mut g, &["", "", "", "", "Cancel", "OK"]);
        }
        if let Some(picker) = &self.spreadsheet_panel.custom_color {
            picker.paint(&mut g);
            chrome::softkeys(&mut g, &["", "", "", "", "Cancel", "Apply"]);
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
                    if wheel.abs() > 1. && self.spreadsheet_panel.custom_color.is_none() {
                        let root_max = self.spreadsheet_root_items().len().saturating_sub(8);
                        if let Some(picker) = &mut self.spreadsheet_panel.tool_picker {
                            picker.action(A::History(if wheel < 0. { 1 } else { -1 }));
                        } else if let Some(choice) = &mut self.spreadsheet_panel.cell_choice {
                            let key = if wheel < 0. { KEY_DOWN } else { KEY_UP };
                            choice.widget.handle(KeyPress::code(key));
                        } else if let Some(menu) = &mut self.spreadsheet_panel.sort_menu {
                            menu.shift(if wheel < 0. { 1 } else { -1 });
                        } else if let Some(menu) = &mut self.spreadsheet_panel.menu {
                            let delta = if wheel < 0. { 1 } else { -1 };
                            if menu.is_palette() {
                                menu.index = palette::moved(menu.index, 0, delta);
                            } else {
                                let maximum = if menu.category.is_none() { root_max } else { 0 };
                                menu.first =
                                    (menu.first as i64 + delta).clamp(0, maximum as i64) as usize;
                            }
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
                                1 if self.spreadsheet_panel.cas_mode.is_some() => {
                                    requests.push(Request::ToggleSpreadsheetCas);
                                }
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
                    } else if let Some(picker) = &mut self.spreadsheet_panel.tool_picker {
                        if let Some(result) = picker.pointer(x, y) {
                            self.spreadsheet_finish_tool_pick(result, requests);
                        }
                        self.dirty();
                    } else if let Some(picker) = &mut self.spreadsheet_panel.custom_color {
                        if y >= 220 && x >= 212 {
                            self.spreadsheet_finish_custom_color(x >= 265);
                        } else {
                            picker.pick(x, y);
                        }
                    } else if self.spreadsheet_panel.show {
                        if y >= 220 {
                            self.spreadsheet_panel.show = false;
                            self.dirty();
                        }
                    } else if let Some(choice) = &mut self.spreadsheet_panel.cell_choice {
                        let result = choice
                            .widget
                            .item_at(x, y)
                            .map(DialogResult::Chosen)
                            .or_else(|| choice.widget.pointer(x, y));
                        if let Some(result) = result {
                            self.spreadsheet_finish_choice(result, requests);
                        }
                    } else if let Some(mut menu) = self.spreadsheet_panel.sort_menu {
                        if y >= 220 && x >= 265 {
                            self.spreadsheet_choose_sort(menu, requests);
                        } else if y >= 220 && x >= 212 {
                            self.spreadsheet_panel.sort_menu = None;
                            self.dirty();
                        } else if (43..279).contains(&x)
                            && (SORT_LIST_TOP
                                ..SORT_LIST_TOP + SORT_ROW_HEIGHT * SortMenu::VISIBLE as i64)
                                .contains(&y)
                        {
                            let index = menu.first_visible()
                                + ((y - SORT_LIST_TOP) / SORT_ROW_HEIGHT) as usize;
                            if index < menu.count() {
                                menu.select(index);
                                self.spreadsheet_choose_sort(menu, requests);
                            }
                        }
                    } else if let Some(index) = self.spreadsheet_panel.paste_choice {
                        if (43..279).contains(&x) && (75..185).contains(&y) {
                            self.spreadsheet_paste(((y - 75) / 22) as usize, requests);
                        } else if y >= 220 && x >= 265 {
                            self.spreadsheet_paste(index, requests);
                        } else if y >= 220 && x >= 212 {
                            self.spreadsheet_panel.paste_choice = None;
                            self.dirty();
                        }
                    } else if let Some(mut menu) = self.spreadsheet_panel.menu {
                        if x <= 128 && (43..211).contains(&y) {
                            let first = if menu.category.is_none() {
                                menu.first
                            } else {
                                self.spreadsheet_root_items()
                                    .iter()
                                    .position(|(code, _)| Some(*code) == menu.category)
                                    .unwrap_or(0)
                                    .saturating_sub(7)
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
                        } else if menu.is_palette()
                            && let Some(index) = palette::item_at(x, y)
                        {
                            menu.index = index;
                            self.spreadsheet_panel.menu = Some(menu);
                            self.spreadsheet_menu_choose(requests);
                        } else if !menu.is_palette()
                            && (130..=229).contains(&x)
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
                        let column = columns.iter().find(|(_, a, b)| (*a..*b).contains(&x));
                        let col = column.map(|v| v.0);
                        let row = rows
                            .iter()
                            .find(|(_, a, b)| (*a..*b).contains(&y))
                            .map(|v| v.0);
                        if let (Some(c), Some(r)) = (col, row) {
                            let choice = column.is_some_and(|&(_, _, right)| {
                                right <= 320 && x >= right - CHOICE_BUTTON_WIDTH - 1
                            }) && self.memory.models.spreadsheet.has_choice(c, r);
                            self.spreadsheet_select(c, r, self.memory.spreadsheet_cursor.selecting);
                            if choice && !self.memory.spreadsheet_cursor.selecting {
                                self.spreadsheet_open_choices(c, r, requests);
                            } else if response.double_clicked() {
                                requests.push(self.spreadsheet_edit(false));
                            }
                        } else if y < 37
                            && x >= gutter
                            && let Some(c) = col
                        {
                            self.spreadsheet_select(c, 1, false);
                            self.memory.spreadsheet_cursor.anchor = Some([c, 1]);
                            self.memory.spreadsheet_cursor.whole_columns = true;
                            self.dirty();
                        } else if x < gutter
                            && y >= 37
                            && let Some(r) = row
                        {
                            self.spreadsheet_select(1, r, false);
                            self.memory.spreadsheet_cursor.anchor = Some([1, r]);
                            self.memory.spreadsheet_cursor.whole_rows = true;
                            self.dirty();
                        } else if x < gutter && y < 37 {
                            self.spreadsheet_select(1, 1, false);
                            self.memory.spreadsheet_cursor.whole_rows = true;
                            self.memory.spreadsheet_cursor.whole_columns = true;
                            self.dirty();
                        }
                    }
                    ctx.request_repaint();
                }
                if response.dragged()
                    && let Some(picker) = &mut self.spreadsheet_panel.custom_color
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / scale;
                    picker.pick(p.x as i64, p.y as i64);
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
                    && self.spreadsheet_panel.tool_picker.is_none()
                    && self.spreadsheet_panel.paste_choice.is_none()
                    && self.spreadsheet_panel.sort_menu.is_none()
                    && self.spreadsheet_panel.cell_choice.is_none()
                    && self.spreadsheet_panel.custom_color.is_none()
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
                    && self.spreadsheet_panel.tool_picker.is_none()
                    && self.spreadsheet_panel.paste_choice.is_none()
                    && self.spreadsheet_panel.sort_menu.is_none()
                    && self.spreadsheet_panel.cell_choice.is_none()
                    && self.spreadsheet_panel.custom_color.is_none()
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
