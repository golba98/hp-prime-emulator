//! Automatic table navigation uses the same decimal coordinates as evaluation.
use super::{
    FUNCTIONS, KeyAction, NAME, NativeTools, Page, Request, SessionSnapshot,
    numeric_custom::{CoordinateDraft, appends_point},
};
use crate::ide::native::expression::EditCommand;
use crate::ide::native_graph3d::numeric_coordinates;
use ppl_math::{MathError, Real};
use ppl_vm::{AngleMode, AppNumericSettings};
use std::borrow::Cow;

const GO_TO_ROW: usize = 5;

#[derive(Default)]
pub(super) struct Navigation {
    cell: [usize; 2],
    grid: [usize; 2],
    reveal_selection: bool,
    show_definition: bool,
    go_to: Option<GoTo>,
    pub(super) coordinate: Option<CoordinateDraft>,
}

impl Navigation {
    fn editing(&self) -> bool {
        self.go_to.is_some() || self.coordinate.is_some()
    }
}

struct GoTo {
    source: AppNumericSettings,
    coordinates: [String; 2],
}

#[derive(Clone, Copy)]
enum Axes {
    Both,
    X,
    Y,
}

#[derive(Clone, Copy)]
enum Zoom {
    In(Axes),
    Out(Axes),
    Decimal,
    Integer,
    Trig,
}

const ZOOM_CHOICES: [(&str, Zoom); 9] = [
    ("In", Zoom::In(Axes::Both)),
    ("Out", Zoom::Out(Axes::Both)),
    ("X In", Zoom::In(Axes::X)),
    ("X Out", Zoom::Out(Axes::X)),
    ("Y In", Zoom::In(Axes::Y)),
    ("Y Out", Zoom::Out(Axes::Y)),
    ("Decimal", Zoom::Decimal),
    ("Integer", Zoom::Integer),
    ("Trig", Zoom::Trig),
];

impl Zoom {
    fn apply(
        self,
        settings: &mut AppNumericSettings,
        row: usize,
        angle: AngleMode,
    ) -> Result<usize, MathError> {
        if let Zoom::In(axes) | Zoom::Out(axes) = self {
            let [selected_x, selected_y] = numeric_coordinates(settings, row)?;
            let index = Real::from_i64(row as i64);
            let anchored = |coordinate: Real, step: Real, factor: Real| -> Result<_, MathError> {
                let next_step = if matches!(self, Zoom::In(_)) {
                    step.div(factor)?
                } else {
                    step.mul(factor)?
                };
                Ok((coordinate.sub(next_step.mul(index)?)?, next_step))
            };
            if matches!(axes, Axes::Both | Axes::X) {
                (settings.start, settings.step) =
                    anchored(selected_x, settings.step, settings.zoom)?;
            }
            if matches!(axes, Axes::Both | Axes::Y) {
                let axis = settings.graph3d.as_mut().expect("Graph 3D Numeric axes");
                (axis.y_start, axis.y_step) = anchored(selected_y, axis.y_step, axis.y_zoom)?;
            }
            return Ok(row);
        }

        let step = match self {
            Zoom::Decimal => AppNumericSettings::for_app(NAME).step,
            Zoom::Integer => Real::ONE,
            Zoom::Trig if angle == AngleMode::Radians => {
                Real::from_f64(std::f64::consts::PI)?.div(Real::from_i64(24))?
            }
            Zoom::Trig => Real::parse("7.5").unwrap(),
            Zoom::In(_) | Zoom::Out(_) => unreachable!(),
        };
        let axis = settings.graph3d.as_mut().expect("Graph 3D Numeric axes");
        (settings.start, axis.y_start) = (Real::ZERO, Real::ZERO);
        (settings.step, axis.y_step) = (step, step);
        Ok(0)
    }
}

impl GoTo {
    fn new(source: AppNumericSettings, row: usize) -> Result<Self, MathError> {
        let coordinates = numeric_coordinates(&source, row)?.map(Real::ppl_source);
        Ok(Self {
            source,
            coordinates,
        })
    }

    fn settings(&self) -> Result<AppNumericSettings, String> {
        let mut settings = self.source.clone();
        let row = Real::from_i64(GO_TO_ROW as i64);
        let start = |value: &str, step: Real| {
            Real::parse(value.trim())
                .ok_or_else(|| "Enter a real coordinate".to_string())?
                .sub(step.mul(row).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())
        };
        let axis = settings.graph3d.as_mut().expect("Graph 3D Numeric axes");
        settings.start = start(&self.coordinates[0], settings.step)?;
        axis.y_start = start(&self.coordinates[1], axis.y_step)?;
        Ok(settings)
    }
}

impl NativeTools {
    pub fn graph3d_select_numeric_cell(&mut self, cell: [usize; 2]) {
        self.graph3d_panel.navigation = Navigation {
            cell,
            reveal_selection: true,
            ..Navigation::default()
        };
    }

    pub fn graph3d_numeric_cell(&self) -> Option<[usize; 2]> {
        (self.memory.selected_app == NAME && self.page == Some(Page::Numeric))
            .then_some(self.graph3d_panel.navigation.cell)
    }

    pub fn graph3d_numeric_navigation_active(&self) -> bool {
        self.memory.selected_app == NAME
            && self.page == Some(Page::Numeric)
            && !self.graph3d_panel.navigation.editing()
    }

    pub(super) fn graph3d_cancel_numeric_draft(&mut self) {
        self.graph3d_panel.cancel();
        self.graph3d_panel.navigation.coordinate = None;
        self.graph3d_panel.navigation.go_to = None;
        self.record_calculation_error("surface", None);
    }

    pub(super) fn graph3d_numeric_action(&mut self, action: KeyAction) -> bool {
        let navigation = &mut self.graph3d_panel.navigation;
        if navigation.editing() {
            if action == KeyAction::Back {
                self.graph3d_cancel_numeric_draft();
                return true;
            }
            return false;
        }
        if navigation.grid[0] == 0 {
            return false;
        }
        let (axis, delta) = match action {
            KeyAction::History(delta) => (0, isize::from(delta)),
            KeyAction::Edit(EditCommand::Left(false)) => (1, -1),
            KeyAction::Edit(EditCommand::Right(false)) => (1, 1),
            _ => return false,
        };
        navigation.cell[axis] = navigation.cell[axis]
            .saturating_add_signed(delta)
            .min(navigation.grid[axis] - 1);
        navigation.reveal_selection = true;
        true
    }

    pub(super) fn show_graph3d_numeric(
        &mut self,
        ui: &mut egui::Ui,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) {
        if self.graph3d_panel.navigation.coordinate.is_some() {
            self.show_graph3d_coordinate_draft(ui, scale, snapshot);
            return;
        }
        if self.graph3d_panel.navigation.go_to.is_some() {
            self.show_graph3d_go_to(ui, scale);
            return;
        }
        let source = snapshot.app_numeric_settings.get(NAME).map_or_else(
            || Cow::Owned(AppNumericSettings::for_app(NAME)),
            Cow::Borrowed,
        );
        let custom = source.kind == 1;
        let running = self.graph3d_panel.calculating();
        let can_append = !running && appends_point(&source, self.table.len());
        let definitions = self
            .memory
            .graph3d
            .definitions
            .iter()
            .zip(FUNCTIONS)
            .filter(|(definition, _)| definition.enabled && !definition.source.trim().is_empty())
            .collect::<Vec<_>>();
        let navigation = &mut self.graph3d_panel.navigation;
        navigation.grid = [
            self.table.len() + usize::from(can_append),
            definitions.len() + 2,
        ];
        if !running {
            for (position, limit) in navigation.cell.iter_mut().zip(navigation.grid) {
                *position = (*position).min(limit.saturating_sub(1));
            }
        }
        let [row, column] = navigation.cell;
        let definition = column
            .checked_sub(2)
            .and_then(|index| definitions.get(index));
        let mut setup = false;
        let mut go_to = false;
        let mut edit = false;
        let mut zoom_choice = None;
        let mut ascending = None;
        ui.horizontal(|ui| {
            setup = ui.button("Setup").clicked();
            if custom {
                let editable = column < 2 && row < navigation.grid[0] && !running;
                edit = ui
                    .add_enabled(editable, egui::Button::new("Edit"))
                    .clicked();
                ui.add_enabled_ui(self.table.len() > 1 && !running, |ui| {
                    ui.menu_button("Sort", |ui| {
                        for (label, order) in [("Ascending", true), ("Descending", false)] {
                            if ui.button(label).clicked() {
                                ascending = Some(order);
                                ui.close();
                            }
                        }
                    });
                });
            } else {
                ui.add_enabled_ui(!self.table.is_empty() && !running, |ui| {
                    ui.menu_button("Zoom", |ui| {
                        for (label, zoom) in ZOOM_CHOICES {
                            if ui.button(label).clicked() {
                                zoom_choice = Some(zoom);
                                ui.close();
                            }
                        }
                    });
                    go_to = ui.button("Go To").clicked();
                });
            }
            if definition.is_some() && ui.button("Defn").clicked() {
                navigation.show_definition = !navigation.show_definition;
            }
        });
        let preview = definition
            .filter(|_| navigation.show_definition)
            .map(|(definition, label)| format!("{label}(X,Y) = {}", definition.source));

        if setup {
            self.open(Page::NumericSetup);
            return;
        }
        if edit {
            navigation.coordinate = Some(CoordinateDraft::new(source.into_owned(), [row, column]));
            return;
        }
        if go_to {
            match GoTo::new(source.into_owned(), row) {
                Ok(go_to) => navigation.go_to = Some(go_to),
                Err(error) => self.record_calculation_error("surface", Some(error.to_string())),
            }
            return;
        }
        if let Some(ascending) = ascending {
            self.graph3d_sort_custom(
                source.into_owned(),
                [row, column],
                ascending,
                snapshot.clone(),
            );
        } else if let Some(zoom) = zoom_choice {
            let angle = snapshot
                .app_settings
                .get(NAME)
                .cloned()
                .unwrap_or_default()
                .effective(&snapshot.settings)
                .angle;
            let source = source.into_owned();
            let mut edited = source.clone();
            match zoom.apply(&mut edited, row, angle) {
                Ok(row) => self.submit_graph3d_navigation(source, edited, [row, column]),
                Err(error) => self.record_calculation_error("surface", Some(error.to_string())),
            }
        }
        if let Some(preview) = preview {
            ui.label(preview);
        }
        self.show_graph3d_rows(ui, scale, can_append);
    }

    fn show_graph3d_rows(&mut self, ui: &mut egui::Ui, scale: f32, can_append: bool) {
        let navigation = &mut self.graph3d_panel.navigation;
        egui::ScrollArea::both()
            .max_height(184. * scale)
            .show(ui, |ui| {
                egui::Grid::new("graph3d-table")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("X");
                        ui.label("Y");
                        for (label, definition) in
                            FUNCTIONS.iter().zip(&self.memory.graph3d.definitions)
                        {
                            if definition.enabled && !definition.source.trim().is_empty() {
                                ui.label(*label);
                            }
                        }
                        ui.end_row();
                        let mut draw_cell = |ui: &mut egui::Ui, location, label| {
                            let selected = navigation.cell == location;
                            let response = ui.selectable_label(selected, label);
                            if response.clicked() {
                                navigation.cell = location;
                            }
                            if selected && navigation.reveal_selection {
                                response.scroll_to_me(Some(egui::Align::Center));
                                navigation.reveal_selection = false;
                            }
                        };
                        for (index, row) in self.table.iter().enumerate() {
                            for (column, value) in row.iter().enumerate() {
                                let label =
                                    value.map_or_else(|| "—".into(), |number| number.to_string());
                                draw_cell(ui, [index, column], label);
                            }
                            ui.end_row();
                        }
                        if can_append {
                            for (column, label) in ["New X", "New Y"].into_iter().enumerate() {
                                draw_cell(ui, [self.table.len(), column], label.into());
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    fn show_graph3d_go_to(&mut self, ui: &mut egui::Ui, scale: f32) {
        let draft = self
            .graph3d_panel
            .navigation
            .go_to
            .as_mut()
            .expect("Go To draft");
        egui::Grid::new("graph3d-go-to").show(ui, |ui| {
            for (axis, value) in ["X", "Y"].into_iter().zip(&mut draft.coordinates) {
                ui.label(axis);
                ui.add(
                    egui::TextEdit::singleline(value)
                        .char_limit(64)
                        .desired_width(90. * scale),
                );
                ui.end_row();
            }
        });
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply = ui.button("Go").clicked();
            cancel = ui.button("Cancel").clicked();
        });
        if apply {
            match draft.settings() {
                Ok(edited) => {
                    let source = draft.source.clone();
                    let column = self.graph3d_panel.navigation.cell[1];
                    self.submit_graph3d_navigation(source, edited, [GO_TO_ROW, column]);
                }
                Err(error) => self.record_calculation_error("surface", Some(error)),
            }
        }
        if cancel {
            self.graph3d_cancel_numeric_draft();
        }
    }

    fn submit_graph3d_navigation(
        &mut self,
        source: AppNumericSettings,
        edited: AppNumericSettings,
        selected_cell: [usize; 2],
    ) {
        match edited.validate() {
            Ok(()) => self
                .pending_actions
                .push(Request::Graph3dNumericConfiguration {
                    source: Box::new(source),
                    edited: Box::new(edited),
                    selected_cell,
                }),
            Err(error) => self.record_calculation_error("surface", Some(error)),
        }
    }
}
