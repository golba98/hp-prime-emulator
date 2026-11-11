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
                            word.resize(bits);
                        }
                        ui.checkbox(&mut word.signed, "Signed");
                    });
                    ui.horizontal_wrapped(|ui| {
                        for bit in (0..word.bits).rev() {
                            if ui
                                .small_button(if word.raw & (1u64 << bit) == 0 {
                                    "0"
                                } else {
                                    "1"
                                })
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
                for (page, label) in [
                    (Page::Symbolic, "Symbolic"),
                    (Page::Plot, "Plot"),
                    (Page::Numeric, "Numeric"),
                    (Page::PlotSetup, "Plot Setup"),
                    (Page::NumericSetup, "Numeric Setup"),
                    (Page::Info, "App Info"),
                ] {
                    if self.supports_page(page) && ui.button(label).clicked() {
                        self.page = Some(page);
                        if page == Page::Plot {
                            self.plot(snapshot.clone());
                        } else if page == Page::Numeric {
                            self.table(snapshot);
                        }
                    }
                }
                if ui.button("Autoscale").clicked() {
                    self.page = Some(Page::Plot);
                    self.autoscale(snapshot.clone());
                }
                for (label, preset) in [
                    ("Decimal", crate::ide::native_plot_controls::Preset::Decimal),
                    ("Integer", crate::ide::native_plot_controls::Preset::Integer),
                    ("Trig", crate::ide::native_plot_controls::Preset::Trig),
                ] {
                    if ui.button(label).clicked() {
                        self.page = Some(Page::Plot);
                        self.preset(preset, snapshot.clone());
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
            Page::Units => {
                for (label, unit) in [
                    ("metre", "_m"),
                    ("centimetre", "_cm"),
                    ("kilometre", "_km"),
                    ("second", "_s"),
                    ("kilogram", "_kg"),
                    ("kelvin", "_K"),
                    ("ampere", "_A"),
                    ("mole", "_mol"),
                    ("newton", "_N"),
                    ("joule", "_J"),
                    ("pascal", "_Pa"),
                    ("hertz", "_Hz"),
                ] {
                    if ui.button(label).clicked() {
                        requests.push(Request::Insert(unit.into()));
                    }
                }
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
            Page::Apps => {
                for name in self
                    .memory
                    .graphs
                    .keys()
                    .cloned()
                    .chain(
                        crate::ide::native_app_ui::MODEL_APPS
                            .iter()
                            .map(|s| s.to_string()),
                    )
                    .collect::<Vec<_>>()
                {
                    if ui
                        .selectable_label(self.memory.selected_app == name, &name)
                        .clicked()
                    {
                        self.memory.selected_app = name;
                        self.page = Some(Self::entry_page(&self.memory.selected_app));
                        if self.page == Some(Page::Plot) {
                            self.plot(snapshot.clone());
                        }
                        self.dirty();
                    }
                }
            }
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
                                    if app.kind == GraphKind::Sequence {
                                        "U"
                                    } else {
                                        "F"
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
            Page::Plot => {
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
                let size = egui::vec2((lcd.width() - 4.).max(1.), 145. * scale);
                if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                    let trace = self.trace.current(&self.points).map(|point| point.point);
                    let interaction = crate::ide::native_plot_view::show(
                        ui,
                        &mut app.window,
                        &self.points,
                        &[],
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
                        ui.monospace(
                            row.iter()
                                .map(|v| {
                                    v.map_or_else(|| "undefined".into(), |v| format!("{v:.8}"))
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
                    let window = self
                        .memory
                        .model_windows
                        .entry(self.memory.selected_app.clone())
                        .or_default();
                    for (label, value) in [
                        ("Xmin", &mut window.xmin),
                        ("Xmax", &mut window.xmax),
                        ("Ymin", &mut window.ymin),
                        ("Ymax", &mut window.ymax),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.add(egui::DragValue::new(value));
                        });
                    }
                }
                if ui.button("Apply").clicked() {
                    self.dirty();
                    if page == Page::NumericSetup {
                        self.page = Some(Page::Numeric);
                        self.table(snapshot);
                    } else {
                        self.page = Some(Page::Plot);
                        self.plot(snapshot.clone());
                    }
                }
            }
            Page::Settings => {
                let mut settings = snapshot.settings.clone();
                let mut changed = false;
                ui.label("Angle measure");
                for (angle, label) in [
                    (ppl_vm::AngleMode::Radians, "Radians"),
                    (ppl_vm::AngleMode::Degrees, "Degrees"),
                    (ppl_vm::AngleMode::Gradians, "Gradians"),
                ] {
                    changed |= ui
                        .selectable_value(&mut settings.angle, angle, label)
                        .changed();
                }
                for (code, label) in [
                    (0, "Standard"),
                    (1, "Fixed"),
                    (2, "Scientific"),
                    (3, "Engineering"),
                ] {
                    if ui
                        .selectable_label(settings.hformat == code, label)
                        .clicked()
                    {
                        settings.set_format(code, settings.hdigits);
                        changed = true;
                    }
                }
                let mut digits = settings.hdigits;
                if ui
                    .add(egui::Slider::new(&mut digits, 0..=11).text("Digits"))
                    .changed()
                {
                    settings.set_format(settings.hformat, digits);
                    changed = true;
                }
                changed |= ui
                    .checkbox(&mut settings.complex_results, "Complex results")
                    .changed();
                egui::ComboBox::from_label("Integer base")
                    .selected_text(format!("{:?}", settings.int_base))
                    .show_ui(ui, |ui| {
                        for (base, label) in [
                            (ppl_math::IntBase::Bin, "Binary"),
                            (ppl_math::IntBase::Oct, "Octal"),
                            (ppl_math::IntBase::Dec, "Decimal"),
                            (ppl_math::IntBase::Hex, "Hexadecimal"),
                        ] {
                            changed |= ui
                                .selectable_value(&mut settings.int_base, base, label)
                                .changed();
                        }
                    });
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut settings.int_bits)
                            .range(1..=64)
                            .prefix("Integer bits "),
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut settings.int_signed, "Signed integers")
                    .changed();
                if changed {
                    requests.push(Request::SetHomeSettings(settings));
                }
            }
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
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("New").clicked() {
                        self.memory.name.clear();
                        self.memory.original_name.clear();
                        self.memory.content.clear();
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
                ui.add(
                    egui::TextEdit::multiline(&mut self.memory.content)
                        .id(egui::Id::new("native_catalog_content"))
                        .desired_rows(4)
                        .desired_width(300. * scale)
                        .font(egui::TextStyle::Monospace),
                );
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
                            Request::SaveNote(self.memory.name.clone(), self.memory.content.clone())
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
            Page::Toolbox => {
                for spec in ppl_compiler::builtins::BUILTINS {
                    if ui
                        .button(format!("{} ({})", spec.name, spec.arity_text()))
                        .clicked()
                    {
                        if spec.min == 0 {
                            requests.push(Request::Insert(spec.name.into()));
                        } else {
                            requests.push(Request::InsertTemplate(
                                crate::ide::native::expression::TemplateKind::Function(spec.name),
                            ));
                        }
                    }
                }
            }
            Page::Templates => {
                use crate::ide::native::expression::TemplateKind as T;
                for (label, template) in [
                    ("Fraction", T::Fraction),
                    ("Power", T::Power),
                    ("Square root", T::Root),
                    ("Nth root", T::NthRoot),
                    ("Absolute value", T::Function("ABS")),
                    ("List", T::List),
                    ("Matrix", T::Matrix),
                ] {
                    if ui.button(label).clicked() {
                        requests.push(Request::InsertTemplate(template));
                    }
                }
            }
            Page::Characters => {
                ui.horizontal_wrapped(|ui| {
                    for text in [
                        "π", "θ", "i", "≤", "≥", "≠", "∞", "→", "_", "#", ":=", "\"", "'", "[",
                        "]", "{", "}",
                    ] {
                        if ui.button(text).clicked() {
                            requests.push(Request::Insert(text.into()));
                        }
                    }
                });
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
            Page::Help => {
                ui.label("Arrow keys edit; Shift selects. Alpha enters letters. ON interrupts. Apps share Plot and Numeric definitions.");
                ui.add(
                    egui::TextEdit::singleline(&mut self.memory.help_query)
                        .hint_text("Find command"),
                );
                if !self.memory.help_topic.is_empty() {
                    if let Some(spec) =
                        ppl_compiler::builtins::lookup_case_insensitive(&self.memory.help_topic)
                    {
                        ui.strong(spec.name);
                        ui.label(format!(
                            "Arguments: {} · {}",
                            spec.arity_text(),
                            spec.category.as_str()
                        ));
                        ui.label(match spec.status {
                            ppl_compiler::builtins::Status::Implemented => {
                                "Native implementation available"
                            }
                            ppl_compiler::builtins::Status::Approximate => {
                                "Native implementation has documented compatibility limits"
                            }
                            ppl_compiler::builtins::Status::Unsupported => {
                                "Requires an implementation in this backend"
                            }
                        });
                        if ui.button("Insert command").clicked() {
                            if spec.min == 0 {
                                requests.push(Request::Insert(spec.name.into()));
                            } else {
                                requests.push(Request::InsertTemplate(
                                    crate::ide::native::expression::TemplateKind::Function(
                                        spec.name,
                                    ),
                                ));
                            }
                        }
                    }
                }
                ui.hyperlink_to(
                    "HP Prime User Guide",
                    "https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf",
                );
                let query = self.memory.help_query.to_ascii_lowercase();
                for spec in ppl_compiler::builtins::BUILTINS.iter().filter(|spec| {
                    !query.is_empty() && spec.name.to_ascii_lowercase().contains(&query)
                }) {
                    if ui
                        .selectable_label(self.memory.help_topic == spec.name, spec.name)
                        .clicked()
                    {
                        self.memory.help_topic = spec.name.into();
                    }
                }
            }
            Page::Memory => {
                ui.label(format!(
                    "{} variables · {} programs · {} notes",
                    snapshot.globals.len(),
                    snapshot.programs.len(),
                    snapshot.notes.len()
                ));
                ui.label("Objects are stored in the Native profile. Firmware memory is separate.");
                for (page, label) in [
                    (Page::Variables, "Variables"),
                    (Page::Programs, "Programs"),
                    (Page::Lists, "Lists"),
                    (Page::Matrices, "Matrices"),
                    (Page::Notes, "Notes"),
                ] {
                    if ui.button(label).clicked() {
                        self.open(page);
                    }
                }
            }
        }
    }
}
