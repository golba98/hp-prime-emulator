//! The Plot definition editor uses the same surface definitions as Symbolic and PPL.
use super::{
    Calculation, FUNCTIONS, NativeTools, Page, SessionSnapshot, SurfacePoint, surface_colors,
};

#[derive(Default)]
pub(super) enum View {
    #[default]
    Hidden,
    Preview,
    Editing {
        selection: SurfacePoint,
        source: String,
    },
}

impl View {
    pub(super) fn visible(&self) -> bool {
        !matches!(self, Self::Hidden)
    }

    pub(super) fn editing(&self) -> bool {
        matches!(self, Self::Editing { .. })
    }
}

impl NativeTools {
    pub(super) fn graph3d_definition_button(&mut self, ui: &mut egui::Ui) {
        let trace = &mut self.graph3d_panel.trace;
        if trace.point.is_some() && ui.small_button("Defn").clicked() {
            trace.definition = if trace.definition.visible() {
                View::Hidden
            } else {
                View::Preview
            };
        }
    }

    pub(super) fn show_graph3d_definition_preview(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &SessionSnapshot,
    ) {
        let Some(point) = self.graph3d_panel.trace.point else {
            return;
        };
        let definitions = &self.memory.graph3d.definitions;
        let colors = &mut self.memory.graph3d.colors[point.index];
        let mut index = point.index;
        let mut edit = false;
        let mut recolored = false;
        ui.add_enabled_ui(!self.graph3d_panel.calculating(), |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("graph3d-plot-definition")
                    .selected_text(format!("{}(X,Y)", FUNCTIONS[index]))
                    .show_ui(ui, |ui| {
                        for (slot, (name, definition)) in
                            FUNCTIONS.iter().zip(definitions).enumerate()
                        {
                            if definition.enabled && !definition.source.trim().is_empty() {
                                ui.selectable_value(&mut index, slot, format!("{name}(X,Y)"));
                            }
                        }
                    });
                edit = ui.button("Edit").clicked();
                recolored = surface_colors::edit(ui, point.index, colors);
            });
        });
        let source = definitions[point.index].source.clone();
        ui.label(&source);
        if recolored {
            self.dirty();
        }
        if index != point.index {
            self.graph3d_trace_at(index, point.coordinates, snapshot.clone());
        } else if edit {
            self.graph3d_panel.trace.definition = View::Editing {
                selection: point,
                source,
            };
        }
    }

    pub(super) fn graph3d_cancel_definition_edit(&mut self) {
        self.graph3d_panel.cancel();
        self.graph3d_panel.trace.definition = View::Preview;
        self.record_calculation_error("surface", None);
    }

    pub(super) fn show_graph3d_definition_editor(
        &mut self,
        ui: &mut egui::Ui,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) {
        let running = self.graph3d_panel.calculating();
        let View::Editing { selection, source } = &mut self.graph3d_panel.trace.definition else {
            return;
        };
        let selection = *selection;
        ui.label(format!("{}(X,Y)", FUNCTIONS[selection.index]));
        let field = ui.add_enabled(
            !running,
            egui::TextEdit::singleline(source)
                .char_limit(16_384)
                .desired_width(245. * scale),
        );
        let mut apply =
            !running && field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply |= ui
                .add_enabled(!running, egui::Button::new("Apply"))
                .clicked();
            cancel = ui.button("Cancel").clicked();
        });
        if apply {
            let source = source.clone();
            let context = snapshot.clone();
            self.spawn_graph3d_calculation(
                Page::Plot,
                context.generation,
                move |mut book, cancel| {
                    let definition = &mut book.definitions[selection.index];
                    if source.trim().is_empty() {
                        definition.enabled = false;
                    }
                    definition.source = source;
                    definition.literal = None;
                    let (mesh, point) = book.mesh(&context, cancel, Some(selection))?;
                    Ok(Calculation::EditedSurface {
                        book: Box::new(book),
                        mesh,
                        point,
                    })
                },
            );
        }
        if cancel {
            self.graph3d_cancel_definition_edit();
        }
    }
}
