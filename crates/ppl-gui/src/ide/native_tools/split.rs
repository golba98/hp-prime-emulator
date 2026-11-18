use super::*;
impl NativeTools {
    pub fn open_special(&mut self, number: i64, _draw: bool, snapshot: SessionSnapshot) -> bool {
        use special_views::SpecialView;
        let Some(action) = special_views::view(&self.memory.selected_app, number) else {
            return false;
        };
        match action {
            SpecialView::PlotDetail => {
                self.open(Page::PlotDetail);
                self.detail = None;
                self.detail_styles.clear();
                self.detail_job = None;
                self.detail_model_job = None;
                if let Some(app) = self.memory.graphs.get(&self.memory.selected_app).cloned() {
                    let mut context = snapshot.clone();
                    context.active_app = self.memory.selected_app.clone();
                    self.detail = Some((app.window.clone(), Vec::new()));
                    self.detail_job = Some(PlotJob::start(
                        app,
                        context,
                        self.memory
                            .models
                            .geometry_book
                            .clone()
                            .map(std::sync::Arc::new),
                    ));
                } else {
                    let window = self.trace_window();
                    let mut context = snapshot.clone();
                    context.active_app = self.memory.selected_app.clone();
                    for (name, value) in [
                        ("Xmin", window.xmin),
                        ("Xmax", window.xmax),
                        ("Ymin", window.ymin),
                        ("Ymax", window.ymax),
                    ] {
                        if let Ok(value) = ppl_math::Real::from_f64(value) {
                            context
                                .globals
                                .insert(name.into(), ppl_vm::PortableValue::Real(value));
                        }
                    }
                    self.detail = Some((window, Vec::new()));
                    self.detail_model_job = Some(model_job::ModelPlotJob::start(
                        self.memory.selected_app.clone(),
                        self.memory.models.clone(),
                        context,
                        false,
                    ));
                }
                self.plot(snapshot);
            }
            SpecialView::PlotTable => {
                self.open(Page::PlotTable);
                self.detail_job = None;
                self.detail_model_job = None;
                self.detail = None;
                self.detail_styles.clear();
                self.plot(snapshot.clone());
                self.table(&snapshot);
            }
            SpecialView::Autoscale => {
                self.open(Page::Plot);
                self.autoscale(snapshot);
            }
            SpecialView::Decimal | SpecialView::Integer | SpecialView::Trig => {
                use super::super::native_plot_controls::Preset;
                self.open(Page::Plot);
                self.preset(
                    match action {
                        SpecialView::Decimal => Preset::Decimal,
                        SpecialView::Integer => Preset::Integer,
                        _ => Preset::Trig,
                    },
                    snapshot,
                );
            }
        }
        true
    }
    pub fn detail_plot(
        &self,
    ) -> Option<&(
        super::super::native_apps::PlotWindow,
        super::super::native_apps::PlotSamples,
    )> {
        self.detail.as_ref()
    }
    fn current_plot_styles(&self) -> Vec<super::super::native_model_plots::PlotStyle> {
        self.memory
            .models
            .results
            .get(&format!("{}:plot", self.memory.selected_app))
            .and_then(|v| {
                serde_json::from_value::<super::super::native_model_plots::PlotData>(v.clone()).ok()
            })
            .map(|data| data.styles)
            .unwrap_or_default()
    }
    pub(super) fn show_split(
        &mut self,
        ui: &mut egui::Ui,
        lcd: Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) {
        let size = egui::vec2(((lcd.width() - 12.) / 2.).max(1.), 118. * scale);
        let mut resample = false;
        let mut chosen = None;
        let colors = self.graph_plot_colors();
        ui.horizontal(|ui| {
            if self.page == Some(Page::PlotDetail) {
                if let Some((window, points)) = &self.detail {
                    // Reference is intentionally immutable during right-hand zoom/pan.
                    let mut window = window.clone();
                    if colors.is_empty() {
                        let _ = super::super::native_plot_view::show(
                            ui,
                            &mut window,
                            points,
                            &self.detail_styles,
                            size,
                            scale,
                            None,
                        );
                    } else {
                        let _ = super::super::native_plot_view::show_graph(
                            ui,
                            &mut window,
                            points,
                            &colors,
                            size,
                            scale,
                            None,
                        );
                    }
                }
            }
            let points = self.trace_samples();
            let styles = self.current_plot_styles();
            let mut window = self.trace_window();
            let trace = self.trace.current(&points).map(|p| p.point);
            let interaction = if colors.is_empty() {
                super::super::native_plot_view::show(
                    ui,
                    &mut window,
                    &points,
                    &styles,
                    size,
                    scale,
                    trace,
                )
            } else {
                super::super::native_plot_view::show_graph(
                    ui,
                    &mut window,
                    &points,
                    &colors,
                    size,
                    scale,
                    trace,
                )
            };
            resample = interaction.drag_stopped
                && (self.memory.graphs.contains_key(&self.memory.selected_app)
                    || matches!(
                        self.memory.selected_app.as_str(),
                        "Solve" | "Statistics 2Var"
                    ));
            if let Some(point) = interaction.clicked {
                if let Err(error) = self.trace.nearest_in_view(
                    point,
                    &window,
                    &points,
                    [f64::from(size.x), f64::from(size.y)],
                ) {
                    self.error = Some(error);
                }
            }
            if let Some(app) = self.memory.graphs.get_mut(&self.memory.selected_app) {
                app.window = window;
            } else {
                self.memory
                    .model_windows
                    .insert(self.memory.selected_app.clone(), window);
            }
            if self.page == Some(Page::PlotTable) {
                ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    egui::ScrollArea::both()
                        .max_height(size.y)
                        .max_width(size.x)
                        .show(ui, |ui| {
                            for row in &self.table {
                                let digits = if row.len() > 2 { 3 } else { 5 };
                                let label = row
                                    .iter()
                                    .map(|v| {
                                        v.map_or_else(
                                            || "undefined".into(),
                                            |v| format!("{v:.digits$}"),
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                if ui
                                    .add(
                                        egui::Button::selectable(
                                            false,
                                            egui::RichText::new(label)
                                                .text_style(egui::TextStyle::Monospace),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                    )
                                    .clicked()
                                {
                                    if let Some(app) =
                                        self.memory.graphs.get(&self.memory.selected_app)
                                    {
                                        let mut context = snapshot.clone();
                                        context.active_app = self.memory.selected_app.clone();
                                        chosen = table_point(
                                            app.kind,
                                            row,
                                            &context.effective_app_settings(),
                                        );
                                    }
                                }
                            }
                        });
                });
            }
        });
        if let Some(point) = chosen {
            let window = self.trace_window();
            if let Err(error) = self.trace.nearest_in_view(
                point,
                &window,
                &self.trace_samples(),
                [f64::from(size.x), f64::from(size.y)],
            ) {
                self.error = Some(error);
            }
        }
        ui.horizontal(|ui| {
            if self.page == Some(Page::PlotDetail)
                && ui
                    .add_enabled(
                        !self.calculating()
                            && self.error.is_none()
                            && !self.trace_samples().is_empty(),
                        egui::Button::new("Replace reference").small(),
                    )
                    .clicked()
            {
                self.detail = Some((self.trace_window(), self.trace_samples()));
                self.detail_styles = self.current_plot_styles();
                // A pending original calculation must not overwrite this replacement.
                self.detail_job = None;
                self.detail_model_job = None;
                self.dirty();
            }
            if self.page == Some(Page::PlotTable) && ui.small_button("Refresh table").clicked() {
                self.table(snapshot);
            }
            if ui.small_button("Unsplit").clicked() {
                self.open(Page::Plot);
            }
        });
        if resample {
            self.plot(snapshot.clone());
        }
        if self.calculating() {
            ui.label("Calculating…");
        }
    }
}

/// Numeric rows store independent parameter followed by each selected curve's
/// values. Parametric/Polar traces must select a Cartesian plot coordinate.
fn table_point(
    kind: GraphKind,
    row: &[Option<f64>],
    settings: &ppl_vm::Settings,
) -> Option<[f64; 2]> {
    let parameter = row.first().copied().flatten()?;
    let first = row.get(1).copied().flatten()?;
    match kind {
        GraphKind::Parametric => Some([first, row.get(2).copied().flatten()?]),
        GraphKind::Polar => {
            let angle = settings.to_radians(ppl_math::Real::from_f64(parameter).ok()?);
            Some([first * angle.cos(), first * angle.sin()])
        }
        _ => Some([parameter, first]),
    }
    .filter(|point| point.iter().all(|v| v.is_finite()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_table_selection_uses_cartesian_coordinates_and_app_angles() {
        let settings = ppl_vm::Settings {
            angle: ppl_vm::AngleMode::Degrees,
            ..Default::default()
        };
        assert_eq!(
            table_point(
                GraphKind::Parametric,
                &[Some(7.), Some(2.), Some(4.)],
                &settings
            ),
            Some([2., 4.])
        );
        let point = table_point(GraphKind::Polar, &[Some(90.), Some(2.)], &settings).unwrap();
        assert!(point[0].abs() < 1E-12 && (point[1] - 2.).abs() < 1E-12);
        assert!(
            table_point(
                GraphKind::Parametric,
                &[Some(1.), Some(2.), None],
                &settings
            )
            .is_none()
        );
    }
}
