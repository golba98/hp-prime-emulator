//! Native calculator views. Computation belongs to the persistent VM session.
pub mod chrome;
pub mod decimal_display;
pub mod expression;
pub mod history;
pub mod home_format;
pub mod keys;
pub mod math_layout;
pub mod polar_display;
pub mod presentation;
pub mod sexagesimal;

use expression::ExpressionEditor;
use keys::{AlphaMode, Tool};
use std::collections::VecDeque;

use ppl_graphics::{Grob, text::draw_text};
use serde::{Deserialize, Serialize};

use super::editor::TabId;

/// Executable calculator source retaining exact values and word metadata.
pub(crate) fn value_source(value: &ppl_vm::Value) -> String {
    home_format::recall(value)
}

pub fn portable_json(value: &ppl_vm::PortableValue) -> serde_json::Value {
    use ppl_vm::PortableValue::*;
    match value {
        Symbolic(v) => serde_json::json!({"type":"symbolic","value":v}),
        Real(v) => {
            let mut value = serde_json::json!({"type":"real","value":v.to_string()});
            let encoded = ppl_vm::session::encode_value(&ppl_vm::PortableValue::Real(*v));
            if let Some(origin) = encoded.get("origin") {
                value["origin"] = origin.clone();
            }
            if v.presentation() == ppl_math::RealPresentation::Sexagesimal {
                value["presentation"] = "sexagesimal".into();
            }
            value
        }
        Integer(v) => {
            serde_json::json!({"type":"integer","value":format!("{v:?}"),"word":ppl_vm::session::encode_value(&ppl_vm::PortableValue::Integer(*v))})
        }
        Complex(v) => {
            let mut value = serde_json::json!({"type":"complex","real":v.re.to_string(),"imaginary":v.im.to_string()});
            for (key, part) in [("real_origin", v.re), ("imaginary_origin", v.im)] {
                let encoded = ppl_vm::session::encode_value(&ppl_vm::PortableValue::Real(part));
                if let Some(origin) = encoded.get("origin") {
                    value[key] = origin.clone();
                }
            }
            match v.presentation() {
                ppl_math::ComplexPresentation::Rectangular => {}
                ppl_math::ComplexPresentation::Sexagesimal => {
                    value["presentation"] = "sexagesimal".into()
                }
                ppl_math::ComplexPresentation::Polar => value["presentation"] = "polar".into(),
            }
            value
        }
        String(v) => serde_json::json!({"type":"string","value":v}),
        Utf16(v) => serde_json::json!({"type":"string","utf16":v}),
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

/// Keep scalar paint text with structural results. History recall retains
/// canonical source while Home settings can refresh the displayed numbers.
pub fn portable_display_json(
    value: &ppl_vm::PortableValue,
    settings: &ppl_vm::Settings,
) -> serde_json::Value {
    fn annotate(
        value: &ppl_vm::PortableValue,
        json: &mut serde_json::Value,
        settings: &ppl_vm::Settings,
    ) {
        use ppl_vm::PortableValue;
        match value {
            PortableValue::List(data) | PortableValue::Matrix { data, .. } => {
                for (value, json) in data.iter().zip(json["value"].as_array_mut().unwrap()) {
                    annotate(value, json, settings);
                }
            }
            _ => json["display"] = value.to_value().string_form(settings.home_format()).into(),
        }
    }
    let mut json = portable_json(value);
    if matches!(
        value,
        ppl_vm::PortableValue::List(_) | ppl_vm::PortableValue::Matrix { .. }
    ) {
        annotate(value, &mut json, settings);
    }
    // Recall belongs to the value, even before a later paint/format refresh.
    home_format::annotate_recall(&mut json, &value.to_value());
    json
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
    /// Painted text can contain grouping marks; recall keeps `result` as source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_display: Option<HistoryDisplay>,
    pub expression: String,
    pub result: String,
    pub error: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub expression_deleted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub result_deleted: bool,
    pub typed_result: Option<serde_json::Value>,
    #[serde(default)]
    pub presentation: Option<AnswerPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryDisplay {
    pub source: String,
    pub text: String,
}
impl HomeEntry {
    pub(crate) fn recall_result(&self) -> &str {
        self.typed_result
            .as_ref()
            .and_then(|value| value["recall"].as_str())
            .unwrap_or(&self.result)
    }

    pub fn display_expression(&self) -> &str {
        // A submitted numeric object is painted in calculator notation; retain
        // its complete original source for recall and exact arithmetic.
        if !self.error
            && let Some(typed) = &self.typed_result
            && typed["type"] == "real"
            && home_format::recover_value(typed).is_some()
            && let (Some(input), Some(text)) = (
                ppl_math::Real::parse(&self.expression),
                typed["value"].as_str(),
            )
            && input.to_string() == text
        {
            return text;
        }
        &self.expression
    }
    pub fn display_result(&self) -> &str {
        self.home_display
            .as_ref()
            .filter(|d| d.source == self.result)
            .map_or(&self.result, |d| d.text.as_str())
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerPresentation {
    Decimal,
    Sexagesimal,
    Polar,
    CommonFraction,
    MixedFraction,
}
impl HomeEntry {
    /// Migrate history written before explicit presentation metadata. Numeric
    /// projections are classified without treating literal text as a format.
    pub(crate) fn legacy_presentation(
        result: &str,
        typed: &serde_json::Value,
    ) -> Option<AnswerPresentation> {
        polar_display::value(typed)?;
        if result.contains('°') {
            return Some(AnswerPresentation::Sexagesimal);
        }
        if result.contains(['∡', '∠']) {
            return Some(AnswerPresentation::Polar);
        }
        let decimal = typed["value"]
            .as_str()
            .filter(|_| typed["type"] == "real")?;
        let fraction = decimal_display::FractionDisplay::from_decimal(decimal)?;
        if result == fraction.common && fraction.common != decimal {
            Some(AnswerPresentation::CommonFraction)
        } else if result == fraction.mixed && fraction.mixed != fraction.common {
            Some(AnswerPresentation::MixedFraction)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn legacy_numeric_history_keeps_its_next_toggle_and_literal_text_is_not_formatted() {
        let real = serde_json::json!({"type":"real","value":"5.25"});
        for (text, expected) in [
            ("21/4", Some(AnswerPresentation::CommonFraction)),
            ("5+1/4", Some(AnswerPresentation::MixedFraction)),
            ("5.25", None),
            ("5°15′00″", Some(AnswerPresentation::Sexagesimal)),
            ("5.25∠0", Some(AnswerPresentation::Polar)),
        ] {
            assert_eq!(HomeEntry::legacy_presentation(text, &real), expected);
        }
        assert_eq!(
            HomeEntry::legacy_presentation(
                "5°15′00″",
                &serde_json::json!({
            "type":"string","value":"5°15′00″"})
            ),
            None
        );
    }
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
    pub presentation: presentation::PresentationSettings,
    /// Default Prime history uses fourteen pixels; bundled fallback is calibrated separately.
    pub history_font_px: f32,
    pub history_format_stamp: String,
    pub cas_mode: bool,
    pub app_title: String,
    pub status_clock: String,
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
    pub history_result: bool,
    pub history_viewer: Option<history::HistoryViewer>,
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
            presentation: Default::default(),
            history_font_px: 13.,
            history_format_stamp: String::new(),
            cas_mode: false,
            app_title: "Function".into(),
            status_clock: String::new(),
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
            history_result: true,
            history_viewer: None,
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
            home_display: None,
            expression,
            result,
            error,
            expression_deleted: false,
            result_deleted: false,
            presentation: typed_result.as_ref().and_then(|value| {
                match value["presentation"].as_str() {
                    Some("sexagesimal") => Some(AnswerPresentation::Sexagesimal),
                    Some("polar") => Some(AnswerPresentation::Polar),
                    _ => None,
                }
            }),
            typed_result,
        });
        self.history_position = None;
        self.history_viewer = None;
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
        self.history_position = None;
        self.editor.sync_external(&self.home_input);
        edit(&mut self.editor);
        self.home_input = self.editor.text().to_string();
        self.touch_view();
    }
    pub fn history_first_visible(&self) -> usize {
        self.history_rows().first().map_or(0, |r| r.index)
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
    pub fn displayed_editor(&self) -> ExpressionEditor {
        if self.editor.text() == self.home_input {
            self.editor.clone()
        } else {
            ExpressionEditor::from_text(&self.home_input)
        }
    }
    pub fn entry_layout(&self, editor: &ExpressionEditor) -> math_layout::MathLayout {
        match self.presentation.entry {
            presentation::EntryMode::Textbook => {
                let tall = editor.templates().iter().any(|t| {
                    matches!(
                        t.kind,
                        expression::TemplateKind::Palette(
                            expression::palette::Form::Sum
                                | expression::palette::Form::Product
                                | expression::palette::Form::Limit
                        )
                    )
                });
                math_layout::layout(editor, if tall { 96 } else { 36 })
            }
            presentation::EntryMode::Algebraic => math_layout::linear(editor),
        }
    }
    pub fn editor_scroll_x(&self) -> i64 {
        let editor = self.displayed_editor();
        let layout = self.entry_layout(&editor);
        (layout.cursor(editor.cursor()).0 - 294).max(0)
    }
    pub fn editor_byte_at_x(&self, x: i64) -> usize {
        self.editor_byte_at_position(x, 200)
    }
    pub fn editor_byte_at_position(&self, x: i64, y: i64) -> usize {
        let editor = self.displayed_editor();
        let layout = self.entry_layout(&editor);
        let (_, cy, h) = layout.cursor(editor.cursor());
        let entry_top = self.entry_top();
        let height = chrome::SOFTKEY_TOP - entry_top - 1;
        let top = entry_top + 1 + (height - layout.height).max(0) / 2 - (cy + h - height).max(0);
        layout.hit_test(x - 1 + self.editor_scroll_x(), y - top)
    }
    pub fn store_to_variable(&mut self) {
        let prefix = if self.home_input.is_empty() {
            "Ans"
        } else {
            ""
        };
        self.edit(|editor| editor.insert(&format!("{prefix}▶")));
    }
    pub fn entry_top(&self) -> i64 {
        let layout = self.entry_layout(&self.displayed_editor());
        chrome::SOFTKEY_TOP - (layout.height + if layout.height > 17 { 2 } else { 1 }).clamp(18, 76)
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
        let title = match self.view {
            NativeView::Home => self.edit_title.as_deref().unwrap_or(if self.cas_mode {
                "CAS"
            } else {
                &self.app_title
            }),
            NativeView::Catalog => "Programs · open in IDE",
            NativeView::Error => "Error",
            NativeView::ProgramOutput => unreachable!(),
        };
        chrome::header(&mut screen, title);
        if self.view == NativeView::Home && self.edit_title.is_none() {
            draw_text(
                &mut screen,
                &self.status_clock,
                280,
                0,
                8.,
                0xffffff,
                Some(30),
                None,
            );
        }
        match self.view {
            NativeView::Home => {
                if let Some(viewer) = &self.history_viewer {
                    self.paint_history_viewer(&mut screen, viewer);
                    return Some(screen);
                }
                self.paint_history(&mut screen);
                let entry_top = self.entry_top();
                screen.line(0, entry_top, 319, entry_top, 0);
                let height = chrome::SOFTKEY_TOP - entry_top - 1;
                let editor = self.displayed_editor();
                let layout = self.entry_layout(&editor);
                let (_, cy, h) = layout.cursor(editor.cursor());
                let top = (height - layout.height).max(0) / 2 - (cy + h - height).max(0);
                let mut entry = Grob::new(320, height as usize, 0xffffff);
                layout.paint(
                    &mut entry,
                    1 - self.editor_scroll_x(),
                    top,
                    editor.selection(),
                    editor.cursor(),
                );
                screen.blit(
                    (0, entry_top + 1, 320, chrome::SOFTKEY_TOP),
                    &entry,
                    (0, 0, 320, height),
                    None,
                    255,
                );
                history::paint_softkeys(
                    &mut screen,
                    &[
                        "Sto ▸",
                        if self.cas_mode { "Simplify" } else { "" },
                        if self.cas_mode { "=" } else { "" },
                        "",
                        if self.history.is_empty() { "" } else { "Copy" },
                        if self.history.is_empty() { "" } else { "Show" },
                    ],
                );
                let indicator = format!(
                    "{}{} {}",
                    if self.shifted { "⇧ " } else { "" },
                    self.alpha.indicator(),
                    self.user_keys.mode.indicator()
                );
                draw_text(
                    &mut screen,
                    &indicator,
                    274,
                    9,
                    8.0,
                    0xffffff,
                    Some(44),
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
