//! Interactive native calculator tools rendered inside the existing LCD.
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
    pub graphs: std::collections::BTreeMap<String, GraphApp>,
    pub model_windows: std::collections::BTreeMap<String, super::native_apps::PlotWindow>,
    pub selected_app: String,
    pub explorers: std::collections::BTreeMap<String, [f64; 3]>,
    pub name: String,
    pub search: String,
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
    pub home_history: std::collections::VecDeque<super::native::HomeEntry>,
    pub cas_history: std::collections::VecDeque<super::native::HomeEntry>,
    pub home_draft: String,
    pub cas_draft: String,
}
impl Default for ToolMemory {
    fn default() -> Self {
        Self {
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
            .map(|(name, kind)| (name.into(), GraphApp::new(kind)))
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
            name: String::new(),
            search: String::new(),
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
            home_history: Default::default(),
            cas_history: Default::default(),
            home_draft: String::new(),
            cas_draft: String::new(),
        }
    }
}
#[derive(Clone)]
pub enum MathTarget {
    SequenceInitial {
        app: String,
        index: usize,
        term: usize,
    },
    Curve {
        app: String,
        index: usize,
        second: bool,
    },
    Object,
    FunctionBody,
}
pub enum Request {
    SaveInteger,
    CancelInteger,
    SelectApp(String),
    GraphConfiguration(String, super::native_apps::GraphApp),
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
    DeleteNote(String),
    SetHomeSettings(ppl_vm::Settings),
    SetAngle(ppl_vm::AngleMode),
    SetFormat(i64, i64),
    Cas(String),
    CasCatalog(String, Option<String>),
    StoreCasVariable(String, String),
    DeleteCasVariable(String),
}
#[derive(Default)]
pub struct NativeTools {
    pub page: Option<Page>,
    pub memory: ToolMemory,
    pub error: Option<String>,
    pub cas_busy: bool,
    pub trace: super::native_plot_controls::TraceState,
    auto_scale_pending: bool,
    pending_actions: Vec<Request>,
    pub pending_model_plot: Option<super::native_app_ui::Calculation>,
    pub integer_edit: Option<super::native_integer::IntegerEdit>,
    pub integer_original: Option<super::native_integer::IntegerEdit>,
    pub integer_key: String,
    pub integer_decimal_draft: String,
    pub integer_hex_draft: String,
    job: Option<PlotJob>,
    table_job: Option<TableJob>,
    points: Vec<Vec<Option<[f64; 2]>>>,
    table: Vec<Vec<Option<f64>>>,
    pub generation: u64,
    memory_generation: u64,
}
impl NativeTools {
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
        if !self.supports_page(page) {
            self.error = Some(format!("{} has no {page:?} view", self.memory.selected_app));
            return;
        }
        self.page = Some(page);
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn close(&mut self) {
        self.page = None;
        self.cancel_calculations();
    }
    pub fn cancel_calculations(&mut self) {
        self.job = None;
        self.table_job = None;
        self.pending_model_plot = None;
    }
    pub fn calculating(&self) -> bool {
        self.job.is_some() || self.table_job.is_some()
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
                .map(|data| super::native_plot_controls::model_samples(&data))
                .unwrap_or_default()
        }
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
            .unwrap_or_default()
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
        snapshot: ppl_vm::SessionSnapshot,
    ) {
        let angle = snapshot.effective_app_settings().angle;
        if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
            match super::native_plot_controls::preset(
                &app.window,
                choice,
                angle,
                super::native_plot_controls::PresetCenter::Origin,
            ) {
                Ok(window) => app.window = window,
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
            self.plot(snapshot);
        }
    }
    pub fn model_plot_completed(
        &mut self,
        name: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let data = serde_json::from_value::<super::native_model_plots::PlotData>(value.clone())
            .map_err(|e| e.to_string())?;
        if self.auto_scale_pending || !self.memory.model_windows.contains_key(name) {
            let current = self
                .memory
                .model_windows
                .get(name)
                .cloned()
                .unwrap_or_default();
            if let Some(window) = super::native_plot_controls::autoscale_model(&current, &data)? {
                self.memory.model_windows.insert(name.into(), window);
            }
            self.auto_scale_pending = false;
        }
        self.dirty();
        Ok(())
    }
    pub fn autoscale(&mut self, snapshot: ppl_vm::SessionSnapshot) {
        self.auto_scale_pending = true;
        self.plot(snapshot);
    }
    pub fn plot(&mut self, mut snapshot: ppl_vm::SessionSnapshot) {
        snapshot.active_app = self.memory.selected_app.clone();
        snapshot.settings = snapshot.effective_app_settings();
        if let Some([a, b, c]) = self.memory.explorers.get(&self.memory.selected_app)
            && let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app)
        {
            app.curves[0].expression = match self.memory.selected_app.as_str() {
                "Linear Explorer" => format!("({a})*X+({b})"),
                "Quadratic Explorer" => format!("({a})*(X-({b}))^2+({c})"),
                _ => format!("({a})*SIN(({b})*X+({c}))"),
            };
        }
        self.table_job = None;
        if let Some(app) = self.memory.graphs.get(&self.memory.selected_app) {
            self.job = Some(PlotJob::start(app.clone(), snapshot));
            self.points.clear();
        } else {
            let mut model = self.memory.models.clone();
            model.results.clear();
            self.pending_model_plot = Some(super::native_app_ui::Calculation::Plot(
                self.memory.selected_app.clone(),
                Box::new(model),
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
        self.job = None;
        if let Some(app) = self.memory.graphs.get(&self.memory.selected_app) {
            let mut snapshot = snapshot.clone();
            snapshot.active_app = self.memory.selected_app.clone();
            snapshot.settings = snapshot.effective_app_settings();
            self.table_job = Some(TableJob::start(app.clone(), snapshot));
        } else {
            self.pending_model_plot =
                super::native_app_ui::numeric(&self.memory.selected_app, &self.memory.models);
        }
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
        if let Some(job) = &self.job {
            if let Some(result) = job.poll() {
                match result {
                    Ok(p) => {
                        self.error = None;
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
                                    Err(error) => self.error = Some(error),
                                }
                            }
                        }
                        self.points = p;
                    }
                    Err(e) => self.error = Some(e),
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
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
                self.table_job = None;
                self.dirty();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if self.memory_generation != snapshot.generation {
            if let Some(app) = self.memory.graphs.get_mut(&snapshot.active_app) {
                for (name, target) in [
                    ("Xmin", &mut app.window.xmin),
                    ("Xmax", &mut app.window.xmax),
                    ("Ymin", &mut app.window.ymin),
                    ("Ymax", &mut app.window.ymax),
                    ("Tmin", &mut app.window.tmin),
                    ("Tmax", &mut app.window.tmax),
                    ("NumStart", &mut app.numeric_start),
                    ("NumStep", &mut app.numeric_step),
                ] {
                    if let Some(value) = snapshot
                        .globals
                        .get(name)
                        .and_then(|value| value.to_value().as_real())
                    {
                        *target = value.to_f64();
                    }
                }
            }
            self.memory_generation = snapshot.generation;
        }
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
        egui::Area::new("native_calculator_tool".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.set_max_size(lcd.size());
                let mut style = (**ui.style()).clone();
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
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .show(ui, |ui| {
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
        let image = serde_json::to_value(self).map_err(|e| e.to_string())?;
        serde_json::from_value::<Self>(image)
            .map_err(|_| "Calculator app inputs must be finite".to_string())?;
        for edit in self.integer_edits.values() {
            edit.validate()?;
        }
        self.models.validate()?;
        if self.graphs.len() > 64 || self.graphs.values().any(|g| g.curves.len() > 10) {
            return Err("Invalid calculator app dimensions".into());
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
}
