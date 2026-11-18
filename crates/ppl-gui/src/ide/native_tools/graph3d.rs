//! Graph 3D page controls share the saved surface book and a bounded worker.
use super::{NativeTools, Page};
use crate::ide::{
    native::{expression::EditCommand, keys::KeyAction},
    native_graph3d::{Book, FUNCTIONS, Mesh, NAME, position},
};
use egui::{Color32, Rect, Stroke, pos2, vec2};
use ppl_vm::SessionSnapshot;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Default)]
pub(super) struct Panel {
    job: Option<Job>,
    mesh: Option<Mesh>,
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
    Mesh(Mesh),
    Table(Vec<Vec<Option<f64>>>),
}
impl Panel {
    pub(super) fn cancel(&mut self) {
        self.job = None;
    }

    pub(super) fn calculating(&self) -> bool {
        self.job.is_some()
    }
}

impl NativeTools {
    pub(super) fn graph3d_calculate(&mut self, page: Page, snapshot: SessionSnapshot) {
        self.graph3d_panel.cancel();
        self.record_calculation_error("surface", None);
        match page {
            Page::Plot => self.graph3d_panel.mesh = None,
            Page::Numeric => self.table.clear(),
            _ => return,
        }
        let generation = snapshot.generation;
        let book = self.memory.graph3d.clone();
        let inputs = book.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let calculation = if page == Page::Plot {
                inputs.mesh(&snapshot, signal).map(Calculation::Mesh)
            } else {
                inputs.numeric(&snapshot, signal).map(Calculation::Table)
            };
            let _ = tx.send(calculation);
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
                Ok(Calculation::Mesh(mesh)) => self.graph3d_panel.mesh = Some(mesh),
                Ok(Calculation::Table(table)) => self.table = table,
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

    pub fn graph3d_rotate_key(&mut self, action: KeyAction) -> bool {
        if self.memory.selected_app != NAME || self.page != Some(Page::Plot) {
            return false;
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
                        for (label, definition) in
                            FUNCTIONS.iter().zip(&mut self.memory.graph3d.definitions)
                        {
                            ui.horizontal(|ui| {
                                let defined = !definition.source.trim().is_empty();
                                changed |= ui
                                    .add_enabled(
                                        defined,
                                        egui::Checkbox::without_text(&mut definition.enabled),
                                    )
                                    .changed();
                                ui.label(format!("{label}(X,Y)"));
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
                ui.horizontal(|ui| {
                    if ui.small_button("Reset view").clicked() {
                        self.memory.graph3d.rotation = Book::default().rotation;
                        self.dirty();
                    }
                    if ui.small_button("Setup").clicked() {
                        self.open(Page::PlotSetup);
                    }
                });
                let size = vec2((lcd.width() - 4. * scale).max(1.), 174. * scale);
                let (response, painter) = ui.allocate_painter(size, egui::Sense::drag());
                if response.dragged() {
                    let delta = ui.input(|input| input.pointer.delta()) / scale.max(0.01);
                    self.graph3d_rotate([f64::from(delta.x), -f64::from(delta.y)]);
                }
                paint(
                    &painter,
                    response.rect,
                    &self.memory.graph3d,
                    self.graph3d_panel.mesh.as_ref(),
                );
            }
            Page::Numeric => {
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
                                for row in &self.table {
                                    for value in row {
                                        ui.label(
                                            value.map_or_else(|| "—".into(), |v| v.to_string()),
                                        );
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }
            Page::PlotSetup => {
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
                    match setup.validate() {
                        Ok(()) => {
                            self.memory.graph3d = setup;
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

fn paint(painter: &egui::Painter, area: Rect, book: &Book, mesh: Option<&Mesh>) {
    painter.rect_filled(area, 0., Color32::from_rgb(16, 21, 29));
    let [yaw, pitch] = book.rotation.map(f64::to_radians);
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let scale = f64::from(area.width().min(area.height())) * 0.25;
    let project = |point: [f64; 3]| {
        let [x, y, z] = std::array::from_fn(|i| {
            let [lo, hi] = book.bounds[i];
            2. * ((point[i] - lo) / (hi - lo)) - 1.
        });
        let depth = sin_yaw * x + cos_yaw * y;
        let u = cos_yaw * x - sin_yaw * y;
        let v = cos_pitch * z - sin_pitch * depth;
        area.center() + vec2((u * scale) as f32, (-v * scale) as f32)
    };
    let axis = Stroke::new(1., Color32::from_rgb(100, 114, 132));
    for axis_index in 0..3 {
        let others = [(axis_index + 1) % 3, (axis_index + 2) % 3];
        for a in book.bounds[others[0]] {
            for b in book.bounds[others[1]] {
                let mut low = [0.; 3];
                low[others[0]] = a;
                low[others[1]] = b;
                low[axis_index] = book.bounds[axis_index][0];
                let mut high = low;
                high[axis_index] = book.bounds[axis_index][1];
                painter.line_segment([project(low), project(high)], axis);
            }
        }
        let [low, high] = book.bounds[axis_index];
        let tick = book.ticks[axis_index];
        for index in 1..32 {
            let coordinate = low + f64::from(index) * tick;
            if coordinate >= high {
                break;
            }
            let mut start = book.bounds.map(|bounds| bounds[0]);
            start[axis_index] = coordinate;
            let mut end = start;
            let extent = book.bounds[others[0]];
            end[others[0]] += (extent[1] - extent[0]) * 0.035;
            painter.line_segment([project(start), project(end)], axis);
        }
    }
    let Some(mesh) = mesh else { return };
    let n = mesh.resolution;
    let palette = [
        Color32::from_rgb(86, 218, 236),
        Color32::from_rgb(255, 139, 114),
        Color32::from_rgb(143, 229, 146),
        Color32::from_rgb(253, 213, 129),
    ];
    for grid in &mesh.grids {
        let stroke = Stroke::new(1., palette[grid.index % palette.len()]);
        let point = |row, column| {
            let z = grid.heights[row * n + column]?;
            Some([
                position(book.bounds[0], column, n),
                position(book.bounds[1], row, n),
                z,
            ])
        };
        let connect = |a, b| {
            if let (Some(a), Some(b)) = (a, b)
                && let Some([a, b]) = clip_depth(a, b, book.bounds[2])
            {
                painter.line_segment([project(a), project(b)], stroke);
            }
        };
        for row in 0..n {
            for column in 0..n {
                let start = point(row, column);
                if row + 1 < n {
                    connect(start, point(row + 1, column));
                }
                if column + 1 < n {
                    connect(start, point(row, column + 1));
                }
            }
        }
    }
    painter.text(
        pos2(area.left() + 5., area.top() + 5.),
        egui::Align2::LEFT_TOP,
        "X / Y / Z",
        egui::FontId::proportional(11.),
        Color32::LIGHT_GRAY,
    );
}

fn clip_depth(a: [f64; 3], b: [f64; 3], [lo, hi]: [f64; 2]) -> Option<[[f64; 3]; 2]> {
    if (a[2] < lo && b[2] < lo) || (a[2] > hi && b[2] > hi) {
        return None;
    }
    let trim = |start: [f64; 3], end: [f64; 3]| {
        let z = start[2].clamp(lo, hi);
        if z == start[2] {
            return start;
        }
        let t = (z - start[2]) / (end[2] - start[2]);
        [
            start[0] + t * (end[0] - start[0]),
            start[1] + t * (end[1] - start[1]),
            z,
        ]
    };
    Some([trim(a, b), trim(b, a)])
}
