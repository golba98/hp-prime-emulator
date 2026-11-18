//! Prime worksheet grid interaction; calculations run outside the UI thread.
mod home_history;
mod palette;
#[cfg(test)]
mod tests;
mod view;
use super::{MathTarget, NativeTools, Page, Request};
use crate::dialog::{ChooseBox, KeyPress, ModalDialog};
use crate::ide::native_app_models::spreadsheet::{
    CellChoiceMenu, ClipboardBlock, Header, PasteMode,
    format::{AUTO_COLOR, Block, CellFormat},
};
use crate::ide::{
    native::expression::EditCommand as E,
    native::keys::KeyAction as A,
    native_app_models::{MAX_COLUMNS, MAX_ROWS, Spreadsheet, cell_coordinates, cell_name},
};
use crate::keymap::{KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_UP};
use ppl_vm::{SessionSnapshot, host::DialogResult};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursor {
    pub first_column: usize,
    pub first_row: usize,
    pub anchor: Option<[usize; 2]>,
    pub selecting: bool,
    pub right: bool,
    pub whole_columns: bool,
    pub whole_rows: bool,
}
impl Default for Cursor {
    fn default() -> Self {
        Self {
            first_column: 1,
            first_row: 1,
            anchor: None,
            selecting: false,
            right: false,
            whole_columns: false,
            whole_rows: false,
        }
    }
}
impl Cursor {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_COLUMNS).contains(&self.first_column)
            || !(1..=MAX_ROWS).contains(&self.first_row)
            || self
                .anchor
                .is_some_and(|a| a[0] == 0 || a[0] > MAX_COLUMNS || a[1] == 0 || a[1] > MAX_ROWS)
        {
            return Err("Invalid Spreadsheet cursor".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub address: String,
    pub source: String,
}
#[derive(Debug, Clone, Copy)]
pub enum Form {
    Goto,
    Name,
    Width,
    Height,
}
impl Form {
    fn caption(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Goto => ("Go To", "Target:", "Enter a cell reference or range"),
            Self::Name => ("Name", "Name:", "Enter name for cell, row or column"),
            Self::Width => ("Column ↔", "Column ↔:", "Enter column(s) width"),
            Self::Height => ("Row ↕", "Row ↕:", "Enter row(s) height"),
        }
    }
}
pub fn dimension(source: &str, width: bool) -> Result<Option<u16>, String> {
    let error = if width {
        "Choose −1 (Auto) or width20 through300"
    } else {
        "Choose −1 (Auto) or height12 through160"
    };
    let n = source
        .trim()
        .replace('−', "-")
        .parse::<i64>()
        .map_err(|_| error.to_string())?;
    if n == -1 {
        return Ok(None);
    }
    if if width {
        (20..=300).contains(&n)
    } else {
        (12..=160).contains(&n)
    } {
        Ok(Some(n as u16))
    } else {
        Err(error.into())
    }
}
#[derive(Default)]
pub(super) struct Panel {
    settings: ppl_vm::Settings,
    editor: Option<crate::ide::native::expression::ExpressionEditor>,
    form: Option<Form>,
    cas_mode: Option<bool>,
    editor_scroll: i64,
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
    menu: Option<Menu>,
    tool_picker: Option<home_history::Picker>,
    custom_color: Option<palette::CustomColor>,
    sort_menu: Option<SortMenu>,
    cell_choice: Option<OpenCellChoice>,
    paste_choice: Option<usize>,
    clipboard: Option<Arc<ClipboardBlock>>,
    show: bool,
    show_offset: [i64; 2],
    busy: bool,
    revision: Option<u64>,
    live: Option<(u64, SessionSnapshot)>,
    pub(super) job: Option<Job>,
    key: Option<CalculationKey>,
}
#[derive(Clone, Copy)]
struct Menu {
    category: Option<usize>,
    index: usize,
    first: usize,
}
impl Menu {
    fn is_palette(self) -> bool {
        matches!(self.category, Some(3 | 4))
    }
}
struct OpenCellChoice {
    address: String,
    widget: ChooseBox,
}
#[derive(Clone, Copy)]
struct SortMenu {
    block: Block,
    column: usize,
    descending: Option<bool>,
}
impl SortMenu {
    const DIRECTIONS: [&'static str; 2] = ["Ascending", "Descending"];
    const VISIBLE: usize = 5;

    fn count(self) -> usize {
        if self.descending.is_some() {
            Self::DIRECTIONS.len()
        } else {
            self.block.right - self.block.left + 1
        }
    }
    fn selected(self) -> usize {
        self.descending
            .map_or(self.column - self.block.left, usize::from)
    }
    fn first_visible(self) -> usize {
        self.selected().saturating_sub(Self::VISIBLE - 1)
    }
    fn select(&mut self, index: usize) {
        if self.descending.is_some() {
            self.descending = Some(index == 1);
        } else {
            self.column = self.block.left + index;
        }
    }
    fn shift(&mut self, delta: i32) {
        let index =
            (self.selected() as i64 + i64::from(delta)).clamp(0, self.count() as i64 - 1) as usize;
        self.select(index);
    }
}
type CalculationKey = (u64, serde_json::Value, Option<Block>);

fn color_style(fill: bool, value: u32) -> CellFormat {
    let mut style = CellFormat::default();
    if fill {
        style.fill = Some(value);
    } else {
        style.color = Some(value);
    }
    style
}

pub(super) struct Job {
    key: CalculationKey,
    rx: mpsc::Receiver<Result<serde_json::Value, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl NativeTools {
    pub fn spreadsheet_cancel(&mut self) {
        self.spreadsheet_panel.tool_picker = None;
        self.spreadsheet_panel.sort_menu = None;
        self.spreadsheet_panel.cell_choice = None;
        self.spreadsheet_panel.custom_color = None;
        if let Some(job) = self.spreadsheet_panel.job.take() {
            self.spreadsheet_panel.key = Some(job.key.clone());
        }
    }
    pub fn spreadsheet_request_refresh(&mut self) {
        self.spreadsheet_panel.key = None;
    }
    fn spreadsheet_submenu_top(&self, category: usize, count: usize) -> i64 {
        (43 + self
            .spreadsheet_root_items()
            .iter()
            .position(|(code, _)| *code == category)
            .unwrap_or(0) as i64
            * 21)
            .min(218 - count.min(8) as i64 * 21)
    }
    pub fn spreadsheet_editor(
        &mut self,
        editor: Option<crate::ide::native::expression::ExpressionEditor>,
        form: Option<Form>,
        cas_mode: Option<bool>,
    ) {
        self.spreadsheet_panel.editor = editor;
        self.spreadsheet_panel.form = form;
        self.spreadsheet_panel.cas_mode = cas_mode;
    }
    pub fn is_spreadsheet_page(&self) -> bool {
        self.memory.selected_app == "Spreadsheet" && self.page == Some(Page::Numeric)
    }
    pub fn spreadsheet_apply_context(&mut self, s: &SessionSnapshot, committed: u64) {
        if s.active_app == "Spreadsheet" {
            self.spreadsheet_panel.revision = Some(committed);
            self.spreadsheet_panel.live = Some((committed, s.clone()));
            self.spreadsheet_panel.key = None;
            self.spreadsheet_panel.job = None;
        }
    }
    pub fn spreadsheet_busy(&mut self, busy: bool) {
        self.spreadsheet_panel.busy = busy;
    }
    pub fn spreadsheet_changed(&mut self, advance: bool) {
        self.spreadsheet_panel.cell_choice = None;
        self.memory.spreadsheet_draft = None;
        self.spreadsheet_panel.key = None;
        self.spreadsheet_panel.job = None;
        self.memory.models.results.remove("Spreadsheet");
        if advance {
            let (c, r) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
            self.spreadsheet_select(
                if self.memory.spreadsheet_cursor.right {
                    (c + 1).min(MAX_COLUMNS)
                } else {
                    c
                },
                if self.memory.spreadsheet_cursor.right {
                    r
                } else {
                    (r + 1).min(MAX_ROWS)
                },
                false,
            );
        }
        self.error = None;
        self.dirty();
    }
    pub fn spreadsheet_entry_changed(&mut self, address: &str) {
        if Header::parse(address).is_some() {
            let (col, row) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
            self.spreadsheet_select(col, row, false);
            self.spreadsheet_changed(false);
        } else {
            self.spreadsheet_changed(true);
        }
    }
    pub fn spreadsheet_source(&self, address: &str, clear: bool) -> String {
        if clear {
            return String::new();
        }
        if let Some(d) = &self.memory.spreadsheet_draft
            && d.address == address
        {
            return d.source.clone();
        }
        self.memory
            .models
            .spreadsheet
            .editor_source(address)
            .unwrap_or_default()
            .to_owned()
    }
    pub fn spreadsheet_goto(&mut self, source: &str) -> Result<(), String> {
        let block = self.spreadsheet_target(source)?;
        self.spreadsheet_select_block(block);
        self.error = None;
        Ok(())
    }
    pub fn spreadsheet_select_block(&mut self, block: Block) {
        self.spreadsheet_select(block.left, block.top, false);
        if block.left != block.right || block.top != block.bottom {
            self.memory.spreadsheet_cursor.anchor = Some([block.right, block.bottom]);
            self.memory.spreadsheet_cursor.selecting = true;
        } else {
            self.memory.spreadsheet_cursor.selecting = false;
        }
    }
    pub fn spreadsheet_target(&self, source: &str) -> Result<Block, String> {
        let source = source.trim();
        if let Some((first, last)) = source.split_once(':') {
            let (a, b) = cell_coordinates(first.trim()).ok_or("Invalid first cell")?;
            let (c, d) = cell_coordinates(last.trim()).ok_or("Invalid last cell")?;
            return Ok(Block {
                left: a.min(c),
                right: a.max(c),
                top: b.min(d),
                bottom: b.max(d),
            });
        }
        self.memory
            .models
            .spreadsheet
            .names
            .get(source)
            .copied()
            .or_else(|| cell_coordinates(source).map(|(c, r)| Block::cell(c, r)))
            .ok_or_else(|| "Choose a cell, range or saved name".into())
    }
    pub fn spreadsheet_block(&self) -> Block {
        let (c, r) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
        let cursor = &self.memory.spreadsheet_cursor;
        let [a, b] = cursor.anchor.unwrap_or([c, r]);
        Block {
            left: if cursor.whole_rows { 1 } else { a.min(c) },
            right: if cursor.whole_rows {
                MAX_COLUMNS
            } else {
                a.max(c)
            },
            top: if cursor.whole_columns { 1 } else { b.min(r) },
            bottom: if cursor.whole_columns {
                MAX_ROWS
            } else {
                b.max(r)
            },
        }
    }
    fn spreadsheet_select(&mut self, c: usize, r: usize, extend: bool) {
        let cursor = &mut self.memory.spreadsheet_cursor;
        if !extend {
            cursor.anchor = None;
            cursor.whole_columns = false;
            cursor.whole_rows = false;
        }
        self.memory.models.select_cell(&cell_name(c, r));
        if c < cursor.first_column {
            cursor.first_column = c;
        }
        if r < cursor.first_row {
            cursor.first_row = r;
        }
        // Keep selected coordinates visible even after Go To or a far arrow move.
        let sheet = &self.memory.models.spreadsheet;
        let mut height = (cursor.first_row..=r)
            .map(|i| u32::from(view::grid_step(sheet, i, false)))
            .sum::<u32>();
        while cursor.first_row < r && height > 167 {
            height -= u32::from(view::grid_step(sheet, cursor.first_row, false));
            cursor.first_row += 1;
        }
        let available_width = (320 - view::row_gutter(sheet, cursor.first_row)) as u32;
        let mut width = (cursor.first_column..=c)
            .map(|i| u32::from(view::grid_step(sheet, i, true)))
            .sum::<u32>();
        while cursor.first_column < c && width > available_width {
            width -= u32::from(view::grid_step(sheet, cursor.first_column, true));
            cursor.first_column += 1;
        }
        self.dirty();
    }
    fn spreadsheet_header(&self) -> Option<Header> {
        let cursor = &self.memory.spreadsheet_cursor;
        let block = self.spreadsheet_block();
        match (cursor.whole_rows, cursor.whole_columns) {
            (true, false) if block.top == block.bottom => Some(Header::Row(block.top)),
            (false, true) if block.left == block.right => Some(Header::Column(block.left)),
            _ => None,
        }
    }
    fn spreadsheet_edit_address(&self) -> String {
        self.spreadsheet_header()
            .map_or_else(|| self.memory.models.selected_cell.clone(), Header::name)
    }
    fn spreadsheet_edit(&self, clear: bool) -> Request {
        Request::EditMath(MathTarget::SpreadsheetCell {
            address: self.spreadsheet_edit_address(),
            clear,
            history: None,
        })
    }
    pub fn spreadsheet_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if !self.is_spreadsheet_page() {
            return None;
        }
        if self.spreadsheet_panel.busy
            || self.spreadsheet_panel.menu.is_some()
            || self.spreadsheet_panel.tool_picker.is_some()
            || self.spreadsheet_panel.sort_menu.is_some()
            || self.spreadsheet_panel.cell_choice.is_some()
            || self.spreadsheet_panel.custom_color.is_some()
            || self.spreadsheet_panel.show
        {
            return Some(vec![]);
        }
        Some(vec![
            self.spreadsheet_edit(true),
            Request::TypeText(text.into()),
        ])
    }
    fn spreadsheet_root_items(&self) -> Vec<(usize, &'static str)> {
        let c = &self.memory.spreadsheet_cursor;
        let b = self.spreadsheet_block();
        let mut v = vec![];
        if (b.left == b.right && b.top == b.bottom) || c.whole_columns || c.whole_rows {
            v.push((0, "Name"));
        }
        v.extend([
            (1, "Number Format ›"),
            (2, "Font Size ›"),
            (3, "Color ›"),
            (4, "Fill ›"),
            (5, "Align ↔ ›"),
            (6, "Align ↕ ›"),
        ]);
        if c.whole_columns {
            v.push((8, "Column ↔"));
        }
        if c.whole_rows {
            v.push((9, "Row ↕"));
        }
        v.push((7, "Show \" \" ›"));
        v.push((10, "Textbook ›"));
        v
    }
    fn spreadsheet_menu_items(&self, category: Option<usize>) -> Vec<&'static str> {
        match category {
            None => self
                .spreadsheet_root_items()
                .into_iter()
                .map(|(_, s)| s)
                .collect(),
            Some(1) => vec![
                "Auto",
                "Standard",
                "Fixed",
                "Scientific",
                "Engineering",
                "Floating",
                "Rounded",
            ],
            Some(2) => vec!["Auto", "10", "12", "14", "16", "18", "20", "22"],
            Some(5) => vec!["Auto", "Left", "Center", "Right"],
            Some(6) => vec!["Auto", "Top", "Center", "Bottom"],
            Some(7 | 10) => vec!["Auto", "Yes", "No"],
            _ => vec![],
        }
    }
    fn spreadsheet_menu_choose(&mut self, requests: &mut Vec<Request>) {
        let Some(menu) = self.spreadsheet_panel.menu else {
            return;
        };
        let index = menu.index;
        let block = self.spreadsheet_block();
        if menu.category.is_none() {
            let Some((code, _)) = self.spreadsheet_root_items().get(index).copied() else {
                return;
            };
            match code {
                0 => requests.push(Request::EditMath(MathTarget::SpreadsheetName { block })),
                1..=7 | 10 => {
                    self.spreadsheet_panel.menu = Some(Menu {
                        category: Some(code),
                        index: 0,
                        first: 0,
                    });
                    return;
                }
                _ => {
                    let property = if code == 8 { "width" } else { "height" };
                    requests.push(Request::EditMath(MathTarget::SpreadsheetDimension {
                        block,
                        property: property.into(),
                    }));
                }
            }
        } else if let Some(category) = menu.category {
            let mut style = CellFormat::default();
            match category {
                1 => style.number = Some(index as i64 - 1),
                2 => style.font = Some(if index == 0 { 0 } else { 8 + index as u8 * 2 }),
                c @ (3 | 4) => {
                    if let Some(color) = palette::color(index) {
                        style = color_style(c == 4, color);
                    } else {
                        self.spreadsheet_panel.menu = None;
                        self.spreadsheet_panel.custom_color =
                            Some(palette::CustomColor::new(block, c == 4));
                        self.dirty();
                        return;
                    }
                }
                5 => style.horizontal = Some(if index == 0 { 3 } else { index as u8 - 1 }),
                6 => style.vertical = Some(if index == 0 { 3 } else { index as u8 - 1 }),
                7 => style.quotes = Some(index as u8),
                10 => style.textbook = Some(index as u8),
                _ => return,
            }
            self.spreadsheet_apply_format(block, style);
        }
        self.spreadsheet_panel.menu = None;
    }
    fn spreadsheet_apply_format(&mut self, block: Block, style: CellFormat) {
        self.error = self
            .memory
            .models
            .spreadsheet
            .apply_format(block, style)
            .err();
        self.dirty();
    }
    fn spreadsheet_finish_custom_color(&mut self, apply: bool) {
        if let Some(picker) = self.spreadsheet_panel.custom_color.take() {
            if apply {
                self.spreadsheet_apply_format(
                    picker.block,
                    color_style(picker.fill, picker.color()),
                );
            } else {
                self.dirty();
            }
        }
    }
    pub fn spreadsheet_copied(&mut self, block: ClipboardBlock) -> String {
        let text = block.text.clone();
        self.spreadsheet_panel.clipboard = Some(Arc::new(block));
        let cursor = &mut self.memory.spreadsheet_cursor;
        cursor.anchor = None;
        cursor.selecting = false;
        cursor.whole_rows = false;
        cursor.whole_columns = false;
        self.dirty();
        text
    }
    pub fn spreadsheet_paste_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if self.spreadsheet_panel.paste_choice.is_some()
            || self.spreadsheet_panel.tool_picker.is_some()
            || self.spreadsheet_panel.sort_menu.is_some()
            || self.spreadsheet_panel.cell_choice.is_some()
            || self.spreadsheet_panel.custom_color.is_some()
        {
            return Some(Vec::new());
        }
        if self
            .spreadsheet_panel
            .clipboard
            .as_ref()
            .is_some_and(|block| block.text == text)
        {
            return self.spreadsheet_action(A::Paste);
        }
        let source = text.trim();
        // The grid clipboard enters an "=" prefixed vector or matrix as one value.
        let single_cell = source
            .strip_prefix('=')
            .filter(|expression| expression.trim_start().starts_with('['));
        if !self.spreadsheet_panel.busy
            && self.spreadsheet_panel.menu.is_none()
            && !self.spreadsheet_panel.show
            && self.spreadsheet_header().is_none()
            && Spreadsheet::is_clipboard_collection(single_cell.unwrap_or(source))
        {
            use super::super::native_app_ui::Calculation;
            let sheet = self.memory.models.spreadsheet.clone();
            let geometry = self.memory.models.geometry_book.clone().map(Arc::new);
            let calculation = if let Some(collection) = single_cell {
                Calculation::SpreadsheetEntry(
                    sheet,
                    self.memory.models.selected_cell.clone(),
                    collection.trim_start().into(),
                    geometry,
                )
            } else {
                let block = self.spreadsheet_block();
                Calculation::SpreadsheetPasteCollection {
                    sheet,
                    text: source.into(),
                    destination: [block.left, block.top],
                    geometry,
                }
            };
            return Some(vec![Request::Calculate(calculation)]);
        }
        self.spreadsheet_text(text)
    }
    fn spreadsheet_paste(&mut self, index: usize, requests: &mut Vec<Request>) {
        self.spreadsheet_panel.paste_choice = None;
        let Some(clipboard) = self.spreadsheet_panel.clipboard.clone() else {
            return;
        };
        let Some(&(mode, _)) = PasteMode::CHOICES.get(index) else {
            return;
        };
        let block = self.spreadsheet_block();
        requests.push(Request::Calculate(
            super::super::native_app_ui::Calculation::SpreadsheetPaste {
                sheet: self.memory.models.spreadsheet.clone(),
                clipboard,
                destination: [block.left, block.top],
                mode,
                geometry: self.memory.models.geometry_book.clone().map(Arc::new),
            },
        ));
        self.dirty();
    }
    fn spreadsheet_can_sort(&self) -> bool {
        let cursor = &self.memory.spreadsheet_cursor;
        (cursor.selecting || cursor.anchor.is_some()) && !cursor.whole_columns && !cursor.whole_rows
    }
    fn spreadsheet_choose_sort(&mut self, menu: SortMenu, requests: &mut Vec<Request>) {
        if let Some(descending) = menu.descending {
            self.spreadsheet_panel.sort_menu = None;
            requests.push(Request::Calculate(
                super::super::native_app_ui::Calculation::SpreadsheetSort {
                    sheet: self.memory.models.spreadsheet.clone(),
                    block: menu.block,
                    column: menu.column,
                    descending,
                    geometry: self.memory.models.geometry_book.clone().map(Arc::new),
                },
            ));
        } else {
            self.spreadsheet_panel.sort_menu = Some(SortMenu {
                descending: Some(false),
                ..menu
            });
        }
        self.dirty();
    }
    fn spreadsheet_open_choices(&mut self, col: usize, row: usize, requests: &mut Vec<Request>) {
        requests.push(Request::Calculate(
            super::super::native_app_ui::Calculation::SpreadsheetChoices(
                self.memory.models.spreadsheet.clone(),
                cell_name(col, row),
                self.memory.models.geometry_book.clone().map(Arc::new),
            ),
        ));
    }
    pub fn spreadsheet_choices_opened(&mut self, address: &str, choices: CellChoiceMenu) {
        if self.is_spreadsheet_page()
            && self.memory.models.selected_cell == address
            && !choices.items.is_empty()
        {
            self.spreadsheet_panel.cell_choice = Some(OpenCellChoice {
                address: address.into(),
                widget: ChooseBox::new(&choices.title, choices.items, choices.current),
            });
            self.dirty();
        }
    }
    fn spreadsheet_finish_choice(&mut self, result: DialogResult, requests: &mut Vec<Request>) {
        if let Some(choice) = self.spreadsheet_panel.cell_choice.take()
            && let DialogResult::Chosen(index) = result
        {
            requests.push(Request::Calculate(
                super::super::native_app_ui::Calculation::SpreadsheetSelectChoice(
                    self.memory.models.spreadsheet.clone(),
                    choice.address,
                    index,
                    self.memory.models.geometry_book.clone().map(Arc::new),
                ),
            ));
        }
        self.dirty();
    }
    pub fn spreadsheet_action(&mut self, action: A) -> Option<Vec<Request>> {
        if !self.is_spreadsheet_page() {
            return None;
        }
        let mut requests = vec![];
        if self.spreadsheet_panel.busy {
            return Some(requests);
        }
        if let Some(picker) = &mut self.spreadsheet_panel.tool_picker {
            if let Some(result) = picker.action(action) {
                self.spreadsheet_finish_tool_pick(result, &mut requests);
            }
            self.dirty();
            return Some(requests);
        }
        if self.spreadsheet_panel.custom_color.is_some() {
            match action {
                A::Back => self.spreadsheet_finish_custom_color(false),
                A::Evaluate => self.spreadsheet_finish_custom_color(true),
                _ => {}
            }
            return Some(requests);
        }
        if let Some(choice) = &mut self.spreadsheet_panel.cell_choice {
            let key = match action {
                A::Back => KEY_ESC,
                A::Evaluate => KEY_ENTER,
                A::History(delta) if delta < 0 => KEY_UP,
                A::History(_) => KEY_DOWN,
                _ => return Some(requests),
            };
            if let Some(result) = choice.widget.handle(KeyPress::code(key)) {
                self.spreadsheet_finish_choice(result, &mut requests);
            }
            self.dirty();
            return Some(requests);
        }
        if let Some(mut menu) = self.spreadsheet_panel.sort_menu {
            match action {
                A::Back | A::Edit(E::Left(_)) => {
                    self.spreadsheet_panel.sort_menu = if menu.descending.take().is_some() {
                        Some(menu)
                    } else {
                        None
                    };
                }
                A::History(delta) => {
                    menu.shift(i32::from(delta));
                    self.spreadsheet_panel.sort_menu = Some(menu);
                }
                A::Evaluate | A::Edit(E::Right(_)) => {
                    self.spreadsheet_choose_sort(menu, &mut requests);
                }
                _ => {}
            }
            self.dirty();
            return Some(requests);
        }
        if let Some(index) = self.spreadsheet_panel.paste_choice {
            match action {
                A::Back => self.spreadsheet_panel.paste_choice = None,
                A::History(delta) => {
                    self.spreadsheet_panel.paste_choice = Some(
                        (index as i64 + i64::from(delta))
                            .clamp(0, PasteMode::CHOICES.len() as i64 - 1)
                            as usize,
                    );
                }
                A::Evaluate => self.spreadsheet_paste(index, &mut requests),
                _ => {}
            }
            self.dirty();
            return Some(requests);
        }
        if let Some(mut menu) = self.spreadsheet_panel.menu {
            if menu.is_palette() {
                let movement = match action {
                    A::History(d) => Some((0, i64::from(d))),
                    A::Edit(E::Left(_)) => Some((-1, 0)),
                    A::Edit(E::Right(_)) => Some((1, 0)),
                    _ => None,
                };
                if let Some((dx, dy)) = movement {
                    menu.index = palette::moved(menu.index, dx, dy);
                    self.spreadsheet_panel.menu = Some(menu);
                    self.dirty();
                    return Some(requests);
                }
            }
            let len = self.spreadsheet_menu_items(menu.category).len();
            match action {
                A::Back => {
                    self.spreadsheet_panel.menu = if menu.category.is_some() {
                        Some(Menu {
                            category: None,
                            index: 0,
                            first: 0,
                        })
                    } else {
                        None
                    }
                }
                A::History(d) => {
                    menu.index = (menu.index as i64 + i64::from(d))
                        .clamp(0, len.saturating_sub(1) as i64)
                        as usize;
                    menu.first = menu.first.min(menu.index).max(menu.index.saturating_sub(7));
                    self.spreadsheet_panel.menu = Some(menu);
                }
                A::Evaluate | A::Edit(E::Right(_)) => self.spreadsheet_menu_choose(&mut requests),
                A::Edit(E::Left(_)) => {
                    self.spreadsheet_panel.menu = Some(Menu {
                        category: None,
                        index: 0,
                        first: 0,
                    })
                }
                _ => {}
            }
            self.dirty();
            return Some(requests);
        }
        if self.spreadsheet_panel.show {
            match action {
                A::Back | A::Evaluate => self.spreadsheet_panel.show = false,
                A::History(d) => {
                    self.spreadsheet_panel.show_offset[1] =
                        (self.spreadsheet_panel.show_offset[1] + i64::from(d) * 14).max(0)
                }
                A::Edit(E::Left(_)) => {
                    self.spreadsheet_panel.show_offset[0] =
                        (self.spreadsheet_panel.show_offset[0] - 40).max(0)
                }
                A::Edit(E::Right(_)) => self.spreadsheet_panel.show_offset[0] += 40,
                _ => {}
            }
            self.dirty();
            return Some(requests);
        }
        let (c, r) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
        let extend = self.memory.spreadsheet_cursor.selecting;
        match action {
            A::Tool(crate::ide::native::keys::Tool::Menu) => {
                self.spreadsheet_panel.tool_picker = Some(home_history::Picker::commands());
                self.dirty();
            }
            A::History(d) => self.spreadsheet_select(
                c,
                (r as i64 + i64::from(d)).clamp(1, MAX_ROWS as i64) as usize,
                extend,
            ),
            A::Edit(E::Left(shift)) => {
                self.spreadsheet_extend(shift, c, r);
                self.spreadsheet_select(c.saturating_sub(1).max(1), r, extend || shift);
            }
            A::Edit(E::Right(shift)) => {
                self.spreadsheet_extend(shift, c, r);
                self.spreadsheet_select((c + 1).min(MAX_COLUMNS), r, extend || shift);
            }
            A::Evaluate => requests.push(self.spreadsheet_edit(false)),
            A::Insert(t) => return self.spreadsheet_text(t),
            A::Template(t) => {
                return Some(vec![
                    self.spreadsheet_edit(true),
                    Request::InsertTemplate(t),
                ]);
            }
            A::Sign => {
                return Some(vec![
                    self.spreadsheet_edit(true),
                    Request::TypeText("−".into()),
                ]);
            }
            A::Back => {
                self.memory.spreadsheet_cursor.anchor = None;
                self.memory.spreadsheet_cursor.selecting = false;
                self.memory.spreadsheet_cursor.whole_rows = false;
                self.memory.spreadsheet_cursor.whole_columns = false;
                self.dirty();
            }
            A::Edit(E::Backspace | E::Delete) => {
                if let Some(header) = self.spreadsheet_header() {
                    self.memory.models.spreadsheet.clear_header(header);
                } else {
                    let block = self.spreadsheet_block();
                    self.memory.models.spreadsheet.clear_block(block);
                }
                self.spreadsheet_changed(false);
            }
            A::Copy => {
                let block = self.spreadsheet_block();
                if (block.right - block.left + 1) * (block.bottom - block.top + 1) <= 10000 {
                    requests.push(Request::Calculate(
                        super::super::native_app_ui::Calculation::SpreadsheetCopy(
                            self.memory.models.spreadsheet.clone(),
                            block,
                            self.memory.models.geometry_book.clone().map(Arc::new),
                        ),
                    ));
                } else {
                    self.error = Some("Select up to 10000 cells to copy".into());
                }
            }
            A::Paste if self.spreadsheet_panel.clipboard.is_some() => {
                self.spreadsheet_panel.paste_choice = Some(0);
                self.dirty();
            }
            _ => return None,
        }
        Some(requests)
    }
    fn spreadsheet_extend(&mut self, shift: bool, c: usize, r: usize) {
        if shift && self.memory.spreadsheet_cursor.anchor.is_none() {
            self.memory.spreadsheet_cursor.anchor = Some([c, r]);
        }
    }
    fn spreadsheet_softkey(&mut self, index: usize, requests: &mut Vec<Request>) {
        if self.spreadsheet_panel.busy {
            return;
        }
        let (col, row) = cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
        let populated = self.memory.models.spreadsheet.has_value(col, row);
        match index {
            0 if self.spreadsheet_header().is_some()
                || self
                    .memory
                    .models
                    .spreadsheet
                    .cells
                    .contains_key(&self.memory.models.selected_cell) =>
            {
                requests.push(self.spreadsheet_edit(false));
            }
            1 => {
                self.spreadsheet_panel.menu = Some(Menu {
                    category: None,
                    index: 0,
                    first: 0,
                })
            }
            2 => requests.push(Request::EditMath(MathTarget::SpreadsheetGoto)),
            3 => {
                let c = &mut self.memory.spreadsheet_cursor;
                c.selecting = !c.selecting;
                if c.selecting {
                    let (col, row) =
                        cell_coordinates(&self.memory.models.selected_cell).unwrap_or((1, 1));
                    c.anchor = Some([col, row]);
                } else {
                    c.anchor = None;
                    c.whole_columns = false;
                    c.whole_rows = false;
                }
            }
            4 => self.memory.spreadsheet_cursor.right = !self.memory.spreadsheet_cursor.right,
            5 if self.spreadsheet_can_sort() => {
                let block = self.spreadsheet_block();
                self.spreadsheet_panel.sort_menu = Some(SortMenu {
                    block,
                    column: block.left,
                    descending: None,
                });
            }
            5 if self.spreadsheet_header().is_none() && populated => {
                self.spreadsheet_panel.show = true;
                self.spreadsheet_panel.show_offset = [0, 0];
            }
            _ => {}
        }
        self.dirty();
    }
    fn spreadsheet_refresh(&mut self, ctx: &egui::Context, snapshot: &SessionSnapshot) {
        if self.spreadsheet_panel.revision != Some(snapshot.generation) {
            self.spreadsheet_panel.revision = Some(snapshot.generation);
            self.spreadsheet_panel.live = None;
            self.spreadsheet_panel.key = None;
            self.spreadsheet_panel.cell_choice = None;
            self.memory.models.results.remove("Spreadsheet");
        }
        let snapshot = self
            .spreadsheet_panel
            .live
            .as_ref()
            .filter(|(g, _)| *g == snapshot.generation)
            .map_or_else(|| snapshot.clone(), |(_, s)| s.clone());
        let mut app = snapshot.clone();
        app.active_app = "Spreadsheet".into();
        self.spreadsheet_panel.settings = app.effective_app_settings();
        let sheet = &self.memory.models.spreadsheet;
        let viewport = if sheet.has_header_entries() {
            let cursor = &self.memory.spreadsheet_cursor;
            let last_column = view::axes(sheet, cursor.first_column, true, cursor.first_row)
                .last()
                .map_or(cursor.first_column, |axis| axis.0);
            let last_row = view::axes(sheet, cursor.first_row, false, cursor.first_row)
                .last()
                .map_or(cursor.first_row, |axis| axis.0);
            Some(Block {
                left: cursor.first_column,
                right: last_column,
                top: cursor.first_row,
                bottom: last_row,
            })
        } else {
            None
        };
        let key = (
            snapshot.generation,
            serde_json::json!({
                "sheet": &self.memory.models.spreadsheet,
                "geometry": self.memory.models.geometry_book.as_ref().map(|book| book.access_fingerprint()),
            }),
            viewport,
        );
        if let Some(job) = &self.spreadsheet_panel.job {
            match job.rx.try_recv() {
                Ok(result) => {
                    let job = self.spreadsheet_panel.job.take().unwrap();
                    if job.key == key {
                        match result {
                            Ok(values) => {
                                self.memory
                                    .models
                                    .results
                                    .insert("Spreadsheet".into(), values);
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        }
                        self.spreadsheet_panel.key = Some(key.clone());
                        self.dirty();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.spreadsheet_panel.job = None;
                    self.spreadsheet_panel.key = Some(key.clone());
                    self.error = Some("Spreadsheet worker stopped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.spreadsheet_panel.key.as_ref() != Some(&key)
            && self
                .spreadsheet_panel
                .job
                .as_ref()
                .is_none_or(|j| j.key != key)
        {
            let sheet = self.memory.models.spreadsheet.clone();
            let geometry = self.memory.models.geometry_book.clone().map(Arc::new);
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(sheet.calculate_for_view(&snapshot, flag, geometry, viewport));
                repaint.request_repaint();
            });
            self.spreadsheet_panel.job = Some(Job { key, rx, cancel });
        }
        if self.spreadsheet_panel.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
