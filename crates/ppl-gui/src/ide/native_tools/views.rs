//! LCD page controls; execution and state ownership stay in NativeTools.
use super::*;
impl NativeTools {
    pub(super) fn show_page(
        &mut self,
        ui: &mut egui::Ui,
        page: Page,
        lcd: Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        if self.memory.selected_app == super::super::native_graph3d::NAME
            && self.show_graph3d(ui, page, lcd, scale, snapshot)
        {
            return;
        }
        let mut app_snapshot = snapshot.clone();
        app_snapshot.active_app = self.memory.selected_app.clone();
        let app_settings = app_snapshot.effective_app_settings();
        match page {
            Page::Base => {
                if let Some(word) = &mut self.integer_edit {
                    ui.label(format!(
                        "Was: {}",
                        self.integer_original
                            .as_ref()
                            .map(|word| word.output())
                            .unwrap_or_default()
                    ));
                    ui.label(format!("Out: {}", word.output()));
                    let mut decimal = self.integer_decimal_draft.clone();
                    ui.horizontal(|ui| {
                        ui.label("Decimal");
                        if ui.text_edit_singleline(&mut decimal).changed() {
                            self.integer_decimal_draft = decimal.clone();
                            if let Err(error) = word.set_decimal(&decimal) {
                                self.error = Some(error);
                            } else {
                                self.integer_hex_draft = format!("{:X}", word.raw);
                                self.error = None;
                            }
                        }
                    });
                    let mut hex = self.integer_hex_draft.clone();
                    ui.horizontal(|ui| {
                        ui.label("Hex");
                        if ui.text_edit_singleline(&mut hex).changed() {
                            self.integer_hex_draft = hex.clone();
                            if let Err(error) = word.set_hex(&hex) {
                                self.error = Some(error);
                            } else {
                                self.integer_decimal_draft = word.decimal();
                                self.error = None;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let mut bits = word.bits;
                        if ui
                            .add(
                                egui::DragValue::new(&mut bits)
                                    .range(1..=64)
                                    .prefix("Bits "),
                            )
                            .changed()
                        {
                            if let Err(error) = word.resize(bits) {
                                self.error = Some(error);
                            }
                        }
                        let mut signed = word.signed;
                        if ui.checkbox(&mut signed, "Signed").changed()
                            && let Err(error) = word.set_signed(signed)
                        {
                            self.error = Some(error);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for bit in (0..word.display_bits()).rev() {
                            if ui
                                .small_button(if !word.bit(bit) { "0" } else { "1" })
                                .clicked()
                            {
                                word.toggle_bit(bit);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Shift ←").clicked() {
                            word.shift(true);
                        }
                        if ui.button("Shift →").clicked() {
                            word.shift(false);
                        }
                        if ui.button("Neg").clicked() {
                            word.negate();
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Base").clicked() {
                            word.cycle(true);
                        }
                        if ui.button("NOT").clicked() {
                            word.complement();
                        }
                        if ui.button("Reset").clicked() {
                            if let Some(original) = &self.integer_original {
                                *word = original.clone();
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            requests.push(Request::SaveInteger);
                        }
                        if ui.button("Cancel").clicked() {
                            requests.push(Request::CancelInteger);
                        }
                    });
                } else {
                    ui.label("Select an integer result in Home.");
                }
            }
            Page::Views => {
                for (index, action) in special_views::views(&self.memory.selected_app)
                    .iter()
                    .copied()
                    .enumerate()
                {
                    if ui.button(action.label()).clicked() {
                        self.open_special(index as i64 + 8, true, snapshot.clone());
                    }
                }
            }

            Page::Define => {
                ui.label("Function name");
                ui.text_edit_singleline(&mut self.memory.function_name);
                ui.label("Parameters (comma separated)");
                ui.text_edit_singleline(&mut self.memory.function_parameters);
                ui.label("Expression");
                if ui
                    .button(if self.memory.function_body.is_empty() {
                        "Enter expression"
                    } else {
                        &self.memory.function_body
                    })
                    .clicked()
                {
                    requests.push(Request::EditMath(MathTarget::FunctionBody));
                }
                if ui.button("Define").clicked() {
                    requests.push(Request::DefineFunction {
                        name: self.memory.function_name.clone(),
                        parameters: self.memory.function_parameters.clone(),
                        body: self.memory.function_body.clone(),
                    });
                }
            }
            Page::Units | Page::Toolbox | Page::Templates | Page::Memory => {
                self.show_picker(ui, page, lcd, scale, snapshot, requests);
            }
            Page::Sexagesimal => {
                use crate::ide::native::expression::TemplateKind as T;
                if ui.button("Decimal → hours/minutes/seconds").clicked() {
                    requests.push(Request::InsertTemplate(T::Function("→HMS")));
                }
                if ui.button("Hours/minutes/seconds → decimal").clicked() {
                    requests.push(Request::InsertTemplate(T::Function("HMS→")));
                }
            }
            Page::Apps => {} // Painted by the full LCD Application Library.
            Page::Symbolic => {
                ui.strong(&self.memory.selected_app);
                if let Some(parameters) = self.memory.explorers.get_mut(&self.memory.selected_app) {
                    let mut changed = false;
                    changed |= ui
                        .add(egui::Slider::new(&mut parameters[0], -10.0..=10.0).text("a"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut parameters[1], -10.0..=10.0).text("b"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut parameters[2], -10.0..=10.0).text("c"))
                        .changed();
                    if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                        if changed || app.curves[0].expression.is_empty() {
                            let [a, b, c] = *parameters;
                            app.curves[0].expression = match self.memory.selected_app.as_str() {
                                "Linear Explorer" => format!("({a})*X+({b})"),
                                "Quadratic Explorer" => format!("({a})*(X-({b}))^2+({c})"),
                                _ => format!("({a})*SIN(({b})*X+({c}))"),
                            };
                        }
                    }
                }
                if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                    for (i, curve) in app.curves.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut curve.enabled,
                                format!(
                                    "{}{}",
                                    match app.kind {
                                        GraphKind::Function => "F",
                                        GraphKind::Parametric => "X",
                                        GraphKind::Polar => "R",
                                        GraphKind::Sequence => "U",
                                        GraphKind::Implicit => "V",
                                    },
                                    i + 1
                                ),
                            );
                            if ui
                                .button(if curve.expression.is_empty() {
                                    "Enter expression"
                                } else {
                                    &curve.expression
                                })
                                .clicked()
                            {
                                requests.push(Request::EditMath(MathTarget::Curve {
                                    app: self.memory.selected_app.clone(),
                                    index: i,
                                    second: false,
                                    clear: false,
                                }));
                            }
                        });
                        if app.kind == GraphKind::Parametric
                            && ui
                                .button(if curve.second.is_empty() {
                                    "Enter Y(T)"
                                } else {
                                    &curve.second
                                })
                                .clicked()
                        {
                            requests.push(Request::EditMath(MathTarget::Curve {
                                app: self.memory.selected_app.clone(),
                                index: i,
                                second: true,
                                clear: false,
                            }));
                        }
                    }
                    if app.kind == GraphKind::Sequence {
                        for (index, curve) in app.curves.iter_mut().enumerate() {
                            ui.separator();
                            ui.label(format!("U{} initial terms", index + 1));
                            let mut recurrence = curve
                                .sequence
                                .as_ref()
                                .is_some_and(|definition| !definition.initial.is_empty());
                            if ui.checkbox(&mut recurrence, "Recurrence").changed() {
                                curve.sequence = if recurrence {
                                    Some(crate::ide::native_apps::SequenceDefinition {
                                        start: 1,
                                        initial: vec!["0".into()],
                                        offset: 0,
                                    })
                                } else {
                                    None
                                };
                            }
                            if let Some(definition) = &mut curve.sequence {
                                ui.add(
                                    egui::DragValue::new(&mut definition.start)
                                        .range(0..=1_000_000)
                                        .prefix("First index "),
                                );
                                ui.add(
                                    egui::Slider::new(&mut definition.offset, 0..=2)
                                        .text("Recurrence offset"),
                                );
                                for (term, value) in definition.initial.iter().enumerate() {
                                    if ui
                                        .button(format!("Initial {}: {value}", term + 1))
                                        .clicked()
                                    {
                                        requests.push(Request::EditMath(
                                            MathTarget::SequenceInitial {
                                                app: self.memory.selected_app.clone(),
                                                index,
                                                term,
                                            },
                                        ));
                                    }
                                }
                                if definition.initial.len() < 2
                                    && ui.button("Second initial term").clicked()
                                {
                                    definition.initial.push("0".into());
                                }
                            }
                        }
                    }
                    if app.curves.len() < 10 && ui.button("Add expression").clicked() {
                        app.curves.push(crate::ide::native_apps::Curve {
                            expression: String::new(),
                            second: String::new(),
                            enabled: true,
                            sequence: None,
                            literal: None,
                        });
                    }
                }
                if crate::ide::native_app_ui::MODEL_APPS
                    .contains(&self.memory.selected_app.as_str())
                {
                    if let Some(calculation) = crate::ide::native_app_ui::show(
                        ui,
                        &self.memory.selected_app,
                        &mut self.memory.models,
                        &app_settings,
                    ) {
                        requests.push(Request::Calculate(calculation));
                    }
                }
                ui.horizontal(|ui| {
                    if self.supports_page(Page::Plot) && ui.button("Plot").clicked() {
                        self.page = Some(Page::Plot);
                        self.plot(snapshot.clone());
                    }
                    if self.supports_page(Page::Numeric) && ui.button("Numeric").clicked() {
                        self.page = Some(Page::Numeric);
                        self.table(snapshot);
                    }
                    if self.supports_page(Page::PlotSetup) && ui.button("Setup").clicked() {
                        self.page = Some(Page::PlotSetup);
                    }
                });
            }
            Page::Plot | Page::PlotDetail | Page::PlotTable => {
                if self.memory.selected_app == "Finance" {
                    let before = self.memory.models.finance_group;
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.memory.models.finance_group)
                                .range(1..=10000)
                                .prefix("Payments per group "),
                        )
                        .changed()
                        && before != self.memory.models.finance_group
                    {
                        self.plot(snapshot.clone());
                    }
                }
                let previous = self
                    .memory
                    .graphs
                    .get(&self.memory.selected_app)
                    .map(|app| app.curves[0].expression.clone());
                self.show_explorer(ui);
                if previous
                    != self
                        .memory
                        .graphs
                        .get(&self.memory.selected_app)
                        .map(|app| app.curves[0].expression.clone())
                {
                    self.plot(snapshot.clone());
                }
                ui.horizontal(|ui| {
                    for (label, factor) in [("+", 0.5), ("−", 2.)] {
                        if ui.small_button(label).clicked() {
                            if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app)
                            {
                                app.window.zoom(factor);
                                self.plot(snapshot.clone());
                            } else {
                                self.memory
                                    .model_windows
                                    .entry(self.memory.selected_app.clone())
                                    .or_default()
                                    .zoom(factor);
                                if matches!(
                                    self.memory.selected_app.as_str(),
                                    "Solve" | "Statistics 2Var"
                                ) {
                                    self.plot(snapshot.clone());
                                }
                            }
                        }
                    }
                    if self.supports_page(Page::Symbolic) && ui.small_button("Symb").clicked() {
                        self.page = Some(Page::Symbolic);
                    }
                    if ui.small_button("Setup").clicked() {
                        self.page = Some(Page::PlotSetup);
                    }
                    if !self.memory.graphs.contains_key(&self.memory.selected_app)
                        && ui.small_button("Auto").clicked()
                    {
                        self.autoscale(snapshot.clone());
                    }
                });
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.trace.enabled, "Trace").clicked() {
                        self.trace_toggle();
                    }
                    if self.memory.graphs.contains_key(&self.memory.selected_app)
                        && ui.small_button("Auto").clicked()
                    {
                        self.autoscale(snapshot.clone());
                    }
                });
                if page != Page::Plot {
                    self.show_split(ui, lcd, scale, snapshot);
                    return;
                }
                let size = egui::vec2((lcd.width() - 4.).max(1.), 145. * scale);
                let colors = self.graph_plot_colors();
                if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                    let trace = self.trace.current(&self.points).map(|point| point.point);
                    let interaction = crate::ide::native_plot_view::show_graph(
                        ui,
                        &mut app.window,
                        &self.points,
                        &colors,
                        size,
                        scale,
                        trace,
                    );
                    if let Some(point) = interaction.clicked {
                        if let Err(error) = self.trace.nearest_in_view(
                            point,
                            &app.window,
                            &self.points,
                            [f64::from(size.x), f64::from(size.y)],
                        ) {
                            self.error = Some(error);
                        }
                    }
                    if interaction.drag_stopped {
                        self.plot(snapshot.clone());
                    }
                } else if let Some(value) = self
                    .memory
                    .models
                    .results
                    .get(&format!("{}:plot", self.memory.selected_app))
                {
                    match serde_json::from_value::<crate::ide::native_model_plots::PlotData>(
                        value.clone(),
                    ) {
                        Ok(data) => {
                            let window = self
                                .memory
                                .model_windows
                                .entry(self.memory.selected_app.clone())
                                .or_insert_with(|| {
                                    let window = Default::default();
                                    crate::ide::native_plot_controls::autoscale_model(
                                        &window, &data,
                                    )
                                    .ok()
                                    .flatten()
                                    .unwrap_or(window)
                                });
                            let curves = data
                                .curves
                                .iter()
                                .map(|curve| curve.iter().copied().map(Some).collect())
                                .collect::<Vec<_>>();
                            let trace = self.trace.current(&curves).map(|point| point.point);
                            let interaction = crate::ide::native_plot_view::show(
                                ui,
                                window,
                                &curves,
                                &data.styles,
                                size,
                                scale,
                                trace,
                            );
                            if let Some(point) = interaction.clicked {
                                if let Err(error) = self.trace.nearest_in_view(
                                    point,
                                    window,
                                    &curves,
                                    [f64::from(size.x), f64::from(size.y)],
                                ) {
                                    self.error = Some(error);
                                }
                            }
                            if interaction.drag_stopped
                                && matches!(
                                    self.memory.selected_app.as_str(),
                                    "Solve" | "Statistics 2Var"
                                )
                            {
                                self.plot(snapshot.clone());
                            }
                            ui.label(format!("{} / {}", data.x_label, data.y_label));
                            for label in data.labels {
                                ui.small(label);
                            }
                        }
                        Err(error) => {
                            ui.colored_label(Color32::DARK_RED, error.to_string());
                        }
                    }
                } else {
                    ui.label("Use Plot to calculate this app's data.");
                }
                if self.job.is_some() {
                    ui.label("Calculating…");
                }
            }
            Page::Numeric => {
                ui.horizontal(|ui| {
                    if self.supports_page(Page::Symbolic) && ui.button("Symb").clicked() {
                        self.page = Some(Page::Symbolic);
                    }
                    if self.supports_page(Page::NumericSetup) && ui.button("Setup").clicked() {
                        self.page = Some(Page::NumericSetup);
                    }
                    if ui.button("Refresh").clicked() {
                        self.table(snapshot);
                    }
                });
                if self.memory.graphs.contains_key(&self.memory.selected_app) {
                    for row in &self.table {
                        crate::ide::native_fonts::monospace(
                            ui,
                            row.iter()
                                .map(|v| {
                                    v.map_or_else(
                                        || "undefined".into(),
                                        |v| {
                                            ppl_math::Real::from_f64(v)
                                                .map(|n| n.format(app_settings.home_format()))
                                                .unwrap_or_else(|_| "undefined".into())
                                        },
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("   "),
                        );
                    }
                } else {
                    if let Some(calculation) = crate::ide::native_app_ui::show(
                        ui,
                        &self.memory.selected_app,
                        &mut self.memory.models,
                        &app_settings,
                    ) {
                        requests.push(Request::Calculate(calculation));
                    }
                    if self.memory.selected_app == "Finance"
                        && let Some(value) = self.memory.models.results.get("Finance:amortization")
                    {
                        ui.separator();
                        crate::ide::native_app_ui::show_result(ui, value, &app_settings);
                    }
                }
            }

            Page::PlotSetup | Page::NumericSetup => {
                if page == Page::PlotSetup
                    && self.statistics_unbounded_plot()
                    && self.statistics_plot_setup.is_none()
                    && let Some(settings) = self.plot_settings_cache.get("Statistics 1Var")
                {
                    self.statistics_plot_setup = Some(Self::settings_window(settings));
                }
                let editing_unbounded =
                    page == Page::PlotSetup && self.statistics_plot_setup.is_some();
                if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                    if page == Page::NumericSetup {
                        if app.kind == GraphKind::Implicit {
                            ui.horizontal(|ui| {
                                ui.label("Y");
                                ui.add(egui::DragValue::new(&mut app.numeric_y));
                            });
                        }
                        for (label, value) in [
                            ("Start", &mut app.numeric_start),
                            ("Step", &mut app.numeric_step),
                        ] {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add(egui::DragValue::new(value));
                            });
                        }
                    } else {
                        let w = &mut app.window;
                        for (label, value) in [
                            ("Xmin", &mut w.xmin),
                            ("Xmax", &mut w.xmax),
                            ("Ymin", &mut w.ymin),
                            ("Ymax", &mut w.ymax),
                            ("Tmin", &mut w.tmin),
                            ("Tmax", &mut w.tmax),
                        ] {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add(egui::DragValue::new(value));
                            });
                        }
                        ui.checkbox(&mut w.axes, "Axes");
                    }
                }
                if !self.memory.graphs.contains_key(&self.memory.selected_app) {
                    if self.memory.selected_app == "Finance" {
                        ui.add(
                            egui::DragValue::new(&mut self.memory.models.finance_group)
                                .range(1..=10000)
                                .prefix("Payments per group "),
                        );
                    }
                    if self.memory.selected_app == "Statistics 1Var" {
                        let mut manual = self.memory.models.histogram.is_some();
                        if ui
                            .checkbox(&mut manual, "Set histogram range/width")
                            .changed()
                        {
                            self.memory.models.histogram = if manual {
                                Some(crate::ide::native_model_plots::HistogramSpec {
                                    minimum: 0.,
                                    maximum: 10.,
                                    width: 1.,
                                })
                            } else {
                                None
                            };
                        }
                        if let Some(spec) = &mut self.memory.models.histogram {
                            for (label, value) in [
                                ("Minimum", &mut spec.minimum),
                                ("Maximum", &mut spec.maximum),
                                ("Width", &mut spec.width),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(label);
                                    ui.add(egui::DragValue::new(value));
                                });
                            }
                        }
                    }
                    let window = if editing_unbounded {
                        self.statistics_plot_setup.as_mut().unwrap()
                    } else {
                        self.memory
                            .model_windows
                            .entry(self.memory.selected_app.clone())
                            .or_default()
                    };
                    for (label, value) in [
                        ("Xmin", &mut window.xmin),
                        ("Xmax", &mut window.xmax),
                        ("Ymin", &mut window.ymin),
                        ("Ymax", &mut window.ymax),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            let control = egui::DragValue::new(value);
                            ui.add(if editing_unbounded && matches!(label, "Ymin" | "Ymax") {
                                control.custom_formatter(|number, _| match number {
                                    f64::NEG_INFINITY => "-Inf".into(),
                                    f64::INFINITY => "+Inf".into(),
                                    value => value.to_string(),
                                })
                            } else {
                                control
                            });
                        });
                    }
                    if page == Page::PlotSetup {
                        ui.checkbox(&mut window.axes, "Axes");
                    }
                }
                if ui.button("Apply").clicked() {
                    let mut next_snapshot = snapshot.clone();
                    if page == Page::PlotSetup {
                        let name = self.memory.selected_app.clone();
                        let mut settings = snapshot.plot_settings_for(&name);
                        if editing_unbounded {
                            settings = self.plot_settings_cache.get(&name).cloned();
                        }
                        if let Some(mut settings) = settings {
                            let window = self
                                .statistics_plot_setup
                                .clone()
                                .filter(|_| editing_unbounded)
                                .unwrap_or_else(|| self.trace_window());
                            let admissible = if editing_unbounded
                                && (window.ymin.is_infinite() || window.ymax.is_infinite())
                            {
                                let mut finite = window.clone();
                                finite.ymin = 0.;
                                finite.ymax = 1.;
                                finite.validate()
                            } else {
                                window.validate()
                            };
                            let result = admissible.and_then(|()| {
                                for (field, number) in [
                                    ("Xmin", window.xmin),
                                    ("Xmax", window.xmax),
                                    ("Ymin", window.ymin),
                                    ("Ymax", window.ymax),
                                    ("Axes", if window.axes { 1. } else { 0. }),
                                ] {
                                    let value = match number {
                                        f64::NEG_INFINITY => {
                                            ppl_vm::PlotBound::NegativeInfinity.to_value()
                                        }
                                        f64::INFINITY => {
                                            ppl_vm::PlotBound::PositiveInfinity.to_value()
                                        }
                                        finite => ppl_vm::Value::Real(
                                            ppl_math::Real::from_f64(finite)
                                                .map_err(|e| e.to_string())?,
                                        ),
                                    };
                                    settings.set(field, &value).map_err(|e| e.to_string())?;
                                }
                                if name == "Statistics 1Var"
                                    && let Some(spec) = self.memory.models.histogram
                                {
                                    Self::set_histogram_settings(&mut settings, spec)?;
                                }
                                settings.validate_for(&name)
                            });
                            if let Err(error) = result {
                                self.error = Some(error);
                                return;
                            }
                            if editing_unbounded {
                                if !settings.has_infinite_y() {
                                    self.memory.model_windows.insert(name.clone(), window);
                                }
                                self.plot_settings_cache
                                    .insert(name.clone(), settings.clone());
                                next_snapshot
                                    .app_plot_settings
                                    .insert(name.clone(), settings.clone());
                                self.statistics_plot_setup = None;
                            }
                            requests.push(Request::PlotConfiguration(name, settings));
                        }
                    }
                    self.dirty();
                    if page == Page::NumericSetup {
                        self.page = Some(Page::Numeric);
                        self.table(snapshot);
                    } else {
                        self.page = Some(Page::Plot);
                        self.plot(next_snapshot);
                    }
                }
            }
            Page::Settings => unreachable!("Home settings use their measured LCD panel"),
            Page::SymbolicSetup => {
                let name = &self.memory.selected_app;
                ui.strong(name);
                let mut settings = snapshot.app_settings.get(name).cloned().unwrap_or_default();
                let mut changed = false;
                for (label, value, choices) in [
                    (
                        "Angle",
                        &mut settings.angle,
                        vec!["System", "Radians", "Degrees", "Gradians"],
                    ),
                    (
                        "Number format",
                        &mut settings.format,
                        vec!["System", "Standard", "Fixed", "Scientific", "Engineering"],
                    ),
                    (
                        "Complex",
                        &mut settings.complex,
                        vec!["System", "On", "Off"],
                    ),
                ] {
                    egui::ComboBox::from_label(label)
                        .selected_text(choices[*value as usize])
                        .show_ui(ui, |ui| {
                            for (index, label) in choices.iter().enumerate() {
                                changed |=
                                    ui.selectable_value(value, index as i64, *label).changed();
                            }
                        });
                }
                if settings.format >= 2 {
                    changed |= ui
                        .add(egui::Slider::new(&mut settings.digits, 1..=10).text("Digits"))
                        .changed();
                }
                if ui.button("Restore System settings").clicked() {
                    settings = Default::default();
                    changed = true;
                }
                if changed {
                    requests.push(Request::SetAppSettings(name.clone(), settings));
                }
            }
            Page::CasSettings => {
                let mut settings = snapshot.cas.settings.clone();
                let mut changed = false;
                for (angle, label) in [
                    (ppl_cas::AngleMode::Radians, "Radians"),
                    (ppl_cas::AngleMode::Degrees, "Degrees"),
                    (ppl_cas::AngleMode::Gradians, "Gradians"),
                ] {
                    if ui
                        .selectable_label(settings.angle == angle, label)
                        .clicked()
                    {
                        settings.angle = angle;
                        changed = true;
                    }
                }
                changed |= ui
                    .checkbox(&mut settings.approximate, "Approximate CAS results")
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut settings.digits, 1..=100).text("Digits"))
                    .changed();
                if changed {
                    requests.push(Request::SetCasSettings(settings));
                }
            }
            Page::Programs | Page::Notes => {
                let objects = if page == Page::Programs {
                    &snapshot.programs
                } else {
                    &snapshot.notes
                };
                ui.add(egui::TextEdit::singleline(&mut self.memory.search).hint_text("Search"));
                for (name, text) in objects {
                    if !name
                        .to_lowercase()
                        .contains(&self.memory.search.to_lowercase())
                    {
                        continue;
                    }
                    if ui
                        .selectable_label(self.memory.name == *name, name)
                        .clicked()
                    {
                        self.memory.name = name.clone();
                        self.memory.original_name = name.clone();
                        self.memory.content = text.clone();
                        if page == Page::Notes {
                            self.load_note_bullets(ui.ctx(), snapshot.note_bullets.get(name));
                        } else {
                            self.clear_note_bullets();
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("New").clicked() {
                        self.memory.name.clear();
                        self.memory.original_name.clear();
                        self.memory.content.clear();
                        if page == Page::Notes {
                            self.load_note_bullets(ui.ctx(), None);
                        } else {
                            self.clear_note_bullets();
                        }
                    }
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.memory.name);
                });
                if !self.memory.original_name.is_empty()
                    && self.memory.name != self.memory.original_name
                    && ui.button("Rename").clicked()
                {
                    requests.push(Request::RenameObject {
                        old: self.memory.original_name.clone(),
                        new: self.memory.name.clone(),
                        program: page == Page::Programs,
                    });
                }
                if page == Page::Notes {
                    let bullets = self.note_bullets.get_or_insert_with(|| {
                        let name = &self.memory.original_name;
                        if snapshot.notes.get(name) == Some(&self.memory.content) {
                            snapshot.note_bullets.get(name).cloned().unwrap_or_default()
                        } else {
                            Default::default()
                        }
                    });
                    super::notes::show(ui, scale, &mut self.memory.content, bullets);
                } else {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.memory.content)
                            .id(egui::Id::new("native_catalog_content"))
                            .desired_rows(4)
                            .desired_width(300. * scale)
                            .font(egui::TextStyle::Monospace),
                    );
                }
                if page == Page::Programs {
                    ui.horizontal(|ui| {
                        ui.label("Entry");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.memory.program_entry)
                                .hint_text("Default export")
                                .desired_width(115. * scale),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Arguments");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.memory.program_args)
                                .hint_text("Prompt if empty")
                                .desired_width(150. * scale),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        requests.push(if page == Page::Programs {
                            Request::SaveProgram(
                                self.memory.name.clone(),
                                self.memory.content.clone(),
                            )
                        } else {
                            Request::SaveBulletedNote(
                                self.memory.name.clone(),
                                self.memory.content.clone(),
                                self.note_bullets.clone().unwrap_or_default(),
                            )
                        });
                    }
                    if ui.button("Delete").clicked() {
                        requests.push(if page == Page::Programs {
                            Request::DeleteProgram(self.memory.name.clone())
                        } else {
                            Request::DeleteNote(self.memory.name.clone())
                        });
                    }
                    if page == Page::Programs && ui.button("Run").clicked() {
                        requests.push(Request::RunProgramConfigured {
                            name: self.memory.name.clone(),
                            source: self.memory.content.clone(),
                            entry: (!self.memory.program_entry.trim().is_empty())
                                .then(|| self.memory.program_entry.trim().to_owned()),
                            args: self.memory.program_args.clone(),
                        });
                    }
                });
            }
            Page::Variables | Page::CasVariables | Page::Lists | Page::Matrices => {
                if matches!(page, Page::Variables | Page::CasVariables) {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(page == Page::Variables, "Home")
                            .clicked()
                        {
                            self.open(Page::Variables);
                        }
                        if ui
                            .selectable_label(page == Page::CasVariables, "CAS")
                            .clicked()
                        {
                            self.open(Page::CasVariables);
                        }
                    });
                }
                if page == Page::CasVariables {
                    for (name, value) in &snapshot.cas.bindings {
                        if ui
                            .selectable_label(self.memory.name == *name, format!("{name}: {value}"))
                            .clicked()
                        {
                            self.memory.name = name.clone();
                            self.memory.content = value.clone();
                        }
                    }
                }
                for (name, value) in &snapshot.globals {
                    if page == Page::CasVariables {
                        continue;
                    }
                    if (page == Page::Lists && !matches!(value, ppl_vm::PortableValue::List(_)))
                        || (page == Page::Matrices
                            && !matches!(value, ppl_vm::PortableValue::Matrix { .. }))
                    {
                        continue;
                    }
                    if ui
                        .selectable_label(
                            self.memory.name == *name,
                            format!("{name}: {}", value.display(&snapshot.settings)),
                        )
                        .clicked()
                    {
                        self.memory.name = name.clone();
                        self.memory.content = value.display(&snapshot.settings);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.memory.name);
                });
                ui.label("Value / expression");
                if ui
                    .button(if self.memory.content.is_empty() {
                        "Enter value"
                    } else {
                        &self.memory.content
                    })
                    .clicked()
                {
                    requests.push(Request::EditMath(MathTarget::Object));
                }
                if matches!(page, Page::Lists | Page::Matrices)
                    && ui.button("Open editor / Create").clicked()
                {
                    requests.push(Request::EditCollection {
                        name: self.memory.name.clone(),
                        matrix: page == Page::Matrices,
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Store").clicked() {
                        requests.push(if page == Page::CasVariables {
                            Request::StoreCasVariable(
                                self.memory.name.clone(),
                                self.memory.content.clone(),
                            )
                        } else {
                            Request::StoreVariable(
                                self.memory.name.clone(),
                                self.memory.content.clone(),
                            )
                        });
                    }
                    if ui.button("Delete").clicked() {
                        requests.push(if page == Page::CasVariables {
                            Request::DeleteCasVariable(self.memory.name.clone())
                        } else {
                            Request::DeleteVariable(self.memory.name.clone())
                        });
                    }
                    if ui.button("Insert").clicked() {
                        requests.push(Request::Insert(self.memory.name.clone()));
                    }
                });
            }
            Page::Characters => {
                let before = self.memory.characters.clone();
                self.memory.characters.show(ui, scale, requests);
                if self.memory.characters != before {
                    self.dirty();
                }
            }
            // HP User Guide ch.27: app Info is its own note, separate from
            // standalone Notes. Text changes use the same durable UI profile.
            Page::Info => {
                ui.strong(&self.memory.selected_app);
                ui.label("Info note");
                let note = self
                    .memory
                    .app_notes
                    .entry(self.memory.selected_app.clone())
                    .or_default();
                ui.add(
                    egui::TextEdit::multiline(note)
                        .id(egui::Id::new("native_app_info"))
                        .desired_width(300. * scale)
                        .desired_rows(8),
                );
            }
            Page::Help => self.show_help(ui, lcd, scale, requests),
        }
    }
}
