//! Plot Go To evaluates the selected surface at a typed coordinate pair.
use super::{Calculation, FUNCTIONS, NativeTools, Page, SessionSnapshot, SurfacePoint};
use ppl_math::Real;

#[derive(Default)]
pub(super) struct Trace {
    pub(super) point: Option<SurfacePoint>,
    pub(super) draft: Option<[String; 2]>,
    pub(super) definition: super::plot_definition::View,
}

impl NativeTools {
    pub(super) fn graph3d_plot_go_to_button(&mut self, ui: &mut egui::Ui) {
        let trace = &mut self.graph3d_panel.trace;
        if let Some(point) = trace.point
            && ui.small_button("Go To").clicked()
        {
            trace.draft = Some(point.coordinates.map(Real::ppl_source));
        }
    }

    pub(super) fn show_graph3d_plot_point(&self, ui: &mut egui::Ui) {
        if let Some(point) = self.graph3d_panel.trace.point {
            let [x, y] = point.coordinates.map(Real::ppl_source);
            let value = point
                .height
                .map_or_else(|| "Undefined".into(), Real::ppl_source);
            ui.label(format!("{}({x}, {y}) = {value}", FUNCTIONS[point.index]));
        }
    }

    pub(super) fn graph3d_cancel_plot_go_to(&mut self) {
        self.graph3d_panel.cancel();
        self.graph3d_panel.trace.draft = None;
        self.record_calculation_error("surface", None);
    }

    pub(super) fn show_graph3d_plot_go_to(
        &mut self,
        ui: &mut egui::Ui,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) {
        let running = self.graph3d_panel.calculating();
        let trace = &mut self.graph3d_panel.trace;
        let Some(fields) = trace.draft.as_mut() else {
            return;
        };
        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("graph3d-plot-go-to").show(ui, |ui| {
                for (label, field) in ["X", "Y"].into_iter().zip(fields.iter_mut()) {
                    ui.label(label);
                    ui.add(
                        egui::TextEdit::singleline(field)
                            .char_limit(64)
                            .desired_width(90. * scale),
                    );
                    ui.end_row();
                }
            });
        });
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply = ui.add_enabled(!running, egui::Button::new("Go")).clicked();
            cancel = ui.button("Cancel").clicked();
        });
        if apply && let Some(point) = trace.point {
            match fields.each_ref().map(|value| Real::parse(value.trim())) {
                [Some(x), Some(y)] => {
                    self.graph3d_trace_at(point.index, [x, y], snapshot.clone());
                }
                _ => self.record_calculation_error(
                    "surface",
                    Some("Enter real X and Y coordinates".into()),
                ),
            }
        }
        if cancel {
            self.graph3d_cancel_plot_go_to();
        }
    }

    pub(super) fn graph3d_trace_at(
        &mut self,
        index: usize,
        coordinates: [Real; 2],
        snapshot: SessionSnapshot,
    ) {
        self.spawn_graph3d_calculation(Page::Plot, snapshot.generation, move |book, cancel| {
            book.trace_point(&snapshot, index, coordinates, cancel)
                .map(Calculation::Trace)
        });
    }

    pub fn graph3d_trace_point(&self) -> Option<serde_json::Value> {
        if self.memory.selected_app != super::NAME || self.page != Some(Page::Plot) {
            return None;
        }
        self.graph3d_panel.trace.point.map(|point| {
            serde_json::json!({
                "function": FUNCTIONS[point.index],
                "coordinates": point.coordinates.map(Real::ppl_source),
                "height": point.height.map(Real::ppl_source),
            })
        })
    }
}
