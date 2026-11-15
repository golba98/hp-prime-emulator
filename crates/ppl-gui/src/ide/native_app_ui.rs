//! Input controls for app models. No evaluation is performed in the egui frame.
use super::native_app_models::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppMemory {
    pub statistics1: Statistics1,
    pub statistics2: Statistics2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistics1_book: Option<super::native_statistics::OneBook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statistics2_book: Option<super::native_statistics::TwoBook>,
    pub linear: LinearSystem,
    pub finance: Finance,
    pub triangle: Triangle,
    pub geometry: Geometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_book: Option<super::native_geometry::Book>,
    pub spreadsheet: Spreadsheet,
    pub selected_cell: String,
    pub cell_address: String,
    pub cell_source: String,
    pub inference: super::native_inference::InferenceRequest,
    pub equation: String,
    pub solve_variable: String,
    pub finance_group: usize,
    pub histogram: Option<super::native_model_plots::HistogramSpec>,
    /// Derived results are rebuilt from persisted app data.
    #[serde(skip)]
    pub results: std::collections::BTreeMap<String, serde_json::Value>,
}
impl Default for AppMemory {
    fn default() -> Self {
        Self {
            statistics1: Default::default(),
            statistics2: Default::default(),
            statistics1_book: None,
            statistics2_book: None,
            linear: LinearSystem {
                coefficients: vec![vec![0.; 2]; 2],
                constants: vec![0.; 2],
            },
            finance: Default::default(),
            triangle: Default::default(),
            geometry: Default::default(),
            geometry_book: None,
            spreadsheet: Default::default(),
            selected_cell: "A1".into(),
            cell_address: "A1".into(),
            cell_source: String::new(),
            inference: Default::default(),
            equation: String::new(),
            solve_variable: "x".into(),
            finance_group: 1,
            histogram: None,
            results: Default::default(),
        }
    }
}
#[derive(Clone)]
pub enum Calculation {
    Plot(String, Box<AppMemory>),
    Amortization(Finance, usize),
    Statistics1(Statistics1),
    Statistics2(Statistics2),
    Statistics1Book(Box<super::native_statistics::OneBook>),
    Statistics2Book(Box<super::native_statistics::TwoBook>),
    StatisticsColumn {
        two: bool,
        column: usize,
        source: String,
        columns: super::native_statistics::Columns,
    },
    StatisticsCell {
        two: bool,
        column: usize,
        row: usize,
        source: String,
        columns: super::native_statistics::Columns,
    },
    Linear(LinearSystem),
    Finance(Finance, FinanceUnknown),
    Triangle(Triangle),
    Solve(String, String),
    Geometry(Geometry),
    GeometryBook(Box<super::native_geometry::Book>),
    GeometryEdit {
        book: Box<super::native_geometry::Book>,
        name: String,
        source: String,
        insert: Option<usize>,
    },
    GeometryMeasurement {
        book: Box<super::native_geometry::Book>,
        index: usize,
        source: String,
    },
    Spreadsheet(Spreadsheet),
    SpreadsheetEntry(Spreadsheet, String, String),
    Inference(super::native_inference::InferenceRequest),
}
impl Calculation {
    pub fn app_name(&self) -> &str {
        match self {
            Self::Plot(name, _) => name,
            Self::Amortization(..) => "Finance",
            Self::Statistics1(_)
            | Self::Statistics1Book(_)
            | Self::StatisticsCell { two: false, .. }
            | Self::StatisticsColumn { two: false, .. } => "Statistics 1Var",
            Self::Statistics2(_)
            | Self::Statistics2Book(_)
            | Self::StatisticsCell { two: true, .. }
            | Self::StatisticsColumn { two: true, .. } => "Statistics 2Var",
            Self::Linear(_) => "Linear Solver",
            Self::Finance(_, _) => "Finance",
            Self::Triangle(_) => "Triangle Solver",
            Self::Solve(_, _) => "Solve",
            Self::Geometry(_)
            | Self::GeometryBook(_)
            | Self::GeometryEdit { .. }
            | Self::GeometryMeasurement { .. } => "Geometry",
            Self::Spreadsheet(_) | Self::SpreadsheetEntry(..) => "Spreadsheet",
            Self::Inference(_) => "Inference",
        }
    }
    pub fn result_key(&self) -> String {
        if matches!(
            self,
            Self::StatisticsCell { .. } | Self::StatisticsColumn { .. }
        ) {
            format!("{}:entry", self.app_name())
        } else if matches!(self, Self::Amortization(..)) {
            "Finance:amortization".into()
        } else if matches!(self, Self::Plot(..)) {
            format!("{}:plot", self.app_name())
        } else {
            self.app_name().into()
        }
    }
    pub fn calculate(
        self,
        cancellation: std::sync::Arc<std::sync::atomic::AtomicBool>,
        snapshot: &ppl_vm::SessionSnapshot,
    ) -> Result<serde_json::Value, String> {
        let cancel = cancellation.as_ref();
        fn json<T: Serialize>(value: T) -> Result<serde_json::Value, String> {
            serde_json::to_value(value).map_err(|e| e.to_string())
        }
        match self {
            Self::Amortization(model, group) => json(
                super::native_model_plots::amortization_schedule(&model, group, Some(2), cancel)?,
            ),
            Self::Plot(name, model) => {
                use super::native_model_plots as plot;
                let data = match name.as_str() {
                    "Statistics 1Var" => {
                        if let Some(book) = &model.statistics1_book {
                            super::native_statistics::plots::one_in(
                                book,
                                model.histogram,
                                snapshot,
                                cancellation.clone(),
                            )
                        } else if let Some(spec) = &model.histogram {
                            plot::histogram(&model.statistics1, spec, cancel)
                        } else {
                            plot::histogram_auto(&model.statistics1, cancel)
                        }
                    }
                    "Statistics 2Var" => {
                        if let Some(book) = &model.statistics2_book {
                            super::native_statistics::plots::two(book, snapshot, cancel)
                        } else {
                            plot::statistics2(&model.statistics2, cancel)
                        }
                    }
                    "Finance" => plot::amortization(&model.finance, model.finance_group, cancel),
                    "Triangle Solver" => plot::triangle(&model.triangle, cancel),
                    "Geometry" => {
                        if let Some(book) = &model.geometry_book {
                            super::native_geometry::plot::build(
                                book,
                                snapshot,
                                cancellation.clone(),
                            )
                        } else {
                            plot::geometry(&model.geometry, cancel)
                        }
                    }
                    "Inference" => super::native_inference_plots::plot(&model.inference, cancel),
                    "Solve" => super::native_equations::plot(
                        &model.equation,
                        &model.solve_variable,
                        snapshot,
                        cancellation.clone(),
                    ),
                    _ => Err("This app does not provide a Native plot view".into()),
                }?;
                json(data)
            }
            Self::Inference(request) => json(request.calculate_with_cancel(cancel)?),
            Self::GeometryBook(book) => json(book.calculate(snapshot, cancellation)?),
            Self::GeometryEdit {
                book,
                name,
                source,
                insert,
            } => {
                let (mut book, mut scene) = book.enter(&name, &source, snapshot, cancellation)?;
                if let Some(index) = insert {
                    let old = book
                        .objects
                        .iter()
                        .position(|o| o.name == name)
                        .ok_or("Missing inserted object")?;
                    let object = book.objects.remove(old);
                    if index > book.objects.len() {
                        return Err("Invalid Geometry insertion row".into());
                    }
                    book.objects.insert(index, object);
                    scene.objects.sort_by_key(|o| {
                        book.objects
                            .iter()
                            .position(|a| a.name == o.name)
                            .unwrap_or(usize::MAX)
                    });
                }
                Ok(serde_json::json!({"book":book,"scene":scene}))
            }
            Self::GeometryMeasurement {
                mut book,
                index,
                source,
            } => {
                if index > book.measurements.len() {
                    return Err("Invalid Geometry measurement row".into());
                }
                if source.trim().is_empty() {
                    if index < book.measurements.len() {
                        book.measurements.remove(index);
                    }
                } else if index == book.measurements.len() {
                    book.measurements.push(super::native_geometry::Measurement {
                        source,
                        label: String::new(),
                        visible: false,
                    });
                } else {
                    book.measurements[index].source = source;
                }
                let scene = book.calculate(snapshot, cancellation)?;
                Ok(serde_json::json!({"book":book,"scene":scene}))
            }
            Self::Geometry(model) => {
                let mut distances = std::collections::BTreeMap::new();
                for segment in &model.segments {
                    let a = model
                        .points
                        .get(&segment[0])
                        .ok_or("Missing segment point")?;
                    let b = model
                        .points
                        .get(&segment[1])
                        .ok_or("Missing segment point")?;
                    distances.insert(format!("{}–{}", segment[0], segment[1]), a.distance(*b)?);
                }
                json(distances)
            }
            Self::Spreadsheet(model) => model.calculate(snapshot, cancellation),
            Self::SpreadsheetEntry(model, name, source) => {
                let sheet = model.enter(&name, &source, snapshot, cancellation.clone())?;
                let values = sheet.calculate(snapshot, cancellation)?;
                Ok(serde_json::json!({"sheet":sheet,"values":values}))
            }
            Self::Statistics1Book(book) => json(book.calculate(cancel)?),
            Self::Statistics2Book(book) => json(book.calculate(snapshot, cancel)?),
            Self::StatisticsColumn {
                two,
                column,
                source,
                columns,
            } => {
                if column >= super::native_statistics::COLUMN_COUNT {
                    return Err("Invalid statistics column".into());
                }
                json(super::native_statistics::evaluation::column_values(
                    &source,
                    two,
                    &columns,
                    snapshot,
                    cancellation,
                )?)
            }
            Self::StatisticsCell {
                two,
                column,
                row,
                source,
                columns,
            } => {
                if column >= super::native_statistics::COLUMN_COUNT
                    || row >= super::native_statistics::MAX_ROWS
                {
                    return Err("Invalid statistics cell address".into());
                }
                json(super::native_statistics::evaluation::cell_value(
                    &source,
                    two,
                    &columns,
                    snapshot,
                    cancellation,
                )?)
            }
            Self::Statistics1(m) => json(m.calculate()?),
            Self::Statistics2(m) => {
                if m.fit_kind == super::native_regression::FitKind::Linear {
                    json(m.linear_fit()?)
                } else {
                    json(m.fit()?)
                }
            }
            Self::Linear(m) => json(m.solve()?),
            Self::Finance(m, unknown) => json(m.solve(unknown)?),
            Self::Triangle(m) => json(m.solve()?),
            Self::Solve(equation, variable) => json(super::native_equations::solve(
                &equation, &variable, snapshot, cancel,
            )?),
        }
    }
}
pub const MODEL_APPS: &[&str] = &[
    "Statistics 1Var",
    "Statistics 2Var",
    "Linear Solver",
    "Triangle Solver",
    "Finance",
    "Solve",
    "Geometry",
    "Spreadsheet",
    "Inference",
];
pub fn numeric(name: &str, memory: &AppMemory) -> Option<Calculation> {
    Some(match name {
        "Statistics 1Var" => {
            if let Some(book) = &memory.statistics1_book {
                Calculation::Statistics1Book(Box::new(book.clone()))
            } else {
                Calculation::Statistics1(memory.statistics1.clone())
            }
        }
        "Statistics 2Var" => {
            if let Some(book) = &memory.statistics2_book {
                Calculation::Statistics2Book(Box::new(book.clone()))
            } else {
                Calculation::Statistics2(memory.statistics2.clone())
            }
        }
        "Finance" => Calculation::Amortization(memory.finance.clone(), memory.finance_group),
        "Triangle Solver" => Calculation::Triangle(memory.triangle.clone()),
        "Linear Solver" => Calculation::Linear(memory.linear.clone()),
        "Geometry" => {
            if let Some(book) = &memory.geometry_book {
                Calculation::GeometryBook(Box::new(book.clone()))
            } else {
                Calculation::Geometry(memory.geometry.clone())
            }
        }
        "Spreadsheet" => Calculation::Spreadsheet(memory.spreadsheet.clone()),
        "Inference" => Calculation::Inference(memory.inference.clone()),
        "Solve" => Calculation::Solve(memory.equation.clone(), memory.solve_variable.clone()),
        _ => return None,
    })
}
fn display_value(value: &serde_json::Value, settings: &ppl_vm::Settings) -> String {
    if let Some(number) = value.as_f64() {
        return ppl_math::Real::from_f64(number)
            .map(|value| value.format(settings.format()))
            .unwrap_or_else(|_| number.to_string());
    }
    if let Some(text) = value.as_str() {
        return text.into();
    }
    if let Some(values) = value.as_array() {
        return format!(
            "[{}]",
            values
                .iter()
                .map(|value| display_value(value, settings))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if value.get("type").is_some()
        && let Ok(value) = serde_json::from_value::<ppl_cas::CasValue>(value.clone())
    {
        return value.to_string();
    }
    if let Some(value) = value.get("Number").or_else(|| value.get("Text")) {
        return display_value(value, settings);
    }
    if let Some(text) = value
        .get("Object")
        .and_then(|value| value["display"].as_str())
    {
        return text.into();
    }
    if let Some(map) = value.as_object() {
        return map
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(key, value)| {
                format!(
                    "{}: {}",
                    key.replace('_', " "),
                    display_value(value, settings)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
    }
    value.to_string()
}
pub fn show_result(ui: &mut egui::Ui, result: &serde_json::Value, settings: &ppl_vm::Settings) {
    if result.get("groups").is_some() {
        match serde_json::from_value::<super::native_model_plots::AmortizationSchedule>(
            result.clone(),
        ) {
            Ok(schedule) => {
                ui.label("Payments · Principal · Interest · Balance");
                for row in schedule.groups {
                    crate::ide::native_fonts::monospace(
                        ui,
                        format!(
                            "{}–{}  {:.2}  {:.2}  {:.2}",
                            row.first_payment,
                            row.last_payment,
                            row.principal,
                            row.interest,
                            row.balance
                        ),
                    );
                }
            }
            Err(_) => {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    "Invalid amortization result; recalculate",
                );
            }
        }
    } else if result.get("type").is_some() {
        crate::ide::native_fonts::monospace(ui, display_value(result, settings));
    } else if let Some(map) = result.as_object() {
        for (key, value) in map.iter().filter(|(_, value)| !value.is_null()) {
            ui.label(format!(
                "{}: {}",
                key.replace('_', " "),
                display_value(value, settings)
            ));
        }
    } else {
        crate::ide::native_fonts::monospace(ui, display_value(result, settings));
    }
}
pub fn show(
    ui: &mut egui::Ui,
    name: &str,
    memory: &mut AppMemory,
    settings: &ppl_vm::Settings,
) -> Option<Calculation> {
    let mut request = None;
    match name {
        "Statistics 1Var" => {
            let model = &mut memory.statistics1;
            for i in 0..model.samples.len() {
                ui.horizontal(|ui| {
                    ui.label(format!("D{}", i + 1));
                    ui.add(egui::DragValue::new(&mut model.samples[i]));
                    if !model.frequencies.is_empty() {
                        ui.add(egui::DragValue::new(&mut model.frequencies[i]).prefix("freq "));
                    }
                });
            }
            if model.samples.len() < 1000 && ui.button("Add observation").clicked() {
                model.samples.push(0.);
                if !model.frequencies.is_empty() {
                    model.frequencies.push(1);
                }
            }
            if !model.samples.is_empty() && ui.button("Remove last observation").clicked() {
                model.samples.pop();
                if !model.frequencies.is_empty() {
                    model.frequencies.pop();
                }
            }
            let mut frequency = !model.frequencies.is_empty();
            if ui.checkbox(&mut frequency, "Frequencies").changed() {
                model.frequencies = if frequency {
                    vec![1; model.samples.len()]
                } else {
                    Vec::new()
                };
            }
            if ui.button("Calculate").clicked() {
                request = Some(Calculation::Statistics1(model.clone()));
            }
        }
        "Statistics 2Var" => {
            let model = &mut memory.statistics2;
            for row in &mut model.pairs {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut row[0]).prefix("x "));
                    ui.add(egui::DragValue::new(&mut row[1]).prefix("y "));
                });
            }
            if model.pairs.len() < 1000 && ui.button("Add pair").clicked() {
                model.pairs.push([0.; 2]);
            }
            if !model.pairs.is_empty() && ui.button("Remove last pair").clicked() {
                model.pairs.pop();
            }
            egui::ComboBox::from_label("Fit type")
                .selected_text(model.fit_kind.label())
                .show_ui(ui, |ui| {
                    for kind in super::native_regression::FitKind::ALL {
                        ui.selectable_value(&mut model.fit_kind, kind, kind.label());
                    }
                });
            if ui.button("Calculate fit").clicked() {
                request = Some(Calculation::Statistics2(model.clone()));
            }
        }
        "Linear Solver" => {
            let model = &mut memory.linear;
            let mut size = model.constants.len();
            if ui
                .add(egui::Slider::new(&mut size, 2..=3).text("Unknowns"))
                .changed()
            {
                model.coefficients.resize_with(size, || vec![0.; size]);
                for row in &mut model.coefficients {
                    row.resize(size, 0.);
                }
                model.constants.resize(size, 0.);
            }
            for i in 0..size {
                ui.horizontal(|ui| {
                    for value in &mut model.coefficients[i] {
                        ui.add(egui::DragValue::new(value).speed(0.1));
                    }
                    ui.label("=");
                    ui.add(egui::DragValue::new(&mut model.constants[i]).speed(0.1));
                });
            }
            if ui.button("Solve").clicked() {
                request = Some(Calculation::Linear(model.clone()));
            }
        }
        "Finance" => {
            let mut selected = None;
            let model = &mut memory.finance;
            for (label, value, unknown) in [
                ("N", &mut model.periods, FinanceUnknown::Periods),
                ("I%YR", &mut model.annual_rate, FinanceUnknown::AnnualRate),
                ("PV", &mut model.present_value, FinanceUnknown::PresentValue),
                ("PMT", &mut model.payment, FinanceUnknown::Payment),
                ("FV", &mut model.future_value, FinanceUnknown::FutureValue),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(
                        egui::DragValue::new(value)
                            .speed(0.01)
                            .min_decimals(2)
                            .max_decimals(12),
                    );
                    if ui.small_button("Solve").clicked() {
                        selected = Some(unknown);
                    }
                });
            }
            if let Some(unknown) = selected {
                request = Some(Calculation::Finance(memory.finance.clone(), unknown));
            }
            ui.horizontal(|ui| {
                ui.label("P/YR");
                ui.add(
                    egui::DragValue::new(&mut memory.finance.payments_per_year).range(1.0..=365.),
                );
                ui.label("C/YR");
                ui.add(
                    egui::DragValue::new(&mut memory.finance.compounds_per_year).range(1.0..=365.),
                );
            });
            ui.checkbox(&mut memory.finance.begin, "Payments at beginning");
        }
        "Triangle Solver" => {
            for i in 0..3 {
                ui.horizontal(|ui| {
                    let mut known = memory.triangle.sides[i].is_some();
                    if ui
                        .checkbox(&mut known, format!("Side {}", ['A', 'B', 'C'][i]))
                        .changed()
                    {
                        memory.triangle.sides[i] = known.then_some(1.);
                    }
                    if let Some(value) = &mut memory.triangle.sides[i] {
                        ui.add(egui::DragValue::new(value).speed(0.1));
                    }
                });
                ui.horizontal(|ui| {
                    let mut known = memory.triangle.angles_degrees[i].is_some();
                    if ui
                        .checkbox(&mut known, format!("Angle {}°", ['A', 'B', 'C'][i]))
                        .changed()
                    {
                        memory.triangle.angles_degrees[i] = known.then_some(60.);
                    }
                    if let Some(value) = &mut memory.triangle.angles_degrees[i] {
                        ui.add(egui::DragValue::new(value).speed(0.1));
                    }
                });
            }
            if ui.button("Solve").clicked() {
                request = Some(Calculation::Triangle(memory.triangle.clone()));
            }
        }
        "Geometry" => {
            for (name, point) in &mut memory.geometry.points {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.add(egui::DragValue::new(&mut point.x).prefix("x ").speed(0.1));
                    ui.add(egui::DragValue::new(&mut point.y).prefix("y ").speed(0.1));
                });
            }
            if ui.button("Add point").clicked() {
                let name = format!("P{}", memory.geometry.points.len() + 1);
                memory.geometry.points.insert(name, Point2 { x: 0., y: 0. });
            }
            if ui.button("Join last two points").clicked() {
                let names = memory
                    .geometry
                    .points
                    .keys()
                    .rev()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>();
                if names.len() == 2 {
                    memory
                        .geometry
                        .segments
                        .push([names[1].clone(), names[0].clone()]);
                }
            }
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 100.), egui::Sense::hover());
            let painter = ui.painter().with_clip_rect(rect);
            let project = |p: &Point2| {
                egui::pos2(
                    rect.center().x + p.x as f32 * rect.width() / 20.,
                    rect.center().y - p.y as f32 * rect.height() / 20.,
                )
            };
            for segment in &memory.geometry.segments {
                if let (Some(a), Some(b)) = (
                    memory.geometry.points.get(&segment[0]),
                    memory.geometry.points.get(&segment[1]),
                ) {
                    painter.line_segment(
                        [project(a), project(b)],
                        egui::Stroke::new(1.0_f32, egui::Color32::BLUE),
                    );
                }
            }
            for (name, point) in &memory.geometry.points {
                painter.circle_filled(project(point), 2., egui::Color32::DARK_RED);
                painter.text(
                    project(point),
                    egui::Align2::LEFT_BOTTOM,
                    name,
                    crate::ide::native_fonts::font_id(ui, egui::TextStyle::Body, 10.),
                    egui::Color32::BLACK,
                );
            }
            if ui.button("Measure segments").clicked() {
                request = Some(Calculation::Geometry(memory.geometry.clone()));
            }
        }
        "Inference" => {
            if super::native_inference_ui::show(ui, &mut memory.inference) {
                request = Some(Calculation::Inference(memory.inference.clone()));
            }
        }
        "Solve" => {
            ui.label("Equation");
            ui.text_edit_singleline(&mut memory.equation);
            ui.label("Variable");
            ui.text_edit_singleline(&mut memory.solve_variable);
            if ui.button("Solve").clicked() {
                request = Some(Calculation::Solve(
                    memory.equation.clone(),
                    memory.solve_variable.clone(),
                ));
            }
        }
        _ => {}
    }
    if let Some(result) = memory.results.get(name) {
        ui.separator();
        show_result(ui, result, settings);
    }
    request
}

impl AppMemory {
    pub fn ensure_statistics_book(&mut self, two: bool) -> Result<bool, String> {
        if two {
            if self.statistics2_book.is_none() {
                self.statistics2_book = Some(super::native_statistics::TwoBook::from_legacy(
                    &self.statistics2,
                ));
                return Ok(true);
            }
        } else if self.statistics1_book.is_none() {
            self.statistics1_book = Some(super::native_statistics::OneBook::from_legacy(
                &self.statistics1,
            )?);
            return Ok(true);
        }
        Ok(false)
    }
    pub fn statistics_columns(&self, two: bool) -> Option<&super::native_statistics::Columns> {
        if two {
            self.statistics2_book.as_ref().map(|book| &book.columns)
        } else {
            self.statistics1_book.as_ref().map(|book| &book.columns)
        }
    }
    pub fn statistics_columns_mut(
        &mut self,
        two: bool,
    ) -> Option<&mut super::native_statistics::Columns> {
        if two {
            self.statistics2_book.as_mut().map(|book| &mut book.columns)
        } else {
            self.statistics1_book.as_mut().map(|book| &mut book.columns)
        }
    }

    pub fn select_cell(&mut self, name: &str) -> bool {
        let Some((col, row)) = cell_coordinates(name) else {
            return false;
        };
        self.selected_cell = cell_name(col, row);
        self.cell_address = self.selected_cell.clone();
        self.cell_source = self
            .spreadsheet
            .cells
            .get(&self.selected_cell)
            .cloned()
            .unwrap_or_default();
        true
    }
    pub fn ensure_geometry_book(&mut self) -> Result<(), String> {
        if self.geometry_book.is_none() {
            self.geometry_book = Some(super::native_geometry::Book::from_legacy(&self.geometry)?);
        }
        Ok(())
    }
    pub fn input_fingerprint(&self) -> serde_json::Value {
        let mut input = self.clone();
        input.results.clear();
        serde_json::to_value(input).expect("finite calculator app data")
    }
    pub fn validate(&self) -> Result<(), String> {
        // JSON has no NaN/infinity. Refuse such drafts before a durable profile
        // can turn their typed numerical fields into unreadable null values.
        let image = serde_json::to_value(self).map_err(|e| e.to_string())?;
        serde_json::from_value::<Self>(image)
            .map_err(|_| "App inputs must be finite".to_string())?;
        if self.statistics1.samples.len() > 1000
            || (!self.statistics1.frequencies.is_empty()
                && self.statistics1.frequencies.len() != self.statistics1.samples.len())
        {
            return Err("Invalid statistics data dimensions".into());
        }
        if self.statistics2.pairs.len() > 1000 {
            return Err("Statistics data exceeds the supported row limit".into());
        }
        if let Some(book) = &self.statistics1_book {
            book.validate()?;
        }
        if let Some(book) = &self.statistics2_book {
            book.validate()?;
        }
        let n = self.linear.constants.len();
        if !(2..=3).contains(&n)
            || self.linear.coefficients.len() != n
            || self.linear.coefficients.iter().any(|r| r.len() != n)
        {
            return Err("Invalid linear solver dimensions".into());
        }
        if let Some(book) = &self.geometry_book {
            book.validate()?;
        }
        if self.geometry.points.len() > 1000 || self.geometry.segments.len() > 2000 {
            return Err("Geometry exceeds object limit".into());
        }
        if self.spreadsheet.cells.len() > 10000 {
            return Err("Spreadsheet exceeds cell limit".into());
        }
        self.spreadsheet.validate()?;
        Ok(())
    }
}
