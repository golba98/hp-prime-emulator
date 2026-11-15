//! Prime worksheet grid interaction; calculations run outside the UI thread.
mod view;
use super::{MathTarget, NativeTools, Page, Request};
use crate::ide::native_app_models::spreadsheet::format::{Block, CellFormat};
use crate::ide::{
    native::expression::EditCommand as E,
    native::keys::KeyAction as A,
    native_app_models::{MAX_COLUMNS, MAX_ROWS, Spreadsheet, cell_coordinates, cell_name},
};
use ppl_vm::SessionSnapshot;
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
    editor_scroll: i64,
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
    menu: Option<Menu>,
    show: bool,
    show_offset: [i64; 2],
    busy: bool,
    revision: Option<u64>,
    live: Option<(u64, SessionSnapshot)>,
    pub(super) job: Option<Job>,
    key: Option<(u64, serde_json::Value)>,
}
#[derive(Clone, Copy)]
struct Menu {
    category: Option<usize>,
    index: usize,
    first: usize,
}
pub(super) struct Job {
    key: (u64, serde_json::Value),
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
    ) {
        self.spreadsheet_panel.editor = editor;
        self.spreadsheet_panel.form = form;
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
            .cells
            .get(address)
            .cloned()
            .unwrap_or_default()
    }
    pub fn spreadsheet_goto(&mut self, source: &str) -> Result<(), String> {
        let block = self.spreadsheet_target(source)?;
        self.spreadsheet_select(block.left, block.top, false);
        if block.left != block.right || block.top != block.bottom {
            self.memory.spreadsheet_cursor.anchor = Some([block.right, block.bottom]);
            self.memory.spreadsheet_cursor.selecting = true;
        } else {
            self.memory.spreadsheet_cursor.selecting = false;
        }
        self.error = None;
        Ok(())
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
        while cursor.first_column < c
            && (cursor.first_column..=c)
                .map(|i| u32::from(*sheet.column_widths.get(&i).unwrap_or(&65)))
                .sum::<u32>()
                > 303
        {
            cursor.first_column += 1;
        }
        while cursor.first_row < r
            && (cursor.first_row..=r)
                .map(|i| u32::from(*sheet.row_heights.get(&i).unwrap_or(&17)))
                .sum::<u32>()
                > 167
        {
            cursor.first_row += 1;
        }
        self.dirty();
    }
    fn spreadsheet_edit(&self, clear: bool) -> Request {
        Request::EditMath(MathTarget::SpreadsheetCell {
            address: self.memory.models.selected_cell.clone(),
            clear,
        })
    }
    pub fn spreadsheet_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if !self.is_spreadsheet_page() {
            return None;
        }
        if self.spreadsheet_panel.busy
            || self.spreadsheet_panel.menu.is_some()
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
            Some(3 | 4) => vec!["Auto", "Black", "Red", "Green", "Blue", "White", "Yellow"],
            Some(5) => vec!["Auto", "Left", "Center", "Right"],
            Some(6) => vec!["Auto", "Top", "Center", "Bottom"],
            Some(7) => vec!["Auto", "Yes", "No"],
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
                1..=7 => {
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
                1 => {
                    style.number = Some(index as i64 - 1);
                    style.digits = Some(if index >= 2 { 6 } else { -1 });
                }
                2 => style.font = Some(if index == 0 { 0 } else { 8 + index as u8 * 2 }),
                c @ (3 | 4) => {
                    let color = [
                        0x1000000, 0, 0xff0000, 0x008000, 0x0000ff, 0xffffff, 0xffff00,
                    ][index];
                    if c == 3 {
                        style.color = Some(color);
                    } else {
                        style.fill = Some(color);
                    }
                }
                5 => style.horizontal = Some(if index == 0 { 3 } else { index as u8 - 1 }),
                6 => style.vertical = Some(if index == 0 { 3 } else { index as u8 - 1 }),
                7 => style.quotes = Some(index as u8),
                _ => return,
            }
            if let Err(e) = self.memory.models.spreadsheet.apply_format(block, style) {
                self.error = Some(e);
            } else {
                self.dirty();
            }
        }
        self.spreadsheet_panel.menu = None;
    }
    pub fn spreadsheet_action(&mut self, action: A) -> Option<Vec<Request>> {
        if !self.is_spreadsheet_page() {
            return None;
        }
        let mut requests = vec![];
        if self.spreadsheet_panel.busy {
            return Some(requests);
        }
        if let Some(mut menu) = self.spreadsheet_panel.menu {
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
                let block = self.spreadsheet_block();
                self.memory.models.spreadsheet.clear_block(block);
                self.spreadsheet_changed(false);
            }
            A::Copy => {
                let block = self.spreadsheet_block();
                if (block.right - block.left + 1) * (block.bottom - block.top + 1) <= 10000 {
                    let text = (block.top..=block.bottom)
                        .map(|row| {
                            (block.left..=block.right)
                                .map(|col| {
                                    self.spreadsheet_cell_text(
                                        col,
                                        row,
                                        &self.spreadsheet_panel.settings,
                                    )
                                    .0
                                })
                                .collect::<Vec<_>>()
                                .join("\t")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    requests.push(Request::CopyText(text));
                } else {
                    self.error = Some("Select up to10000 cells to copy".into());
                }
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
        let populated = self
            .memory
            .models
            .spreadsheet
            .cells
            .contains_key(&self.memory.models.selected_cell);
        match index {
            0 if populated => requests.push(self.spreadsheet_edit(false)),
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
            5 if populated => {
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
        let key = (
            snapshot.generation,
            serde_json::to_value(&self.memory.models.spreadsheet).expect("valid worksheet"),
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
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(sheet.calculate(&snapshot, flag));
                repaint.request_repaint();
            });
            self.spreadsheet_panel.job = Some(Job { key, rx, cancel });
        }
        if self.spreadsheet_panel.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
