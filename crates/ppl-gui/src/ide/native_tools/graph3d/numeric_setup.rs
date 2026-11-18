//! Decimal Numeric Setup edits are committed together through calculator memory.
use super::{NAME, NativeTools, Page, Request};
use crate::ide::native_graph3d::NUMERIC_ROWS;
use ppl_math::Real;
use ppl_vm::{AppNumericSettings, SessionSnapshot, Value};

const FIELDS: [(&str, &str); 6] = [
    ("X start", "NumXStart"),
    ("Y start", "NumYStart"),
    ("X step", "NumXStep"),
    ("Y step", "NumYStep"),
    ("X zoom", "NumXZoom"),
    ("Y zoom", "NumYZoom"),
];

pub(super) struct Draft {
    source: AppNumericSettings,
    fields: [String; FIELDS.len()],
    custom: bool,
    points: Vec<[String; 2]>,
}

impl Draft {
    fn from_settings(source: AppNumericSettings) -> Self {
        let fields = FIELDS.map(|(_, field)| {
            source
                .get_graph3d(field)
                .unwrap()
                .as_real()
                .unwrap()
                .ppl_source()
        });
        let points = source
            .graph3d
            .as_ref()
            .unwrap()
            .points
            .iter()
            .take(NUMERIC_ROWS)
            .map(|pair| pair.map(|number| number.ppl_source()))
            .collect();
        Self {
            fields,
            custom: source.kind == 1,
            source,
            points,
        }
    }

    fn settings(&self) -> Result<AppNumericSettings, String> {
        let mut settings = self.source.clone();
        let parse = |value: &str, field: &str| {
            Real::parse(value.trim()).ok_or_else(|| format!("{field} requires a real number"))
        };
        for ((label, field), value) in FIELDS.iter().zip(&self.fields) {
            let value = Value::Real(parse(value, label)?);
            settings
                .set_graph3d(field, &value)
                .map_err(|error| error.to_string())?;
        }
        settings.kind = i64::from(self.custom);
        let points = &mut settings.graph3d.as_mut().unwrap().points;
        let edited = self
            .points
            .iter()
            .map(|[x, y]| Ok([parse(x, "X")?, parse(y, "Y")?]))
            .collect::<Result<Vec<_>, String>>()?;
        points.splice(..points.len().min(NUMERIC_ROWS), edited);
        settings.validate()?;
        Ok(settings)
    }
}

impl NativeTools {
    pub(super) fn show_graph3d_numeric_setup(
        &mut self,
        ui: &mut egui::Ui,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) {
        let draft = self.graph3d_panel.numeric_draft.get_or_insert_with(|| {
            Draft::from_settings(
                snapshot
                    .app_numeric_settings
                    .get(NAME)
                    .cloned()
                    .unwrap_or_else(|| AppNumericSettings::for_app(NAME)),
            )
        });
        let mut apply = false;
        let mut cancel = false;
        egui::ScrollArea::vertical()
            .max_height(166. * scale)
            .show(ui, |ui| {
                if !draft.custom {
                    egui::Grid::new("graph3d-numeric-setup").show(ui, |ui| {
                        for ((label, _), field) in FIELDS.iter().zip(&mut draft.fields) {
                            ui.label(*label);
                            ui.add(
                                egui::TextEdit::singleline(field)
                                    .char_limit(64)
                                    .desired_width(90. * scale),
                            );
                            ui.end_row();
                        }
                    });
                }
                egui::ComboBox::from_label("Table type")
                    .selected_text(if draft.custom { "Custom" } else { "Automatic" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.custom, false, "Automatic");
                        ui.selectable_value(&mut draft.custom, true, "Custom");
                    });
                if draft.custom {
                    let mut remove = None;
                    for (row, [x, y]) in draft.points.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label("X");
                            ui.add(
                                egui::TextEdit::singleline(x)
                                    .char_limit(64)
                                    .desired_width(56. * scale),
                            );
                            ui.label("Y");
                            ui.add(
                                egui::TextEdit::singleline(y)
                                    .char_limit(64)
                                    .desired_width(56. * scale),
                            );
                            if ui.small_button("Remove").clicked() {
                                remove = Some(row);
                            }
                        });
                    }
                    if let Some(row) = remove {
                        draft.points.remove(row);
                    }
                    if ui.button("Add point").clicked() && draft.points.len() < 500_000 {
                        draft.points.push(["0".into(), "0".into()]);
                    }
                }
            });
        ui.horizontal(|ui| {
            apply = ui.button("Apply").clicked();
            cancel = ui.button("Cancel").clicked();
        });
        if apply {
            match draft.settings() {
                Ok(settings) => self
                    .pending_actions
                    .push(Request::Graph3dNumericConfiguration {
                        source: Box::new(draft.source.clone()),
                        edited: Box::new(settings),
                        selected_cell: [0, 0],
                    }),
                Err(error) => self.record_calculation_error("surface", Some(error)),
            }
            self.dirty();
        }
        if cancel {
            self.open(Page::Numeric);
            self.graph3d_calculate(Page::Numeric, snapshot.clone());
        }
    }
}
