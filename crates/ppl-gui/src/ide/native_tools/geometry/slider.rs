//! A Geometry parameter is stored as an instruction, so a slider change
//! recomputes dependent constructions via the same worker as a Symbolic edit.
use super::*;
use geo::{Parameter, SliderMotion, SliderOptions};

#[derive(Default)]
pub(super) struct Controls {
    pub(super) form: Option<Form>,
    active: Option<String>,
    animation: Option<(String, f64, bool)>,
}
pub(super) struct Form {
    name: String,
    range: [f64; 2],
    value: f64,
    options: SliderOptions,
    error: Option<String>,
}
impl NativeTools {
    pub(super) fn geometry_new_slider(&mut self) {
        if let Some(book) = &self.memory.models.geometry_book {
            let count = book.objects.iter().filter(|o| o.slider.is_some()).count();
            self.geometry_panel.sliders.form = Some(Form {
                name: book.next_name(),
                range: [0., 1.],
                value: 0.5,
                options: SliderOptions {
                    position: [0., (count % 12) as f32 * 16.],
                    ..Default::default()
                },
                error: None,
            });
        }
    }
    fn parameter_edit(
        &self,
        name: &str,
        source: String,
        options: SliderOptions,
    ) -> Option<Request> {
        let mut book = self.memory.models.geometry_book.clone()?;
        if let Some(o) = book.objects.iter_mut().find(|o| o.name == name) {
            if o.source == source && o.slider == Some(options) {
                return None;
            }
            o.slider = Some(options);
        } else {
            book.objects.push(Object {
                name: name.into(),
                source: source.clone(),
                slider: Some(options),
                ..Default::default()
            });
        }
        Some(Request::Calculate(
            crate::ide::native_app_ui::Calculation::GeometryEdit {
                book: Box::new(book),
                name: name.into(),
                source,
                insert: None,
            },
        ))
    }
    fn apply_slider_form(&mut self) -> Vec<Request> {
        let Some(f) = self.geometry_panel.sliders.form.as_mut() else {
            return vec![];
        };
        let [lo, hi] = f.range;
        if !(lo.is_finite() && hi.is_finite() && lo < hi && (lo..=hi).contains(&f.value))
            || f.options.validate().is_err()
        {
            f.error = Some("Check the interval, value, step and time".into());
            return vec![];
        }
        let f = self.geometry_panel.sliders.form.take().unwrap();
        let source = format!(
            "element({},{},{})",
            geo::canonical(lo),
            geo::canonical(hi),
            geo::canonical(f.value)
        );
        self.geometry_panel.sliders.active = None;
        self.geometry_panel.sliders.animation = None;
        self.parameter_edit(&f.name, source, f.options)
            .into_iter()
            .collect()
    }
    pub(super) fn slider_key(&mut self, action: A) -> Option<Vec<Request>> {
        if self.geometry_panel.sliders.form.is_some() {
            return match action {
                A::Back => {
                    self.geometry_panel.sliders.form = None;
                    Some(vec![])
                }
                A::Evaluate => Some(self.apply_slider_form()),
                A::Home | A::Tool(_) => None,
                _ => Some(vec![]),
            };
        }
        let name = self.geometry_panel.sliders.active.clone()?;
        if action == A::Back {
            self.geometry_panel.sliders.active = None;
            self.geometry_panel.sliders.animation = None;
            return Some(vec![]);
        }
        let sign = match action {
            A::Edit(E::Left(_)) => -1.,
            A::Edit(E::Right(_)) => 1.,
            _ => return None,
        };
        let scene = self.geometry_scene()?;
        let p = scene
            .objects
            .iter()
            .find(|o| o.name == name)
            .and_then(|o| o.value.as_ref().ok())?;
        let Shape::Parameter(p) = p else { return None };
        let opts = self
            .memory
            .models
            .geometry_book
            .as_ref()?
            .objects
            .iter()
            .find(|o| o.name == name)?
            .slider
            .unwrap_or_default();
        let value = (p.value + sign * opts.step)
            .clamp(p.limits[0].min(p.limits[1]), p.limits[0].max(p.limits[1]));
        Some(
            self.parameter_edit(&name, p.updated(value), opts)
                .into_iter()
                .collect(),
        )
    }
    pub(super) fn show_slider_form(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        requests: &mut Vec<Request>,
    ) {
        let Some(f) = self.geometry_panel.sliders.form.as_mut() else {
            return;
        };
        let at = |x, y, w, h| {
            egui::Rect::from_min_size(lcd.min + egui::vec2(x, y) * scale, egui::vec2(w, h) * scale)
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Area::new("geometry_slider_form".into())
            .fixed_pos(lcd.min)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_min_size(lcd.size());
                ui.set_clip_rect(lcd);
                ui.painter().rect_filled(lcd, 0., ui.visuals().panel_fill);
                ui.put(
                    at(0., 0., 320., 20.),
                    egui::Label::new("Edit or animate slider"),
                );
                let [lo, hi] = &mut f.range;
                for (label, value, x, y) in [
                    ("Minimum", lo, 0., 25.),
                    ("Maximum", hi, 160., 25.),
                    ("Value", &mut f.value, 0., 52.),
                    ("Step", &mut f.options.step, 160., 52.),
                ] {
                    ui.put(at(x, y, 84., 23.), egui::Label::new(label));
                    ui.put(
                        at(x + 88., y, 67., 23.),
                        egui::DragValue::new(value).speed(0.1),
                    );
                }
                ui.put(at(0., 90., 84., 24.), egui::Label::new("Animation"));
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(at(90., 90., 220., 25.)),
                    |ui| {
                        egui::ComboBox::from_id_salt("geo_slider_motion")
                            .selected_text(format!("{:?}", f.options.motion))
                            .show_ui(ui, |ui| {
                                for mode in [
                                    SliderMotion::None,
                                    SliderMotion::Forward,
                                    SliderMotion::Backward,
                                    SliderMotion::Alternate,
                                ] {
                                    ui.selectable_value(
                                        &mut f.options.motion,
                                        mode,
                                        format!("{mode:?}"),
                                    );
                                }
                            });
                    },
                );
                if f.options.motion != SliderMotion::None {
                    for (label, val, y) in [
                        ("Step time (s)", &mut f.options.seconds, 125.),
                        ("Pause (s)", &mut f.options.pause, 152.),
                    ] {
                        ui.put(at(0., y, 110., 24.), egui::Label::new(label));
                        ui.put(at(112., y, 86., 24.), egui::DragValue::new(val).speed(0.1));
                    }
                }
                if let Some(e) = &f.error {
                    ui.put(at(1., 192., 315., 25.), egui::Label::new(e));
                }
                cancel = ui
                    .put(at(212., 220., 53., 20.), egui::Button::new("Cancel"))
                    .clicked();
                apply = ui
                    .put(at(266., 220., 53., 20.), egui::Button::new("OK"))
                    .clicked();
            });
        if cancel {
            self.geometry_panel.sliders.form = None;
        }
        if apply {
            requests.extend(self.apply_slider_form());
        }
    }
    pub(super) fn show_slider_controls(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        book: &Book,
        scene: Option<&Scene>,
        requests: &mut Vec<Request>,
    ) {
        let Some(scene) = scene else { return };
        let mut index = 0;
        for node in &scene.objects {
            let Ok(Shape::Parameter(p)) = &node.value else {
                continue;
            };
            let Some(o) = book
                .objects
                .iter()
                .find(|o| o.visible && o.name == node.name)
            else {
                continue;
            };
            let opts = o.slider.unwrap_or_else(|| SliderOptions {
                position: [0., 16. * index as f32],
                ..Default::default()
            });
            index += 1;
            self.slider_control(ctx, lcd, scale, &o.name, p, opts, requests);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn slider_control(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        name: &str,
        p: &Parameter,
        mut opts: SliderOptions,
        requests: &mut Vec<Request>,
    ) {
        let active = self.geometry_panel.sliders.active.as_deref() == Some(name);
        let pos = lcd.min + egui::vec2(opts.position[0], opts.position[1]) * scale;
        let mut value = p.value;
        let mut changed = false;
        let lo = p.limits[0].min(p.limits[1]);
        let hi = p.limits[0].max(p.limits[1]);
        egui::Area::new(egui::Id::new(("geo_parameter", name)))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .movable(false)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                let (rect, label) = ui.allocate_exact_size(
                    egui::vec2(64., 16.) * scale,
                    egui::Sense::click_and_drag(),
                );
                ui.painter().text(
                    rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    format!("{name}: {}", geo::canonical(value)),
                    egui::FontId::proportional(13. * scale),
                    egui::Color32::BLACK,
                );
                if label.clicked() || label.long_touched() {
                    self.geometry_panel.sliders.active = Some(name.into());
                }
                if label.drag_stopped() {
                    let d = ui
                        .input(|i| i.pointer.latest_pos().zip(i.pointer.press_origin()))
                        .map_or(egui::Vec2::ZERO, |(end, start)| (end - start) / scale);
                    opts.position = [
                        (opts.position[0] + d.x).clamp(0., 250.),
                        (opts.position[1] + d.y).clamp(0., 190.),
                    ];
                    changed = true;
                }
                if active {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut value, lo..=hi).step_by(opts.step))
                            .changed();
                        ui.horizontal(|ui| {
                            if ui.small_button("Settings").clicked() {
                                self.geometry_panel.sliders.form = Some(Form {
                                    name: name.into(),
                                    range: p.limits,
                                    value,
                                    options: opts,
                                    error: None,
                                });
                            }
                            if opts.motion != SliderMotion::None
                                && ui.small_button("Animate").clicked()
                            {
                                let a = &mut self.geometry_panel.sliders.animation;
                                *a = if a.is_some() {
                                    None
                                } else {
                                    Some((
                                        name.into(),
                                        ctx.input(|i| i.time),
                                        opts.motion != SliderMotion::Backward,
                                    ))
                                };
                            }
                            if ui.small_button("Close").clicked() {
                                self.geometry_panel.sliders.active = None;
                            }
                        });
                    });
                }
            });
        if let Some((target, next, forward)) = &mut self.geometry_panel.sliders.animation
            && target == name
        {
            let now = ctx.input(|i| i.time);
            if now >= *next && !self.geometry_panel.busy && self.model_job.is_none() {
                let mut candidate = value + if *forward { opts.step } else { -opts.step };
                let end = candidate > hi || candidate < lo;
                if end {
                    if opts.motion == SliderMotion::Alternate {
                        *forward = !*forward;
                        candidate = value + if *forward { opts.step } else { -opts.step };
                    } else {
                        candidate = if *forward { lo } else { hi };
                    }
                }
                value = candidate.clamp(lo, hi);
                changed = true;
                *next = now + opts.seconds + if end { opts.pause } else { 0. };
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        if changed {
            requests.extend(self.parameter_edit(name, p.updated(value), opts));
        }
    }
}
