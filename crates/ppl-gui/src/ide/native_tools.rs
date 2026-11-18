//! Interactive native calculator tools rendered inside the existing LCD.
pub mod characters;
pub mod function_numeric;
pub mod function_views;
mod gallery;
pub mod geometry;
mod graph3d;
pub mod graph_symbolic;
pub mod help;
mod home_settings;
mod model_job;
mod notes;
mod pickers;
pub mod special_views;
mod split;
pub mod spreadsheet;
pub mod statistics;
mod templates;
mod views;
use super::native_apps::{GraphApp, GraphKind, PlotJob, TableJob};
use egui::{Color32, Rect};
use ppl_vm::SessionSnapshot;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
    Base,
    Apps,
    Views,
    Define,
    Units,
    Sexagesimal,
    Symbolic,
    Plot,
    PlotDetail,
    PlotTable,
    Numeric,
    PlotSetup,
    NumericSetup,
    Settings,
    CasSettings,
    SymbolicSetup,
    Programs,
    Lists,
    Matrices,
    Notes,
    Variables,
    CasVariables,
    Toolbox,
    Templates,
    Characters,
    Help,
    Info,
    Memory,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolMemory {
    pub presentation: super::native::presentation::PresentationSettings,
    pub characters: characters::CharacterSelection,
    pub graphs: std::collections::BTreeMap<String, GraphApp>,
    pub model_windows: std::collections::BTreeMap<String, super::native_apps::PlotWindow>,
    pub selected_app: String,
    pub gallery: gallery::Selection,
    pub graph_symbolic: std::collections::BTreeMap<String, graph_symbolic::Cursor>,
    pub function_views: function_views::Memory,
    pub function_numeric: function_numeric::Memory,
    #[serde(skip_serializing_if = "super::native_graph3d::Book::is_default")]
    pub graph3d: super::native_graph3d::Book,
    pub explorers: std::collections::BTreeMap<String, [f64; 3]>,
    pub name: String,
    pub search: String,
    pub commands: pickers::PickerState,
    pub units: pickers::PickerState,
    pub unit_prefix: String,
    pub template_selection: String,
    pub memory_selection: String,
    pub help: help::Memory,
    pub help_query: String,
    pub help_topic: String,
    pub app_notes: std::collections::BTreeMap<String, String>,
    pub original_name: String,
    pub content: String,
    pub program_entry: String,
    pub program_args: String,
    pub integer_edits: std::collections::BTreeMap<String, super::native_integer::IntegerEdit>,
    pub function_name: String,
    pub function_parameters: String,
    pub function_body: String,
    pub models: super::native_app_ui::AppMemory,
    pub statistics_cursor: [statistics::Cursor; 2],
    pub statistics_draft: Option<statistics::CellDraft>,
    pub statistics_make_source: [String; 2],
    pub statistics_fit_visible: [bool; 2],
    pub geometry_cursor: geometry::Cursor,
    pub geometry_draft: Option<geometry::Draft>,
    pub geometry_legacy_window: Option<super::native_apps::PlotWindow>,
    pub spreadsheet_cursor: spreadsheet::Cursor,
    pub spreadsheet_draft: Option<spreadsheet::Draft>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub spreadsheet_cas_mode: bool,
    pub home_history: std::collections::VecDeque<super::native::HomeEntry>,
    pub cas_history: std::collections::VecDeque<super::native::HomeEntry>,
    pub home_draft: String,
    pub cas_draft: String,
}
impl Default for ToolMemory {
    fn default() -> Self {
        Self {
            presentation: Default::default(),
            characters: Default::default(),
            graphs: [
                ("Function", GraphKind::Function),
                ("Advanced Graphing", GraphKind::Implicit),
                ("Linear Explorer", GraphKind::Function),
                ("Quadratic Explorer", GraphKind::Function),
                ("Trig Explorer", GraphKind::Function),
                ("Parametric", GraphKind::Parametric),
                ("Polar", GraphKind::Polar),
                ("Sequence", GraphKind::Sequence),
            ]
            .into_iter()
            .map(|(name, kind)| {
                let mut graph = GraphApp::new(kind);
                if name == "Function" {
                    graph.window = NativeTools::default_model_window(name);
                    graph.numeric_step = 0.1;
                    graph.curves[0].enabled = false;
                }
                if name == "Sequence" {
                    graph.window = NativeTools::default_model_window(name);
                }
                (name.into(), graph)
            })
            .collect(),
            explorers: [
                ("Linear Explorer".into(), [1., 0., 0.]),
                ("Quadratic Explorer".into(), [1., 0., 0.]),
                ("Trig Explorer".into(), [1., 1., 0.]),
            ]
            .into_iter()
            .collect(),
            model_windows: Default::default(),
            selected_app: "Function".into(),
            gallery: Default::default(),
            graph_symbolic: Default::default(),
            function_views: Default::default(),
            function_numeric: Default::default(),
            graph3d: Default::default(),
            name: String::new(),
            search: String::new(),
            commands: Default::default(),
            units: Default::default(),
            unit_prefix: String::new(),
            template_selection: String::new(),
            memory_selection: String::new(),
            help: Default::default(),
            help_query: String::new(),
            help_topic: String::new(),
            app_notes: Default::default(),
            original_name: String::new(),
            content: String::new(),
            program_entry: String::new(),
            program_args: String::new(),
            integer_edits: Default::default(),
            function_name: String::new(),
            function_parameters: "X".into(),
            function_body: String::new(),
            models: Default::default(),
            statistics_cursor: Default::default(),
            statistics_draft: None,
            statistics_make_source: Default::default(),
            statistics_fit_visible: Default::default(),
            geometry_cursor: Default::default(),
            geometry_draft: None,
            geometry_legacy_window: None,
            spreadsheet_cursor: Default::default(),
            spreadsheet_draft: None,
            spreadsheet_cas_mode: false,
            home_history: Default::default(),
            cas_history: Default::default(),
            home_draft: String::new(),
            cas_draft: String::new(),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpreadsheetHistory {
    Home,
    Cas,
}

impl SpreadsheetHistory {
    pub fn is_cas(self) -> bool {
        self == Self::Cas
    }
}

#[derive(Clone)]
pub enum MathTarget {
    FunctionNumeric {
        field: function_numeric::Field,
        clear: bool,
    },
    SpreadsheetCell {
        address: String,
        clear: bool,
        history: Option<SpreadsheetHistory>,
    },
    SpreadsheetGoto,
    SpreadsheetName {
        block: super::native_app_models::spreadsheet::format::Block,
    },
    SpreadsheetDimension {
        block: super::native_app_models::spreadsheet::format::Block,
        property: String,
    },
    SequenceInitial {
        app: String,
        index: usize,
        term: usize,
    },
    Curve {
        app: String,
        index: usize,
        second: bool,
        clear: bool,
    },
    StatisticsCell {
        two: bool,
        column: usize,
        row: usize,
        clear: bool,
    },
    StatisticsFit {
        analysis: usize,
    },
    StatisticsTwoReference {
        analysis: usize,
        field: usize,
        clear: bool,
    },
    StatisticsDefinition {
        analysis: usize,
        frequency: bool,
        clear: bool,
    },
    StatisticsColumn {
        two: bool,
        column: usize,
    },
    StatisticsPlotX,
    GeometryPlotField(String),
    GeometryObject {
        name: String,
        preset: Option<String>,
        insert: Option<usize>,
    },
    GeometryMeasurement {
        index: usize,
        preset: Option<String>,
    },
    GeometryName {
        name: String,
    },
    GeometryLabel {
        index: usize,
    },
    StatisticsGoto {
        two: bool,
    },
    Object,
    FunctionBody,
}
#[derive(Clone, Copy)]
pub enum ResetApp {
    Sequence,
    Geometry,
    StatisticsOne,
    StatisticsTwo,
}
impl ResetApp {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "Sequence" => Some(Self::Sequence),
            "Geometry" => Some(Self::Geometry),
            "Statistics 1Var" => Some(Self::StatisticsOne),
            "Statistics 2Var" => Some(Self::StatisticsTwo),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Sequence => "Sequence",
            Self::Geometry => "Geometry",
            Self::StatisticsOne => "Statistics 1Var",
            Self::StatisticsTwo => "Statistics 2Var",
        }
    }
}

pub enum Request {
    MathCursor {
        position: usize,
        extend: bool,
    },
    FinishMath(bool),
    ToggleSpreadsheetCas,
    SpreadsheetHistory(SpreadsheetHistory),
    TypeText(String),
    CopyText(String),
    StatisticsCellValue {
        two: bool,
        column: usize,
        row: usize,
        value: f64,
    },
    StatisticsWrite {
        two: bool,
        columns: Vec<(usize, Vec<ppl_vm::PortableValue>)>,
    },
    StatisticsDelete {
        two: bool,
        column: usize,
        row: usize,
    },
    SaveInteger,
    CancelInteger,
    SelectApp(String),
    StartApp(String),
    GraphConfiguration(String, super::native_apps::GraphApp),
    PlotConfiguration(String, ppl_vm::AppPlotSettings),
    Graph3dPlotZoom {
        source: Box<ppl_vm::AppPlotSettings>,
        edited: Box<ppl_vm::AppPlotSettings>,
    },
    GeometryPlotConfiguration(ppl_vm::GeometryPlotSettings),
    ResetApp(ResetApp),
    FunctionNumericConfiguration(ppl_vm::AppNumericSettings),
    Graph3dNumericConfiguration {
        source: Box<ppl_vm::AppNumericSettings>,
        edited: Box<ppl_vm::AppNumericSettings>,
        selected_cell: [usize; 2],
    },
    EditCollection {
        name: String,
        matrix: bool,
    },
    RenameObject {
        old: String,
        new: String,
        program: bool,
    },
    DefineFunction {
        name: String,
        parameters: String,
        body: String,
    },
    EditMath(MathTarget),
    InsertTemplate(super::native::expression::TemplateKind),
    SetCasSettings(ppl_cas::CasSettings),
    SetAppSettings(String, ppl_vm::AppSettings),
    Calculate(super::native_app_ui::Calculation),
    Close,
    Dismiss,
    Insert(String),
    StoreVariable(String, String),
    DeleteVariable(String),
    SaveProgram(String, String),
    DeleteProgram(String),
    RunProgram(String, String),
    RunProgramConfigured {
        name: String,
        source: String,
        entry: Option<String>,
        args: String,
    },
    SaveNote(String, String),
    SaveBulletedNote(String, String, std::collections::BTreeSet<usize>),
    DeleteNote(String),
    SetHomeSettings(ppl_vm::Settings),
    SetAngle(ppl_vm::AngleMode),
    SetFormat(i64, i64),
    Cas(String),
    CasDisplay {
        source: String,
        display: String,
    },
    CasCatalog(String, Option<String>),
    StoreCasVariable(String, String),
    DeleteCasVariable(String),
}
#[derive(Default)]
pub struct NativeTools {
    function_panel: function_views::Panel,
    numeric_panel: function_numeric::Panel,
    graph_symbolic_panel: graph_symbolic::Panel,
    graph3d_panel: graph3d::Panel,
    gallery_panel: gallery::Panel,
    home_settings: home_settings::Panel,
    help_panel: help::Panel,
    note_bullets: Option<std::collections::BTreeSet<usize>>,
    statistics_panel: statistics::Panel,
    prime_font: bool,
    geometry_panel: geometry::Panel,
    spreadsheet_panel: spreadsheet::Panel,
    template_icons: Vec<egui::TextureHandle>,
    template_strip: Option<egui::TextureHandle>,
    picker_focus_pending: bool,
    picker_scroll_pending: bool,
    picker_category_cursor: Option<usize>,
    unit_prefix_cursor: Option<usize>,
    pub page: Option<Page>,
    pub memory: ToolMemory,
    pub error: Option<String>,
    pub cas_busy: bool,
    pub presentation_pending: bool,
    pub trace: super::native_plot_controls::TraceState,
    auto_scale_pending: bool,
    statistics_plot_setup: Option<super::native_apps::PlotWindow>,
    pending_actions: Vec<Request>,
    pub pending_model_plot: Option<super::native_app_ui::Calculation>,
    pub integer_edit: Option<super::native_integer::IntegerEdit>,
    pub integer_original: Option<super::native_integer::IntegerEdit>,
    pub integer_key: String,
    pub integer_decimal_draft: String,
    pub integer_hex_draft: String,
    detail_job: Option<PlotJob>,
    model_job: Option<model_job::ModelPlotJob>,
    detail_model_job: Option<model_job::ModelPlotJob>,
    calculation_errors: std::collections::BTreeMap<&'static str, String>,
    detail_styles: Vec<super::native_model_plots::PlotStyle>,
    detail: Option<(
        super::native_apps::PlotWindow,
        super::native_apps::PlotSamples,
    )>,
    job: Option<PlotJob>,
    table_job: Option<TableJob>,
    points: Vec<Vec<Option<[f64; 2]>>>,
    table: Vec<Vec<Option<f64>>>,
    pub generation: u64,
    memory_generation: u64,
    plot_settings_cache: std::collections::BTreeMap<String, ppl_vm::AppPlotSettings>,
    observed_plot_settings: std::collections::BTreeMap<String, ppl_vm::AppPlotSettings>,
    observed_geometry_settings: Option<ppl_vm::GeometryPlotSettings>,
}
impl NativeTools {
    pub(crate) fn set_prime_font(&mut self, selected: bool) {
        self.prime_font = selected;
        self.template_icons.clear();
        self.template_strip = None;
        self.dirty();
    }
    fn apply_lcd_families(&self, style: &mut egui::Style) {
        if self.prime_font {
            for (text_style, font) in &mut style.text_styles {
                let name = if font.family == egui::FontFamily::Monospace {
                    super::native_fonts::MONO
                } else if *text_style == egui::TextStyle::Heading {
                    super::native_fonts::BOLD
                } else {
                    super::native_fonts::REGULAR
                };
                font.family = egui::FontFamily::Name(name.into());
            }
        }
    }

    fn project_numeric_graph(&mut self, snapshot: &SessionSnapshot) {
        let numeric = snapshot.app_numeric_settings();
        if let Some(graph) = self.memory.graphs.get_mut(&snapshot.active_app) {
            let start = numeric.start.to_f64();
            let step = numeric.step.to_f64();
            if start.is_finite() {
                graph.numeric_start = start;
            }
            if step.is_finite() && step > 0. {
                graph.numeric_step = step;
            }
        }
    }
    fn project_geometry_settings(&mut self, snapshot: &SessionSnapshot) {
        if let Some(settings) = &snapshot.geometry_plot_settings
            && self.observed_geometry_settings.as_ref() != Some(settings)
        {
            self.geometry_settings_changed(settings);
            self.observed_geometry_settings = Some(settings.clone());
        }
    }
    fn project_plot_settings(&mut self, snapshot: &SessionSnapshot) {
        self.project_geometry_settings(snapshot);
        // An explicit qualified assignment can update an inactive app. Project
        // every changed scoped record before a later app switch can persist an
        // older GUI window over that assignment. Never synthesize defaults for
        // profiles that already retain their windows in the UI state.
        for (name, settings) in &snapshot.app_plot_settings {
            if self.observed_plot_settings.get(name) == Some(settings) {
                continue;
            }
            self.plot_settings_cache
                .insert(name.clone(), settings.clone());
            self.observed_plot_settings
                .insert(name.clone(), settings.clone());
            if name == "Statistics 1Var" {
                self.statistics_plot_setup = None;
                self.memory.models.histogram = Some(super::native_model_plots::HistogramSpec {
                    minimum: settings.hmin.to_f64(),
                    maximum: settings.hmax.to_f64(),
                    width: settings.hwidth.to_f64(),
                });
                if settings.has_infinite_y() {
                    if self.memory.selected_app == *name {
                        self.trace.disable();
                    }
                    continue;
                }
            }
            let window = if let Some(app) = self.memory.graphs.get_mut(name) {
                Some(&mut app.window)
            } else if super::native_app_views::supports(
                name,
                super::native_app_views::AppView::Plot,
            ) {
                Some(self.memory.model_windows.entry(name.clone()).or_default())
            } else {
                None
            };
            if let Some(window) = window {
                window.xmin = settings.xmin.to_f64();
                window.xmax = settings.xmax.to_f64();
                window.ymin = settings.ymin.to_f64();
                window.ymax = settings.ymax.to_f64();
                window.axes = settings.axes;
            }
        }
    }
    pub fn apply_view_context(&mut self, context: &SessionSnapshot, committed_generation: u64) {
        if self.memory.selected_app != context.active_app {
            self.gallery_started(&context.active_app);
        }
        self.memory.selected_app = context.active_app.clone();
        self.statistics_apply_view_context(context, committed_generation);
        self.geometry_apply_context(context, committed_generation);
        self.spreadsheet_apply_context(context, committed_generation);
        self.function_numeric_apply_context(context);
        if let Err(error) = self.graph3d_apply_plot_context(context) {
            self.record_calculation_error("surface", Some(error));
        }
        self.project_numeric_graph(context);
        if let Some(app) = self.memory.graphs.get_mut(&context.active_app) {
            for (name, target) in [
                ("Xmin", &mut app.window.xmin),
                ("Xmax", &mut app.window.xmax),
                ("Ymin", &mut app.window.ymin),
                ("Ymax", &mut app.window.ymax),
                ("Tmin", &mut app.window.tmin),
                ("Tmax", &mut app.window.tmax),
            ] {
                if let Some(value) = context
                    .plot_variable_for(&context.active_app, name)
                    .and_then(|v| v.to_value().as_real())
                {
                    *target = value.to_f64();
                }
            }
        }
        if context.active_app != "Geometry"
            && context.active_app != super::native_graph3d::NAME
            && !self.memory.graphs.contains_key(&context.active_app)
            && super::native_app_views::supports(
                &context.active_app,
                super::native_app_views::AppView::Plot,
            )
            && ["Xmin", "Xmax", "Ymin", "Ymax"].iter().any(|name| {
                context
                    .plot_variable_for(&context.active_app, name)
                    .is_some()
            })
        {
            let window = self
                .memory
                .model_windows
                .entry(context.active_app.clone())
                .or_default();
            for (name, target) in [
                ("Xmin", &mut window.xmin),
                ("Xmax", &mut window.xmax),
                ("Ymin", &mut window.ymin),
                ("Ymax", &mut window.ymax),
            ] {
                if let Some(value) = context
                    .plot_variable_for(&context.active_app, name)
                    .and_then(|v| v.to_value().as_real())
                {
                    *target = value.to_f64();
                }
            }
        }
        self.project_plot_settings(context);
        // The operation has not committed. Do not replace its live values with
        // the preceding committed revision when the next GUI frame is painted.
        self.memory_generation = committed_generation;
    }
    pub fn view_outcome(&self, changed: bool) -> ppl_vm::host::CalculatorViewOutcome {
        let mut globals = std::collections::BTreeMap::new();
        if self.memory.selected_app == super::native_graph3d::NAME {
            return ppl_vm::host::CalculatorViewOutcome { changed, globals };
        }
        if changed && self.memory.selected_app == "Geometry" {
            let settings = self.geometry_plot_configuration().unwrap_or_default();
            for name in ppl_vm::GeometryPlotSettings::NAMES {
                if let Ok(value) = settings.get(name) {
                    globals.insert(name.into(), ppl_vm::PortableValue::from(&value));
                }
            }
            return ppl_vm::host::CalculatorViewOutcome { changed, globals };
        }
        if changed
            && self.statistics_unbounded_plot()
            && let Some(settings) = self.plot_settings_cache.get("Statistics 1Var")
        {
            for name in [
                "Xmin", "Xmax", "Ymin", "Ymax", "Hmin", "Hmax", "Hwidth", "Axes",
            ] {
                globals.insert(
                    name.into(),
                    ppl_vm::PortableValue::from(&settings.get(name).unwrap()),
                );
            }
            return ppl_vm::host::CalculatorViewOutcome { changed, globals };
        }
        let default_window = Self::default_model_window(&self.memory.selected_app);
        let window = self
            .memory
            .graphs
            .get(&self.memory.selected_app)
            .map(|app| &app.window)
            .or_else(|| self.memory.model_windows.get(&self.memory.selected_app))
            .or_else(|| {
                super::native_app_views::supports(
                    &self.memory.selected_app,
                    super::native_app_views::AppView::Plot,
                )
                .then_some(&default_window)
            });
        if changed
            && self.memory.selected_app == "Statistics 1Var"
            && let Some(histogram) = self.memory.models.histogram
        {
            for (name, number) in [
                ("Hmin", histogram.minimum),
                ("Hmax", histogram.maximum),
                ("Hwidth", histogram.width),
            ] {
                if let Ok(number) = ppl_math::Real::from_f64(number) {
                    globals.insert(name.into(), ppl_vm::PortableValue::Real(number));
                }
            }
        }
        if changed && let Some(window) = window {
            for (name, value) in [
                ("Xmin", window.xmin),
                ("Xmax", window.xmax),
                ("Ymin", window.ymin),
                ("Ymax", window.ymax),
            ] {
                if value.is_finite() {
                    globals.insert(
                        name.into(),
                        ppl_vm::PortableValue::Real(
                            ppl_math::Real::parse(&value.to_string())
                                .expect("finite plot coordinate"),
                        ),
                    );
                }
            }
        }
        if changed && let Some(window) = window {
            globals.insert(
                "Axes".into(),
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(i64::from(window.axes))),
            );
        }
        if changed && let Some(app) = self.memory.graphs.get(&self.memory.selected_app) {
            for (name, value) in [("Tmin", app.window.tmin), ("Tmax", app.window.tmax)] {
                if let Ok(value) = ppl_math::Real::from_f64(value) {
                    globals.insert(name.into(), ppl_vm::PortableValue::Real(value));
                }
            }
        }
        ppl_vm::host::CalculatorViewOutcome { changed, globals }
    }
    pub fn entry_page(name: &str) -> Page {
        use super::native_app_views::{AppView, entry};
        match entry(name) {
            AppView::Plot => Page::Plot,
            AppView::Numeric => Page::Numeric,
            _ => Page::Symbolic,
        }
    }
    pub fn supports_page(&self, page: Page) -> bool {
        use super::native_app_views::{AppView, supports};
        if page == Page::Views {
            return !special_views::views(&self.memory.selected_app).is_empty();
        }
        if matches!(page, Page::PlotDetail | Page::PlotTable) {
            let action = if page == Page::PlotDetail {
                special_views::SpecialView::PlotDetail
            } else {
                special_views::SpecialView::PlotTable
            };
            return special_views::views(&self.memory.selected_app).contains(&action);
        }
        let view = match page {
            Page::Symbolic => AppView::Symbolic,
            Page::Plot => AppView::Plot,
            Page::Numeric => AppView::Numeric,
            Page::PlotSetup => AppView::PlotSetup,
            Page::NumericSetup => AppView::NumericSetup,
            _ => return true,
        };
        supports(&self.memory.selected_app, view)
    }
    pub fn open(&mut self, page: Page) {
        if page == Page::NumericSetup {
            self.graph3d_panel.reset_numeric_draft();
        }
        if page == Page::Plot
            && self.page != Some(Page::Plot)
            && statistics::mode(&self.memory.selected_app) == Some(false)
        {
            self.trace.curve = 0;
            self.trace.sample = 0;
        }
        if page == Page::Apps {
            self.gallery_open();
        }
        self.statistics_reset_popups();
        self.statistics_plot_setup = None;
        if !self.supports_page(page) {
            self.error = Some(format!(
                "Native {page:?} view is unavailable for {}",
                self.memory.selected_app
            ));
            return;
        }
        if !matches!(page, Page::PlotDetail | Page::PlotTable) {
            self.detail_job = None;
            self.detail_model_job = None;
            self.detail = None;
            self.detail_styles.clear();
        }
        if page == Page::Templates {
            self.memory.template_selection =
                super::native::expression::palette::CELLS[0].name.into();
        }
        self.page = Some(page);
        if self.memory.selected_app == super::native_graph3d::NAME && page == Page::Numeric {
            self.graph3d_select_numeric_cell([0, 0]);
        }
        if page == Page::Settings {
            self.home_settings = Default::default();
        }
        self.picker_focus_pending = matches!(page, Page::Toolbox | Page::Units);
        self.picker_scroll_pending = true;
        self.picker_category_cursor = None;
        self.unit_prefix_cursor = None;
        self.error = None;
        self.calculation_errors.clear();
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn close(&mut self) {
        self.statistics_reset_popups();
        self.statistics_plot_setup = None;
        self.function_numeric_close();
        self.page = None;
        self.cancel_calculations();
    }
    pub fn cancel_calculations(&mut self) {
        self.function_numeric_cancel_all();
        self.graph3d_panel.cancel();
        self.spreadsheet_cancel();
        self.presentation_pending = false;
        self.detail_job = None;
        self.detail_model_job = None;
        self.calculation_errors.clear();
        self.model_job = None;
        self.job = None;
        self.table_job = None;
        self.pending_model_plot = None;
    }
    pub fn calculating(&self) -> bool {
        (self.page == Some(Page::Help) && self.help_panel.loading())
            || self.function_numeric_calculating()
            || self.graph3d_panel.calculating()
            || self.spreadsheet_panel.job.is_some()
            || self.detail_job.is_some()
            || self.detail_model_job.is_some()
            || self.model_job.is_some()
            || self.job.is_some()
            || self.table_job.is_some()
            || self.pending_model_plot.is_some()
    }
    pub fn plot_samples(&self) -> &[super::native_apps::CurveSamples] {
        &self.points
    }
    pub fn numeric_table(&self) -> &[Vec<Option<f64>>] {
        &self.table
    }
    pub fn dirty(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn trace_samples(&self) -> super::native_apps::PlotSamples {
        if self.statistics_unbounded_plot() {
            return Vec::new();
        }
        if self.memory.graphs.contains_key(&self.memory.selected_app) {
            self.points.clone()
        } else {
            self.memory
                .models
                .results
                .get(&format!("{}:plot", self.memory.selected_app))
                .and_then(|value| {
                    serde_json::from_value::<super::native_model_plots::PlotData>(value.clone())
                        .ok()
                })
                .map(|data| {
                    super::native_plot_controls::model_trace_samples(&data, &self.trace_window())
                })
                .unwrap_or_default()
        }
    }
    pub fn reset_sequence(&mut self, settings: &ppl_vm::AppPlotSettings) {
        let name = "Sequence";
        let mut graph = GraphApp::new(GraphKind::Sequence);
        graph.window = Self::settings_window(settings);
        graph.curves[0].enabled = false;
        self.memory.graphs.insert(name.into(), graph);
        self.pending_actions.retain(|request| {
            !matches!(request,
                Request::GraphConfiguration(app, _) | Request::PlotConfiguration(app, _)
                if app == name
            )
        });
        if self.memory.selected_app == name {
            self.job = None;
            self.table_job = None;
            self.points.clear();
            self.table.clear();
            self.trace = Default::default();
            self.error = None;
        }
        self.dirty();
    }
    pub fn default_model_window(name: &str) -> super::native_apps::PlotWindow {
        if name == "Geometry" {
            return super::native_geometry::default_window();
        }
        if let Some(settings) = ppl_vm::AppPlotSettings::for_app(name) {
            return Self::settings_window(&settings);
        }
        Default::default()
    }
    fn settings_window(settings: &ppl_vm::AppPlotSettings) -> super::native_apps::PlotWindow {
        super::native_apps::PlotWindow {
            xmin: settings.xmin.to_f64(),
            xmax: settings.xmax.to_f64(),
            ymin: settings.ymin.to_f64(),
            ymax: settings.ymax.to_f64(),
            axes: settings.axes,
            ..Default::default()
        }
    }
    fn statistics_unbounded_plot(&self) -> bool {
        self.memory.selected_app == "Statistics 1Var"
            && self
                .plot_settings_cache
                .get("Statistics 1Var")
                .is_some_and(ppl_vm::AppPlotSettings::has_infinite_y)
    }
    pub fn trace_window(&self) -> super::native_apps::PlotWindow {
        self.memory
            .graphs
            .get(&self.memory.selected_app)
            .map(|app| app.window.clone())
            .or_else(|| {
                self.memory
                    .model_windows
                    .get(&self.memory.selected_app)
                    .cloned()
            })
            .unwrap_or_else(|| Self::default_model_window(&self.memory.selected_app))
    }
    pub fn trace_step(&mut self, direction: super::native_plot_controls::TraceDirection) {
        let samples = self.trace_samples();
        if !self.trace.enabled {
            let window = self.trace_window();
            if let Err(error) = self.trace.nearest_in_view(
                [
                    (window.xmin + window.xmax) / 2.,
                    (window.ymin + window.ymax) / 2.,
                ],
                &window,
                &samples,
                [316., 145.],
            ) {
                self.error = Some(error);
                return;
            }
        }
        self.trace.step(direction, &samples);
        self.dirty();
    }
    pub fn trace_toggle(&mut self) {
        if self.statistics_trace_toggle() {
            return;
        }
        if self.trace.enabled {
            self.trace.disable();
        } else {
            let window = self.trace_window();
            if let Err(error) = self.trace.nearest_in_view(
                [
                    (window.xmin + window.xmax) / 2.,
                    (window.ymin + window.ymax) / 2.,
                ],
                &window,
                &self.trace_samples(),
                [316., 145.],
            ) {
                self.error = Some(error);
                return;
            }
        }
        self.dirty();
    }
    pub fn preset(
        &mut self,
        choice: super::native_plot_controls::Preset,
        mut snapshot: ppl_vm::SessionSnapshot,
    ) {
        snapshot.active_app = self.memory.selected_app.clone();
        let angle = snapshot.effective_app_settings().angle;
        let window = match super::native_plot_controls::preset(
            &self.trace_window(),
            choice,
            angle,
            super::native_plot_controls::PresetCenter::Origin,
        ) {
            Ok(window) => window,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
            app.window = window;
        } else {
            self.memory
                .model_windows
                .insert(self.memory.selected_app.clone(), window);
        }
        self.plot(snapshot);
    }
    pub fn model_plot_completed(
        &mut self,
        name: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let data = serde_json::from_value::<super::native_model_plots::PlotData>(value.clone())
            .map_err(|e| e.to_string())?;
        if name == "Geometry"
            && let Some(scene) = &data.geometry
            && let Some(book) = &mut self.memory.models.geometry_book
        {
            book.cache_scene(scene)?;
        }
        if name == "Statistics 1Var" {
            use super::native_model_plots::PlotScaleBounds;
            let bounds = data.scale.as_ref().and_then(|scale| scale.bounds.as_ref());
            let histogram = data.scale.as_ref().and_then(|scale| scale.histogram);
            let mut settings = self
                .plot_settings_cache
                .get(name)
                .cloned()
                .or_else(|| ppl_vm::AppPlotSettings::for_app(name))
                .ok_or("Missing Statistics Plot defaults")?;
            if let Some(PlotScaleBounds::UnboundedY { x: [xmin, xmax] }) = bounds {
                if !self.presentation_pending {
                    return Err("Insufficient numerical resolution".into());
                }
                settings.xmin = ppl_math::Real::from_f64(*xmin).map_err(|e| e.to_string())?;
                settings.xmax = ppl_math::Real::from_f64(*xmax).map_err(|e| e.to_string())?;
                settings.ymin = ppl_vm::PlotBound::NegativeInfinity;
                settings.ymax = ppl_vm::PlotBound::PositiveInfinity;
                if let Some(histogram) = histogram {
                    Self::set_histogram_settings(&mut settings, histogram)?;
                    self.memory.models.histogram = Some(histogram);
                }
                self.queue_model_plot_settings(name, settings);
                self.trace.disable();
                self.points.clear();
                self.auto_scale_pending = false;
                self.dirty();
                return Ok(());
            }
            if settings.has_infinite_y() && bounds.is_none() {
                if let Some(histogram) = histogram {
                    Self::set_histogram_settings(&mut settings, histogram)?;
                    self.memory.models.histogram = Some(histogram);
                    self.queue_model_plot_settings(name, settings);
                }
                self.points.clear();
                self.auto_scale_pending = false;
                self.dirty();
                return Ok(());
            }
            let existing = self.memory.model_windows.get(name);
            let mut window = existing
                .cloned()
                .unwrap_or_else(|| Self::settings_window(&settings));
            if let Some(PlotScaleBounds::Finite([xmin, xmax, ymin, ymax])) = bounds {
                window.xmin = *xmin;
                window.xmax = *xmax;
                window.ymin = *ymin;
                window.ymax = *ymax;
            }
            if existing.is_none() || bounds.is_some() {
                window.validate()?;
                self.memory.model_windows.insert(name.into(), window);
            }
            if let Some(histogram) = histogram {
                self.memory.models.histogram = Some(histogram);
            }
            self.auto_scale_pending = false;
        } else if name == "Statistics 2Var"
            && let Some(scale) = &data.scale
        {
            if let Some(super::native_model_plots::PlotScaleBounds::Finite(
                [xmin, xmax, ymin, ymax],
            )) = scale.bounds
            {
                let default_settings = || {
                    self.plot_settings_cache
                        .get(name)
                        .cloned()
                        .or_else(|| ppl_vm::AppPlotSettings::for_app(name))
                        .map(|settings| Self::settings_window(&settings))
                        .unwrap_or_default()
                };
                let mut window = self
                    .memory
                    .model_windows
                    .get(name)
                    .cloned()
                    .unwrap_or_else(default_settings);
                window.xmin = xmin;
                window.xmax = xmax;
                window.ymin = ymin;
                window.ymax = ymax;
                window.validate()?;
                self.memory.model_windows.insert(name.into(), window);
            }
            self.auto_scale_pending = false;
        } else if self.auto_scale_pending
            || (name != "Geometry" && !self.memory.model_windows.contains_key(name))
        {
            let current = self
                .memory
                .model_windows
                .get(name)
                .cloned()
                .unwrap_or_default();
            let fitted = if name == "Solve" {
                // PlotData separates discontinuities into labeled fragments.
                // Autoscale all fragments of the first side, not just one.
                let first = data.labels.first();
                let curve = data
                    .labels
                    .iter()
                    .zip(&data.curves)
                    .filter(|(label, _)| Some(*label) == first)
                    .flat_map(|(_, curve)| curve.iter().copied().map(Some))
                    .collect::<Vec<_>>();
                super::native_plot_controls::autoscale(GraphKind::Function, &current, &[curve])?
            } else {
                super::native_plot_controls::autoscale_model(&current, &data)?
            };
            if let Some(window) = fitted {
                self.memory.model_windows.insert(name.into(), window);
            }
            self.auto_scale_pending = false;
        }
        if let Some(settings) = self
            .plot_settings_cache
            .get(name)
            .cloned()
            .or_else(|| ppl_vm::AppPlotSettings::for_app(name))
        {
            let mut settings = settings;
            if let Some(window) = self.memory.model_windows.get(name) {
                for (field, number) in [
                    ("Xmin", window.xmin),
                    ("Xmax", window.xmax),
                    ("Ymin", window.ymin),
                    ("Ymax", window.ymax),
                ] {
                    let value = ppl_math::Real::from_f64(number).map_err(|e| e.to_string())?;
                    settings
                        .set(field, &ppl_vm::Value::Real(value))
                        .map_err(|e| e.to_string())?;
                }
                settings.axes = window.axes;
            }
            if let Some(histogram) = data.scale.as_ref().and_then(|scale| scale.histogram) {
                Self::set_histogram_settings(&mut settings, histogram)?;
            }
            self.queue_model_plot_settings(name, settings);
        }
        if name == "Statistics 2Var"
            && let Some(book) = &mut self.memory.models.statistics2_book
        {
            for (index, source, columns_revision) in &data.statistics_two_fits {
                book.set_calculated_fit(*index, source.clone(), *columns_revision);
            }
        }
        self.points = super::native_plot_controls::model_samples(&data);
        self.dirty();
        Ok(())
    }
    fn set_histogram_settings(
        settings: &mut ppl_vm::AppPlotSettings,
        histogram: super::native_model_plots::HistogramSpec,
    ) -> Result<(), String> {
        for (field, number) in [
            ("Hmin", histogram.minimum),
            ("Hmax", histogram.maximum),
            ("Hwidth", histogram.width),
        ] {
            let value = ppl_math::Real::from_f64(number).map_err(|e| e.to_string())?;
            settings
                .set(field, &ppl_vm::Value::Real(value))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn queue_model_plot_settings(&mut self, name: &str, settings: ppl_vm::AppPlotSettings) {
        if self.plot_settings_cache.get(name) != Some(&settings) {
            self.plot_settings_cache
                .insert(name.into(), settings.clone());
            self.pending_actions
                .push(Request::PlotConfiguration(name.into(), settings));
        }
    }
    pub fn autoscale(&mut self, snapshot: ppl_vm::SessionSnapshot) {
        self.auto_scale_pending = true;
        self.plot(snapshot);
    }
    pub fn plot(&mut self, mut snapshot: ppl_vm::SessionSnapshot) {
        if self.memory.selected_app == super::native_graph3d::NAME {
            self.graph3d_calculate(Page::Plot, snapshot);
            return;
        }
        if self.memory.selected_app == "Geometry" {
            if self.geometry_panel.live_context.is_none()
                && self.geometry_panel.memory_revision != Some(snapshot.generation)
            {
                self.geometry_panel.memory_revision = Some(snapshot.generation);
                self.memory.models.results.remove("Geometry");
                self.memory.models.results.remove("Geometry:plot");
            }
            self.geometry_panel.scene_generation = Some(snapshot.generation);
            self.geometry_panel.refresh = false;
            if let Err(error) = self.memory.models.ensure_geometry_book() {
                self.record_calculation_error("plot", Some(error));
                return;
            }
        }

        self.record_calculation_error("plot", None);
        snapshot.active_app = self.memory.selected_app.clone();
        if let Some([a, b, c]) = self.memory.explorers.get(&self.memory.selected_app)
            && let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app)
        {
            app.curves[0].expression = match self.memory.selected_app.as_str() {
                "Linear Explorer" => format!("({a})*X+({b})"),
                "Quadratic Explorer" => format!("({a})*(X-({b}))^2+({c})"),
                _ => format!("({a})*SIN(({b})*X+({c}))"),
            };
        }
        if self.page != Some(Page::PlotTable) {
            self.table_job = None;
        }
        let window = self.trace_window();
        let unbounded = snapshot
            .plot_settings_for(&snapshot.active_app)
            .is_some_and(|settings| settings.has_infinite_y());
        if self.memory.selected_app == "Geometry" {
            let settings = self
                .geometry_plot_configuration()
                .unwrap_or_else(|| snapshot.geometry_plot_settings());
            snapshot.geometry_plot_settings = Some(settings.clone());
            self.geometry_settings_changed(&settings);
            self.geometry_panel.refresh = false;
        }
        for (name, value) in [
            ("Xmin", window.xmin),
            ("Xmax", window.xmax),
            ("Ymin", window.ymin),
            ("Ymax", window.ymax),
        ] {
            if self.memory.selected_app == "Geometry" || unbounded {
                continue;
            }
            if let Ok(value) = ppl_math::Real::from_f64(value) {
                if let Some(mut settings) = snapshot.plot_settings_for(&snapshot.active_app) {
                    if settings.set(name, &ppl_vm::Value::Real(value)).is_ok() {
                        snapshot
                            .app_plot_settings
                            .insert(snapshot.active_app.clone(), settings);
                    }
                } else {
                    snapshot
                        .globals
                        .insert(name.into(), ppl_vm::PortableValue::Real(value));
                }
            }
        }
        if !self.memory.graphs.contains_key(&self.memory.selected_app)
            && let Some(settings) = snapshot.app_plot_settings.get(&snapshot.active_app)
        {
            self.queue_model_plot_settings(&snapshot.active_app, settings.clone());
        }
        if let Some(app) = self.memory.graphs.get(&self.memory.selected_app) {
            self.model_job = None;
            self.job = Some(PlotJob::start(
                app.clone(),
                snapshot,
                self.memory
                    .models
                    .geometry_book
                    .clone()
                    .map(std::sync::Arc::new),
            ));
            self.points.clear();
        } else {
            self.job = None;
            self.points.clear();
            let mut model = self.memory.models.clone();
            model.results.clear();
            self.model_job = Some(model_job::ModelPlotJob::start(
                self.memory.selected_app.clone(),
                model,
                snapshot,
                self.auto_scale_pending,
            ));
        }
    }
    pub fn show_explorer(&mut self, ui: &mut egui::Ui) {
        if let Some(parameters) = self.memory.explorers.get_mut(&self.memory.selected_app) {
            for (index, label) in ["a", "b", "c"].into_iter().enumerate() {
                ui.add(egui::Slider::new(&mut parameters[index], -10.0..=10.0).text(label));
            }
            let [a, b, c] = *parameters;
            if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                app.curves[0].expression = match self.memory.selected_app.as_str() {
                    "Linear Explorer" => format!("({a})*X+({b})"),
                    "Quadratic Explorer" => format!("({a})*(X-({b}))^2+({c})"),
                    _ => format!("({a})*SIN(({b})*X+({c}))"),
                };
            }
        }
    }
    pub fn table(&mut self, snapshot: &ppl_vm::SessionSnapshot) {
        if self.memory.selected_app == super::native_graph3d::NAME {
            self.graph3d_calculate(Page::Numeric, snapshot.clone());
            return;
        }
        if self.is_function_numeric_page() {
            self.table_job = None;
            self.function_numeric_start_table(snapshot);
            return;
        }
        if self.memory.selected_app == "Spreadsheet" {
            self.pending_model_plot = None;
            self.table_job = None;
            self.spreadsheet_request_refresh();
            return;
        }
        if self.memory.selected_app == "Geometry" {
            self.pending_model_plot = None;
            self.table_job = None;
            self.plot(snapshot.clone());
            return;
        }

        if statistics::mode(&self.memory.selected_app).is_some() {
            self.pending_model_plot = None;
            self.table_job = None;
            return;
        }
        self.record_calculation_error("table", None);
        if self.page != Some(Page::PlotTable) {
            self.job = None;
        }
        if let Some(app) = self.memory.graphs.get(&self.memory.selected_app) {
            let mut snapshot = snapshot.clone();
            snapshot.active_app = self.memory.selected_app.clone();
            let numeric = snapshot
                .app_numeric_settings
                .get(&self.memory.selected_app)
                .cloned()
                .unwrap_or_else(|| ppl_vm::AppNumericSettings::for_app(&self.memory.selected_app));
            let mut app = app.clone();
            app.numeric_start = numeric.start.to_f64();
            app.numeric_step = numeric.step.to_f64();
            self.table_job = Some(TableJob::start(app, snapshot));
        } else {
            self.pending_model_plot =
                super::native_app_ui::numeric(&self.memory.selected_app, &self.memory.models);
        }
    }
    fn record_calculation_error(&mut self, pane: &'static str, error: Option<String>) {
        if let Some(error) = error {
            self.calculation_errors.insert(pane, error);
        } else {
            self.calculation_errors.remove(pane);
        }
        self.error = self.calculation_errors.values().next().cloned();
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        lcd: Rect,
        snapshot: &SessionSnapshot,
    ) -> Vec<Request> {
        let Some(page) = self.page else {
            return Vec::new();
        };
        self.function_numeric_poll(snapshot);
        self.graph3d_poll(ctx, snapshot);
        if self.memory.selected_app == "Geometry" {
            self.geometry_sync_context(snapshot);
            if self.memory_generation != snapshot.generation {
                self.project_geometry_settings(snapshot);
            }
        }
        if let Some(job) = &self.detail_model_job {
            if let Some(result) = job.poll() {
                let job = self.detail_model_job.take().unwrap();
                let result = if job.name == self.memory.selected_app
                    && job.source == self.memory.models.input_fingerprint()
                {
                    result.and_then(|value| {
                        let data: super::native_model_plots::PlotData =
                            serde_json::from_value(value).map_err(|e| e.to_string())?;
                        if let Some((_, stored)) = &mut self.detail {
                            *stored = super::native_plot_controls::model_samples(&data);
                        }
                        self.detail_styles = data.styles;
                        Ok(())
                    })
                } else {
                    Err("App inputs changed during reference calculation".into())
                };
                self.record_calculation_error("detail", result.err());
                self.dirty();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if let Some(job) = &self.model_job {
            if let Some(result) = job.poll() {
                let job = self.model_job.take().unwrap();
                if job.name == "Geometry"
                    && (job.name != self.memory.selected_app
                        || job.source != self.memory.models.input_fingerprint()
                        || self
                            .geometry_panel
                            .scene_generation
                            .is_some_and(|g| g != job.generation))
                {
                    self.geometry_panel.refresh = true;
                    self.record_calculation_error("plot", None);
                } else if job.name == self.memory.selected_app
                    && job.source == self.memory.models.input_fingerprint()
                {
                    let result = result.and_then(|value| {
                        self.model_plot_completed(&job.name, &value)?;
                        self.memory
                            .models
                            .results
                            .insert(format!("{}:plot", job.name), value);
                        Ok(())
                    });
                    self.record_calculation_error("plot", result.err());
                } else {
                    self.record_calculation_error(
                        "plot",
                        Some("App inputs changed during plot calculation".into()),
                    );
                }
                self.auto_scale_pending = false;
                self.dirty();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if let Some(job) = &self.detail_job {
            if let Some(result) = job.poll() {
                match result {
                    Ok(points) => {
                        if let Some((_, stored)) = &mut self.detail {
                            *stored = points;
                        }
                        self.record_calculation_error("detail", None);
                    }
                    Err(error) => self.record_calculation_error("detail", Some(error)),
                }
                self.detail_job = None;
                self.dirty();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if let Some(job) = &self.job {
            if let Some(result) = job.poll() {
                match result {
                    Ok(p) => {
                        self.record_calculation_error("plot", None);
                        if self.auto_scale_pending {
                            self.auto_scale_pending = false;
                            if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app)
                            {
                                match super::native_plot_controls::autoscale(
                                    app.kind,
                                    &app.window,
                                    &p,
                                ) {
                                    Ok(Some(window)) => {
                                        app.window = window;
                                        self.pending_actions.push(Request::GraphConfiguration(
                                            self.memory.selected_app.clone(),
                                            app.clone(),
                                        ));
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        self.record_calculation_error("plot", Some(error))
                                    }
                                }
                            }
                        }
                        self.points = p;
                    }
                    Err(e) => self.record_calculation_error("plot", Some(e)),
                }
                self.job = None;
                self.dirty();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if let Some(job) = &self.table_job {
            if let Some(result) = job.poll() {
                match result {
                    Ok(table) => {
                        self.table = table;
                        self.record_calculation_error("table", None);
                    }
                    Err(error) => self.record_calculation_error("table", Some(error)),
                }
                self.table_job = None;
                self.dirty();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if self.memory_generation != snapshot.generation {
            self.function_numeric_apply_context(snapshot);
            self.project_numeric_graph(snapshot);
            if let Some(app) = self.memory.graphs.get_mut(&snapshot.active_app) {
                for (name, target) in [
                    ("Xmin", &mut app.window.xmin),
                    ("Xmax", &mut app.window.xmax),
                    ("Ymin", &mut app.window.ymin),
                    ("Ymax", &mut app.window.ymax),
                    ("Tmin", &mut app.window.tmin),
                    ("Tmax", &mut app.window.tmax),
                ] {
                    if let Some(value) = snapshot
                        .plot_variable_for(&snapshot.active_app, name)
                        .and_then(|value| value.to_value().as_real())
                    {
                        *target = value.to_f64();
                    }
                }
            }
            self.project_plot_settings(snapshot);
            self.memory_generation = snapshot.generation;
        }
        let old_presentation = self.memory.presentation;
        let old_integer = self.integer_edit.clone();
        let old_model = self.memory.models.input_fingerprint();
        let old_page = self.page;
        let old_app = self.memory.selected_app.clone();
        let old_graph = self
            .memory
            .graphs
            .get(&old_app)
            .and_then(|graph| serde_json::to_value(graph).ok());
        let mut requests = std::mem::take(&mut self.pending_actions);
        let scale = lcd.width() / 320.;
        if scale <= 0. {
            return requests;
        }
        if page == Page::Apps {
            self.show_gallery(ctx, lcd, &mut requests);
            return requests;
        }
        if self.is_graph_symbolic_page() {
            self.show_graph_symbolic(ctx, lcd, &mut requests);
            return requests;
        }
        if self.is_function_numeric_page() {
            self.show_function_numeric(ctx, lcd, snapshot, &mut requests);
            return requests;
        }
        if self.is_function_view() {
            self.show_function_view(ctx, lcd, snapshot, &mut requests);
            return requests;
        }
        if self.is_geometry_page() {
            self.show_geometry(ctx, lcd, scale, snapshot, &mut requests);
            return requests;
        }
        if self.is_spreadsheet_page() {
            self.show_spreadsheet(ctx, lcd, scale, snapshot, &mut requests);
            return requests;
        }
        if statistics::mode(&self.memory.selected_app).is_some() && page == Page::Plot {
            self.show_statistics_plot(ctx, lcd, scale, snapshot, &mut requests);
            return requests;
        }
        if self.is_statistics_page() {
            self.show_statistics(ctx, lcd, scale, snapshot, &mut requests);
            if old_model != self.memory.models.input_fingerprint() {
                let name = self.memory.selected_app.clone();
                self.memory
                    .models
                    .results
                    .retain(|key, _| key != &name && !key.starts_with(&format!("{name}:")));
                self.dirty();
            }
            return requests;
        }
        if page == Page::Settings {
            self.show_home_settings(ctx, lcd, scale, &snapshot.settings, &mut requests);
            if old_presentation != self.memory.presentation {
                self.dirty();
            }
            return requests;
        }
        if page == Page::Templates {
            self.show_templates(ctx, lcd, scale, &mut requests);
            return requests;
        }
        egui::Area::new("native_calculator_tool".into())
            .fade_in(false)
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.set_max_size(lcd.size());
                let mut style = (**ui.style()).clone();
                self.apply_lcd_families(&mut style);
                for font in style.text_styles.values_mut() {
                    font.size = (font.size * scale).max(6.);
                }
                style.spacing.item_spacing *= scale;
                style.spacing.button_padding *= scale;
                style.spacing.interact_size *= scale;
                style.spacing.text_edit_width = 295. * scale;
                style.visuals = egui::Visuals::light();
                ui.set_style(style);
                ui.painter().rect_filled(lcd, 0., Color32::WHITE);
                ui.horizontal(|ui| {
                    if ui.small_button("‹ Home").clicked() {
                        requests.push(Request::Close);
                    }
                    ui.strong(format!("{page:?}"));
                });
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::DARK_RED, error);
                }
                let height = (lcd.bottom() - ui.cursor().top() - 3. * scale).max(0.);
                let scroll = egui::ScrollArea::vertical()
                    .id_salt(("native_tool_page", format!("{page:?}")))
                    .animated(false)
                    .max_height(height);
                scroll.show(ui, |ui| {
                    self.show_page(ui, page, lcd, scale, snapshot, &mut requests)
                });
            });
        if old_model != self.memory.models.input_fingerprint() {
            let prefix = format!("{}:", self.memory.selected_app);
            let name = &self.memory.selected_app;
            self.memory
                .models
                .results
                .retain(|key, _| key != name && !key.starts_with(&prefix));
        }
        if old_page != self.page {
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        if old_presentation != self.memory.presentation {
            self.dirty();
        }
        if old_integer != self.integer_edit {
            if let Some(word) = &self.integer_edit {
                self.integer_decimal_draft = word.decimal();
                self.integer_hex_draft = format!("{:X}", word.raw);
            }
            self.dirty();
        }
        if old_app != self.memory.selected_app {
            requests.push(Request::SelectApp(self.memory.selected_app.clone()));
        } else if let Some(graph) = self.memory.graphs.get(&old_app)
            && serde_json::to_value(graph).ok() != old_graph
        {
            requests.push(Request::GraphConfiguration(old_app, graph.clone()));
        }
        requests
    }
}
impl ToolMemory {
    pub fn validate(&self) -> Result<(), String> {
        self.characters.validate()?;
        self.help.validate()?;
        if self.help_query.len() > 4096 || self.help_topic.len() > 4096 {
            return Err("Saved Help search exceeds its limit".into());
        }
        self.gallery.validate()?;
        self.function_views.validate()?;
        self.function_numeric.validate()?;
        self.graph3d.validate()?;
        for (app, cursor) in &self.graph_symbolic {
            if app != "Function" {
                return Err("Invalid Symbolic app selection".into());
            }
            cursor.validate()?;
        }
        self.spreadsheet_cursor.validate()?;
        if self.spreadsheet_draft.as_ref().is_some_and(|d| {
            (super::native_app_models::cell_coordinates(&d.address).is_none()
                && super::native_app_models::spreadsheet::Header::parse(&d.address).is_none())
                || d.source.len() > 65536
        }) {
            return Err("Invalid Spreadsheet draft".into());
        }
        for cursor in &self.statistics_cursor {
            cursor.validate()?;
        }
        if self.statistics_draft.as_ref().is_some_and(|draft| {
            draft.column >= 10
                || draft.row >= super::native_statistics::MAX_ROWS
                || draft.source.len() > 65536
        }) {
            return Err("Invalid statistics draft".into());
        }
        let image = serde_json::to_value(self).map_err(|e| e.to_string())?;
        serde_json::from_value::<Self>(image)
            .map_err(|_| "Calculator app inputs must be finite".to_string())?;
        for edit in self.integer_edits.values() {
            edit.validate()?;
        }
        if self.geometry_cursor.object > super::native_geometry::MAX_OBJECTS
            || self.geometry_cursor.measurement > super::native_geometry::MAX_OBJECTS
            || self.geometry_cursor.first_object > super::native_geometry::MAX_OBJECTS
            || self.geometry_cursor.first_measurement > super::native_geometry::MAX_OBJECTS
            || self.geometry_cursor.pointer.iter().any(|p| !p.is_finite())
        {
            return Err("Invalid Geometry cursor".into());
        }
        if self.geometry_draft.as_ref().is_some_and(|d| {
            d.index > super::native_geometry::MAX_OBJECTS
                || d.source.len() > 65536
                || d.name.as_ref().is_some_and(|n| n.len() > 24)
        }) {
            return Err("Invalid Geometry draft".into());
        }
        self.models.validate()?;
        if self.graphs.len() > 64 || self.graphs.values().any(|g| g.curves.len() > 10) {
            return Err("Invalid calculator app dimensions".into());
        }
        for graph in self.graphs.values() {
            for curve in &graph.curves {
                curve.literal_value()?;
            }
        }
        if !self.graphs.contains_key(&self.selected_app)
            && !super::native_app_ui::MODEL_APPS.contains(&self.selected_app.as_str())
        {
            return Err("Unknown saved calculator app".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod interaction_tests {
    use super::*;
    #[test]
    fn entry_picker_remains_open_after_the_opening_pointer_release() {
        let mut tools = NativeTools::default();
        tools.open(Page::Settings);
        let ctx = egui::Context::default();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let frame = |tools: &mut NativeTools, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    tools.show(
                        ctx,
                        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 240.)),
                        &snapshot,
                    );
                },
            );
        };
        frame(&mut tools, vec![]);
        frame(&mut tools, vec![]);
        let pos = egui::pos2(305., 112.);
        for pressed in [true, false] {
            frame(
                &mut tools,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            !tools.home_settings_snapshot(&snapshot.settings)["menu"].is_null(),
            "picker must stay open after a click"
        );
        frame(&mut tools, vec![]);
        assert!(!tools.home_settings_snapshot(&snapshot.settings)["menu"].is_null());
        tools.home_settings_action(
            super::super::native::keys::KeyAction::History(1),
            &snapshot.settings,
        );
        tools.home_settings_action(
            super::super::native::keys::KeyAction::Evaluate,
            &snapshot.settings,
        );
        assert_eq!(
            tools.memory.presentation.entry,
            super::super::native::presentation::EntryMode::Algebraic
        );
    }
    #[test]
    fn plot_pointer_drag_really_pans_the_window_and_starts_resampling() {
        let mut tools = NativeTools::default();
        tools.open(Page::Plot);
        tools.memory.graphs.get_mut("Function").unwrap().curves[0].expression = "X^2".into();
        let ctx = egui::Context::default();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let frame = |tools: &mut NativeTools, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                tools.show(
                    ctx,
                    Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 240.)),
                    &snapshot,
                );
            });
        };
        frame(&mut tools, vec![]);
        frame(&mut tools, vec![]);
        let start = egui::pos2(150., 100.);
        let end = egui::pos2(180., 110.);
        frame(
            &mut tools,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(&mut tools, vec![egui::Event::PointerMoved(end)]);
        frame(
            &mut tools,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let app = &tools.memory.graphs["Function"];
        assert!(
            app.window.xmin < -10.,
            "drag must change Xmin: {:?}",
            app.window
        );
        assert!(app.window.ymin > -10.);
        assert!(tools.calculating());
    }
    #[test]
    fn inactive_scoped_plot_writes_reach_the_saved_window_without_resetting_legacy_profiles_or_zoom()
     {
        let mut tools = NativeTools::default();
        tools.memory.graphs.get_mut("Function").unwrap().window.xmin = -123.;
        let mut context = ppl_vm::SessionContext::default();
        tools.project_plot_settings(&context.snapshot());
        assert_eq!(tools.memory.graphs["Function"].window.xmin, -123.);
        let evaluate = |context: &mut ppl_vm::SessionContext, source| {
            context
                .evaluate_home(
                    source,
                    Box::new(ppl_vm::HeadlessHost::new()),
                    ppl_vm::Limits::default(),
                )
                .result
                .unwrap();
        };
        evaluate(
            &mut context,
            "Statistics_1Var.Xmin:=2;Statistics_1Var.Xmax:=8",
        );
        tools.apply_view_context(&context.snapshot(), context.snapshot().generation);
        assert_eq!(tools.memory.selected_app, "Function");
        assert_eq!(tools.memory.model_windows["Statistics 1Var"].xmin, 2.);
        assert_eq!(tools.memory.model_windows["Statistics 1Var"].xmax, 8.);
        tools
            .memory
            .model_windows
            .get_mut("Statistics 1Var")
            .unwrap()
            .zoom(0.5);
        let zoomed = tools.memory.model_windows["Statistics 1Var"].xmin;
        evaluate(&mut context, "A:=42");
        tools.project_plot_settings(&context.snapshot());
        assert_eq!(tools.memory.model_windows["Statistics 1Var"].xmin, zoomed);
        evaluate(&mut context, "Statistics_1Var.Xmin:=1");
        tools.project_plot_settings(&context.snapshot());
        assert_eq!(tools.memory.model_windows["Statistics 1Var"].xmin, 1.);
        assert_eq!(tools.memory.graphs["Function"].window.xmin, -123.);
    }
}

#[cfg(test)]
mod statistics_autoscale_tests {
    use super::*;
    #[test]
    fn data_redraws_preserve_the_factory_or_explicit_statistics_window() {
        let mut tools = NativeTools::default();
        tools.memory.selected_app = "Statistics 1Var".into();
        let mut plot = super::super::native_model_plots::PlotData::new("Value", "Frequency");
        plot.push(
            "H1 bars",
            super::super::native_model_plots::PlotStyle::Line,
            vec![[100., 400.], [200., 800.]],
        )
        .unwrap();
        tools
            .model_plot_completed("Statistics 1Var", &serde_json::to_value(&plot).unwrap())
            .unwrap();
        let factory = tools.memory.model_windows["Statistics 1Var"].clone();
        assert_eq!(
            [factory.xmin, factory.xmax, factory.ymin, factory.ymax],
            [-1.8, 30., -1.8, 20.]
        );
        plot.scale = Some(super::super::native_model_plots::PlotScale {
            bounds: Some(super::super::native_model_plots::PlotScaleBounds::Finite([
                2., 8., -3., 5.,
            ])),
            histogram: None,
        });
        tools
            .model_plot_completed("Statistics 1Var", &serde_json::to_value(&plot).unwrap())
            .unwrap();
        plot.scale = None;
        tools
            .model_plot_completed("Statistics 1Var", &serde_json::to_value(&plot).unwrap())
            .unwrap();
        let saved = &tools.memory.model_windows["Statistics 1Var"];
        assert_eq!(
            [saved.xmin, saved.xmax, saved.ymin, saved.ymax],
            [2., 8., -3., 5.]
        );
        plot.scale = Some(super::super::native_model_plots::PlotScale {
            bounds: None,
            histogram: None,
        });
        tools
            .model_plot_completed("Statistics 1Var", &serde_json::to_value(&plot).unwrap())
            .unwrap();
        let saved = &tools.memory.model_windows["Statistics 1Var"];
        assert_eq!(
            [saved.xmin, saved.xmax, saved.ymin, saved.ymax],
            [2., 8., -3., 5.]
        );
    }
}
