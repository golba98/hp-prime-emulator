//! Function's measured Numeric view and its own numeric setup/settings.
mod table;
mod view;
use super::{MathTarget, NativeTools, Page, Request};
use crate::ide::native::{
    expression::{EditCommand as E, ExpressionEditor},
    keys::KeyAction as A,
};
use ppl_math::Real;
use ppl_vm::{AppNumericSettings, PortableValue, SessionSnapshot};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use table::TypedTableJob;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub row: usize,
    pub column: usize,
    pub first: usize,
    pub horizontal_scroll: usize,
    pub font: usize,
    pub focus: usize,
    #[serde(skip, default = "numeric_defaults")]
    pub settings: AppNumericSettings,
    pub definition: bool,
    pub selecting: bool,
    pub anchor: Option<[usize; 2]>,
    pub headers: bool,
}
fn numeric_defaults() -> AppNumericSettings {
    AppNumericSettings::for_app("Function")
}
impl Default for Memory {
    fn default() -> Self {
        Self {
            row: 0,
            column: 0,
            first: 0,
            horizontal_scroll: 0,
            font: 1,
            focus: 0,
            settings: numeric_defaults(),
            definition: false,
            selecting: false,
            anchor: None,
            headers: false,
        }
    }
}
impl Memory {
    pub fn validate(&self) -> Result<(), String> {
        if self.row >= 1000
            || self.column > 10
            || self.first >= 1000
            || self.horizontal_scroll > 600
            || self.font > 2
            || self.focus > 3
            || self.anchor.is_some_and(|a| a[0] > 10 || a[1] >= 1000)
        {
            return Err("Invalid Function numeric settings".into());
        }
        self.settings.validate()?;
        Ok(())
    }
    fn pixels(&self) -> i64 {
        [10, 14, 18][self.font.min(2)]
    }
    fn line(&self) -> usize {
        (self.pixels() + 3) as usize
    }
    fn header(&self) -> usize {
        20 + self.line()
    }
    fn visible(&self) -> usize {
        (203 - self.header()).div_ceil(self.line())
    }
    fn selected_row(&self) -> usize {
        if self.settings.kind == 1 {
            self.first + self.row
        } else {
            self.row
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Value,
    Goto,
    Start,
    Step,
    Zoom,
    SwapFirst,
    SwapSecond,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    Zoom,
    More,
    Select,
    Font,
    Delete,
    Sort,
    Type,
}
struct Menu {
    kind: MenuKind,
    index: usize,
}
#[derive(Default)]
pub(super) struct Panel {
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
    menu: Option<Menu>,
    editor: Option<ExpressionEditor>,
    field: Option<Field>,
    scroll: i64,
    pub(super) job: Option<NumberJob>,
    undo: Option<(Real, Real)>,
    plot_confirmation: bool,
    busy: bool,
    swap_rows: Option<[usize; 2]>,
    swap_focus: usize,
    revision: Option<u64>,
    table_job: Option<TypedTableJob>,
    table_values: Vec<Vec<Option<PortableValue>>>,
}
pub(super) struct NumberJob {
    source: String,
    field: Field,
    generation: u64,
    graph: serde_json::Value,
    rx: mpsc::Receiver<Result<Real, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for NumberJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl NativeTools {
    pub fn is_function_numeric_page(&self) -> bool {
        self.memory.selected_app == "Function"
            && matches!(self.page, Some(Page::Numeric | Page::NumericSetup))
    }
    pub fn function_numeric_apply_context(&mut self, snapshot: &SessionSnapshot) {
        if snapshot.active_app != "Function" {
            return;
        }
        self.memory.function_numeric.settings = snapshot
            .app_numeric_settings
            .get("Function")
            .cloned()
            .unwrap_or_else(numeric_defaults);
        self.numeric_panel.revision = Some(snapshot.generation);
    }
    pub fn function_numeric_editor(
        &mut self,
        editor: Option<ExpressionEditor>,
        field: Option<Field>,
    ) {
        self.numeric_panel.editor = editor;
        self.numeric_panel.field = field;
    }
    pub fn function_numeric_busy(&mut self, busy: bool) {
        self.numeric_panel.busy = busy;
    }
    pub fn function_numeric_close(&mut self) {
        self.numeric_panel.job = None;
        self.numeric_panel.table_job = None;
        self.numeric_panel.swap_rows = None;
        self.numeric_panel.menu = None;
        self.numeric_panel.plot_confirmation = false;
    }
    pub fn function_numeric_cancel(&mut self) {
        self.numeric_panel.job = None;
    }
    pub fn function_numeric_source(&self, field: Field) -> String {
        let state = &self.memory.function_numeric;
        let value = match field {
            Field::SwapFirst | Field::SwapSecond => {
                return self
                    .numeric_panel
                    .swap_rows
                    .map_or(1, |rows| rows[usize::from(field == Field::SwapSecond)])
                    .to_string();
            }
            Field::Start => state.settings.start,
            Field::Step => state.settings.step,
            Field::Zoom => state.settings.zoom,
            _ if state.settings.kind == 1 => state
                .settings
                .independent
                .get(state.selected_row())
                .copied()
                .unwrap_or(Real::ZERO),
            _ => {
                return ppl_math::with_overflow_policy(ppl_math::OverflowPolicy::Saturate, || {
                    state
                        .settings
                        .step
                        .mul(Real::from_i64(state.row as i64))
                        .and_then(|offset| state.settings.start.add(offset))
                })
                .map_or_else(|_| String::new(), |value| value.ppl_source());
            }
        };
        value.ppl_source()
    }
    fn function_numeric_source_key(&self) -> serde_json::Value {
        serde_json::json!({
            "curves": &self.memory.graphs["Function"].curves,
            "sheet": &self.memory.models.spreadsheet,
            "geometry": self.memory.models.geometry_book.as_ref().map(|book| book.access_fingerprint()),
        })
    }
    pub fn function_numeric_submit(
        &mut self,
        field: Field,
        source: String,
        snapshot: &SessionSnapshot,
    ) {
        self.numeric_panel.job = None;
        let graph = self.function_numeric_source_key();
        let function = self.memory.graphs["Function"].clone();
        let sheet = self.memory.models.spreadsheet.clone();
        let geometry = self.memory.models.geometry_book.clone().map(Arc::new);
        let mut context = snapshot.clone();
        context.active_app = "Function".into();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let text = source.clone();
        std::thread::spawn(move || {
            let result = crate::ide::native_application_access::evaluate_expression(
                &text, sheet, function, geometry, &context, flag,
            )
            .map_err(|e| e.to_string())
            .and_then(|value| {
                let real = value
                    .expect_real("Numeric field")
                    .map_err(|e| e.to_string())?;
                if field == Field::Step && real <= Real::ZERO {
                    Err("Num step must be positive".into())
                } else if field == Field::Zoom && real <= Real::ONE {
                    Err("Num zoom must be greater than1".into())
                } else {
                    Ok(real)
                }
            });
            let _ = tx.send(result);
        });
        self.error = None;
        self.numeric_panel.job = Some(NumberJob {
            source,
            field,
            generation: snapshot.generation,
            graph,
            rx,
            cancel,
        });
        self.dirty();
    }
    pub(super) fn function_numeric_poll(&mut self, snapshot: &SessionSnapshot) {
        self.function_numeric_poll_table(snapshot);
        let Some(job) = &self.numeric_panel.job else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Numeric calculation stopped".into()),
        };
        let job = self.numeric_panel.job.take().unwrap();
        let current = self.function_numeric_source_key();
        if job.generation != snapshot.generation
            || job.graph != current
            || self.numeric_panel.field != Some(job.field)
            || self
                .numeric_panel
                .editor
                .as_ref()
                .is_none_or(|e| e.text() != job.source)
        {
            self.error = Some("Numeric input changed during calculation".into());
            self.dirty();
            return;
        }
        match result {
            Ok(value) => {
                if let Err(error) = self.function_numeric_commit(job.field, value, snapshot) {
                    self.error = Some(error);
                } else {
                    self.pending_actions.push(Request::FinishMath(false));
                }
            }
            Err(error) => self.error = Some(error),
        }
        self.dirty();
    }
    fn function_numeric_commit(
        &mut self,
        field: Field,
        value: Real,
        snapshot: &SessionSnapshot,
    ) -> Result<(), String> {
        if matches!(field, Field::SwapFirst | Field::SwapSecond) {
            let row = value
                .to_i64()
                .filter(|n| {
                    *n >= 1
                        && *n as usize <= self.memory.function_numeric.settings.independent.len()
                })
                .ok_or_else(|| {
                    format!(
                        "Choose row1 through{}",
                        self.memory.function_numeric.settings.independent.len()
                    )
                })?;
            if let Some(rows) = &mut self.numeric_panel.swap_rows {
                rows[usize::from(field == Field::SwapSecond)] = row as usize;
                return Ok(());
            }
            return Err("Swap Rows was closed".into());
        }
        let mut state = self.memory.function_numeric.clone();
        match field {
            Field::Start => {
                state.settings.start = value;
                state.row = 0;
            }
            Field::Step => state.settings.step = value,
            Field::Zoom => state.settings.zoom = value,
            Field::Value if state.settings.kind == 1 => {
                let row = state.selected_row();
                if row >= 1000 {
                    return Err("Custom numeric table limit reached".into());
                }
                while state.settings.independent.len() <= row {
                    state.settings.independent.push(Real::ZERO);
                }
                state.settings.independent[row] = value;
                if state.row + 1 < state.visible() {
                    state.row += 1;
                } else {
                    state.first += 1;
                }
            }
            Field::Goto => {
                state.row = 5.min(state.visible() - 1);
                state.settings.start = value
                    .sub(
                        state
                            .settings
                            .step
                            .mul(Real::from_i64(state.row as i64))
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
            }
            Field::Value => {
                state.settings.start = value
                    .sub(
                        state
                            .settings
                            .step
                            .mul(Real::from_i64(state.row as i64))
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?
            }
            Field::SwapFirst | Field::SwapSecond => unreachable!(),
        }
        state.settings.validate()?;
        self.memory.function_numeric = state;
        self.function_numeric_changed(snapshot);
        Ok(())
    }
    fn function_numeric_changed(&mut self, _snapshot: &SessionSnapshot) {
        self.numeric_panel.menu = None;
        self.error = None;
        self.pending_actions
            .push(Request::FunctionNumericConfiguration(
                self.memory.function_numeric.settings.clone(),
            ));
        self.numeric_panel.table_job = None;
        self.numeric_panel.table_values.clear();
        self.table_job = None;
        self.table.clear();
        self.dirty();
    }
    fn function_numeric_count(&self) -> usize {
        self.memory.graphs["Function"]
            .curves
            .iter()
            .filter(|c| c.enabled && !c.expression.trim().is_empty())
            .count()
    }
    fn function_numeric_move(&mut self, dc: i64, dr: i64, snapshot: &SessionSnapshot) {
        let columns = self.function_numeric_count();
        let state = &mut self.memory.function_numeric;
        if state.selecting && state.anchor.is_none() {
            state.anchor = Some([state.column, state.selected_row()]);
        }
        if !state.selecting {
            state.anchor = None;
            state.headers = false;
        }
        state.column = (state.column as i64 + dc).clamp(0, columns as i64) as usize;
        if state.column > 0 {
            let n = columns.max(1);
            let width = 256usize.div_ceil(n).max(59);
            let left = 65
                + if width == 59 {
                    59 * (state.column - 1)
                } else {
                    (256 * (state.column - 1)).div_ceil(n)
                };
            let right = 64
                + if width == 59 {
                    59 * state.column
                } else {
                    (256 * state.column).div_ceil(n)
                };
            if left < 65 + state.horizontal_scroll {
                state.horizontal_scroll = left - 65;
            }
            if right > 319 + state.horizontal_scroll {
                state.horizontal_scroll = right - 319;
            }
        }
        let next = state.row as i64 + dr;
        let mut changed = false;
        if next < 0 {
            if state.settings.kind == 1 {
                state.first = state.first.saturating_sub(1);
            } else {
                match state.settings.start.sub(state.settings.step) {
                    Ok(value) => {
                        state.settings.start = value;
                        changed = true;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        } else if next >= state.visible() as i64 {
            if state.settings.kind == 1 {
                state.first = (state.first + 1).min(state.settings.independent.len().min(999));
            } else {
                match state.settings.start.add(state.settings.step) {
                    Ok(value) => {
                        state.settings.start = value;
                        changed = true;
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        } else {
            state.row = next as usize;
        }
        if changed {
            self.function_numeric_changed(snapshot);
        } else {
            self.dirty();
        }
    }
    fn function_numeric_items(&self, kind: MenuKind) -> Vec<String> {
        let state = &self.memory.function_numeric;
        match kind {
            MenuKind::Zoom => {
                let mut v = vec![
                    format!("In   ×{}", state.settings.zoom),
                    format!("Out  ×{}", state.settings.zoom),
                    "Decimal".into(),
                    "Integer".into(),
                    "Trig".into(),
                ];
                if self.numeric_panel.undo.is_some() {
                    v.push("Un-Zoom".into());
                }
                v
            }
            MenuKind::More if state.settings.kind == 1 => [
                "Insert Row",
                "Delete  ›",
                "Select  ›",
                "Selection",
                "Swap Rows",
                "Font Size  ›",
            ]
            .map(str::to_owned)
            .to_vec(),
            MenuKind::More => ["Select  ›", "Selection", "Font Size  ›"]
                .map(str::to_owned)
                .to_vec(),
            MenuKind::Select => ["Row", "Include Headers"].map(str::to_owned).to_vec(),
            MenuKind::Font => ["Small Font", "Medium Font", "Large Font"]
                .map(str::to_owned)
                .to_vec(),
            MenuKind::Delete => ["Row", "Column"].map(str::to_owned).to_vec(),
            MenuKind::Sort => ["Ascending", "Descending"].map(str::to_owned).to_vec(),
            MenuKind::Type => ["Automatic", "BuildYourOwn"].map(str::to_owned).to_vec(),
        }
    }
    fn function_numeric_menu(&mut self, kind: MenuKind) {
        self.numeric_panel.menu = Some(Menu {
            kind,
            index: match kind {
                MenuKind::Font => self.memory.function_numeric.font,
                MenuKind::Type => usize::from(self.memory.function_numeric.settings.kind == 1),
                _ => 0,
            },
        });
        self.dirty();
    }
    fn function_numeric_choose(
        &mut self,
        kind: MenuKind,
        index: usize,
        snapshot: &SessionSnapshot,
        out: &mut Vec<Request>,
    ) {
        self.numeric_panel.menu = None;
        if self.numeric_panel.busy
            && matches!(
                kind,
                MenuKind::Zoom | MenuKind::Type | MenuKind::Delete | MenuKind::Sort
            )
        {
            self.error = Some("Calculator is busy".into());
            return;
        }
        match kind {
            MenuKind::Zoom => {
                let old = self.memory.function_numeric.settings.clone();
                let mut next = old.clone();
                let mut row = self.memory.function_numeric.row;
                let selected = if index < 2 {
                    match next
                        .step
                        .mul(Real::from_i64(row as i64))
                        .and_then(|offset| next.start.add(offset))
                    {
                        Ok(value) => Some(value),
                        Err(error) => {
                            self.error = Some(error.to_string());
                            return;
                        }
                    }
                } else {
                    None
                };
                let changed = match index {
                    0 => next.step.div(next.zoom).map(|n| next.step = n),
                    1 => next.step.mul(next.zoom).map(|n| next.step = n),
                    2 => {
                        next.start = Real::ZERO;
                        next.step = Real::parse("0.1").unwrap();
                        row = 0;
                        Ok(())
                    }
                    3 => {
                        next.start = Real::ZERO;
                        next.step = Real::ONE;
                        row = 0;
                        Ok(())
                    }
                    4 => {
                        next.start = Real::ZERO;
                        next.step = match snapshot
                            .app_settings
                            .get("Function")
                            .cloned()
                            .unwrap_or_default()
                            .effective(&snapshot.settings)
                            .angle
                        {
                            ppl_vm::AngleMode::Radians => Real::from_f64(std::f64::consts::PI)
                                .unwrap()
                                .div(Real::from_i64(24))
                                .unwrap(),
                            _ => Real::parse("7.5").unwrap(),
                        };
                        row = 0;
                        Ok(())
                    }
                    _ => {
                        if let Some((start, step)) = self.numeric_panel.undo {
                            next.start = start;
                            next.step = step;
                        }
                        Ok(())
                    }
                };
                if let Err(error) = changed {
                    self.error = Some(error.to_string());
                    return;
                }
                if index < 2 {
                    match next
                        .step
                        .mul(Real::from_i64(row as i64))
                        .and_then(|offset| selected.unwrap().sub(offset))
                    {
                        Ok(n) => next.start = n,
                        Err(e) => {
                            self.error = Some(e.to_string());
                            return;
                        }
                    }
                }
                if let Err(error) = next.validate() {
                    self.error = Some(error);
                    return;
                }
                if index < 5 {
                    self.numeric_panel.undo = Some((old.start, old.step));
                } else {
                    self.numeric_panel.undo = None;
                }
                self.memory.function_numeric.row = row;
                self.memory.function_numeric.settings = next;
                self.function_numeric_changed(snapshot);
            }
            MenuKind::More => {
                let custom = self.memory.function_numeric.settings.kind == 1;
                match if custom {
                    index
                } else {
                    [2, 3, 5][index.min(2)]
                } {
                    0 => {
                        let state = &mut self.memory.function_numeric;
                        if state.settings.independent.len() >= 1000 {
                            self.error = Some("Custom numeric table limit reached".into());
                        } else {
                            state.settings.independent.insert(
                                state.selected_row().min(state.settings.independent.len()),
                                Real::ZERO,
                            );
                            self.function_numeric_changed(snapshot);
                        }
                    }
                    1 => self.function_numeric_menu(MenuKind::Delete),
                    2 => self.function_numeric_menu(MenuKind::Select),
                    3 => {
                        let state = &mut self.memory.function_numeric;
                        state.selecting = !state.selecting;
                        state.anchor = None;
                        self.dirty();
                    }
                    4 => {
                        if self.memory.function_numeric.settings.independent.is_empty() {
                            self.error = Some("Enter at least one custom row".into());
                        } else {
                            let row = (self.memory.function_numeric.selected_row() + 1)
                                .min(self.memory.function_numeric.settings.independent.len());
                            self.numeric_panel.swap_rows = Some([row, row]);
                            self.numeric_panel.swap_focus = 0;
                            self.dirty();
                        }
                    }
                    _ => self.function_numeric_menu(MenuKind::Font),
                }
            }
            MenuKind::Select => {
                let count = self.function_numeric_count();
                let state = &mut self.memory.function_numeric;
                state.anchor = Some([0, state.selected_row()]);
                state.column = count;
                state.headers = index == 1;
                out.push(Request::CopyText(self.function_numeric_copy()));
                self.dirty();
            }
            MenuKind::Font => {
                self.memory.function_numeric.font = index.min(2);
                self.memory.function_numeric.row = self
                    .memory
                    .function_numeric
                    .row
                    .min(self.memory.function_numeric.visible() - 1);
                self.dirty();
            }
            MenuKind::Delete => {
                let state = &mut self.memory.function_numeric;
                let row = state.selected_row();
                if index == 1 {
                    state.settings.independent.clear();
                    state.row = 0;
                    state.first = 0;
                } else if row < state.settings.independent.len() {
                    state.settings.independent.remove(row);
                }
                self.function_numeric_changed(snapshot);
            }
            MenuKind::Sort => {
                self.memory
                    .function_numeric
                    .settings
                    .independent
                    .sort_by(|a, b| a.partial_cmp(b).unwrap());
                if index == 1 {
                    self.memory.function_numeric.settings.independent.reverse();
                }
                self.function_numeric_changed(snapshot);
            }
            MenuKind::Type => {
                self.memory.function_numeric.settings.kind = i64::from(index == 1);
                self.memory.function_numeric.row = 0;
                self.memory.function_numeric.first = 0;
                self.function_numeric_changed(snapshot);
            }
        }
    }
    fn function_numeric_copy(&self) -> String {
        let state = &self.memory.function_numeric;
        let row = state.selected_row();
        let [col0, row0] = state.anchor.unwrap_or([state.column, row]);
        let (left, right) = (col0.min(state.column), col0.max(state.column));
        let (top, bottom) = (row0.min(row), row0.max(row));
        let labels = self.function_numeric_labels();
        let values = &self.numeric_panel.table_values;
        let mut rows = vec![];
        let source = |r: usize, c: usize| {
            values
                .get(r)
                .and_then(|row| row.get(c))
                .and_then(Option::as_ref)
                .map_or_else(
                    || "NaN".into(),
                    |v| {
                        if matches!(v,PortableValue::Complex(z) if !z.im.is_zero()) {
                            "NaN".into()
                        } else {
                            v.to_value().string_form(ppl_vm::value::default_format())
                        }
                    },
                )
        };
        if state.headers {
            rows.push(format!(
                "{{{}}}",
                labels
                    .iter()
                    .skip(left)
                    .take(right - left + 1)
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        for r in top..=bottom.min(values.len().saturating_sub(1)) {
            rows.push(format!(
                "{{{}}}",
                (left..=right)
                    .map(|c| source(r, c))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if state.anchor.is_none() {
            source(row, state.column)
        } else {
            format!("{{{}}}", rows.join(","))
        }
    }
    fn function_numeric_labels(&self) -> Vec<String> {
        std::iter::once("X".into())
            .chain(
                self.memory.graphs["Function"]
                    .curves
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.enabled && !c.expression.trim().is_empty())
                    .map(|(i, _)| format!("F{}", if i == 9 { 0 } else { i + 1 })),
            )
            .collect()
    }
    pub fn function_numeric_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if !self.is_function_numeric_page() {
            return None;
        }
        if self.numeric_panel.menu.is_some() {
            return Some(vec![]);
        }
        if self.numeric_panel.busy {
            return Some(vec![]);
        }
        let field = if self.numeric_panel.swap_rows.is_some() {
            if self.numeric_panel.swap_focus == 0 {
                Field::SwapFirst
            } else {
                Field::SwapSecond
            }
        } else if self.page == Some(Page::NumericSetup) {
            if self.memory.function_numeric.focus == 3 {
                return Some(vec![]);
            }
            [Field::Start, Field::Step, Field::Zoom, Field::Zoom]
                [self.memory.function_numeric.focus]
        } else if self.memory.function_numeric.column == 0 {
            Field::Value
        } else {
            return Some(vec![]);
        };
        Some(vec![
            Request::EditMath(MathTarget::FunctionNumeric { field, clear: true }),
            Request::TypeText(text.into()),
        ])
    }
    pub fn function_numeric_action(
        &mut self,
        action: A,
        snapshot: &SessionSnapshot,
    ) -> Option<Vec<Request>> {
        if !self.is_function_numeric_page() {
            return None;
        }
        if self.numeric_panel.revision != Some(snapshot.generation) {
            self.function_numeric_apply_context(snapshot);
        }
        if matches!(action, A::Home | A::Tool(_)) {
            self.numeric_panel.menu = None;
            self.numeric_panel.swap_rows = None;
            self.numeric_panel.plot_confirmation = false;
            return None;
        }
        let mut out = vec![];
        if self.numeric_panel.swap_rows.is_some() {
            match action {
                A::History(d) => {
                    self.numeric_panel.swap_focus =
                        (self.numeric_panel.swap_focus as i64 + i64::from(d)).clamp(0, 1) as usize
                }
                A::Evaluate => out.push(Request::EditMath(MathTarget::FunctionNumeric {
                    field: if self.numeric_panel.swap_focus == 0 {
                        Field::SwapFirst
                    } else {
                        Field::SwapSecond
                    },
                    clear: false,
                })),
                A::Back | A::On => self.numeric_panel.swap_rows = None,
                A::Insert(text) => return self.function_numeric_text(text),
                _ => {}
            }
            self.dirty();
            return Some(out);
        }
        if self.numeric_panel.plot_confirmation {
            match action {
                A::Evaluate => self.numeric_accept_plot(snapshot),
                A::Back | A::On => self.numeric_panel.plot_confirmation = false,
                _ => {}
            }
            self.dirty();
            return Some(out);
        }
        if let Some(menu) = &mut self.numeric_panel.menu {
            let (kind, index) = (menu.kind, menu.index);
            match action {
                A::History(d) => {
                    let count = self.function_numeric_items(kind).len();
                    self.numeric_panel.menu.as_mut().unwrap().index =
                        (index as i64 + i64::from(d)).clamp(0, count as i64 - 1) as usize;
                }
                A::Evaluate | A::Edit(E::Right(_)) => {
                    self.function_numeric_choose(kind, index, snapshot, &mut out)
                }
                A::Back | A::On | A::Edit(E::Left(_)) => self.numeric_panel.menu = None,
                _ => {}
            }
            self.dirty();
            return Some(out);
        }
        if self.page == Some(Page::NumericSetup) {
            match action {
                A::History(d) => {
                    self.memory.function_numeric.focus = (self.memory.function_numeric.focus as i64
                        + i64::from(d))
                    .clamp(0, 3) as usize
                }
                A::Evaluate | A::Edit(E::Right(_)) => {
                    let focus = self.memory.function_numeric.focus;
                    if focus == 3 {
                        self.function_numeric_menu(MenuKind::Type);
                    } else {
                        out.push(Request::EditMath(MathTarget::FunctionNumeric {
                            field: [Field::Start, Field::Step, Field::Zoom][focus],
                            clear: false,
                        }));
                    }
                }
                A::Edit(E::Backspace | E::Delete) => {
                    match self.memory.function_numeric.focus {
                        0 => self.memory.function_numeric.settings.start = Real::ZERO,
                        1 => {
                            self.memory.function_numeric.settings.step = Real::parse("0.1").unwrap()
                        }
                        2 => self.memory.function_numeric.settings.zoom = Real::from_i64(2),
                        _ => self.memory.function_numeric.settings.kind = 0,
                    };
                    self.function_numeric_changed(snapshot);
                }
                A::Edit(E::Clear) => {
                    self.memory.function_numeric.settings.start = Real::ZERO;
                    self.memory.function_numeric.settings.step = Real::parse("0.1").unwrap();
                    self.memory.function_numeric.settings.zoom = Real::from_i64(2);
                    self.memory.function_numeric.settings.kind = 0;
                    self.function_numeric_changed(snapshot);
                }
                A::Back | A::On => {
                    self.open(Page::Numeric);
                    self.table(snapshot);
                }
                A::Insert(text) => return self.function_numeric_text(text),
                _ => {}
            }
        } else {
            match action {
                A::History(d) => self.function_numeric_move(0, i64::from(d), snapshot),
                A::Edit(E::Left(extend)) => {
                    self.memory.function_numeric.selecting |= extend;
                    self.function_numeric_move(-1, 0, snapshot);
                }
                A::Edit(E::Right(extend)) => {
                    self.memory.function_numeric.selecting |= extend;
                    self.function_numeric_move(1, 0, snapshot);
                }
                A::Copy => out.push(Request::CopyText(self.function_numeric_copy())),
                A::Evaluate if self.memory.function_numeric.column == 0 => {
                    out.push(Request::EditMath(MathTarget::FunctionNumeric {
                        field: Field::Value,
                        clear: false,
                    }))
                }
                A::Edit(E::Backspace | E::Delete)
                    if self.memory.function_numeric.settings.kind == 1 =>
                {
                    self.function_numeric_choose(MenuKind::Delete, 0, snapshot, &mut out)
                }
                A::Insert("+") => {
                    self.function_numeric_choose(MenuKind::Zoom, 0, snapshot, &mut out)
                }
                A::Insert("-" | "−") => {
                    self.function_numeric_choose(MenuKind::Zoom, 1, snapshot, &mut out)
                }
                A::Insert(text) => return self.function_numeric_text(text),
                A::Back | A::On => out.push(Request::Close),
                _ => {}
            }
        }
        self.dirty();
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_zoom_keeps_selected_x_and_overflow_is_an_atomic_rejection() {
        let mut tools = NativeTools::default();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        tools.memory.function_numeric.row = 2;
        tools.function_numeric_choose(MenuKind::Zoom, 0, &snapshot, &mut vec![]);
        let state = &tools.memory.function_numeric.settings;
        assert_eq!(
            (state.start, state.step),
            (Real::parse("0.1").unwrap(), Real::parse("0.05").unwrap())
        );
        assert_eq!(tools.function_numeric_source(Field::Value), "0.2");
        tools.memory.function_numeric.row = 9;
        tools.memory.function_numeric.settings.step = Real::parse("1E499").unwrap();
        let before = (
            tools.memory.function_numeric.row,
            tools.memory.function_numeric.settings.clone(),
        );
        assert!(
            tools
                .function_numeric_commit(Field::Value, Real::parse("-1E499").unwrap(), &snapshot)
                .is_err()
        );
        assert_eq!(
            (
                tools.memory.function_numeric.row,
                tools.memory.function_numeric.settings.clone()
            ),
            before
        );
    }
    #[test]
    fn zoom_rejects_unrepresentable_selected_x_without_losing_settings_or_undo() {
        let mut tools = NativeTools::default();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        tools.memory.function_numeric.settings.start = Real::parse("9E499").unwrap();
        tools.memory.function_numeric.settings.step = Real::parse("1E499").unwrap();
        tools.memory.function_numeric.row = 2;
        tools.numeric_panel.undo = Some((Real::ZERO, Real::ONE));
        let before = tools.memory.function_numeric.clone();
        let undo = tools.numeric_panel.undo;
        tools.function_numeric_choose(MenuKind::Zoom, 0, &snapshot, &mut vec![]);
        assert!(tools.error.is_some());
        assert_eq!(tools.memory.function_numeric.row, before.row);
        assert_eq!(tools.memory.function_numeric.settings, before.settings);
        assert_eq!(tools.numeric_panel.undo, undo);
        assert!(tools.pending_actions.is_empty());
        // Runtime table values saturate; configuration/Zoom remains strict.
        assert_eq!(
            tools.function_numeric_source(Field::Value),
            Real::max_finite(false).ppl_source()
        );
        // Decimal resets an unusable selected X without trying to evaluate it.
        tools.function_numeric_choose(MenuKind::Zoom, 2, &snapshot, &mut vec![]);
        assert!(tools.error.is_none());
        assert_eq!(tools.memory.function_numeric.row, 0);
        assert_eq!(tools.memory.function_numeric.settings.start, Real::ZERO);
        assert_eq!(
            tools.memory.function_numeric.settings.step,
            Real::parse("0.1").unwrap()
        );
    }
    #[test]
    fn plot_copy_calculates_the_span_as_decimal_and_rejects_invalid_bounds_atomically() {
        let mut tools = NativeTools::default();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let window = &mut tools.memory.graphs.get_mut("Function").unwrap().window;
        window.xmin = -1E308;
        window.xmax = 1E308;
        tools.numeric_accept_plot(&snapshot);
        assert!(tools.error.is_none());
        assert_eq!(
            tools.memory.function_numeric.settings.start,
            Real::parse("-1E308").unwrap()
        );
        assert_eq!(
            tools.memory.function_numeric.settings.step,
            Real::parse("6.2893081761E305").unwrap()
        );
        let before = tools.memory.function_numeric.settings.clone();
        tools.pending_actions.clear();
        tools.memory.graphs.get_mut("Function").unwrap().window.xmin = f64::NAN;
        tools.numeric_accept_plot(&snapshot);
        assert!(tools.error.is_some());
        assert_eq!(tools.memory.function_numeric.settings, before);
        assert!(tools.pending_actions.is_empty());
    }
    #[test]
    fn typed_table_keeps_bcd_extremes_real_app_settings_and_complex_results() {
        use crate::ide::native_apps::{GraphApp, GraphKind};
        let mut graph = GraphApp::new(GraphKind::Function);
        graph.curves[0].expression = "X/1E400+NumStep".into();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let (rows, error) = crate::ide::native_application_access::evaluate_numeric_rows(
            Default::default(),
            graph.clone(),
            None,
            &snapshot,
            vec![
                Real::parse("1E400").unwrap(),
                Real::parse("1.1E400").unwrap(),
            ],
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(error.is_none(), "{error:?}");
        assert_eq!(
            rows,
            vec![
                vec![
                    Some(PortableValue::Real(Real::parse("1E400").unwrap())),
                    Some(PortableValue::Real(Real::parse("1.1").unwrap()))
                ],
                vec![
                    Some(PortableValue::Real(Real::parse("1.1E400").unwrap())),
                    Some(PortableValue::Real(Real::parse("1.2").unwrap()))
                ]
            ]
        );
        graph.curves[0].expression = "(X,1)".into();
        let (rows, error) = crate::ide::native_application_access::evaluate_numeric_rows(
            Default::default(),
            graph,
            None,
            &snapshot,
            vec![Real::from_i64(2)],
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(error.is_none());
        assert_eq!(
            rows[0][1],
            Some(PortableValue::Complex(ppl_math::Complex::new(
                Real::from_i64(2),
                Real::ONE
            )))
        );
    }
    #[test]
    fn numeric_domain_nan_keeps_other_rows_and_does_not_become_a_screen_error() {
        use crate::ide::native_apps::{GraphApp, GraphKind};
        let mut graph = GraphApp::new(GraphKind::Function);
        graph.curves[0].expression = "1/X".into();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let (rows, error) = crate::ide::native_application_access::evaluate_numeric_rows(
            Default::default(),
            graph,
            None,
            &snapshot,
            vec![Real::ZERO, Real::ONE],
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(error.is_none());
        assert_eq!(rows[0][1], None);
        assert_eq!(rows[1][1], Some(PortableValue::Real(Real::ONE)));
    }
    #[test]
    fn custom_rows_evaluate_each_entered_argument_and_depend_on_real_function_rows() {
        use crate::ide::native_apps::{Curve, GraphApp, GraphKind};
        let mut graph = GraphApp::new(GraphKind::Function);
        graph.curves[0].expression = "X^2+1".into();
        graph.curves[0].enabled = false;
        graph.curves.push(Curve {
            expression: "F1(X)+2".into(),
            second: String::new(),
            enabled: true,
            sequence: None,
            literal: None,
        });
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        assert_eq!(
            graph
                .table_values_with_cancel(
                    &snapshot,
                    &[2., 4., -1.],
                    Arc::new(AtomicBool::new(false))
                )
                .unwrap(),
            vec![
                vec![Some(2.), Some(7.)],
                vec![Some(4.), Some(19.)],
                vec![Some(-1.), Some(4.)]
            ]
        );
        assert!(
            graph
                .table_values_with_cancel(
                    &snapshot,
                    &[f64::INFINITY],
                    Arc::new(AtomicBool::new(false))
                )
                .is_err()
        );
    }
}
