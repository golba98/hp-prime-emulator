//! Statistics LCD state and the shared C/D-column boundary.
mod plot;
mod view;
use super::{MathTarget, NativeTools, Request};
use crate::ide::{
    native_app_ui::Calculation,
    native_statistics::{self as model, Columns},
};
use ppl_vm::{PortableValue, SessionSnapshot};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Enabled,
    #[default]
    X,
    Y,
    Frequency,
    Fit,
    Expression,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    pub first_row: usize,
    pub first_column: usize,
    pub analysis: usize,
    pub field: Field,
    pub symbolic_scroll: i64,
}
impl Cursor {
    pub fn validate(&self) -> Result<(), String> {
        if self.row >= model::MAX_ROWS
            || self.column >= 10
            || self.first_row >= model::MAX_ROWS
            || self.first_column >= 10
            || self.analysis >= 5
            || !(0..=400).contains(&self.symbolic_scroll)
        {
            return Err("Invalid saved statistics selection".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellDraft {
    pub two: bool,
    pub column: usize,
    pub row: usize,
    pub source: String,
}
#[derive(Default)]
pub(super) struct Panel {
    pub(super) sync_error: Option<String>,
    pub(super) last_generation: Option<u64>,
    last_mode: Option<bool>,
    pinned_generation: Option<u64>,
    busy: bool,
    report: bool,
    report_row: usize,
    report_first: usize,
    equation: Option<String>,
    plot_controls: bool,
    plot_zoom: Option<usize>,
    plot_equation: Option<usize>,
    equation_x: i64,
    equation_y: i64,
    stem_first: usize,
    stem_x: i64,
    menu: Option<Menu>,
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
}
impl Panel {
    fn scroll_stem(&mut self, rows: i64, pixels: i64, count: usize, width: i64) {
        self.stem_first =
            (self.stem_first as i64 + rows).clamp(0, count.saturating_sub(12) as i64) as usize;
        self.stem_x = (self.stem_x + pixels).clamp(0, (width - 310).max(0));
    }
}
pub fn app(two: bool) -> &'static str {
    if two {
        "Statistics 2Var"
    } else {
        "Statistics 1Var"
    }
}
pub fn mode(name: &str) -> Option<bool> {
    match name {
        "Statistics 1Var" => Some(false),
        "Statistics 2Var" => Some(true),
        _ => None,
    }
}
fn real(n: ppl_math::Real) -> Result<f64, String> {
    let value = n.to_f64();
    if !value.is_finite() || (!n.is_zero() && value == 0.) {
        return Err("Statistics value exceeds numerical range".into());
    }
    Ok(value)
}
impl NativeTools {
    pub fn statistics_seed_columns(
        &mut self,
        snapshot: &SessionSnapshot,
    ) -> Result<Vec<(String, PortableValue)>, String> {
        let mut seeds = Vec::new();
        for two in [false, true] {
            let selected = mode(&self.memory.selected_app) == Some(two);
            let has_book = if two {
                self.memory.models.statistics2_book.is_some()
            } else {
                self.memory.models.statistics1_book.is_some()
            };
            let has_legacy = if two {
                !self.memory.models.statistics2.pairs.is_empty()
            } else {
                !self.memory.models.statistics1.samples.is_empty()
            };
            if !selected && !has_book && !has_legacy {
                continue;
            }
            if self.memory.models.ensure_statistics_book(two)? {
                self.statistics_panel.last_generation = None;
            }
            let columns = self.memory.models.statistics_columns(two).unwrap();
            columns.validate()?;
            for (index, data) in columns.0.iter().enumerate() {
                let name = Columns::label(index, two);
                if !data.is_empty() && !snapshot.globals.contains_key(&name) {
                    let values = data
                        .iter()
                        .map(|&v| {
                            ppl_math::Real::from_f64(v)
                                .map(PortableValue::Real)
                                .map_err(|e| e.to_string())
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    seeds.push((name, PortableValue::List(values)));
                }
            }
        }
        Ok(seeds)
    }
    pub fn import_statistics_columns(
        &mut self,
        snapshot: &SessionSnapshot,
    ) -> Result<bool, String> {
        let selected = mode(&self.memory.selected_app);
        if self.statistics_panel.last_generation == Some(snapshot.generation)
            && self.statistics_panel.last_mode == selected
        {
            return Ok(false);
        }
        let mut updates = Vec::new();
        for two in [false, true] {
            if selected != Some(two) {
                continue;
            }
            let Some(existing) = self.memory.models.statistics_columns(two) else {
                continue;
            };
            let mut columns = Columns::default();
            for (index, data) in columns.0.iter_mut().enumerate() {
                let name = Columns::label(index, two);
                if let Some(value) = snapshot.globals.get(&name) {
                    let PortableValue::List(values) = value else {
                        return Err(format!("{name} is not a statistics data list"));
                    };
                    *data = values
                        .iter()
                        .map(|v| {
                            v.to_value()
                                .expect_real(&name)
                                .map_err(|e| e.to_string())
                                .and_then(real)
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                }
            }
            columns.validate()?;
            if existing.0 != columns.0 {
                updates.push((two, columns));
            }
        }
        let changed = !updates.is_empty();
        for (two, columns) in updates {
            *self.memory.models.statistics_columns_mut(two).unwrap() = columns;
            let name = app(two);
            self.memory
                .models
                .results
                .retain(|key, _| key != name && !key.starts_with(&format!("{name}:")));
        }
        self.statistics_panel.last_generation = Some(snapshot.generation);
        self.statistics_panel.last_mode = selected;
        if changed {
            self.dirty();
        }
        Ok(changed)
    }
    pub fn statistics_context_pinned(&self, generation: u64) -> bool {
        self.statistics_panel.pinned_generation == Some(generation)
    }
    pub fn statistics_apply_view_context(
        &mut self,
        snapshot: &SessionSnapshot,
        committed_generation: u64,
    ) {
        if let Some(two) = mode(&self.memory.selected_app) {
            let result = self
                .memory
                .models
                .ensure_statistics_book(two)
                .and_then(|_| {
                    self.statistics_panel.last_generation = None;
                    self.import_statistics_columns(snapshot).map(|_| ())
                });
            self.statistics_sync_error(result.err());
            self.statistics_panel.last_generation = Some(committed_generation);
            self.statistics_panel.pinned_generation = Some(committed_generation);
        }
    }
    pub fn statistics_sync_error(&mut self, error: Option<String>) {
        if self.statistics_panel.sync_error != error {
            self.statistics_panel.sync_error = error;
            self.dirty();
        }
    }
    pub fn statistics_cell_source(
        &self,
        two: bool,
        column: usize,
        row: usize,
        clear: bool,
    ) -> String {
        if clear {
            return String::new();
        }
        if let Some(draft) = &self.memory.statistics_draft
            && (draft.two, draft.column, draft.row) == (two, column, row)
        {
            return draft.source.clone();
        }
        self.memory
            .models
            .statistics_columns(two)
            .and_then(|columns| columns.0.get(column))
            .and_then(|values| values.get(row))
            .and_then(|&n| ppl_math::Real::from_f64(n).ok())
            .map(|n| n.to_string())
            .unwrap_or_default()
    }
    pub fn statistics_cell_request(
        &mut self,
        two: bool,
        column: usize,
        row: usize,
        source: String,
    ) -> Result<Request, String> {
        self.memory.models.ensure_statistics_book(two)?;
        if source.trim().is_empty() {
            return Ok(Request::StatisticsDelete { two, column, row });
        }
        if let Some(number) = ppl_math::Real::parse(&source) {
            return Ok(Request::StatisticsCellValue {
                two,
                column,
                row,
                value: real(number)?,
            });
        }
        self.memory.statistics_draft = Some(CellDraft {
            two,
            column,
            row,
            source: source.clone(),
        });
        Ok(Request::Calculate(Calculation::StatisticsCell {
            two,
            column,
            row,
            source,
            columns: self.memory.models.statistics_columns(two).unwrap().clone(),
        }))
    }
    pub fn statistics_entry_target(&self, two: bool, clear: bool) -> MathTarget {
        let cursor = self.memory.statistics_cursor[usize::from(two)];
        MathTarget::StatisticsCell {
            two,
            column: cursor.column,
            row: cursor.row,
            clear,
        }
    }
    pub fn statistics_cell_committed(&mut self, two: bool, column: usize, row: usize) {
        if self
            .memory
            .statistics_draft
            .as_ref()
            .is_some_and(|draft| (draft.two, draft.column, draft.row) == (two, column, row))
        {
            self.memory.statistics_draft = None;
        }
        let cursor = &mut self.memory.statistics_cursor[usize::from(two)];
        if (cursor.column, cursor.row) == (column, row) {
            cursor.row = (row + 1).min(model::MAX_ROWS - 1);
        }
        self.statistics_panel.last_generation = None;
        self.dirty();
    }
}
#[derive(Clone, Copy)]
enum MenuKind {
    Column(Field),
    Fit,
    More,
    Sort,
}
#[derive(Clone, Copy)]
struct Menu {
    kind: MenuKind,
    index: usize,
    first: usize,
}
impl NativeTools {
    pub fn is_statistics_page(&self) -> bool {
        mode(&self.memory.selected_app).is_some()
            && matches!(
                self.page,
                Some(super::Page::Numeric | super::Page::Symbolic)
            )
    }
    pub fn statistics_busy(&mut self, busy: bool) {
        if self.statistics_panel.busy != busy {
            self.statistics_panel.busy = busy;
            self.dirty();
        }
    }
    fn statistics_invalidate(&mut self, two: bool) {
        let name = app(two);
        self.memory
            .models
            .results
            .retain(|key, _| key != name && !key.starts_with(&format!("{name}:")));
        self.dirty();
    }
    fn statistics_fields(two: bool) -> &'static [Field] {
        if two {
            &[
                Field::Enabled,
                Field::X,
                Field::Y,
                Field::Frequency,
                Field::Fit,
                Field::Expression,
            ]
        } else {
            &[Field::Enabled, Field::X, Field::Frequency]
        }
    }
    fn statistics_menu_items(&self, two: bool, kind: MenuKind) -> Vec<String> {
        match kind {
            MenuKind::Column(field) => std::iter::once(if field == Field::Frequency {
                "1".into()
            } else {
                "None".into()
            })
            .chain((0..10).map(|i| Columns::label(i, two)))
            .collect(),
            MenuKind::Fit => crate::ide::native_regression::FitKind::ALL
                .iter()
                .map(|fit| fit.label().into())
                .collect(),
            MenuKind::More => ["Insert row", "Delete column", "Copy column"]
                .map(str::to_owned)
                .to_vec(),
            MenuKind::Sort => [
                "Column ascending",
                "Column descending",
                "Analysis ascending",
                "Analysis descending",
            ]
            .map(str::to_owned)
            .to_vec(),
        }
    }
    fn statistics_choose(&mut self, two: bool, cursor: Cursor, menu: Menu) -> Vec<Request> {
        let mut requests = Vec::new();
        match menu.kind {
            MenuKind::Column(field) => {
                let value = menu.index.checked_sub(1);
                if two {
                    let analysis = &mut self
                        .memory
                        .models
                        .statistics2_book
                        .as_mut()
                        .unwrap()
                        .analyses[cursor.analysis];
                    match field {
                        Field::X => analysis.x = value,
                        Field::Y => analysis.y = value,
                        Field::Frequency => analysis.frequency = value,
                        _ => {}
                    }
                } else {
                    let analysis = &mut self
                        .memory
                        .models
                        .statistics1_book
                        .as_mut()
                        .unwrap()
                        .analyses[cursor.analysis];
                    match field {
                        Field::X => analysis.data = value,
                        Field::Frequency => analysis.frequency = value,
                        _ => {}
                    }
                }
                self.statistics_invalidate(two);
            }
            MenuKind::Fit => {
                self.memory
                    .models
                    .statistics2_book
                    .as_mut()
                    .unwrap()
                    .analyses[cursor.analysis]
                    .fit = crate::ide::native_regression::FitKind::ALL[menu.index];
                self.statistics_invalidate(two);
            }
            MenuKind::More => {
                let columns = self.memory.models.statistics_columns(two).unwrap();
                let mut data = columns.0[cursor.column].clone();
                match menu.index {
                    0 => {
                        if data.len() >= model::MAX_ROWS {
                            self.error = Some("Statistics column has reached1000 rows".into());
                        } else {
                            data.insert(cursor.row.min(data.len()), 0.);
                            requests.push(Request::StatisticsWrite {
                                two,
                                columns: vec![(cursor.column, data)],
                            });
                        }
                    }
                    1 => requests.push(Request::StatisticsWrite {
                        two,
                        columns: vec![(cursor.column, vec![])],
                    }),
                    2 => requests.push(Request::CopyText(format!(
                        "{{{}}}",
                        data.iter()
                            .map(|&n| canonical(n))
                            .collect::<Vec<_>>()
                            .join(",")
                    ))),
                    _ => {}
                }
            }
            MenuKind::Sort => {
                let columns = self.memory.models.statistics_columns(two).unwrap();
                let mut selected = std::collections::BTreeSet::new();
                let mut key = Some(cursor.column);
                if menu.index >= 2 {
                    if two {
                        let a = &self
                            .memory
                            .models
                            .statistics2_book
                            .as_ref()
                            .unwrap()
                            .analyses[cursor.analysis];
                        key = a.x;
                        selected.extend([a.x, a.y, a.frequency].into_iter().flatten());
                    } else {
                        let a = &self
                            .memory
                            .models
                            .statistics1_book
                            .as_ref()
                            .unwrap()
                            .analyses[cursor.analysis];
                        key = a.data;
                        selected.extend([a.data, a.frequency].into_iter().flatten());
                    }
                }
                if let Some(key) = key {
                    selected.insert(key);
                    let count = columns.0[key].len();
                    if selected.iter().any(|&i| columns.0[i].len() != count) {
                        self.error = Some("Related columns must have equal lengths to sort".into());
                    } else {
                        let mut order = (0..count).collect::<Vec<_>>();
                        order.sort_by(|&a, &b| {
                            let cmp = columns.0[key][a].total_cmp(&columns.0[key][b]);
                            if menu.index % 2 == 1 {
                                cmp.reverse()
                            } else {
                                cmp
                            }
                        });
                        requests.push(Request::StatisticsWrite {
                            two,
                            columns: selected
                                .into_iter()
                                .map(|i| (i, order.iter().map(|&row| columns.0[i][row]).collect()))
                                .collect(),
                        });
                    }
                } else {
                    self.error = Some("Choose the analysis data column before sorting".into());
                }
            }
        }
        requests
    }
    fn statistics_activate(&mut self, two: bool, cursor: Cursor) -> Vec<Request> {
        if self.page == Some(super::Page::Numeric) {
            return vec![Request::EditMath(self.statistics_entry_target(two, false))];
        }
        if cursor.field == Field::Enabled {
            if two {
                let a = &mut self
                    .memory
                    .models
                    .statistics2_book
                    .as_mut()
                    .unwrap()
                    .analyses[cursor.analysis];
                a.enabled = !a.enabled;
            } else {
                let a = &mut self
                    .memory
                    .models
                    .statistics1_book
                    .as_mut()
                    .unwrap()
                    .analyses[cursor.analysis];
                a.enabled = !a.enabled;
            }
            self.statistics_invalidate(two);
            return Vec::new();
        }
        let (kind, index) = match cursor.field {
            Field::X | Field::Y | Field::Frequency => {
                let selected = if two {
                    let a = &self
                        .memory
                        .models
                        .statistics2_book
                        .as_ref()
                        .unwrap()
                        .analyses[cursor.analysis];
                    match cursor.field {
                        Field::X => a.x,
                        Field::Y => a.y,
                        _ => a.frequency,
                    }
                } else {
                    let a = &self
                        .memory
                        .models
                        .statistics1_book
                        .as_ref()
                        .unwrap()
                        .analyses[cursor.analysis];
                    if cursor.field == Field::X {
                        a.data
                    } else {
                        a.frequency
                    }
                };
                (
                    MenuKind::Column(cursor.field),
                    selected.map_or(0, |i| i + 1),
                )
            }
            Field::Fit if two => (
                MenuKind::Fit,
                self.memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis]
                    .fit
                    .code() as usize
                    - 1,
            ),
            Field::Expression if two => {
                if self
                    .memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis]
                    .fit
                    == crate::ide::native_regression::FitKind::UserDefined
                {
                    return vec![Request::EditMath(MathTarget::StatisticsFit {
                        analysis: cursor.analysis,
                    })];
                }
                self.error = Some("Choose User Defined to edit the fit expression".into());
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        self.statistics_panel.menu = Some(Menu {
            kind,
            index,
            first: index.saturating_sub(8),
        });
        self.dirty();
        Vec::new()
    }
    fn statistics_show_field(&mut self, two: bool, cursor: Cursor, evaluate: bool) {
        let source = if two && matches!(cursor.field, Field::Expression | Field::Fit) {
            self.statistics_fit_equation(cursor.analysis)
        } else {
            let selected = if two {
                let a = &self
                    .memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis];
                match cursor.field {
                    Field::Y => a.y,
                    Field::Frequency => a.frequency,
                    _ => a.x,
                }
            } else {
                let a = &self
                    .memory
                    .models
                    .statistics1_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis];
                if cursor.field == Field::Frequency {
                    a.frequency
                } else {
                    a.data
                }
            };
            match selected {
                Some(column) if evaluate => format!(
                    "{{{}}}",
                    self.memory.models.statistics_columns(two).unwrap().0[column]
                        .iter()
                        .map(|&n| canonical(n))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Some(column) => Columns::label(column, two),
                None if cursor.field == Field::Frequency => "1".into(),
                None => String::new(),
            }
        };
        self.statistics_panel.equation = Some(source);
        self.statistics_panel.equation_x = 0;
        self.statistics_panel.equation_y = 0;
        self.dirty();
    }
    pub fn statistics_reset_popups(&mut self) {
        self.statistics_panel.menu = None;
        self.statistics_panel.report = false;
        self.statistics_panel.equation = None;
        self.statistics_panel.plot_equation = None;
        self.statistics_panel.plot_zoom = None;
    }
    pub fn statistics_text(&mut self, text: &str) -> Option<Vec<Request>> {
        let two = mode(&self.memory.selected_app)?;
        if !self.is_statistics_page() {
            return None;
        }
        if self.statistics_panel.busy
            || self.statistics_panel.sync_error.is_some()
            || self.statistics_panel.menu.is_some()
            || self.statistics_panel.report
            || self.statistics_panel.equation.is_some()
        {
            return Some(Vec::new());
        }
        if let Err(error) = self.memory.models.ensure_statistics_book(two) {
            self.error = Some(error);
            return Some(Vec::new());
        }
        let cursor = self.memory.statistics_cursor[usize::from(two)];
        if self.page == Some(super::Page::Numeric) {
            return Some(vec![
                Request::EditMath(self.statistics_entry_target(two, true)),
                Request::TypeText(text.into()),
            ]);
        }
        if two
            && cursor.field == Field::Expression
            && self.memory.models.statistics2_book.as_ref()?.analyses[cursor.analysis].fit
                == crate::ide::native_regression::FitKind::UserDefined
        {
            return Some(vec![
                Request::EditMath(MathTarget::StatisticsFit {
                    analysis: cursor.analysis,
                }),
                Request::TypeText(text.into()),
            ]);
        }
        Some(Vec::new())
    }
    pub fn statistics_action(
        &mut self,
        action: crate::ide::native::keys::KeyAction,
    ) -> Option<Vec<Request>> {
        use crate::ide::native::{
            expression::{EditCommand as E, TemplateKind},
            keys::KeyAction as A,
        };
        let two = mode(&self.memory.selected_app)?;
        if !self.is_statistics_page() {
            return None;
        }
        if let Err(error) = self.memory.models.ensure_statistics_book(two) {
            self.error = Some(error);
            return Some(Vec::new());
        }
        let i = usize::from(two);
        let mut cursor = self.memory.statistics_cursor[i];
        let mut requests = Vec::new();
        if matches!(action, A::Home | A::Tool(_)) {
            self.statistics_panel.menu = None;
            self.statistics_panel.report = false;
            self.statistics_panel.equation = None;
            return None;
        }
        if self.statistics_panel.busy {
            return Some(Vec::new());
        }
        if self.statistics_panel.sync_error.is_some() {
            return Some(if matches!(action, A::Back | A::On | A::Evaluate) {
                vec![Request::Close]
            } else {
                Vec::new()
            });
        }
        if let Some(mut menu) = self.statistics_panel.menu {
            match action {
                A::History(delta) => {
                    let count = self.statistics_menu_items(two, menu.kind).len();
                    menu.index = (menu.index as i64 + if delta < 0 { -1 } else { 1 })
                        .rem_euclid(count as i64) as usize;
                    menu.first = menu.first.min(menu.index);
                    if menu.index >= menu.first + 9 {
                        menu.first = menu.index - 8;
                    }
                    self.statistics_panel.menu = Some(menu);
                }
                A::Evaluate => {
                    self.statistics_panel.menu = None;
                    requests = self.statistics_choose(two, cursor, menu);
                }
                A::Back | A::On => self.statistics_panel.menu = None,
                _ => {}
            }
        } else if let Some(source) = &self.statistics_panel.equation {
            let layout = crate::ide::native::math_layout::expression(source, 17.);
            match action {
                A::Back | A::On | A::Evaluate => self.statistics_panel.equation = None,
                A::History(delta) => {
                    self.statistics_panel.equation_y = (self.statistics_panel.equation_y
                        + 20 * delta as i64)
                        .clamp(0, (layout.height - 176).max(0))
                }
                A::Edit(E::Left(_)) => {
                    self.statistics_panel.equation_x =
                        (self.statistics_panel.equation_x - 20).max(0)
                }
                A::Edit(E::Right(_)) => {
                    self.statistics_panel.equation_x =
                        (self.statistics_panel.equation_x + 20).min((layout.width - 300).max(0))
                }
                _ => {}
            }
        } else if self.statistics_panel.report {
            match action {
                A::History(delta) => {
                    self.statistics_panel.report_row =
                        (self.statistics_panel.report_row as i64 + delta as i64).clamp(0, 30)
                            as usize
                }
                A::Edit(E::Left(_)) => {
                    self.statistics_panel.report_first =
                        self.statistics_panel.report_first.saturating_sub(1)
                }
                A::Edit(E::Right(_)) => {
                    self.statistics_panel.report_first =
                        (self.statistics_panel.report_first + 1).min(4)
                }
                A::Back | A::On | A::Evaluate => self.statistics_panel.report = false,
                _ => {}
            }
        } else if self.page == Some(super::Page::Numeric) {
            let columns = self.memory.models.statistics_columns(two)?;
            match action {
                A::History(delta) => {
                    cursor.row = (cursor.row as i64 + delta as i64).clamp(
                        0,
                        columns.0[cursor.column].len().min(model::MAX_ROWS - 1) as i64,
                    ) as usize
                }
                A::Edit(E::Left(_)) => cursor.column = cursor.column.saturating_sub(1),
                A::Edit(E::Right(_)) => cursor.column = (cursor.column + 1).min(9),
                A::Evaluate => requests = self.statistics_activate(two, cursor),
                A::Insert(text) => return self.statistics_text(text),
                A::Template(template) => {
                    requests = vec![
                        Request::EditMath(self.statistics_entry_target(two, true)),
                        Request::InsertTemplate(template),
                    ]
                }
                A::Fraction => {
                    requests = vec![
                        Request::EditMath(self.statistics_entry_target(two, true)),
                        Request::InsertTemplate(TemplateKind::Fraction),
                    ]
                }
                A::Edit(E::Backspace | E::Delete) => {
                    if cursor.row < columns.0[cursor.column].len() {
                        requests.push(Request::StatisticsDelete {
                            two,
                            column: cursor.column,
                            row: cursor.row,
                        });
                    }
                }
                A::Copy => requests.push(Request::CopyText(self.statistics_cell_source(
                    two,
                    cursor.column,
                    cursor.row,
                    false,
                ))),
                A::Back | A::On => requests.push(Request::Close),
                _ => {}
            }
        } else {
            let fields = Self::statistics_fields(two);
            let position = fields.iter().position(|f| *f == cursor.field).unwrap_or(1);
            match action {
                A::History(delta) => {
                    let next = (cursor.analysis * fields.len() + position) as i64
                        + if delta < 0 { -1 } else { 1 };
                    let next = next.clamp(0, (5 * fields.len() - 1) as i64) as usize;
                    cursor.analysis = next / fields.len();
                    cursor.field = fields[next % fields.len()];
                }
                A::Edit(E::Left(_)) => cursor.field = fields[position.saturating_sub(1)],
                A::Edit(E::Right(_)) => cursor.field = fields[(position + 1).min(fields.len() - 1)],
                A::Evaluate => requests = self.statistics_activate(two, cursor),
                A::Insert(text) => return self.statistics_text(text),
                A::Back | A::On => requests.push(Request::Close),
                _ => {}
            }
        }
        let length = self.memory.models.statistics_columns(two)?.0[cursor.column].len();
        cursor.row = cursor.row.min(length.min(model::MAX_ROWS - 1));
        if cursor.row < cursor.first_row {
            cursor.first_row = cursor.row;
        }
        if cursor.row >= cursor.first_row + 9 {
            cursor.first_row = cursor.row - 8;
        }
        if cursor.column < cursor.first_column {
            cursor.first_column = cursor.column;
        }
        if cursor.column >= cursor.first_column + 4 {
            cursor.first_column = cursor.column - 3;
        }
        cursor.first_column = cursor.first_column.min(6);
        let top = 22
            + cursor.analysis as i64 * 79
            + match cursor.field {
                Field::Fit => 27,
                Field::Expression => 54,
                _ => 0,
            };
        if top - cursor.symbolic_scroll < 20 {
            cursor.symbolic_scroll = top - 20;
        }
        if top + 20 - cursor.symbolic_scroll > 202 {
            cursor.symbolic_scroll = top + 20 - 202;
        }
        self.memory.statistics_cursor[i] = cursor;
        self.dirty();
        Some(requests)
    }
}
fn canonical(n: f64) -> String {
    ppl_math::Real::from_f64(n)
        .map(|n| n.to_string())
        .unwrap_or_else(|_| n.to_string())
}
