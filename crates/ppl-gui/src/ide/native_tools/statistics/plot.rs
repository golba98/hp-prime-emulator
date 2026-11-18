//! Full-screen Statistics plot, sharing computed samples and trace state.
use super::view::{display, text};
use super::*;
use crate::ide::{native::chrome, native_model_plots::PlotStyle, native_tools::Page};
use ppl_graphics::Grob;
const ZOOM: [&str; 6] = ["In", "Out", "Auto Scale", "Decimal", "Integer", "Trig"];
fn grouped_count(count: u64, settings: &ppl_vm::Settings) -> String {
    let raw = count.to_string();
    let mut result = String::new();
    for (index, digit) in raw.chars().enumerate() {
        result.push(digit);
        let remaining = raw.len() - index - 1;
        if let Some((first, repeat, separator)) = settings.digit_grouping.grouping()
            && remaining >= first
            && (remaining - first) % repeat == 0
        {
            result.push(separator);
        }
    }
    result
}
fn trace_fields(
    series: &crate::ide::native_model_plots::StatisticsTraceSeries,
    sample: &crate::ide::native_model_plots::StatisticsTraceSample,
    settings: &ppl_vm::Settings,
) -> (String, String) {
    use crate::ide::native_statistics::one::Chart;
    let number = |value: &Option<serde_json::Value>| {
        value
            .as_ref()
            .and_then(|value| ppl_vm::session::decode_value(value).ok())
            .and_then(|value| value.to_value().expect_real("Statistics trace").ok())
            .map_or_else(
                || "NaN".into(),
                |value| value.format(settings.home_format()),
            )
    };
    let value = number(&sample.value);
    let auxiliary = number(&sample.auxiliary);
    let percentage = |places| {
        sample
            .auxiliary
            .as_ref()
            .and_then(|value| ppl_vm::session::decode_value(value).ok())
            .and_then(|value| value.to_value().expect_real("Statistics percentage").ok())
            .map_or_else(
                || "NaN".into(),
                |value| {
                    value.round_places(places).format(ppl_math::FormatOptions {
                        format: ppl_math::NumFormat::Standard,
                        ..settings.home_format()
                    })
                },
            )
    };
    let owner = format!("H{}", series.analysis);
    let indexed = || format!("{owner}[{}]:{value}", sample.row.unwrap_or(1));
    let frequency = || {
        if let Some(count) = sample.frequency {
            format!("F:{}", grouped_count(count, settings))
        } else if sample.fractional_frequency.is_some() {
            format!("F:{}", number(&sample.fractional_frequency))
        } else {
            String::new()
        }
    };
    match series.chart {
        Chart::Histogram => (
            format!(
                "{owner}[{value}...{auxiliary}{}",
                sample.label.as_deref().unwrap_or(")")
            ),
            frequency(),
        ),
        Chart::BoxWhisker => (
            format!("{owner}[{}]:{value}", sample.label.as_deref().unwrap_or("")),
            String::new(),
        ),
        Chart::NormalProbability => (format!("{owner}: X:{value}"), format!("Y:{auxiliary}")),
        Chart::Line => (indexed(), frequency()),
        Chart::Bar | Chart::Control | Chart::StemLeaf => (indexed(), String::new()),
        Chart::Pareto => (
            format!("{owner}: {value}"),
            format!("% of Total {}", percentage(2)),
        ),
        Chart::Dot => (format!("{owner}: X:{value}"), frequency()),
        Chart::Pie => (
            format!("{owner}: [{}]:{value}", sample.row.unwrap_or(1)),
            format!("% of Total {}", percentage(5)),
        ),
    }
}
fn footer_field(image: &mut Grob, source: &str, left: i64, right: i64) {
    let mut field = Grob::new((right - left) as usize, 20, 0xe8f1f6);
    text(&mut field, source, 0, 2);
    image.blit(
        (left, 220, right, 240),
        &field,
        (0, 0, right - left, 20),
        None,
        255,
    );
}
impl NativeTools {
    pub fn statistics_trace_toggle(&mut self) -> bool {
        if mode(&self.memory.selected_app) != Some(false) {
            return false;
        }
        self.statistics_panel.plot_definition = false;
        if self.trace.enabled {
            self.trace.disable();
        } else {
            let samples = self.trace_samples();
            if self.trace.enable(&samples).is_none() {
                self.trace.disable();
                self.error = Some("Plot the statistics data before Trace".into());
                return true;
            }
            self.statistics_panel.plot_controls = false;
        }
        self.dirty();
        true
    }
    pub fn statistics_trace_snapshot(&self, snapshot: &SessionSnapshot) -> serde_json::Value {
        let Some(data) = self
            .memory
            .models
            .results
            .get("Statistics 1Var:plot")
            .and_then(|value| {
                serde_json::from_value::<crate::ide::native_model_plots::PlotData>(value.clone())
                    .ok()
            })
            .filter(|_| mode(&self.memory.selected_app) == Some(false))
        else {
            return serde_json::Value::Null;
        };
        let Some(point) = self.trace.current(&self.trace_samples()) else {
            return serde_json::Value::Null;
        };
        let Some(series) = data.statistics_traces.get(point.curve) else {
            return serde_json::Value::Null;
        };
        let Some(sample) = series.samples.get(point.sample) else {
            return serde_json::Value::Null;
        };
        let settings = snapshot
            .app_settings
            .get(app(false))
            .cloned()
            .unwrap_or_default()
            .effective(&snapshot.settings);
        let (left, right) = trace_fields(series, sample, &settings);
        serde_json::json!({"analysis":series.analysis,"index":point.sample,"left":left,"right":right,"sample":sample})
    }

    pub fn statistics_plot_definition(&self) -> Option<String> {
        if !self.statistics_panel.plot_definition || mode(&self.memory.selected_app) != Some(false)
        {
            return None;
        }
        let data = self
            .memory
            .models
            .results
            .get("Statistics 1Var:plot")
            .and_then(|value| {
                serde_json::from_value::<crate::ide::native_model_plots::PlotData>(value.clone())
                    .ok()
            })?;
        let series = data
            .statistics_traces
            .get(self.trace.curve)
            .or_else(|| data.statistics_traces.first())?;
        let analysis = self
            .memory
            .models
            .statistics1_book
            .as_ref()?
            .analyses
            .get(series.analysis.checked_sub(1)?)?;
        let mut caption = format!("H{}: {}", series.analysis, analysis.data_expression());
        let frequency = analysis.frequency_expression();
        if matches!(
            analysis.chart,
            crate::ide::native_statistics::one::Chart::Histogram
                | crate::ide::native_statistics::one::Chart::BoxWhisker
                | crate::ide::native_statistics::one::Chart::Line
                | crate::ide::native_statistics::one::Chart::Dot
        ) && !frequency.is_empty()
        {
            caption.push(',');
            caption.push_str(&frequency);
        }
        Some(caption)
    }

    pub fn statistics_plot_x(&self) -> f64 {
        self.trace
            .current(&self.trace_samples())
            .map_or(0., |p| p.point[0])
    }
    pub fn statistics_plot_goto(&mut self, x: f64) {
        let samples = self.trace_samples();
        let app_mode = mode(&self.memory.selected_app);
        let one = app_mode == Some(false);
        // Statistics 1Var Go To moves along the active H analysis. Up and
        // Down change the analysis separately.
        let active_curve = if one {
            let mut trace = self.trace;
            trace.enable(&samples).map(|point| point.curve)
        } else {
            None
        };
        let visible = self.memory.statistics_fit_visible[usize::from(app_mode.unwrap_or(false))];
        let styles = self
            .memory
            .models
            .results
            .get(&format!("{}:plot", self.memory.selected_app))
            .and_then(|v| {
                serde_json::from_value::<crate::ide::native_model_plots::PlotData>(v.clone()).ok()
            })
            .map(|p| p.styles)
            .unwrap_or_default();
        let nearest = samples
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                if one {
                    Some(*i) == active_curve
                } else {
                    visible || styles.get(*i) != Some(&PlotStyle::Line)
                }
            })
            .flat_map(|(curve, values)| {
                values
                    .iter()
                    .enumerate()
                    .filter_map(move |(sample, point)| {
                        point.map(|p| (curve, sample, (p[0] - x).abs()))
                    })
            })
            .min_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((curve, sample, _)) = nearest {
            self.trace.enabled = true;
            self.trace.curve = curve;
            self.trace.sample = sample;
        } else {
            self.error = Some("Plot the statistics data before Go To".into());
        }
        self.dirty();
    }
    fn statistics_plot_zoom(&mut self, choice: usize, snapshot: &SessionSnapshot) {
        use crate::ide::native_plot_controls::Preset;
        match choice {
            0 | 1 => {
                let mut w = self.trace_window();
                w.zoom(if choice == 0 { 0.5 } else { 2. });
                self.memory
                    .model_windows
                    .insert(self.memory.selected_app.clone(), w);
                self.plot(snapshot.clone());
            }
            2 => self.autoscale(snapshot.clone()),
            3 => self.preset(Preset::Decimal, snapshot.clone()),
            4 => self.preset(Preset::Integer, snapshot.clone()),
            5 => self.preset(Preset::Trig, snapshot.clone()),
            _ => {}
        }
        self.statistics_panel.plot_zoom = None;
        self.dirty();
    }
    pub fn statistics_plot_action(
        &mut self,
        action: crate::ide::native::keys::KeyAction,
        snapshot: &SessionSnapshot,
    ) -> bool {
        use crate::ide::native::{expression::EditCommand as E, keys::KeyAction as A};
        if mode(&self.memory.selected_app).is_none() || self.page != Some(Page::Plot) {
            return false;
        }
        if matches!(action, A::Home | A::Tool(_)) {
            self.statistics_panel.equation = None;
            self.statistics_panel.plot_equation = None;
            self.statistics_panel.plot_zoom = None;
            return false;
        }
        if let Some(index) = self.statistics_panel.plot_zoom {
            match action {
                A::History(delta) => {
                    self.statistics_panel.plot_zoom = Some(
                        (index as i64 + i64::from(delta)).rem_euclid(ZOOM.len() as i64) as usize,
                    )
                }
                A::Evaluate => self.statistics_plot_zoom(index, snapshot),
                A::Back | A::On => self.statistics_panel.plot_zoom = None,
                _ => {}
            }
            self.dirty();
            return true;
        }
        if self.statistics_panel.equation.is_some() || self.statistics_panel.plot_equation.is_some()
        {
            match action {
                A::Evaluate | A::Back | A::On => {
                    self.statistics_panel.equation = None;
                    self.statistics_panel.plot_equation = None;
                }
                A::Edit(E::Left(_)) => {
                    self.statistics_panel.equation_x =
                        (self.statistics_panel.equation_x - 20).max(0)
                }
                A::Edit(E::Right(_)) => {
                    self.statistics_panel.equation_x =
                        (self.statistics_panel.equation_x + 20).min(4000)
                }
                _ => {}
            }
            self.dirty();
            return true;
        }
        false
    }
    pub(crate) fn show_statistics_plot(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        let two = mode(&self.memory.selected_app).unwrap();
        let unbounded = self.statistics_unbounded_plot();
        let data = self
            .memory
            .models
            .results
            .get(&format!("{}:plot", app(two)))
            .and_then(|v| {
                serde_json::from_value::<crate::ide::native_model_plots::PlotData>(v.clone()).ok()
            });
        let mut settings = snapshot.settings.clone();
        if let Some(app) = snapshot.app_settings.get(app(two)) {
            settings = app.effective(&settings);
        }
        let equation = self
            .statistics_panel
            .plot_equation
            .map(|analysis| {
                if self.statistics_panel.busy {
                    "Calculating…".into()
                } else if self
                    .memory
                    .models
                    .results
                    .get(app(two))
                    .and_then(|r| r.get(format!("S{}", analysis + 1)))
                    .is_some()
                {
                    self.statistics_fit_equation(analysis)
                } else {
                    "Fit unavailable".into()
                }
            })
            .or_else(|| self.statistics_panel.equation.clone());
        let mut image = Grob::new(320, 240, 0xffffff);
        image.fill_rect(0, 220, 319, 239, 0xe8f1f6);
        if let Some(source) = &equation {
            let layout = crate::ide::native::math_layout::expression(source, 17.);
            let mut body = Grob::new(320, 220, 0xffffff);
            layout.paint_static(&mut body, 10 - self.statistics_panel.equation_x, 6, 0);
            image.blit((0, 0, 320, 220), &body, (0, 0, 320, 220), None, 255);
            chrome::softkeys(&mut image, &["", "", "", "", "", "OK"]);
        } else if self.statistics_panel.plot_controls {
            chrome::softkeys(
                &mut image,
                &[
                    "Zoom",
                    "Trace",
                    "Go To",
                    if two { "Fit" } else { "" },
                    if two { "Fnc" } else { "Defn" },
                    "Menu",
                ],
            );
        } else {
            chrome::softkeys(&mut image, &["", "", "", "", "", "Menu"]);
            // The status strip shares the first five softkey positions.
            image.fill_rect(0, 220, 264, 239, 0xe8f1f6);
            if let Some(caption) = self.statistics_plot_definition() {
                footer_field(&mut image, &caption, 1, 265);
            } else if let Some(trace) = self.trace.current(&self.trace_samples()) {
                let typed = data.as_ref().and_then(|data| {
                    let series = data.statistics_traces.get(trace.curve)?;
                    Some((series, series.samples.get(trace.sample)?))
                });
                if let Some((series, sample)) = typed {
                    let (left, right) = trace_fields(series, sample, &settings);
                    let split = if matches!(
                        series.chart,
                        crate::ide::native_statistics::one::Chart::Pie
                            | crate::ide::native_statistics::one::Chart::Pareto
                    ) {
                        85
                    } else {
                        154
                    };
                    footer_field(&mut image, &left, 1, split);
                    footer_field(&mut image, &right, split, 265);
                } else {
                    text(
                        &mut image,
                        &format!(
                            "{}  X:{}",
                            data.as_ref()
                                .and_then(|d| d.labels.get(trace.curve))
                                .and_then(|s| s.split_whitespace().next())
                                .unwrap_or(""),
                            display(trace.point[0], &settings)
                        ),
                        1,
                        222,
                    );
                    let frequency = data.as_ref().and_then(|data| {
                        if data.styles.get(trace.curve) != Some(&PlotStyle::TraceOnly) {
                            return None;
                        }
                        let analysis = data
                            .labels
                            .get(trace.curve)?
                            .split_whitespace()
                            .next()?
                            .strip_prefix('H')?
                            .parse::<usize>()
                            .ok()?;
                        data.dots
                            .iter()
                            .filter(|column| column.analysis == analysis)
                            .nth(trace.sample)
                            .map(|column| column.count)
                    });
                    text(
                        &mut image,
                        &frequency.map_or_else(
                            || format!("Y:{}", display(trace.point[1], &settings)),
                            |count| format!("F:{}", grouped_count(count, &settings)),
                        ),
                        154,
                        222,
                    );
                }
            }
        }
        if let Some(index) = self.statistics_panel.plot_zoom {
            image.rect(0, 89, 150, 219, 0, 0xffffff);
            for (row, label) in ZOOM.iter().enumerate() {
                let y = 90 + row as i64 * 21;
                if row == index {
                    image.fill_rect(1, y, 149, y + 20, 0xadccff);
                }
                text(&mut image, label, 5, y + 1);
            }
        }
        if let Some(error) = &self.error {
            text(&mut image, error, 1, 2);
        }
        if self.statistics_panel.pixels != image.pixels() {
            self.statistics_panel.pixels = image.pixels().to_vec();
            let pixels = image
                .pixels()
                .iter()
                .map(|&v| egui::Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
                .collect();
            let pixels = egui::ColorImage::new([320, 240], pixels);
            if let Some(texture) = &mut self.statistics_panel.texture {
                texture.set(pixels, egui::TextureOptions::NEAREST);
            } else {
                self.statistics_panel.texture = Some(ctx.load_texture(
                    "native_statistics",
                    pixels,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        egui::Area::new("native_statistics_plot".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                let texture = self.statistics_panel.texture.as_ref().unwrap().id();
                ui.painter().image(
                    texture,
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                if equation.is_none() {
                    if unbounded {
                        ui.painter().text(
                            lcd.min + egui::vec2(5. * scale, 10. * scale),
                            egui::Align2::LEFT_TOP,
                            "Y rng is too broad for a split-screen view",
                            egui::FontId::proportional(14. * scale),
                            egui::Color32::BLACK,
                        );
                    } else if let Some(data) = &data {
                        let visible = self.memory.statistics_fit_visible[usize::from(two)];
                        let curves = data
                            .curves
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                if !two || visible || data.styles.get(i) != Some(&PlotStyle::Line) {
                                    c.iter().copied().map(Some).collect()
                                } else {
                                    Vec::new()
                                }
                            })
                            .collect::<Vec<_>>();
                        let mut window = self.trace_window();
                        let plot_rect = egui::Rect::from_min_size(
                            lcd.min,
                            egui::vec2(lcd.width(), 220. * scale),
                        );
                        let trace_samples = self.trace_samples();
                        let trace = self.trace.current(&trace_samples).map(|t| t.point);
                        let interaction = ui
                            .scope_builder(egui::UiBuilder::new().max_rect(plot_rect), |ui| {
                                crate::ide::native_plot_view::show_prime_data(
                                    ui,
                                    &mut window,
                                    &curves,
                                    data,
                                    plot_rect.size(),
                                    scale,
                                    trace,
                                )
                            })
                            .inner;
                        if let Some(point) = interaction.clicked {
                            if let Err(error) = self.trace.nearest_in_view(
                                point,
                                &window,
                                &trace_samples,
                                [320., 220.],
                            ) {
                                self.error = Some(error);
                            }
                            self.dirty();
                        }
                        self.memory.model_windows.insert(app(two).into(), window);
                        if interaction.drag_stopped {
                            self.plot(snapshot.clone());
                        }
                    } else {
                        ui.painter().text(
                            lcd.min + egui::vec2(5. * scale, 10. * scale),
                            egui::Align2::LEFT_TOP,
                            if self.calculating() {
                                "Calculating…"
                            } else {
                                "No statistics plot"
                            },
                            egui::FontId::proportional(14. * scale),
                            egui::Color32::BLACK,
                        );
                    }
                }
                if self.statistics_panel.plot_zoom.is_some() {
                    let menu = egui::Rect::from_min_max(
                        lcd.min + egui::vec2(0., 89. * scale),
                        lcd.min + egui::vec2(151. * scale, 220. * scale),
                    );
                    ui.painter().with_clip_rect(menu).image(
                        texture,
                        lcd,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                    if ui
                        .interact(menu, ui.id().with("stats_zoom"), egui::Sense::click())
                        .clicked()
                        && let Some(p) = ctx.pointer_interact_pos()
                    {
                        let row = (((p.y - lcd.top()) / scale - 90.) / 21.).max(0.) as usize;
                        self.statistics_plot_zoom(row.min(5), snapshot);
                    }
                }
                let footer =
                    egui::Rect::from_min_max(lcd.min + egui::vec2(0., 220. * scale), lcd.max);
                if ui
                    .interact(
                        footer,
                        ui.id().with("stats_plot_footer"),
                        egui::Sense::click(),
                    )
                    .clicked()
                    && let Some(p) = ctx.pointer_interact_pos()
                {
                    let key = chrome::softkey_index(((p.x - lcd.left()) / scale) as i64);
                    if equation.is_some() {
                        if key == 5 {
                            self.statistics_panel.equation = None;
                            self.statistics_panel.plot_equation = None;
                        }
                    } else if key == 5 {
                        self.statistics_panel.plot_controls = !self.statistics_panel.plot_controls;
                    } else if self.statistics_panel.plot_controls {
                        match key {
                            0 => self.statistics_panel.plot_zoom = Some(0),
                            1 => self.trace_toggle(),
                            2 => requests.push(Request::EditMath(MathTarget::StatisticsPlotX)),
                            3 if two => {
                                self.memory.statistics_fit_visible[1] =
                                    !self.memory.statistics_fit_visible[1];
                            }
                            4 if !two => {
                                self.statistics_panel.plot_definition = true;
                                self.statistics_panel.plot_controls = false;
                            }
                            4 if two => {
                                self.statistics_panel.plot_equation =
                                    Some(self.memory.statistics_cursor[1].analysis);
                                if let Some(calculation) = crate::ide::native_app_ui::numeric(
                                    app(two),
                                    &self.memory.models,
                                ) {
                                    requests.push(Request::Calculate(calculation));
                                }
                            }
                            _ => {}
                        }
                    }
                    self.dirty();
                    ctx.request_repaint();
                }
            });
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    use crate::ide::native_model_plots::{StatisticsTraceSample, StatisticsTraceSeries};
    use crate::ide::native_statistics::one::Chart;
    #[test]
    fn undefined_moving_range_is_displayed_as_nan_without_a_fabricated_numeric_value() {
        let series = StatisticsTraceSeries {
            analysis: 1,
            chart: Chart::Control,
            samples: vec![],
        };
        let sample = StatisticsTraceSample {
            x: 1.,
            y: None,
            value: None,
            auxiliary: None,
            frequency: None,
            fractional_frequency: None,
            row: Some(1),
            label: None,
        };
        assert_eq!(
            trace_fields(&series, &sample, &ppl_vm::Settings::default()),
            ("H1[1]:NaN".into(), String::new())
        );
        assert!(sample.y.is_none() && sample.value.is_none());
    }
    #[test]
    fn frequency_footer_retains_all_u64_digits_and_uses_current_grouping_preferences() {
        let mut settings = ppl_vm::Settings::default();
        settings.set_separator(3).unwrap();
        let series = StatisticsTraceSeries {
            analysis: 2,
            chart: Chart::Dot,
            samples: vec![],
        };
        let sample = StatisticsTraceSample {
            x: 3.,
            y: Some(u64::MAX as f64),
            value: Some(ppl_vm::session::encode_value(&PortableValue::Real(
                ppl_math::Real::from_i64(3),
            ))),
            auxiliary: None,
            frequency: Some(u64::MAX),
            fractional_frequency: None,
            row: None,
            label: None,
        };
        assert_eq!(
            trace_fields(&series, &sample, &settings).1,
            "F:18446744073709551615"
        );
        settings.set_separator(0).unwrap();
        assert_eq!(
            trace_fields(&series, &sample, &settings).1,
            "F:18﹐446﹐744﹐073﹐709﹐551﹐615"
        );
    }
}
