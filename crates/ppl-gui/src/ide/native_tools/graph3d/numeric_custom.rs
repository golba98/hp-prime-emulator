//! Coordinate editing and sorting use the shared decimal table settings.
use super::{AppNumericSettings, Calculation, NAME, NativeTools, Page, SessionSnapshot};
use crate::ide::native_graph3d::NUMERIC_ROWS;
use ppl_math::Real;
use std::sync::Arc;

pub(super) struct CoordinateDraft {
    source: AppNumericSettings,
    cell: [usize; 2],
    text: String,
}

impl CoordinateDraft {
    pub(super) fn new(source: AppNumericSettings, cell: [usize; 2]) -> Self {
        let points = &source
            .graph3d
            .as_ref()
            .expect("Graph 3D Numeric axes")
            .points;
        let [row, column] = cell;
        let text = points
            .get(row)
            .map_or_else(String::new, |pair| pair[column].ppl_source());
        Self { source, cell, text }
    }
}

pub(super) fn appends_point(source: &AppNumericSettings, visible_rows: usize) -> bool {
    source.kind == 1
        && source
            .graph3d
            .as_ref()
            .is_some_and(|axis| axis.points.len() == visible_rows && visible_rows < NUMERIC_ROWS)
}

impl NativeTools {
    pub(super) fn show_graph3d_coordinate_draft(
        &mut self,
        ui: &mut egui::Ui,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) {
        let running = self.graph3d_panel.calculating();
        let Some(draft) = &mut self.graph3d_panel.navigation.coordinate else {
            return;
        };
        let axis = ["X", "Y"][draft.cell[1]];
        ui.add_enabled_ui(!running, |ui| {
            ui.horizontal(|ui| {
                ui.label(axis);
                ui.add(
                    egui::TextEdit::singleline(&mut draft.text)
                        .char_limit(512)
                        .desired_width(130. * scale),
                );
            });
        });
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply = ui
                .add_enabled(!running, egui::Button::new("Apply"))
                .clicked();
            cancel = ui.button("Cancel").clicked();
        });
        if apply {
            let source = draft.source.clone();
            let selected_cell = draft.cell;
            let text = draft.text.clone();
            let mut context = snapshot.clone();
            context.active_app = NAME.into();
            let sheet = self.memory.models.spreadsheet.clone();
            let geometry = self.memory.models.geometry_book.clone().map(Arc::new);
            let function = self.memory.graphs["Function"].clone();
            self.spawn_graph3d_calculation(Page::Numeric, snapshot.generation, move |_, signal| {
                let value = crate::ide::native_application_access::evaluate_expression(
                    &text, sheet, function, geometry, &context, signal,
                )
                .and_then(|value| value.expect_real("Graph 3D coordinate"))
                .map_err(|error| error.to_string())?;
                let mut edited = source.clone();
                let points = &mut edited
                    .graph3d
                    .as_mut()
                    .expect("Graph 3D Numeric axes")
                    .points;
                let [row, column] = selected_cell;
                if row == points.len() {
                    points.push([Real::ZERO; 2]);
                }
                points[row][column] = value;
                Ok(Calculation::Configure {
                    source: Box::new(source),
                    edited: Box::new(edited),
                    selected_cell,
                })
            });
        }
        if cancel {
            self.graph3d_cancel_numeric_draft();
        }
    }

    pub(super) fn graph3d_sort_custom(
        &mut self,
        source: AppNumericSettings,
        selected_cell: [usize; 2],
        ascending: bool,
        snapshot: SessionSnapshot,
    ) {
        self.spawn_graph3d_calculation(Page::Numeric, snapshot.generation, move |book, signal| {
            let mut edited = source.clone();
            book.sort_numeric_points(&mut edited, selected_cell[1], ascending, &snapshot, signal)?;
            Ok(Calculation::Configure {
                source: Box::new(source),
                edited: Box::new(edited),
                selected_cell,
            })
        });
    }
}
