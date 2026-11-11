//! Native calculator views. Computation belongs to the persistent VM session.
pub mod decimal_display;
pub mod expression;
pub mod keys;
pub mod math_layout;
pub mod polar_display;
pub mod sexagesimal;

use expression::ExpressionEditor;
use keys::{AlphaMode, Tool};
use std::collections::VecDeque;

use ppl_graphics::{Grob, text::draw_text};
use serde::{Deserialize, Serialize};

use super::editor::TabId;

pub fn portable_json(value: &ppl_vm::PortableValue) -> serde_json::Value {
    use ppl_vm::PortableValue::*;
    match value {
        Symbolic(v) => serde_json::json!({"type":"symbolic","value":v}),
        Real(v) => serde_json::json!({"type":"real","value":v.to_string()}),
        Integer(v) => serde_json::json!({"type":"integer","value":format!("{v:?}")}),
        Complex(v) => {
            serde_json::json!({"type":"complex","real":v.re.to_string(),"imaginary":v.im.to_string()})
        }
        String(v) => serde_json::json!({"type":"string","value":v}),
        List(v) => {
            serde_json::json!({"type":"list","value":v.iter().map(portable_json).collect::<Vec<_>>()})
        }
        Matrix {
            rows,
            cols,
            vector,
            data,
        } => {
            serde_json::json!({"type":"matrix","rows":rows,"cols":cols,"vector":vector,"value":data.iter().map(portable_json).collect::<Vec<_>>()})
        }
        Grob(v) => serde_json::json!({"type":"grob","index":v}),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum NativeView {
    Home,
    #[default]
    Catalog,
    ProgramOutput,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum NativeCaller {
    Home,
    #[default]
    Catalog,
}

impl NativeCaller {
    pub fn view(self) -> NativeView {
        match self {
            Self::Home => NativeView::Home,
            Self::Catalog => NativeView::Catalog,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum InputOwner {
    IdeModal,
    CalculatorModal,
    Program,
    NativeIdle,
    Firmware,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub tab: Option<TabId>,
    pub revision: u64,
    pub file: String,
    pub name: String,
    pub params: usize,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeEntry {
    pub expression: String,
    pub result: String,
    pub error: bool,
    pub typed_result: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum NativeOperation {
    Home {
        expression: String,
    },
    Program {
        label: String,
        home_expression: Option<String>,
    },
}

#[derive(Debug)]
pub struct NativeUiState {
    pub cas_mode: bool,
    pub edit_title: Option<String>,
    pub view: NativeView,
    pub caller: NativeCaller,
    pub home_input: String,
    pub editor: ExpressionEditor,
    pub alpha: AlphaMode,
    pub user_keys: super::native_user_keys::UserKeyboard,
    pub alpha_default_lower: bool,
    pub pending_tool: Option<Tool>,
    pub sleeping: bool,
    pub history_position: Option<usize>,
    pub history_draft: String,
    pub history: VecDeque<HomeEntry>,
    pub catalog: Vec<CatalogEntry>,
    pub selected: usize,
    pub error: Option<String>,
    pub operation: Option<NativeOperation>,
    pub outcome_handled: bool,
    pub last_outcome: Option<serde_json::Value>,
    pub run_id: u64,
    pub view_generation: u64,
    pub input_epoch: u64,
    pub shifted: bool,
    pub trace: VecDeque<serde_json::Value>,
    trace_sequence: u64,
}

impl Default for NativeUiState {
    fn default() -> Self {
        Self {
            cas_mode: false,
            edit_title: None,
            view: NativeView::Catalog,
            caller: NativeCaller::Catalog,
            home_input: String::new(),
            editor: ExpressionEditor::default(),
            alpha: AlphaMode::Off,
            user_keys: Default::default(),
            alpha_default_lower: false,
            pending_tool: None,
            sleeping: false,
            history_position: None,
            history_draft: String::new(),
            history: VecDeque::new(),
            catalog: Vec::new(),
            selected: 0,
            error: None,
            operation: None,
            outcome_handled: false,
            last_outcome: None,
            run_id: 0,
            view_generation: 0,
            input_epoch: 0,
            shifted: false,
            trace: VecDeque::new(),
            trace_sequence: 0,
        }
    }
}

impl NativeUiState {
    pub fn touch_view(&mut self) {
        self.view_generation = self.view_generation.wrapping_add(1);
    }
    pub fn set_view(&mut self, view: NativeView) {
        if self.view != view {
            self.view = view;
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.shifted = false;
            self.touch_view();
        }
    }
    pub fn idle_caller(&self) -> NativeCaller {
        match self.view {
            NativeView::Home => NativeCaller::Home,
            NativeView::Catalog => NativeCaller::Catalog,
            _ => self.caller,
        }
    }
    pub fn begin(&mut self, operation: NativeOperation, caller: NativeCaller) {
        self.caller = caller;
        self.operation = Some(operation);
        self.outcome_handled = false;
        self.run_id = self.run_id.wrapping_add(1);
        self.error = None;
        self.set_view(NativeView::ProgramOutput);
    }
    pub fn add_history(
        &mut self,
        expression: String,
        result: String,
        error: bool,
        typed_result: Option<serde_json::Value>,
    ) {
        self.history.push_back(HomeEntry {
            expression,
            result,
            error,
            typed_result,
        });
        self.history_position = None;
        self.history_draft.clear();
        self.touch_view();
    }
    pub fn record(&mut self, mut record: serde_json::Value) {
        record["timestamp_unix_ms"] = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis().min(u64::MAX as u128) as u64)
            .into();
        self.trace_sequence = self.trace_sequence.wrapping_add(1);
        record["sequence"] = self.trace_sequence.into();
        record["run_id"] = self.run_id.into();
        record["view_input_epoch"] = self.input_epoch.into();
        record["view"] = serde_json::to_value(self.view).unwrap();
        record["view_generation"] = self.view_generation.into();
        if let Some(path) = std::env::var_os("PPL_IDE_INPUT_TRACE") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{record}");
            }
        }
        self.trace.push_back(record);
        while self.trace.len() > 2048 {
            self.trace.pop_front();
        }
    }
    /// Legacy home_input remains a compatibility mirror for automation and
    /// persisted profiles; all edits pass through the document.
    pub fn edit(&mut self, edit: impl FnOnce(&mut ExpressionEditor)) {
        self.editor.sync_external(&self.home_input);
        edit(&mut self.editor);
        self.home_input = self.editor.text().to_string();
        self.touch_view();
    }
    pub fn history_move(&mut self, delta: i8) {
        if self.history.is_empty() {
            return;
        }
        let next = if delta < 0 {
            match self.history_position {
                Some(index) => index.saturating_sub(1),
                None => {
                    self.history_draft = self.home_input.clone();
                    self.history.len() - 1
                }
            }
        } else {
            let Some(index) = self.history_position else {
                return;
            };
            if index + 1 == self.history.len() {
                let draft = self.history_draft.clone();
                self.edit(|e| e.set_text(&draft));
                self.history_position = None;
                return;
            }
            index + 1
        };
        let expression = self.history[next].expression.clone();
        self.edit(|e| e.set_text(&expression));
        self.history_position = Some(next);
    }
    pub fn history_first_visible(&self) -> usize {
        self.history_position.map_or_else(
            || self.history.len().saturating_sub(4),
            |p| p.saturating_sub(3),
        )
    }
    pub fn supports_key(&self, code: i64) -> bool {
        let Some(action) = keys::resolve_key(code, self.shifted) else {
            return false;
        };
        if matches!(self.view, NativeView::ProgramOutput | NativeView::Error) {
            return matches!(
                action,
                keys::KeyAction::Back
                    | keys::KeyAction::Home
                    | keys::KeyAction::Tool(_)
                    | keys::KeyAction::Shift
                    | keys::KeyAction::Alpha
                    | keys::KeyAction::On
                    | keys::KeyAction::Off
                    | keys::KeyAction::Programs
            );
        }
        true
    }
    fn displayed_editor(&self) -> ExpressionEditor {
        if self.editor.text() == self.home_input {
            self.editor.clone()
        } else {
            ExpressionEditor::from_text(&self.home_input)
        }
    }
    pub fn editor_scroll_x(&self) -> i64 {
        let editor = self.displayed_editor();
        let layout = math_layout::layout(&editor, 34);
        (layout.cursor(editor.cursor()).0 - 294).max(0)
    }
    pub fn editor_byte_at_x(&self, x: i64) -> usize {
        self.editor_byte_at_position(x, 220)
    }
    pub fn editor_byte_at_position(&self, x: i64, y: i64) -> usize {
        let editor = self.displayed_editor();
        let layout = math_layout::layout(&editor, 34);
        let (_, cy, h) = layout.cursor(editor.cursor());
        let top = 203 + (34 - layout.height).max(0) / 2 - (cy + h - 34).max(0);
        layout.hit_test(x - 9 + self.editor_scroll_x(), y - top)
    }
    pub fn screen(&self) -> Option<Grob> {
        if self.sleeping {
            let mut screen = Grob::screen();
            screen.fill_rect(0, 0, 319, 239, 0);
            return Some(screen);
        }
        if self.view == NativeView::ProgramOutput {
            return None;
        }
        let mut screen = Grob::screen();
        screen.fill_rect(0, 0, 319, 23, 0x26384a);
        let title = match self.view {
            NativeView::Home => {
                self.edit_title
                    .as_deref()
                    .unwrap_or(if self.cas_mode { "CAS" } else { "Home" })
            }
            NativeView::Catalog => "Programs · open in IDE",
            NativeView::Error => "Error",
            NativeView::ProgramOutput => unreachable!(),
        };
        draw_text(&mut screen, title, 8, 4, 14.0, 0xffffff, Some(304), None);
        match self.view {
            NativeView::Home => {
                let first = self.history_first_visible();
                for (i, entry) in self.history.iter().skip(first).take(4).enumerate() {
                    let y = 29 + i as i64 * 39;
                    if self.history_position == Some(first + i) {
                        screen.fill_rect(3, y - 1, 316, y + 36, 0xe0eaf8);
                    }
                    draw_text(
                        &mut screen,
                        &entry.expression,
                        8,
                        y,
                        11.0,
                        0x555555,
                        Some(304),
                        None,
                    );
                    draw_text(
                        &mut screen,
                        &entry.result,
                        12,
                        y + 16,
                        13.0,
                        if entry.error { 0xaa2222 } else { 0x111111 },
                        Some(300),
                        None,
                    );
                }
                screen.fill_rect(0, 202, 319, 239, 0xeaf0f6);
                let editor = self.displayed_editor();
                let layout = math_layout::layout(&editor, 34);
                let (_, cy, h) = layout.cursor(editor.cursor());
                let top = 1 + (34 - layout.height).max(0) / 2 - (cy + h - 34).max(0);
                let mut entry = Grob::new(320, 38, 0xeaf0f6);
                layout.paint(
                    &mut entry,
                    9 - self.editor_scroll_x(),
                    top,
                    editor.selection(),
                    editor.cursor(),
                );
                screen.blit((0, 202, 320, 240), &entry, (0, 0, 320, 38), None, 255);
                let indicator = format!(
                    "{}{} {}",
                    if self.shifted { "⇧ " } else { "" },
                    self.alpha.indicator(),
                    self.user_keys.mode.indicator()
                );
                draw_text(
                    &mut screen,
                    &indicator,
                    255,
                    5,
                    11.0,
                    0xffffff,
                    Some(60),
                    None,
                );
            }
            NativeView::Catalog => {
                if self.catalog.is_empty() {
                    draw_text(
                        &mut screen,
                        "Open a PPL program in the IDE.",
                        8,
                        40,
                        12.0,
                        0x333333,
                        Some(304),
                        None,
                    );
                }
                let first = self.selected.saturating_sub(7);
                for (i, entry) in self.catalog.iter().skip(first).take(8).enumerate() {
                    let index = first + i;
                    let y = 29 + i as i64 * 22;
                    let selected = index == self.selected;
                    if selected {
                        screen.fill_rect(4, y - 1, 315, y + 20, 0x3366aa);
                    }
                    let label = format!(
                        "{}({})",
                        entry.name,
                        if entry.params == 0 {
                            String::new()
                        } else {
                            format!("{} args", entry.params)
                        }
                    );
                    draw_text(
                        &mut screen,
                        &label,
                        9,
                        y + 2,
                        12.0,
                        if selected { 0xffffff } else { 0x111111 },
                        Some(300),
                        None,
                    );
                }
                draw_text(
                    &mut screen,
                    "Enter: Run    Esc: Home",
                    8,
                    218,
                    11.0,
                    0x555555,
                    Some(304),
                    None,
                );
            }
            NativeView::Error => {
                let chars: Vec<char> = self
                    .error
                    .as_deref()
                    .unwrap_or("Unknown error")
                    .chars()
                    .collect();
                for (i, line) in chars.chunks(43).take(9).enumerate() {
                    draw_text(
                        &mut screen,
                        &line.iter().collect::<String>(),
                        8,
                        31 + i as i64 * 17,
                        12.0,
                        0xaa2222,
                        Some(304),
                        None,
                    );
                }
                draw_text(
                    &mut screen,
                    "Esc: return",
                    8,
                    218,
                    11.0,
                    0x555555,
                    Some(304),
                    None,
                );
            }
            NativeView::ProgramOutput => unreachable!(),
        }
        Some(screen)
    }
}
