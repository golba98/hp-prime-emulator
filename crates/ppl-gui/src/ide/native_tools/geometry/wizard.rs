//! Geometry Plot's Slopefield form. Editing stays local until the form is accepted.
use super::*;
use crate::ide::{native::expression::EditCommand, native_apps::PlotWindow};

pub(super) struct Slopefield {
    fields: [String; 9],
    pub(super) selected: usize,
}
impl Slopefield {
    pub(super) fn new(w: &PlotWindow) -> Self {
        Self {
            fields: [
                String::new(),
                "x".into(),
                "y".into(),
                geo::canonical(w.xmin),
                geo::canonical(w.xmax),
                geo::canonical(w.ymin),
                geo::canonical(w.ymax),
                "2.5".into(),
                "2.5".into(),
            ],
            selected: 0,
        }
    }
    fn source(&self) -> Result<String, String> {
        let [expr, x, y, xmin, xmax, ymin, ymax, dx, dy] = &self.fields;
        if self.fields.iter().any(|s| s.trim().is_empty()) {
            return Err("Complete the slopefield fields".into());
        }
        let valid = |s: &str| {
            s.chars().next().is_some_and(char::is_alphabetic)
                && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        };
        if x == y || !valid(x) || !valid(y) {
            return Err("Use two distinct variable names".into());
        }
        Ok(format!(
            "plotfield({expr},[{x}={xmin}..{xmax},{y}={ymin}..{ymax}],xstep={dx},ystep={dy})"
        ))
    }
}
impl NativeTools {
    pub(in crate::ide::native_tools) fn close_geometry_wizard(&mut self) {
        self.geometry_panel.wizard = None;
        self.geometry_panel.field_solution = None;
    }
    pub fn slopefield_value(&self, index: usize) -> String {
        self.geometry_panel
            .wizard
            .as_ref()
            .and_then(|w| w.fields.get(index))
            .cloned()
            .unwrap_or_default()
    }
    pub fn slopefield_set(&mut self, index: usize, source: String) {
        if let Some(w) = &mut self.geometry_panel.wizard
            && let Some(f) = w.fields.get_mut(index)
        {
            *f = source;
            w.selected = (index + 1).min(w.fields.len() - 1);
        }
        self.dirty();
    }
    pub(super) fn create_wizard_object(&self, source: String) -> Vec<Request> {
        let Some(book) = &self.memory.models.geometry_book else {
            return vec![];
        };
        vec![Request::Calculate(
            crate::ide::native_app_ui::Calculation::GeometryEdit {
                book: Box::new(book.clone()),
                name: book.next_name(),
                source,
                insert: None,
            },
        )]
    }
    fn slopefield_submit(&mut self) -> Vec<Request> {
        let Some(w) = self.geometry_panel.wizard.as_ref() else {
            return vec![];
        };
        match w.source() {
            Ok(source) => {
                self.geometry_panel.field_solution = Some((
                    w.fields[0].clone(),
                    w.fields[1].clone(),
                    w.fields[2].clone(),
                ));
                self.geometry_panel.wizard = None;
                self.dirty();
                self.create_wizard_object(source)
            }
            Err(error) => {
                self.error = Some(error);
                self.dirty();
                vec![]
            }
        }
    }
    pub(super) fn slopefield_action(&mut self, action: A) -> Option<Vec<Request>> {
        let w = self.geometry_panel.wizard.as_mut()?;
        match action {
            A::Back => self.geometry_panel.wizard = None,
            A::Home | A::Tool(_) => {
                self.close_geometry_wizard();
                return None;
            }
            A::Evaluate => {
                return Some(vec![Request::EditMath(MathTarget::Slopefield {
                    index: w.selected,
                    clear: false,
                })]);
            }
            A::History(d) => w.selected = (w.selected as i32 + i32::from(d)).clamp(0, 8) as usize,
            A::Edit(EditCommand::Left(_)) => w.selected = w.selected.saturating_sub(1),
            A::Edit(EditCommand::Right(_)) => w.selected = (w.selected + 1).min(8),
            _ => {}
        }
        self.dirty();
        Some(vec![])
    }
    pub(super) fn show_slopefield(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        requests: &mut Vec<Request>,
    ) {
        let Some(w) = &mut self.geometry_panel.wizard else {
            return;
        };
        let rect = |x: f32, y: f32, width: f32, height: f32| {
            egui::Rect::from_min_size(
                lcd.min + egui::vec2(x, y) * scale,
                egui::vec2(width, height) * scale,
            )
        };
        let mut submit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("native_slopefield"))
            .fixed_pos(lcd.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                let p = ui.painter().clone();
                let ink = egui::Color32::BLACK;
                let font = egui::FontId::proportional(13. * scale);
                p.rect_filled(lcd, 0., egui::Color32::from_rgb(232, 241, 246));
                p.rect_filled(
                    rect(0., 0., 320., 20.),
                    0.,
                    egui::Color32::from_rgb(29, 66, 119),
                );
                p.text(
                    rect(0., 0., 320., 20.).center(),
                    egui::Align2::CENTER_CENTER,
                    "plotfield",
                    font.clone(),
                    egui::Color32::WHITE,
                );
                for (row, label) in ["y'=f(x,y):", "Variables:", "X Rng:", "Y Rng:", "Step:"]
                    .iter()
                    .enumerate()
                {
                    let y = [61., 86., 121., 144., 166.][row];
                    p.text(
                        rect(0., y, 90., 0.).right_center(),
                        egui::Align2::RIGHT_CENTER,
                        label,
                        font.clone(),
                        ink,
                    );
                }
                for (i, text) in w.fields.iter().enumerate() {
                    let (x, y, width) = match i {
                        0 => (92., 49., 222.),
                        1 => (92., 75., 98.),
                        2 => (195., 75., 119.),
                        _ => (
                            if i % 2 == 1 { 92. } else { 180. },
                            111. + ((i - 3) / 2) as f32 * 22.,
                            82.,
                        ),
                    };
                    let field = rect(x, y, width, 22.);
                    p.rect_filled(field, 0., egui::Color32::WHITE);
                    p.rect_stroke(
                        field,
                        0.,
                        egui::Stroke::new(
                            if w.selected == i { 2. } else { 0.5 } * scale,
                            if w.selected == i {
                                ink
                            } else {
                                egui::Color32::GRAY
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    p.with_clip_rect(field.shrink(scale)).text(
                        field.left_center() + egui::vec2(3. * scale, 0.),
                        egui::Align2::LEFT_CENTER,
                        text,
                        font.clone(),
                        ink,
                    );
                    if ui
                        .interact(field, ui.id().with(i), egui::Sense::click())
                        .clicked()
                    {
                        w.selected = i;
                        requests.push(Request::EditMath(MathTarget::Slopefield {
                            index: i,
                            clear: false,
                        }));
                    }
                }
                let hints = [
                    "Enter expression f(x,y)",
                    "Independent variable",
                    "Dependent variable",
                    "Minimum x",
                    "Maximum x",
                    "Minimum y",
                    "Maximum y",
                    "Horizontal step",
                    "Vertical step",
                ];
                p.text(
                    rect(0., 197., 320., 21.).left_center(),
                    egui::Align2::LEFT_CENTER,
                    hints[w.selected],
                    font.clone(),
                    ink,
                );
                for (x, label) in [(0., "Edit"), (212., "Cancel"), (266., "OK")] {
                    let r = rect(x, 220., 53., 20.);
                    p.rect_filled(r, 2., egui::Color32::DARK_GRAY);
                    p.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        font.clone(),
                        egui::Color32::WHITE,
                    );
                    if ui
                        .interact(r, ui.id().with(label), egui::Sense::click())
                        .clicked()
                    {
                        match label {
                            "OK" => submit = true,
                            "Cancel" => cancel = true,
                            _ => requests.push(Request::EditMath(MathTarget::Slopefield {
                                index: w.selected,
                                clear: false,
                            })),
                        }
                    }
                }
            });
        if cancel {
            self.close_geometry_wizard();
            self.dirty();
        }
        if submit {
            requests.extend(self.slopefield_submit());
        }
    }
}
