use super::{
    A, ChooseBox, DialogResult, E, KeyPress, MathTarget, ModalDialog, NativeTools, Request,
};
use crate::ide::{native::HomeEntry, native_tools::SpreadsheetHistory};
use crate::keymap::{KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_UP};
use ppl_cas::CasValue;
use std::collections::VecDeque;

const AGGREGATE_COMMANDS: [&str; 2] = ["SUM", "AVERAGE"];

pub(super) enum Picker {
    Commands {
        widget: ChooseBox,
        inline: bool,
    },
    History {
        widget: ChooseBox,
        drafts: Vec<Draft>,
        origin: SpreadsheetHistory,
        inline: bool,
    },
}

pub(super) struct Draft {
    source: String,
    row: usize,
    result: bool,
}

impl Picker {
    pub(super) fn commands(inline: bool) -> Self {
        let items = ["Get from Home", "Get from CAS"]
            .into_iter()
            .chain(AGGREGATE_COMMANDS)
            .map(str::to_owned)
            .collect();
        Self::Commands {
            widget: ChooseBox::new("Spreadsheet commands", items, 1),
            inline,
        }
    }

    fn history(
        origin: SpreadsheetHistory,
        entries: &VecDeque<HomeEntry>,
        inline: bool,
    ) -> Option<Self> {
        let mut labels = Vec::new();
        let mut drafts = Vec::new();
        for (row, entry) in entries.iter().enumerate() {
            if !entry.expression_deleted && !entry.expression.is_empty() {
                labels.push(format!("Input   {}", entry.display_expression()));
                drafts.push(Draft {
                    source: entry.expression.clone(),
                    row,
                    result: false,
                });
            }
            if !entry.error && !entry.result_deleted && !entry.recall_result().is_empty() {
                labels.push(format!("Result  {}", entry.display_result()));
                drafts.push(Draft {
                    source: origin.result_source(entry),
                    row,
                    result: true,
                });
            }
        }
        let current = labels.len();
        (current > 0).then(|| Self::History {
            widget: ChooseBox::new(origin.title(), labels, current),
            drafts,
            origin,
            inline,
        })
    }

    pub(super) fn widget(&self) -> &ChooseBox {
        match self {
            Self::Commands { widget, .. } | Self::History { widget, .. } => widget,
        }
    }

    fn widget_mut(&mut self) -> &mut ChooseBox {
        match self {
            Self::Commands { widget, .. } | Self::History { widget, .. } => widget,
        }
    }

    pub(super) fn action(&mut self, action: A) -> Option<DialogResult> {
        let code = match action {
            A::Back => KEY_ESC,
            A::Evaluate => KEY_ENTER,
            A::History(delta) if delta < 0 => KEY_UP,
            A::History(_) => KEY_DOWN,
            A::Edit(E::Left(_) | E::Right(_)) => {
                let Self::History { widget, drafts, .. } = self else {
                    return None;
                };
                let result = matches!(action, A::Edit(E::Right(_)));
                let current = &drafts[widget.selected];
                let index = if result {
                    widget.selected.checked_add(1)
                } else {
                    widget.selected.checked_sub(1)
                };
                let other = drafts.get(index?)?;
                if current.result == result || current.row != other.row {
                    return None;
                }
                if result { KEY_DOWN } else { KEY_UP }
            }
            _ => return None,
        };
        self.widget_mut().handle(KeyPress::code(code))
    }

    pub(super) fn pointer(&mut self, x: i64, y: i64) -> Option<DialogResult> {
        if y >= 220 && x >= 212 {
            return self.action(if x >= 265 { A::Evaluate } else { A::Back });
        }
        if let Self::Commands { widget, .. } = self
            && let Some(index) = widget.item_at(x, y)
        {
            return Some(DialogResult::Chosen(index));
        }
        self.widget_mut().pointer(x, y)
    }
}

impl SpreadsheetHistory {
    fn environment(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Cas => "CAS",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Home => "Get from Home",
            Self::Cas => "Get from CAS",
        }
    }

    fn result_source(self, entry: &HomeEntry) -> String {
        if self.is_cas()
            && let Some(typed) = &entry.typed_result
            && let Ok(value) = serde_json::from_value::<CasValue>(typed.clone())
            && value.validate().is_ok()
        {
            return cas_worksheet_source(&value);
        }
        entry.recall_result().to_owned()
    }
}

// Giac's explicit list syntax is not worksheet source. Keep the exact scalar
// expressions and use the worksheet's collection syntax before loading the draft.
fn cas_worksheet_source(value: &CasValue) -> String {
    fn items(values: &[CasValue]) -> String {
        values
            .iter()
            .map(cas_worksheet_source)
            .collect::<Vec<_>>()
            .join(",")
    }
    match value {
        CasValue::List(values) => format!("{{{}}}", items(values)),
        CasValue::Vector(values) => format!("[{}]", items(values)),
        CasValue::Matrix { cols, values, .. } => format!(
            "[{}]",
            values
                .chunks((*cols).max(1))
                .map(|row| format!("[{}]", items(row)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        value => value.to_string(),
    }
}

impl NativeTools {
    pub fn spreadsheet_tool_picker_open(&self) -> bool {
        self.is_spreadsheet_page() && self.spreadsheet_panel.tool_picker.is_some()
    }

    pub fn spreadsheet_open_command_picker(&mut self, inline: bool) {
        self.spreadsheet_panel.tool_picker = Some(Picker::commands(inline));
        self.dirty();
    }

    pub fn spreadsheet_open_history(
        &mut self,
        origin: SpreadsheetHistory,
        active: Option<&VecDeque<HomeEntry>>,
        inline: bool,
    ) {
        if self.is_spreadsheet_page() {
            let entries = active.unwrap_or(match origin {
                SpreadsheetHistory::Home => &self.memory.home_history,
                SpreadsheetHistory::Cas => &self.memory.cas_history,
            });
            self.spreadsheet_panel.tool_picker = Picker::history(origin, entries, inline);
            self.error = self
                .spreadsheet_panel
                .tool_picker
                .is_none()
                .then(|| format!("{} history is empty", origin.environment()));
            self.dirty();
        }
    }

    pub(super) fn spreadsheet_finish_tool_pick(
        &mut self,
        result: DialogResult,
        requests: &mut Vec<Request>,
    ) {
        match (self.spreadsheet_panel.tool_picker.take(), result) {
            (Some(Picker::Commands { .. }), DialogResult::Chosen(1)) => {
                requests.push(Request::SpreadsheetHistory(SpreadsheetHistory::Home));
            }
            (Some(Picker::Commands { .. }), DialogResult::Chosen(2)) => {
                requests.push(Request::SpreadsheetHistory(SpreadsheetHistory::Cas));
            }
            (Some(Picker::Commands { inline, .. }), DialogResult::Chosen(index)) => {
                if let Some(&name) = AGGREGATE_COMMANDS.get(index - 3) {
                    if !inline {
                        requests.push(self.spreadsheet_edit(true));
                    }
                    requests.push(Request::TypeText(name.into()));
                }
            }
            (
                Some(Picker::History {
                    drafts,
                    origin,
                    inline,
                    ..
                }),
                DialogResult::Chosen(index),
            ) => {
                if let Some(draft) = drafts.into_iter().nth(index - 1) {
                    if !inline {
                        requests.push(Request::EditMath(MathTarget::SpreadsheetCell {
                            address: self.spreadsheet_edit_address(),
                            clear: true,
                            history: Some(origin),
                        }));
                    }
                    requests.push(Request::TypeText(draft.source));
                }
            }
            _ => {}
        }
        self.dirty();
    }
}
