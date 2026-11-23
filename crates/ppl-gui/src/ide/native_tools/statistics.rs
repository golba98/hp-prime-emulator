//! Statistics LCD state and the shared C/D-column boundary.
mod cursor_overlay;
mod plot;
mod plot_background;
mod plot_cursor;
mod plot_grid;
mod plot_setup;
pub use plot_setup::PlotSetupField;
mod view;
mod zoom_box;
mod zoom_factors;
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
    Color,
    FitColor,
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
    typed_columns: [Option<[Vec<ppl_math::Real>; model::COLUMN_COUNT]>; 2],
    last_mode: Option<bool>,
    pinned_generation: Option<u64>,
    busy: bool,
    report: bool,
    report_row: usize,
    report_row_offset: i64,
    report_first: usize,
    report_show_last: bool,
    report_selection: [usize; 2],
    report_anchor: Option<[usize; 2]>,
    report_include_headers: bool,
    report_copy_pending: bool,
    equation: Option<String>,
    plot_controls: bool,
    plot_setup: plot_setup::SetupSelection,
    plot_pictures: plot_background::Pictures,
    plot_definition: bool,
    plot_zoom: Option<usize>,
    zoom_box: Option<zoom_box::ZoomBox>,
    zoom_undo: Option<crate::ide::native_apps::PlotWindow>,
    free_plot_cursor: Option<plot_cursor::PlotCursor>,
    cursor_overlay: cursor_overlay::CursorOverlay,
    zoom_factors: Option<zoom_factors::ZoomFactorsDraft>,
    plot_equation: Option<usize>,
    equation_x: i64,
    equation_y: i64,
    menu: Option<Menu>,
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
}
impl Panel {
    fn reveal_report_column(&mut self) {
        match self.report_selection[1] {
            0 | 1 => self.report_show_last = false,
            model::ANALYSIS_COUNT => self.report_show_last = true,
            _ => {}
        }
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
fn stats_two_reference(field: Field) -> Option<usize> {
    match field {
        Field::X => Some(1),
        Field::Y => Some(2),
        Field::Frequency => Some(3),
        _ => None,
    }
}

impl NativeTools {
    pub(super) fn paint_plot_cursor(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        center: egui::Pos2,
        scale: f32,
        mode: ppl_vm::PlotCursorMode,
    ) {
        let offset = (center - rect.min) / scale;
        let center = rect.min + scale * egui::vec2(offset.x.floor() + 0.5, offset.y.floor() + 0.5);
        plot_cursor::paint_at(
            &painter.with_clip_rect(rect),
            center,
            scale,
            mode,
            &self.statistics_panel.cursor_overlay,
        );
    }

    pub(crate) fn release_statistics_cursor_overlay(&self, gl: &eframe::glow::Context) {
        self.statistics_panel.cursor_overlay.release(gl);
    }

    pub(super) fn statistics_reset_free_plot_cursor(&mut self) {
        self.statistics_panel.free_plot_cursor = None;
    }

    pub(super) fn statistics_record_zoom(&mut self, previous: crate::ide::native_apps::PlotWindow) {
        self.statistics_panel.zoom_undo = Some(previous);
    }
    pub(crate) fn statistics_one_summary_open(&self) -> bool {
        self.page == Some(super::Page::Numeric)
            && mode(&self.memory.selected_app) == Some(false)
            && self.statistics_panel.report
    }

    pub(crate) fn statistics_summary_clipboard(
        &self,
        snapshot: &SessionSnapshot,
    ) -> Option<String> {
        if !self.statistics_one_summary_open() || self.statistics_panel.busy {
            return None;
        }
        let analyses = self
            .memory
            .models
            .results
            .get(app(false))?
            .as_object()?
            .keys()
            .filter(|name| name.starts_with('H'))
            .map(String::as_str)
            .collect::<Vec<_>>();
        if analyses.is_empty() {
            return None;
        }
        let settings = snapshot.app_settings.get(app(false)).map_or_else(
            || snapshot.settings.clone(),
            |app| app.effective(&snapshot.settings),
        );
        let [row, column] = self.statistics_panel.report_selection;
        let include_headers = self.statistics_panel.report_include_headers;
        let Some(([first_row, first_column], [last_row, last_column])) = self
            .statistics_report_range()
            .filter(|(first, last)| include_headers || first != last)
        else {
            return self.statistics_report_clipboard_cell(row, column, &analyses, &settings);
        };
        let separator = if settings.digit_grouping.decimal() == ',' {
            ";"
        } else {
            ","
        };
        let mut columns = (first_column..=last_column)
            .map(|column| {
                let mut cells = (first_row..=last_row)
                    .map(|row| {
                        self.statistics_report_clipboard_cell(row, column, &analyses, &settings)
                    })
                    .collect::<Option<Vec<_>>>()?;
                if include_headers {
                    let heading = if column == 0 {
                        ""
                    } else {
                        analyses[column - 1]
                    };
                    cells.insert(0, format!("\"{heading}\""));
                }
                Some(format!("{{{}}}", cells.join(separator)))
            })
            .collect::<Option<Vec<_>>>()?;
        if columns.len() == 1 {
            columns.pop()
        } else {
            Some(format!("{{{}}}", columns.join(separator)))
        }
    }

    fn statistics_report_clipboard_cell(
        &self,
        row: usize,
        column: usize,
        analyses: &[&str],
        settings: &ppl_vm::Settings,
    ) -> Option<String> {
        let (label, path, _) = view::ONE_REPORT_ROWS.get(row)?;
        if column == 0 {
            return Some(format!("\"{label}\""));
        }
        let analysis = analyses.get(column - 1)?;
        let (_, variable) = model::one_data::DISPLAY_FIELDS
            .iter()
            .find(|(field, _)| field == path)?;
        let encoded = self
            .memory
            .models
            .statistics1_book
            .as_ref()?
            .result_values
            .get(*analysis)?
            .get(*variable)?;
        match ppl_vm::session::decode_value(encoded).ok()? {
            PortableValue::Real(value) => {
                let text = value.format(ppl_math::FormatOptions {
                    digit_grouping: None,
                    ..settings.home_format()
                });
                Some(if settings.digit_grouping.decimal() == ',' {
                    text.replace('.', ",")
                } else {
                    text
                })
            }
            PortableValue::Symbolic(ppl_cas::CasValue::Undefined) => Some("NaN".into()),
            PortableValue::Symbolic(ppl_cas::CasValue::Infinity(
                ppl_cas::InfinitySign::Positive,
            )) => Some("+Inf".into()),
            _ => None,
        }
    }

    fn statistics_report_range(&self) -> Option<([usize; 2], [usize; 2])> {
        let [row, column] = self.statistics_panel.report_selection;
        let [anchor_row, anchor_column] = self.statistics_panel.report_anchor?;
        Some((
            [row.min(anchor_row), column.min(anchor_column)],
            [row.max(anchor_row), column.max(anchor_column)],
        ))
    }

    fn statistics_report_analysis_count(&self) -> usize {
        self.memory
            .models
            .results
            .get(app(false))
            .and_then(serde_json::Value::as_object)
            .map_or(0, |result| {
                result.keys().filter(|name| name.starts_with('H')).count()
            })
    }

    fn statistics_move_report_selection(&mut self, rows: isize, columns: isize) {
        let available = self.statistics_report_analysis_count();
        let panel = &mut self.statistics_panel;
        let [row, column] = panel.report_selection;
        let row = row
            .saturating_add_signed(rows)
            .min(view::ONE_REPORT_ROWS.len() - 1);
        let column = column.saturating_add_signed(columns).min(available);
        panel.report_selection = [row, column];
        panel.report_row = panel
            .report_row
            .clamp(row.saturating_sub(view::REPORT_VISIBLE_ROWS - 1), row);
        if row == panel.report_row {
            panel.report_row_offset = 0;
        }
        panel.reveal_report_column();
    }
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
            let (columns, typed) = Columns::from_snapshot(snapshot, two)?;
            if existing.0 != columns.0
                || self.statistics_panel.typed_columns[usize::from(two)].as_ref() != Some(&typed)
            {
                updates.push((two, columns, typed));
            }
        }
        let changed = !updates.is_empty();
        for (two, columns, typed) in updates {
            *self.memory.models.statistics_columns_mut(two).unwrap() = columns;
            self.statistics_panel.typed_columns[usize::from(two)] = Some(typed);
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
        if let Some(value) = self.statistics_panel.typed_columns[usize::from(two)]
            .as_ref()
            .and_then(|columns| columns.get(column))
            .and_then(|values| values.get(row))
        {
            return value.ppl_source();
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
        if two
            && let Some(number) = ppl_math::Real::parse(&source)
            && ppl_math::Real::parse_versioned(&source, ppl_math::NumericVersion::Current15751)
                .is_some_and(|exact| exact.cmp_value(number).is_eq())
        {
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
    Chart,
    Option,
    Color,
    More,
    Sort,
    ReportMore,
    ReportSelect,
}
impl MenuKind {
    fn report(self) -> bool {
        matches!(self, Self::ReportMore | Self::ReportSelect)
    }
}
#[derive(Clone, Copy)]
struct Menu {
    kind: MenuKind,
    index: usize,
    first: usize,
}
impl Menu {
    fn report_select() -> Self {
        Self {
            kind: MenuKind::ReportSelect,
            index: 0,
            first: 0,
        }
    }
}
impl NativeTools {
    pub fn is_statistics_page(&self) -> bool {
        mode(&self.memory.selected_app).is_some()
            && matches!(
                self.page,
                Some(super::Page::Numeric | super::Page::Symbolic)
            )
    }
    pub fn reset_statistics(&mut self, two: bool, snapshot: &SessionSnapshot) {
        if !two {
            self.statistics_panel.zoom_undo = None;
            self.statistics_panel.free_plot_cursor = None;
        }
        let app = app(two);
        let index = usize::from(two);
        if two {
            self.memory.models.statistics2 = Default::default();
            self.memory.models.statistics2_book = Some(Default::default());
            self.memory.statistics_fit_visible[index] = false;
        } else {
            self.memory.models.statistics1 = Default::default();
            self.memory.models.statistics1_book = Some(Default::default());
        }
        self.memory.statistics_cursor[index] = Default::default();
        self.statistics_panel.typed_columns[index] = None;
        self.statistics_panel.last_generation = None;
        self.observed_plot_settings.remove(app);
        self.pending_actions.retain(
            |request| !matches!(request, Request::PlotConfiguration(name, _) if name == app),
        );
        self.project_plot_settings(snapshot);
        self.statistics_invalidate(two);
    }

    pub fn statistics_busy(&mut self, busy: bool) {
        if self.statistics_panel.busy != busy {
            self.statistics_panel.busy = busy;
            self.dirty();
        }
    }
    pub(crate) fn statistics_invalidate(&mut self, two: bool) {
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
                Field::Color,
                Field::FitColor,
            ]
        } else {
            &[
                Field::Enabled,
                Field::X,
                Field::Frequency,
                Field::Fit,
                Field::Expression,
                Field::Color,
            ]
        }
    }
    fn statistics_frequency_enabled(&self, two: bool, analysis: usize) -> bool {
        two || self
            .memory
            .models
            .statistics1_book
            .as_ref()
            .is_none_or(|book| book.analyses[analysis].chart.uses_frequencies())
    }
    fn statistics_current_cursor(&mut self, two: bool) -> Cursor {
        let index = usize::from(two);
        let mut cursor = self.memory.statistics_cursor[index];
        if self.page == Some(super::Page::Symbolic)
            && cursor.field == Field::Frequency
            && !self.statistics_frequency_enabled(two, cursor.analysis)
        {
            // Earlier profiles could save focus on an inactive Frequency field.
            cursor.field = Field::X;
            self.memory.statistics_cursor[index] = cursor;
            self.dirty();
        }
        cursor
    }
    fn statistics_menu_items(&self, two: bool, kind: MenuKind) -> Vec<String> {
        match kind {
            MenuKind::ReportMore => ["Select", "Selection"].map(str::to_owned).to_vec(),
            MenuKind::ReportSelect => ["Row", "Column", "Include Headers"]
                .map(str::to_owned)
                .to_vec(),
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
            MenuKind::Chart => model::one::Chart::ALL
                .iter()
                .map(|chart| chart.label().into())
                .collect(),
            MenuKind::Option => self
                .memory
                .models
                .statistics1_book
                .as_ref()
                .map(|book| {
                    book.analyses[self.memory.statistics_cursor[0].analysis]
                        .chart
                        .options()
                        .iter()
                        .map(|label| (*label).into())
                        .collect()
                })
                .unwrap_or_default(),
            MenuKind::Color => COLORS.iter().map(|(label, _)| (*label).into()).collect(),
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
            MenuKind::ReportMore => {
                if menu.index == 0 {
                    self.statistics_panel.menu = Some(Menu::report_select());
                } else {
                    let panel = &mut self.statistics_panel;
                    panel.report_anchor = panel
                        .report_anchor
                        .is_none()
                        .then_some(panel.report_selection);
                }
            }
            MenuKind::ReportSelect => {
                let [row, column] = self.statistics_panel.report_selection;
                match menu.index {
                    0 => {
                        self.statistics_panel.report_anchor = Some([row, 0]);
                        let last = self.statistics_report_analysis_count();
                        self.statistics_move_report_selection(0, last as isize - column as isize);
                    }
                    1 => {
                        self.statistics_panel.report_anchor = Some([0, column]);
                        let last = view::ONE_REPORT_ROWS.len() - 1;
                        self.statistics_move_report_selection(last as isize - row as isize, 0);
                        self.statistics_panel.report_row_offset = view::REPORT_LAST_ROW_CLIP;
                    }
                    2 => {
                        self.statistics_panel.report_include_headers =
                            !self.statistics_panel.report_include_headers;
                    }
                    _ => {}
                }
            }
            MenuKind::Column(field) => {
                let value = menu.index.checked_sub(1);
                if two {
                    let book = self.memory.models.statistics2_book.as_mut().unwrap();
                    let analysis = &mut book.analyses[cursor.analysis];
                    match field {
                        Field::X => analysis.set_column(1, value),
                        Field::Y => analysis.set_column(2, value),
                        Field::Frequency => analysis.set_column(3, value),
                        _ => {}
                    }
                    book.fit_columns_revisions[cursor.analysis] = None;
                } else {
                    let analysis = &mut self
                        .memory
                        .models
                        .statistics1_book
                        .as_mut()
                        .unwrap()
                        .analyses[cursor.analysis];
                    match field {
                        Field::X => {
                            analysis.data = value;
                            analysis.data_source = None;
                        }
                        Field::Frequency => {
                            analysis.frequency = value;
                            analysis.frequency_source = None;
                        }
                        _ => {}
                    }
                }
                self.statistics_invalidate(two);
            }
            MenuKind::Fit => {
                let book = self.memory.models.statistics2_book.as_mut().unwrap();
                book.assign_field(
                    cursor.analysis,
                    4,
                    &PortableValue::Real(ppl_math::Real::from_i64(menu.index as i64 + 1)),
                )
                .expect("fit menu always supplies a valid code");
                self.statistics_invalidate(two);
            }
            MenuKind::Color if two => {
                let field = if cursor.field == Field::FitColor {
                    8
                } else {
                    6
                };
                if let Some((_, color)) = COLORS.get(menu.index) {
                    let value = PortableValue::Real(ppl_math::Real::from_i64(i64::from(*color)));
                    let book = self.memory.models.statistics2_book.as_mut().unwrap();
                    match book.assign_field(cursor.analysis, field, &value) {
                        Ok(()) => self.statistics_invalidate(two),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            MenuKind::Chart | MenuKind::Option | MenuKind::Color => {
                let value = match menu.kind {
                    MenuKind::Chart => model::one::Chart::ALL
                        .get(menu.index)
                        .map(|chart| (3, i64::from(chart.code()))),
                    MenuKind::Option => Some((4, menu.index as i64 + 1)),
                    MenuKind::Color => COLORS
                        .get(menu.index)
                        .map(|(_, color)| (5, i64::from(*color))),
                    _ => unreachable!(),
                };
                if let Some((field, value)) = value {
                    let analysis = &mut self
                        .memory
                        .models
                        .statistics1_book
                        .as_mut()
                        .unwrap()
                        .analyses[cursor.analysis];
                    match analysis
                        .assign_field(field, &PortableValue::Real(ppl_math::Real::from_i64(value)))
                    {
                        Ok(_) => self.statistics_invalidate(false),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            MenuKind::More => {
                let Some(columns) = self.statistics_panel.typed_columns[usize::from(two)].as_ref()
                else {
                    self.error = Some("Statistics data is not synchronized".into());
                    return requests;
                };
                let mut data = columns[cursor.column]
                    .iter()
                    .copied()
                    .map(PortableValue::Real)
                    .collect::<Vec<_>>();
                match menu.index {
                    0 => {
                        if data.len() >= model::MAX_ROWS {
                            self.error = Some("Statistics column has reached1000 rows".into());
                        } else {
                            data.insert(
                                cursor.row.min(data.len()),
                                PortableValue::Real(ppl_math::Real::ZERO),
                            );
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
                            .map(|value| crate::ide::native::value_source(&value.to_value()))
                            .collect::<Vec<_>>()
                            .join(",")
                    ))),
                    _ => {}
                }
            }
            MenuKind::Sort => {
                let Some(columns) = self.statistics_panel.typed_columns[usize::from(two)].as_ref()
                else {
                    self.error = Some("Statistics data is not synchronized".into());
                    return requests;
                };
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
                    let count = columns[key].len();
                    if selected.iter().any(|&i| columns[i].len() != count) {
                        self.error = Some("Related columns must have equal lengths to sort".into());
                    } else {
                        let mut order = (0..count).collect::<Vec<_>>();
                        order.sort_by(|&a, &b| {
                            let cmp = columns[key][a].cmp_value(columns[key][b]);
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
                                .map(|i| {
                                    (
                                        i,
                                        order
                                            .iter()
                                            .map(|&row| PortableValue::Real(columns[i][row]))
                                            .collect(),
                                    )
                                })
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
                if a.enabled || a.defined() {
                    a.enabled = !a.enabled;
                }
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
            Field::Fit => (
                MenuKind::Chart,
                self.memory
                    .models
                    .statistics1_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis]
                    .chart
                    .code() as usize
                    - 1,
            ),
            Field::Expression if !two => {
                let analysis = &self
                    .memory
                    .models
                    .statistics1_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis];
                if analysis.chart.options().is_empty() {
                    self.error = Some("This chart has no additional option".into());
                    return Vec::new();
                }
                (
                    MenuKind::Option,
                    usize::from(analysis.option.saturating_sub(1)),
                )
            }
            Field::Color | Field::FitColor if two => {
                let a = &self
                    .memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis];
                let color = if cursor.field == Field::FitColor {
                    a.fit_color(cursor.analysis)
                } else {
                    a.observation_color(cursor.analysis)
                };
                (
                    MenuKind::Color,
                    COLORS
                        .iter()
                        .position(|(_, value)| *value == color)
                        .unwrap_or(0),
                )
            }
            Field::Color if !two => {
                let color = self
                    .memory
                    .models
                    .statistics1_book
                    .as_ref()
                    .unwrap()
                    .analyses[cursor.analysis]
                    .plot_color(cursor.analysis);
                (
                    MenuKind::Color,
                    COLORS
                        .iter()
                        .position(|(_, value)| *value == color)
                        .unwrap_or(0),
                )
            }
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
    fn statistics_two_editor(cursor: Cursor, clear: bool) -> Option<MathTarget> {
        Some(MathTarget::StatisticsTwoReference {
            analysis: cursor.analysis,
            field: stats_two_reference(cursor.field)?,
            clear,
        })
    }

    fn statistics_show_field(&mut self, two: bool, cursor: Cursor, evaluate: bool) -> Vec<Request> {
        if !two {
            let analysis = &self
                .memory
                .models
                .statistics1_book
                .as_ref()
                .unwrap()
                .analyses[cursor.analysis];
            let source = match cursor.field {
                Field::Frequency => {
                    let source = analysis.frequency_expression();
                    if source.is_empty() {
                        "1".into()
                    } else {
                        source
                    }
                }
                Field::Fit => analysis.chart.label().into(),
                Field::Expression => analysis
                    .chart
                    .options()
                    .get(usize::from(analysis.option.saturating_sub(1)))
                    .copied()
                    .unwrap_or("No additional option")
                    .into(),
                Field::Color => format!("#{:06X}h", analysis.plot_color(cursor.analysis)),
                _ => analysis.data_expression(),
            };
            if evaluate && matches!(cursor.field, Field::X | Field::Frequency) && !source.is_empty()
            {
                return vec![Request::Calculate(Calculation::StatisticsDefinitionValue {
                    two: false,
                    source,
                    geometry: self
                        .memory
                        .models
                        .geometry_book
                        .clone()
                        .map(std::sync::Arc::new),
                })];
            }
            self.statistics_definition_evaluated(source);
            return Vec::new();
        }
        let source = if matches!(cursor.field, Field::Expression | Field::Fit) {
            self.statistics_fit_equation(cursor.analysis)
        } else {
            let a = &self
                .memory
                .models
                .statistics2_book
                .as_ref()
                .unwrap()
                .analyses[cursor.analysis];
            let Some(field) = stats_two_reference(cursor.field) else {
                return Vec::new();
            };
            let mut source = a.reference(field);
            if source.is_empty() && field == 3 {
                source = "1".into();
            }
            if evaluate && !source.is_empty() {
                return vec![Request::Calculate(Calculation::StatisticsDefinitionValue {
                    two: true,
                    source,
                    geometry: self
                        .memory
                        .models
                        .geometry_book
                        .clone()
                        .map(std::sync::Arc::new),
                })];
            }
            source
        };
        self.statistics_definition_evaluated(source);
        Vec::new()
    }
    pub fn statistics_definition_evaluated(&mut self, source: String) {
        self.statistics_panel.equation = Some(source);
        self.statistics_panel.equation_x = 0;
        self.statistics_panel.equation_y = 0;
        self.dirty();
    }
    pub fn statistics_reset_popups(&mut self) {
        self.statistics_panel.menu = None;
        self.statistics_panel.report = false;
        self.statistics_panel.report_copy_pending = false;
        self.statistics_panel.report_anchor = None;
        self.statistics_panel.equation = None;
        self.statistics_panel.plot_equation = None;
        self.statistics_panel.plot_zoom = None;
        self.statistics_panel.zoom_box = None;
        self.statistics_panel.zoom_factors = None;
        self.statistics_panel.plot_definition = false;
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
        let cursor = self.statistics_current_cursor(two);
        if self.page == Some(super::Page::Numeric) {
            return Some(vec![
                Request::EditMath(self.statistics_entry_target(two, true)),
                Request::TypeText(text.into()),
            ]);
        }
        if !two && matches!(cursor.field, Field::X | Field::Frequency) {
            return Some(vec![
                Request::EditMath(MathTarget::StatisticsDefinition {
                    analysis: cursor.analysis,
                    frequency: cursor.field == Field::Frequency,
                    clear: true,
                }),
                Request::TypeText(text.into()),
            ]);
        }
        if two && let Some(target) = Self::statistics_two_editor(cursor, true) {
            return Some(vec![
                Request::EditMath(target),
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
        let mut cursor = self.statistics_current_cursor(two);
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
                A::Edit(E::Right(_))
                    if matches!(menu.kind, MenuKind::ReportMore) && menu.index == 0 =>
                {
                    self.statistics_panel.menu = Some(Menu::report_select());
                }
                A::Edit(E::Left(_)) | A::Back if matches!(menu.kind, MenuKind::ReportSelect) => {
                    self.statistics_panel.menu = Some(Menu {
                        kind: MenuKind::ReportMore,
                        index: 0,
                        first: 0,
                    });
                }
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
                A::History(delta) if !two => {
                    self.statistics_move_report_selection(delta as isize, 0);
                }
                A::Edit(E::Left(_)) if !two => self.statistics_move_report_selection(0, -1),
                A::Edit(E::Right(_)) if !two => self.statistics_move_report_selection(0, 1),
                A::Copy if !two => self.statistics_panel.report_copy_pending = true,
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
                    // The 2Var color swatches are additional horizontal controls.
                    // Keep vertical navigation through the original six row fields.
                    let fields = if two { &fields[..6] } else { fields };
                    let position = position.min(fields.len() - 1);
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
                A::Template(template)
                    if !two && matches!(cursor.field, Field::X | Field::Frequency) =>
                {
                    requests = vec![
                        Request::EditMath(MathTarget::StatisticsDefinition {
                            analysis: cursor.analysis,
                            frequency: cursor.field == Field::Frequency,
                            clear: true,
                        }),
                        Request::InsertTemplate(template),
                    ];
                }
                A::Fraction if !two && matches!(cursor.field, Field::X | Field::Frequency) => {
                    requests = vec![
                        Request::EditMath(MathTarget::StatisticsDefinition {
                            analysis: cursor.analysis,
                            frequency: cursor.field == Field::Frequency,
                            clear: true,
                        }),
                        Request::InsertTemplate(TemplateKind::Fraction),
                    ];
                }
                A::Edit(E::Backspace | E::Delete)
                    if !two && matches!(cursor.field, Field::X | Field::Frequency) =>
                {
                    let field = if cursor.field == Field::Frequency {
                        2
                    } else {
                        1
                    };
                    let analysis = &mut self
                        .memory
                        .models
                        .statistics1_book
                        .as_mut()
                        .unwrap()
                        .analyses[cursor.analysis];
                    match analysis.assign_field(field, &PortableValue::String(String::new())) {
                        Ok(_) => self.statistics_invalidate(false),
                        Err(error) => self.error = Some(error),
                    }
                }
                A::Back | A::On => requests.push(Request::Close),
                _ => {}
            }
            if cursor.field == Field::Frequency
                && !self.statistics_frequency_enabled(two, cursor.analysis)
            {
                cursor.field = if matches!(action, A::Edit(E::Right(_)) | A::History(1..)) {
                    Field::Fit
                } else {
                    Field::X
                };
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
                Field::Expression | Field::Color => 54,
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
const COLORS: [(&str, u32); 8] = [
    ("Blue", 0x0000ff),
    ("Red", 0xff0000),
    ("Green", 0x008000),
    ("Orange", 0xffa500),
    ("Purple", 0x800080),
    ("Black", 0x000000),
    ("Cyan", 0x00ffff),
    ("Magenta", 0xff00ff),
];
