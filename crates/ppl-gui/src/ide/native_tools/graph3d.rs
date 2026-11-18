//! Graph 3D page controls share the saved surface book and a bounded worker.
use super::{NativeTools, Page, Request};
use crate::ide::{
    native::{expression::EditCommand, keys::KeyAction},
    native_graph3d::{Book, FUNCTIONS, Mesh, NAME, SurfacePoint},
};
mod numeric_custom;
mod numeric_navigation;
mod numeric_setup;
mod plot_definition;
mod plot_render;
mod plot_trace;
mod plot_zoom;
mod surface_colors;

use egui::{Rect, vec2};
use ppl_vm::{AppNumericSettings, SessionSnapshot};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Default)]
pub(super) struct Panel {
    job: Option<Job>,
    mesh: Option<Mesh>,
    surfaces: plot_render::SurfaceLayer,
    drag_origin: Option<(egui::Pos2, [f64; 2])>,
    numeric_draft: Option<numeric_setup::Draft>,
    navigation: numeric_navigation::Navigation,
    trace: plot_trace::Trace,
}
struct Job {
    page: Page,
    book: Book,
    generation: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Calculation, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
enum Calculation {
    Mesh(Mesh, Option<SurfacePoint>),
    Trace(SurfacePoint),
    EditedSurface {
        book: Box<Book>,
        mesh: Mesh,
        point: Option<SurfacePoint>,
    },
    Table(Vec<Vec<Option<f64>>>),
    Configure {
        source: Box<AppNumericSettings>,
        edited: Box<AppNumericSettings>,
        selected_cell: [usize; 2],
    },
}
impl Panel {
    pub(super) fn cancel(&mut self) {
        self.job = None;
    }

    pub(super) fn reset_numeric_draft(&mut self) {
        self.numeric_draft = None;
    }

    pub(super) fn calculating(&self) -> bool {
        self.job.is_some()
    }
}

impl NativeTools {
    pub(super) fn graph3d_apply_plot_context(
        &mut self,
        context: &SessionSnapshot,
    ) -> Result<(), String> {
        if let Some(settings) = context.app_plot_settings.get(NAME) {
            self.memory.graph3d.apply_plot_settings(settings)?;
        }
        Ok(())
    }

    pub(super) fn graph3d_calculate(&mut self, page: Page, snapshot: SessionSnapshot) {
        self.graph3d_panel.cancel();
        self.record_calculation_error("surface", None);
        match page {
            Page::Plot => {
                self.graph3d_panel.mesh = None;
                self.graph3d_panel.trace = Default::default();
            }
            Page::Numeric => self.table.clear(),
            _ => return,
        }
        if let Err(error) = self.graph3d_apply_plot_context(&snapshot) {
            self.record_calculation_error("surface", Some(error));
            return;
        }
        let generation = snapshot.generation;
        self.spawn_graph3d_calculation(page, generation, move |book, signal| {
            if page == Page::Plot {
                book.mesh(&snapshot, signal, None)
                    .map(|(mesh, point)| Calculation::Mesh(mesh, point))
            } else {
                book.numeric(&snapshot, signal).map(Calculation::Table)
            }
        });
    }

    fn spawn_graph3d_calculation(
        &mut self,
        page: Page,
        generation: u64,
        calculate: impl FnOnce(Book, Arc<AtomicBool>) -> Result<Calculation, String> + Send + 'static,
    ) {
        self.graph3d_panel.cancel();
        self.record_calculation_error("surface", None);
        let book = self.memory.graph3d.clone();
        let inputs = book.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(calculate(inputs, signal));
        });
        self.graph3d_panel.job = Some(Job {
            page,
            book,
            generation,
            cancel,
            rx,
        });
        self.dirty();
    }

    pub(super) fn graph3d_poll(&mut self, ctx: &egui::Context, snapshot: &SessionSnapshot) {
        let Some(job) = &self.graph3d_panel.job else {
            return;
        };
        let outcome = match job.rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("Surface calculation stopped".into()),
        };
        let job = self.graph3d_panel.job.take().unwrap();
        if self.memory.selected_app != NAME || self.page != Some(job.page) {
            return;
        }
        let book = &self.memory.graph3d;
        if job.book.definitions != book.definitions
            || job.book.bounds != book.bounds
            || (!self.presentation_pending && job.generation != snapshot.generation)
        {
            self.record_calculation_error(
                "surface",
                Some("Surface inputs changed during calculation".into()),
            );
        } else {
            match outcome {
                Ok(Calculation::Mesh(mesh, point)) => {
                    self.graph3d_panel.mesh = Some(mesh);
                    self.graph3d_panel.surfaces.invalidate();
                    self.graph3d_panel.trace.point = point;
                }
                Ok(Calculation::Trace(point)) => {
                    self.graph3d_panel.trace.point = Some(point);
                    self.graph3d_panel.trace.draft = None;
                }
                Ok(Calculation::EditedSurface { book, mesh, point }) => {
                    self.memory.graph3d = *book;
                    self.graph3d_panel.mesh = Some(mesh);
                    self.graph3d_panel.surfaces.invalidate();
                    self.graph3d_panel.trace.point = point;
                    self.graph3d_panel.trace.definition = plot_definition::View::Preview;
                }
                Ok(Calculation::Table(table)) => self.table = table,
                Ok(Calculation::Configure {
                    source,
                    edited,
                    selected_cell,
                }) => {
                    self.pending_actions
                        .push(Request::Graph3dNumericConfiguration {
                            source,
                            edited,
                            selected_cell,
                        });
                }
                Err(error) => self.record_calculation_error("surface", Some(error)),
            }
        }
        self.dirty();
    }

    pub fn graph3d_definitions_changed(&mut self, snapshot: &SessionSnapshot) {
        self.graph3d_panel.cancel();
        self.graph3d_panel.mesh = None;
        if self.memory.selected_app == NAME
            && let Some(page @ (Page::Plot | Page::Numeric)) = self.page
        {
            self.graph3d_calculate(page, snapshot.clone());
        }
        self.dirty();
    }

    pub fn graph3d_key(&mut self, action: KeyAction) -> bool {
        if self.memory.selected_app == NAME && self.page == Some(Page::Numeric) {
            return self.graph3d_numeric_action(action);
        }
        if self.memory.selected_app != NAME || self.page != Some(Page::Plot) {
            return false;
        }
        if self.graph3d_panel.trace.definition.editing() {
            if action == KeyAction::Back {
                self.graph3d_cancel_definition_edit();
            }
            return true;
        }
        if self.graph3d_panel.trace.draft.is_some() {
            if action == KeyAction::Back {
                self.graph3d_cancel_plot_go_to();
            }
            return true;
        }
        let delta = match action {
            KeyAction::Edit(EditCommand::Left(false)) => [-10., 0.],
            KeyAction::Edit(EditCommand::Right(false)) => [10., 0.],
            KeyAction::History(direction) if direction < 0 => [0., 10.],
            KeyAction::History(_) => [0., -10.],
            _ => return false,
        };
        self.graph3d_rotate(delta);
        true
    }

    fn graph3d_rotate(&mut self, [yaw, pitch]: [f64; 2]) {
        let angles = &mut self.memory.graph3d.rotation;
        angles[0] = (angles[0] + yaw).rem_euclid(360.);
        angles[1] = (angles[1] + pitch).clamp(-85., 85.);
        self.dirty();
    }

    pub(super) fn show_graph3d(
        &mut self,
        ui: &mut egui::Ui,
        page: Page,
        lcd: Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
    ) -> bool {
        match page {
            Page::Symbolic => {
                let mut changed = false;
                egui::ScrollArea::vertical()
                    .max_height(170. * scale)
                    .show(ui, |ui| {
                        for (index, label) in FUNCTIONS.iter().enumerate() {
                            let definition = &mut self.memory.graph3d.definitions[index];
                            let colors = &mut self.memory.graph3d.colors[index];
                            ui.horizontal(|ui| {
                                let defined = !definition.source.trim().is_empty();
                                changed |= ui
                                    .add_enabled(
                                        defined,
                                        egui::Checkbox::without_text(&mut definition.enabled),
                                    )
                                    .changed();
                                ui.label(format!("{label}(X,Y)"));
                                changed |= surface_colors::edit(ui, index, colors);
                                let input = egui::TextEdit::singleline(&mut definition.source)
                                    .desired_width(ui.available_width())
                                    .char_limit(16_384);
                                if ui.add(input).changed() {
                                    definition.literal = None;
                                    if definition.source.trim().is_empty() {
                                        definition.enabled = false;
                                    }
                                    changed = true;
                                }
                            });
                        }
                    });
                if changed {
                    self.dirty();
                }
                ui.horizontal(|ui| {
                    for (label, target) in [
                        ("Plot", Page::Plot),
                        ("Numeric", Page::Numeric),
                        ("Setup", Page::PlotSetup),
                    ] {
                        if ui.button(label).clicked() {
                            self.open(target);
                            if target != Page::PlotSetup {
                                self.graph3d_calculate(target, snapshot.clone());
                            }
                        }
                    }
                });
            }
            Page::Plot => {
                if self.graph3d_panel.trace.definition.editing() {
                    self.show_graph3d_definition_editor(ui, scale, snapshot);
                    return true;
                }
                if self.graph3d_panel.trace.draft.is_some() {
                    self.show_graph3d_plot_go_to(ui, scale, snapshot);
                    return true;
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Reset view").clicked() {
                        self.memory.graph3d.rotation = Book::default().rotation;
                        self.dirty();
                    }
                    if ui.small_button("Setup").clicked() {
                        self.open(Page::PlotSetup);
                    }
                    self.graph3d_plot_go_to_button(ui);
                    self.graph3d_definition_button(ui);
                    self.graph3d_plot_zoom_menu(ui, snapshot);
                });
                let height = if self.graph3d_panel.trace.definition.visible() {
                    125.
                } else {
                    174.
                };
                let size = vec2((lcd.width() - 4. * scale).max(1.), height * scale);
                let (response, painter) = ui.allocate_painter(size, egui::Sense::drag());
                if response.drag_started() {
                    self.graph3d_panel.drag_origin = ui
                        .input(|i| i.pointer.press_origin())
                        .map(|p| (p, self.memory.graph3d.rotation));
                }
                if (response.dragged() || response.drag_stopped())
                    && let Some((origin, angles)) = self.graph3d_panel.drag_origin
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    // The displacement starts at the press, even when the first
                    // pointer event in this frame arrived from outside the plot.
                    let delta = (pointer - origin) / scale;
                    let camera = [
                        (angles[0] + 0.3 * f64::from(delta.x)).rem_euclid(360.),
                        (angles[1] + 0.3 * f64::from(delta.y)).clamp(-85., 85.),
                    ];
                    if self.memory.graph3d.rotation != camera {
                        self.memory.graph3d.rotation = camera;
                        self.dirty();
                    }
                }
                if response.drag_stopped() {
                    self.graph3d_panel.drag_origin = None;
                }
                self.graph3d_panel.surfaces.paint(
                    &painter,
                    response.rect,
                    height as usize,
                    &self.memory.graph3d,
                    self.graph3d_panel.mesh.as_ref(),
                    self.graph3d_panel
                        .trace
                        .point
                        .and_then(SurfacePoint::projectable),
                );
                self.show_graph3d_plot_point(ui);
                if self.graph3d_panel.trace.definition.visible() {
                    self.show_graph3d_definition_preview(ui, snapshot);
                }
            }
            Page::Numeric => self.show_graph3d_numeric(ui, scale, snapshot),
            Page::NumericSetup => {
                self.show_graph3d_numeric_setup(ui, scale, snapshot);
            }
            Page::PlotSetup => {
                if let Err(error) = self.graph3d_apply_plot_context(snapshot) {
                    self.record_calculation_error("surface", Some(error));
                    return true;
                }
                let mut setup = self.memory.graph3d.clone();
                let mut changed = false;
                for (index, name) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{name} range"));
                        for bound in &mut setup.bounds[index] {
                            changed |= ui.add(egui::DragValue::new(bound).speed(0.25)).changed();
                        }
                        ui.label("Tick");
                        changed |= ui
                            .add(egui::DragValue::new(&mut setup.ticks[index]).speed(0.1))
                            .changed();
                    });
                }
                if changed {
                    match setup.plot_settings() {
                        Ok(settings) => {
                            self.memory.graph3d = setup;
                            self.pending_actions
                                .push(Request::PlotConfiguration(NAME.into(), settings));
                            self.record_calculation_error("surface", None);
                        }
                        Err(error) => self.record_calculation_error("surface", Some(error)),
                    }
                    self.dirty();
                }
                if ui.button("Plot").clicked() {
                    self.open(Page::Plot);
                    self.graph3d_calculate(Page::Plot, snapshot.clone());
                }
            }
            _ => return false,
        }
        true
    }
}
