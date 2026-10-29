//! Native calculator views. Computation belongs to the persistent VM session.
use std::collections::VecDeque;

use ppl_graphics::{Grob, text::draw_text};
use serde::Serialize;

use super::editor::TabId;

pub fn portable_json(value: &ppl_vm::PortableValue) -> serde_json::Value {
    use ppl_vm::PortableValue::*;
    match value {
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
    pub tab: TabId,
    pub revision: u64,
    pub file: String,
    pub name: String,
    pub params: usize,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
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
    pub view: NativeView,
    pub caller: NativeCaller,
    pub home_input: String,
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
            view: NativeView::Catalog,
            caller: NativeCaller::Catalog,
            home_input: String::new(),
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
        while self.history.len() > 200 {
            self.history.pop_front();
        }
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
    pub fn supports_key(&self, code: i64) -> bool {
        use crate::keymap::*;
        match self.view {
            NativeView::ProgramOutput | NativeView::Error => {
                matches!(code, KEY_ESC | KEY_HOME | KEY_MENU)
            }
            NativeView::Catalog => matches!(
                code,
                KEY_ESC | KEY_HOME | KEY_MENU | KEY_UP | KEY_DOWN | KEY_ENTER | KEY_SHIFT | KEY_1
            ),
            NativeView::Home => {
                crate::dialog::char_of_key(code).is_some()
                    || matches!(
                        code,
                        KEY_ESC
                            | KEY_HOME
                            | KEY_MENU
                            | KEY_ENTER
                            | KEY_DEL
                            | KEY_UP
                            | KEY_DOWN
                            | KEY_SHIFT
                            | KEY_SIN
                            | KEY_COS
                            | KEY_TAN
                            | KEY_LN
                            | KEY_LOG
                            | KEY_SQR
                    )
            }
        }
    }
    pub fn screen(&self) -> Option<Grob> {
        if self.view == NativeView::ProgramOutput {
            return None;
        }
        let mut screen = Grob::screen();
        screen.fill_rect(0, 0, 319, 23, 0x26384a);
        let title = match self.view {
            NativeView::Home => "Home",
            NativeView::Catalog => "Programs · open in IDE",
            NativeView::Error => "Error",
            NativeView::ProgramOutput => unreachable!(),
        };
        draw_text(&mut screen, title, 8, 4, 14.0, 0xffffff, Some(304), None);
        match self.view {
            NativeView::Home => {
                let first = self.history.len().saturating_sub(4);
                for (i, entry) in self.history.iter().skip(first).enumerate() {
                    let y = 29 + i as i64 * 39;
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
                draw_text(
                    &mut screen,
                    &format!("> {}▏", self.home_input),
                    6,
                    208,
                    14.0,
                    0x111111,
                    Some(308),
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
