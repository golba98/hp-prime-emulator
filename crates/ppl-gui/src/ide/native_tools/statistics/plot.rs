//! Full-screen Statistics plot, sharing computed samples and trace state.
use super::view::{display, text};
use super::*;
use crate::ide::{native::chrome, native_model_plots::PlotStyle, native_tools::Page};
use ppl_graphics::Grob;
const ZOOM: [&str; 6] = ["In", "Out", "Auto Scale", "Decimal", "Integer", "Trig"];
impl NativeTools {
    pub fn statistics_plot_x(&self) -> f64 {
        self.trace
            .current(&self.trace_samples())
            .map_or(0., |p| p.point[0])
    }
    pub fn statistics_plot_goto(&mut self, x: f64) {
        let samples = self.trace_samples();
        let visible = self.memory.statistics_fit_visible
            [usize::from(mode(&self.memory.selected_app).unwrap_or(false))];
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
            .filter(|(i, _)| visible || styles.get(*i) != Some(&PlotStyle::Line))
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
                    if two { "Fnc" } else { "" },
                    "Menu",
                ],
            );
        } else {
            chrome::softkeys(&mut image, &["", "", "", "", "", "Menu"]);
            // The status strip shares the first five softkey positions.
            image.fill_rect(0, 220, 264, 239, 0xe8f1f6);
            if let Some(trace) = self.trace.current(&self.trace_samples()) {
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
                text(
                    &mut image,
                    &format!("Y:{}", display(trace.point[1], &settings)),
                    154,
                    222,
                );
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
                    if let Some(data) = &data {
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
                        let trace = self.trace.current(&curves).map(|t| t.point);
                        let interaction = ui
                            .scope_builder(egui::UiBuilder::new().max_rect(plot_rect), |ui| {
                                crate::ide::native_plot_view::show_prime(
                                    ui,
                                    &mut window,
                                    &curves,
                                    &data.styles,
                                    plot_rect.size(),
                                    scale,
                                    trace,
                                )
                            })
                            .inner;
                        if let Some(point) = interaction.clicked {
                            if let Err(error) =
                                self.trace
                                    .nearest_in_view(point, &window, &curves, [320., 220.])
                            {
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
