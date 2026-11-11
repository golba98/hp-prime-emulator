//! Input controls for app models. No evaluation is performed in the egui frame.
use super::native_app_models::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppMemory {
    pub statistics1: Statistics1,
    pub statistics2: Statistics2,
    pub linear: LinearSystem,
    pub finance: Finance,
    pub triangle: Triangle,
    pub geometry: Geometry,
    pub spreadsheet: Spreadsheet,
    pub selected_cell: String,
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
            linear: LinearSystem {
                coefficients: vec![vec![0.; 2]; 2],
                constants: vec![0.; 2],
            },
            finance: Default::default(),
            triangle: Default::default(),
            geometry: Default::default(),
            spreadsheet: Default::default(),
            selected_cell: "A1".into(),
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
    Linear(LinearSystem),
    Finance(Finance, FinanceUnknown),
    Triangle(Triangle),
    Solve(String, String),
    Geometry(Geometry),
    Spreadsheet(Spreadsheet),
    Inference(super::native_inference::InferenceRequest),
}
impl Calculation {
    pub fn app_name(&self) -> &str {
        match self {
            Self::Plot(name, _) => name,
            Self::Amortization(..) => "Finance",
            Self::Statistics1(_) => "Statistics 1Var",
            Self::Statistics2(_) => "Statistics 2Var",
            Self::Linear(_) => "Linear Solver",
            Self::Finance(_, _) => "Finance",
            Self::Triangle(_) => "Triangle Solver",
            Self::Solve(_, _) => "Solve",
            Self::Geometry(_) => "Geometry",
            Self::Spreadsheet(_) => "Spreadsheet",
            Self::Inference(_) => "Inference",
        }
    }
    pub fn result_key(&self) -> String {
        if matches!(self, Self::Amortization(..)) {
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
                        if let Some(spec) = &model.histogram {
                            plot::histogram(&model.statistics1, spec, cancel)
                        } else {
                            plot::histogram_auto(&model.statistics1, cancel)
                        }
                    }
                    "Statistics 2Var" => plot::statistics2(&model.statistics2, cancel),
                    "Finance" => plot::amortization(&model.finance, model.finance_group, cancel),
                    "Triangle Solver" => plot::triangle(&model.triangle, cancel),
                    "Geometry" => plot::geometry(&model.geometry, cancel),
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
            Self::Spreadsheet(model) => {
                let mut values = serde_json::Map::new();
                for name in model.cells.keys() {
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        return Err("Calculation interrupted".into());
                    }
                    values.insert(
                        name.clone(),
                        match model.evaluate(name, &Default::default()) {
                            Ok(value) => serde_json::to_value(value).map_err(|e| e.to_string())?,
                            Err(error) => serde_json::json!({"Error":error}),
                        },
                    );
                }
                Ok(serde_json::Value::Object(values))
            }
            Self::Statistics1(m) => json(m.calculate()?),
            Self::Statistics2(m) => json(m.linear_fit()?),
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
        "Statistics 1Var" => Calculation::Statistics1(memory.statistics1.clone()),
        "Statistics 2Var" => Calculation::Statistics2(memory.statistics2.clone()),
        "Finance" => Calculation::Amortization(memory.finance.clone(), memory.finance_group),
        "Triangle Solver" => Calculation::Triangle(memory.triangle.clone()),
        "Linear Solver" => Calculation::Linear(memory.linear.clone()),
        "Geometry" => Calculation::Geometry(memory.geometry.clone()),
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
                    ui.monospace(format!(
                        "{}–{}  {:.2}  {:.2}  {:.2}",
                        row.first_payment,
                        row.last_payment,
                        row.principal,
                        row.interest,
                        row.balance
                    ));
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
        ui.monospace(display_value(result, settings));
    } else if let Some(map) = result.as_object() {
        for (key, value) in map.iter().filter(|(_, value)| !value.is_null()) {
            ui.label(format!(
                "{}: {}",
                key.replace('_', " "),
                display_value(value, settings)
            ));
        }
    } else {
        ui.monospace(display_value(result, settings));
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
            if ui.button("Linear fit").clicked() {
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
                        egui::Stroke::new(1., egui::Color32::BLUE),
                    );
                }
            }
            for (name, point) in &memory.geometry.points {
                painter.circle_filled(project(point), 2., egui::Color32::DARK_RED);
                painter.text(
                    project(point),
                    egui::Align2::LEFT_BOTTOM,
                    name,
                    egui::FontId::proportional(10.),
                    egui::Color32::BLACK,
                );
            }
            if ui.button("Measure segments").clicked() {
                request = Some(Calculation::Geometry(memory.geometry.clone()));
            }
        }
        "Spreadsheet" => {
            ui.horizontal(|ui| {
                ui.label(&memory.selected_cell);
                ui.text_edit_singleline(&mut memory.cell_source);
            });
            if ui.button("Apply / calculate").clicked() {
                match memory
                    .spreadsheet
                    .set(&memory.selected_cell, &memory.cell_source)
                {
                    Ok(()) => request = Some(Calculation::Spreadsheet(memory.spreadsheet.clone())),
                    Err(error) => {
                        memory
                            .results
                            .insert(name.into(), serde_json::json!({"Error":error}));
                    }
                }
            }
            for row in 1..=10 {
                ui.horizontal(|ui| {
                    ui.label(row.to_string());
                    for col in ['A', 'B', 'C', 'D', 'E'] {
                        let name = format!("{col}{row}");
                        let value = memory.results.get("Spreadsheet").and_then(|v| v.get(&name));
                        let text = value
                            .and_then(|v| v.get("Number"))
                            .map(|v| v.to_string())
                            .or_else(|| {
                                value
                                    .and_then(|v| v.get("Text"))
                                    .and_then(|v| v.as_str())
                                    .map(str::to_owned)
                            })
                            .or_else(|| value.and_then(|v| v.get("Error")).map(|_| "#ERROR".into()))
                            .unwrap_or_else(|| {
                                memory
                                    .spreadsheet
                                    .cells
                                    .get(&name)
                                    .cloned()
                                    .unwrap_or_default()
                            });
                        if ui
                            .add(
                                egui::Button::new(format!("{name}: {text}"))
                                    .min_size(egui::vec2(45., 18.)),
                            )
                            .clicked()
                        {
                            memory.selected_cell = name.clone();
                            memory.cell_source = memory
                                .spreadsheet
                                .cells
                                .get(&name)
                                .cloned()
                                .unwrap_or_default();
                        }
                    }
                });
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
        let n = self.linear.constants.len();
        if !(2..=3).contains(&n)
            || self.linear.coefficients.len() != n
            || self.linear.coefficients.iter().any(|r| r.len() != n)
        {
            return Err("Invalid linear solver dimensions".into());
        }
        if self.geometry.points.len() > 1000 || self.geometry.segments.len() > 2000 {
            return Err("Geometry exceeds object limit".into());
        }
        if self.spreadsheet.cells.len() > 10000 {
            return Err("Spreadsheet exceeds cell limit".into());
        }
        let mut sheet = Spreadsheet::default();
        for (name, text) in &self.spreadsheet.cells {
            sheet.set(name, text)?;
        }
        Ok(())
    }
}
