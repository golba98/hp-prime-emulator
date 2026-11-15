//! Geometry's LCD forms and interactive construction state.
mod view;
use super::{MathTarget, NativeTools, Page, Request};
use crate::ide::{
    native::expression::EditCommand as E,
    native::keys::KeyAction as A,
    native_geometry::{self as geo, Book, Object, Point2, Scene, Shape},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursor {
    pub object: usize,
    pub measurement: usize,
    pub first_object: usize,
    pub first_measurement: usize,
    pub visibility: bool,
    pub pointer: [f64; 2],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub name: Option<String>,
    pub index: usize,
    pub source: String,
}
#[derive(Default)]
pub(super) struct Panel {
    pixels: Vec<u32>,
    texture: Option<egui::TextureHandle>,
    menu: Option<Menu>,
    tool: Option<Command>,
    args: Vec<String>,
    selected: Option<String>,
    hover: Option<String>,
    pub(super) refresh: bool,
    busy: bool,
    drag: Option<String>,
    angle_turn: f64,
    pub(super) memory_revision: Option<u64>,
    pub(super) scene_generation: Option<u64>,
    pub(super) live_context: Option<(u64, ppl_vm::SessionSnapshot)>,
}
#[derive(Clone, Copy)]
struct Menu {
    category: Option<usize>,
    index: usize,
    first: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    Point,
    Line,
    Circle,
    Shape,
    Curve,
}
#[derive(Debug, Clone, Copy)]
struct Command {
    label: &'static str,
    function: &'static str,
    needs: &'static [Need],
}
const POINTS: &[Command] = &[
    Command {
        label: "Point",
        function: "point",
        needs: &[],
    },
    Command {
        label: "Midpoint",
        function: "midpoint",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Center",
        function: "center",
        needs: &[Need::Circle],
    },
    Command {
        label: "Intersection",
        function: "single_inter",
        needs: &[Need::Curve, Need::Curve],
    },
];
const LINES: &[Command] = &[
    Command {
        label: "Segment",
        function: "segment",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Ray",
        function: "half_line",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Line",
        function: "line",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Parallel",
        function: "parallel",
        needs: &[Need::Point, Need::Line],
    },
    Command {
        label: "Perpendicular",
        function: "perpendicular",
        needs: &[Need::Point, Need::Line],
    },
    Command {
        label: "Perpendicular bisector",
        function: "perpen_bisector",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Median",
        function: "median_line",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Altitude",
        function: "altitude",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Angle bisector",
        function: "bisector",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
];
const POLYGONS: &[Command] = &[Command {
    label: "Triangle",
    function: "triangle",
    needs: &[Need::Point, Need::Point, Need::Point],
}];
const CURVES: &[Command] = &[
    Command {
        label: "Circle",
        function: "circle",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Circumcircle",
        function: "circumcircle",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Incircle",
        function: "incircle",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
];
const TRANSFORMS: &[Command] = &[
    Command {
        label: "Reflection",
        function: "reflection",
        needs: &[Need::Line, Need::Point],
    },
    Command {
        label: "Rotation",
        function: "rotation",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Dilation",
        function: "homothety",
        needs: &[Need::Point, Need::Point],
    },
];
const MEASURES: &[Command] = &[
    Command {
        label: "Distance",
        function: "distance",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Area",
        function: "area",
        needs: &[Need::Shape],
    },
    Command {
        label: "Perimeter",
        function: "perimeter",
        needs: &[Need::Shape],
    },
    Command {
        label: "Angle",
        function: "angle",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Slope",
        function: "slope",
        needs: &[Need::Line],
    },
    Command {
        label: "Abscissa",
        function: "abscissa",
        needs: &[Need::Point],
    },
    Command {
        label: "Ordinate",
        function: "ordinate",
        needs: &[Need::Point],
    },
    Command {
        label: "Radius",
        function: "radius",
        needs: &[Need::Circle],
    },
];
const CATEGORIES: &[&str] = &[
    "Zoom",
    "Point",
    "Line",
    "Polygon",
    "Curve",
    "Transform",
    "Measure",
];
fn commands(category: usize) -> &'static [Command] {
    match category {
        1 => POINTS,
        2 => LINES,
        3 => POLYGONS,
        4 => CURVES,
        5 => TRANSFORMS,
        6 => MEASURES,
        _ => &[],
    }
}
impl NativeTools {
    pub fn is_geometry_page(&self) -> bool {
        self.memory.selected_app == "Geometry"
            && matches!(self.page, Some(Page::Plot | Page::Symbolic | Page::Numeric))
    }
    pub fn geometry_apply_context(&mut self, context: &ppl_vm::SessionSnapshot, committed: u64) {
        if context.active_app != "Geometry" {
            return;
        }
        self.geometry_panel.memory_revision = Some(committed);
        self.geometry_panel.scene_generation = Some(context.generation);
        self.geometry_panel.live_context = Some((committed, context.clone()));
        self.memory.models.results.remove("Geometry");
        self.memory.models.results.remove("Geometry:plot");
        self.geometry_panel.refresh = true;
    }
    fn geometry_context(&mut self, snapshot: &ppl_vm::SessionSnapshot) -> ppl_vm::SessionSnapshot {
        if self.geometry_panel.memory_revision != Some(snapshot.generation) {
            self.geometry_panel.memory_revision = Some(snapshot.generation);
            self.geometry_panel.live_context = None;
            self.geometry_panel.scene_generation = Some(snapshot.generation);
            self.memory.models.results.remove("Geometry");
            self.memory.models.results.remove("Geometry:plot");
            self.geometry_panel.refresh = true;
        }
        self.geometry_panel
            .live_context
            .as_ref()
            .filter(|(committed, _)| *committed == snapshot.generation)
            .map_or_else(|| snapshot.clone(), |(_, context)| context.clone())
    }
    pub fn geometry_busy(&mut self, busy: bool) {
        self.geometry_panel.busy = busy;
    }
    pub fn geometry_changed(&mut self) {
        self.memory.geometry_draft = None;
        if self.geometry_panel.selected.as_ref().is_some_and(|n| {
            !self
                .memory
                .models
                .geometry_book
                .as_ref()
                .is_some_and(|b| b.objects.iter().any(|o| &o.name == n))
        }) {
            self.geometry_panel.selected = None;
        }
        if self.geometry_panel.args.iter().any(|n| {
            !self
                .memory
                .models
                .geometry_book
                .as_ref()
                .is_some_and(|b| b.objects.iter().any(|o| &o.name == n))
        }) {
            self.geometry_panel.args.clear();
        }

        self.memory.models.results.remove("Geometry");
        self.memory.models.results.remove("Geometry:plot");
        self.geometry_panel.refresh = true;
        self.error = None;
        self.dirty();
    }
    fn geometry_scene(&self) -> Option<Scene> {
        self.memory
            .models
            .results
            .get("Geometry")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .or_else(|| {
                self.memory
                    .models
                    .results
                    .get("Geometry:plot")
                    .and_then(|v| {
                        serde_json::from_value::<crate::ide::native_model_plots::PlotData>(
                            v.clone(),
                        )
                        .ok()
                    })
                    .and_then(|p| p.geometry.map(|v| *v))
            })
    }
    pub fn geometry_object_source(&self, name: &str, preset: &Option<String>) -> String {
        if let Some(preset) = preset {
            return preset.clone();
        }
        if let Some(d) = &self.memory.geometry_draft
            && d.name.as_deref() == Some(name)
        {
            return d.source.clone();
        }
        self.memory
            .models
            .geometry_book
            .as_ref()
            .and_then(|b| b.objects.iter().find(|o| o.name == name))
            .map(|o| o.source.clone())
            .unwrap_or_default()
    }
    pub fn geometry_measurement_source(&self, index: usize, preset: &Option<String>) -> String {
        if let Some(preset) = preset {
            return preset.clone();
        }
        if let Some(d) = &self.memory.geometry_draft
            && d.name.is_none()
            && d.index == index
        {
            return d.source.clone();
        }
        self.memory
            .models
            .geometry_book
            .as_ref()
            .and_then(|b| b.measurements.get(index))
            .map(|m| m.source.clone())
            .unwrap_or_default()
    }
    fn geometry_target(&self, preset: Option<String>) -> MathTarget {
        let book = self.memory.models.geometry_book.as_ref().unwrap();
        if self.page == Some(Page::Numeric) {
            MathTarget::GeometryMeasurement {
                index: self.memory.geometry_cursor.measurement,
                preset,
            }
        } else {
            let name = book
                .objects
                .get(self.memory.geometry_cursor.object)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| book.next_name());
            MathTarget::GeometryObject {
                name,
                preset,
                insert: None,
            }
        }
    }
    pub fn geometry_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if !self.is_geometry_page() {
            return None;
        }
        if self.geometry_panel.busy || self.geometry_panel.menu.is_some() {
            return Some(vec![]);
        }
        if self.page == Some(Page::Plot) {
            return Some(vec![]);
        }
        if let Err(error) = self.memory.models.ensure_geometry_book() {
            self.error = Some(error);
            return Some(vec![]);
        }
        Some(vec![
            Request::EditMath(self.geometry_target(Some(String::new()))),
            Request::TypeText(text.into()),
        ])
    }
    fn geometry_categories(&self) -> Vec<usize> {
        match self.page {
            Some(Page::Symbolic) => (1..=5).collect(),
            Some(Page::Numeric) => (1..=6).collect(),
            _ => (0..=6).collect(),
        }
    }
    fn geometry_menu_items(&self, category: Option<usize>) -> Vec<String> {
        match category {
            None => self
                .geometry_categories()
                .into_iter()
                .map(|c| CATEGORIES[c].into())
                .collect(),
            Some(7) => [
                "Hide / Show",
                "Hide / Show Label",
                "Black",
                "Blue",
                "Red",
                "Green",
                "Magenta",
                "Delete",
            ]
            .map(str::to_owned)
            .to_vec(),
            Some(0) => ["In", "Out", "Auto Scale", "Decimal"]
                .map(str::to_owned)
                .to_vec(),
            Some(c) => commands(c).iter().map(|c| c.label.into()).collect(),
        }
    }
    fn geometry_choose_menu(&mut self, menu: Menu) -> Vec<Request> {
        if menu.category.is_none() {
            self.geometry_panel.menu = Some(Menu {
                category: self.geometry_categories().get(menu.index).copied(),
                index: 0,
                first: 0,
            });
            return vec![];
        }
        let category = menu.category.unwrap();
        self.geometry_panel.menu = None;
        if category == 7 {
            let book = self.memory.models.geometry_book.as_mut().unwrap();
            if let Some(index) = book
                .objects
                .iter()
                .position(|o| Some(o.name.as_str()) == self.geometry_panel.selected.as_deref())
            {
                match menu.index {
                    0 => book.objects[index].visible = !book.objects[index].visible,
                    1 => book.objects[index].label = !book.objects[index].label,
                    2..=6 => {
                        book.objects[index].color =
                            [0, 0x0000ff, 0xff0000, 0x008000, 0xff00ff][menu.index - 2]
                    }
                    7 => {
                        book.objects.remove(index);
                        self.geometry_panel.selected = None;
                    }
                    _ => {}
                }
                self.geometry_changed();
            }
            return vec![];
        }
        if category == 0 {
            let mut w = self.trace_window();
            match menu.index {
                0 => w.zoom(0.5),
                1 => w.zoom(2.),
                2 => {
                    self.geometry_panel.refresh = true;
                    self.auto_scale_pending = true;
                    self.dirty();
                    return vec![];
                }
                _ => w = geo::default_window(),
            }
            self.memory.model_windows.insert("Geometry".into(), w);
            self.geometry_panel.refresh = true;
            self.dirty();
            return vec![];
        }
        let Some(command) = commands(category).get(menu.index).copied() else {
            return vec![];
        };
        if self.page == Some(Page::Plot) && category != 6 {
            self.geometry_panel.tool = Some(command);
            self.geometry_panel.args.clear();
        } else {
            return vec![Request::EditMath(
                self.geometry_target(Some(format!("{}()", command.function))),
            )];
        }
        self.dirty();
        vec![]
    }
    fn geometry_confirm(&mut self) -> Vec<Request> {
        let p = Point2 {
            x: self.memory.geometry_cursor.pointer[0],
            y: self.memory.geometry_cursor.pointer[1],
        };
        if let Some(command) = self.geometry_panel.tool {
            if command.function == "point" {
                self.geometry_add_point(p);
                self.geometry_panel.args.clear();
                return vec![];
            }
            let index = self.geometry_panel.args.len();
            let Some(need) = command.needs.get(index) else {
                return vec![];
            };
            let scene = self.geometry_scene();
            let hovered = self.geometry_panel.hover.clone().and_then(|name| {
                scene
                    .as_ref()?
                    .objects
                    .iter()
                    .find(|o| o.name == name)
                    .map(|o| (name.clone(), o.value.clone()))
            });
            let name = if let Some((name, Ok(shape))) = hovered {
                let matches = match need {
                    Need::Point => matches!(shape, Shape::Point(_)),
                    Need::Line => matches!(shape, Shape::Line { .. }),
                    Need::Circle => matches!(shape, Shape::Circle { .. }),
                    Need::Curve => matches!(shape, Shape::Line { .. } | Shape::Circle { .. }),
                    Need::Shape => matches!(
                        shape,
                        Shape::Line { .. } | Shape::Circle { .. } | Shape::Polygon(_)
                    ),
                };
                if !matches {
                    self.error = Some(format!("Select a {need:?}"));
                    return vec![];
                }
                name
            } else if *need == Need::Point {
                let Some(name) = self.geometry_add_point(p) else {
                    return vec![];
                };
                name
            } else {
                self.error = Some(format!("Select a {need:?}"));
                return vec![];
            };
            self.geometry_panel.args.push(name);
            if self.geometry_panel.args.len() == command.needs.len() {
                let args = self
                    .geometry_panel
                    .args
                    .iter()
                    .map(|n| format!("G{n}"))
                    .collect::<Vec<_>>();
                let source = if command.function == "single_inter" {
                    format!(
                        "single_inter({},{},point({},{}))",
                        args[0],
                        args[1],
                        geo::canonical(p.x),
                        geo::canonical(p.y)
                    )
                } else if command.function == "circle" {
                    format!("circle({},{}-{})", args[0], args[1], args[0])
                } else if command.function == "rotation" {
                    format!(
                        "rotation({},{},{})",
                        args[0],
                        match self.geometry_panel.angle_turn {
                            360. => "90",
                            400. => "100",
                            _ => "π/2",
                        },
                        args[1]
                    )
                } else if command.function == "homothety" {
                    format!("homothety({},2,{})", args[0], args[1])
                } else {
                    format!("{}({})", command.function, args.join(","))
                };
                self.geometry_panel.args.clear();
                let name = self
                    .memory
                    .models
                    .geometry_book
                    .as_ref()
                    .unwrap()
                    .next_name();
                if matches!(command.function, "rotation" | "homothety") {
                    return vec![Request::EditMath(MathTarget::GeometryObject {
                        name,
                        preset: Some(source),
                        insert: None,
                    })];
                }
                let book = self.memory.models.geometry_book.as_ref().unwrap().clone();
                return vec![Request::Calculate(
                    crate::ide::native_app_ui::Calculation::GeometryEdit {
                        book: Box::new(book),
                        name,
                        source,
                        insert: None,
                    },
                )];
            }
        } else {
            self.geometry_panel.selected = self.geometry_panel.hover.clone();
        }
        self.dirty();
        vec![]
    }
    fn geometry_add_point(&mut self, p: Point2) -> Option<String> {
        let book = self.memory.models.geometry_book.as_mut()?;
        if book.objects.len() >= geo::MAX_OBJECTS || geo::point(p).is_err() {
            self.error = Some("Geometry point exceeds its limit".into());
            return None;
        }
        let name = book.next_name();
        book.objects.push(Object {
            name: name.clone(),
            source: format!("point({},{})", geo::canonical(p.x), geo::canonical(p.y)),
            ..Default::default()
        });
        self.geometry_changed();
        Some(name)
    }
    pub fn geometry_action(&mut self, action: A) -> Option<Vec<Request>> {
        if !self.is_geometry_page() {
            return None;
        }
        if let Err(error) = self.memory.models.ensure_geometry_book() {
            self.error = Some(error);
            return Some(vec![]);
        }
        if self.geometry_panel.busy {
            return Some(vec![]);
        }
        if action == A::Tool(crate::ide::native::keys::Tool::Menu) {
            self.geometry_panel.menu = Some(Menu {
                category: if self.geometry_panel.selected.is_some() {
                    Some(7)
                } else {
                    None
                },
                index: 0,
                first: 0,
            });
            self.dirty();
            return Some(vec![]);
        }
        if matches!(action, A::Home | A::Tool(_)) {
            return None;
        }
        if let Some(mut menu) = self.geometry_panel.menu {
            match action {
                A::History(d) => {
                    let count = self.geometry_menu_items(menu.category).len();
                    menu.index =
                        (menu.index as i64 + i64::from(d)).rem_euclid(count as i64) as usize;
                    menu.first = menu.first.min(menu.index);
                    if menu.index >= menu.first + 8 {
                        menu.first = menu.index - 7;
                    }
                    self.geometry_panel.menu = Some(menu);
                }
                A::Evaluate => return Some(self.geometry_choose_menu(menu)),
                A::Back | A::On => {
                    if menu.category.is_some() {
                        self.geometry_panel.menu = Some(Menu {
                            category: None,
                            index: 0,
                            first: 0,
                        });
                    } else {
                        self.geometry_panel.menu = None;
                    }
                }
                _ => {}
            }
            self.dirty();
            return Some(vec![]);
        }
        if self.page == Some(Page::Plot) {
            let w = self.trace_window();
            match action {
                A::Evaluate => return Some(self.geometry_confirm()),
                A::History(d) => {
                    self.memory.geometry_cursor.pointer[1] -=
                        f64::from(d) * (w.ymax - w.ymin) / 218.
                }
                A::Edit(E::Left(_)) => {
                    self.memory.geometry_cursor.pointer[0] -= (w.xmax - w.xmin) / 318.
                }
                A::Edit(E::Right(_)) => {
                    self.memory.geometry_cursor.pointer[0] += (w.xmax - w.xmin) / 318.
                }
                A::Insert("+" | "-") => {
                    let mut w = w;
                    w.zoom(if action == A::Insert("+") { 0.5 } else { 2. });
                    self.memory.model_windows.insert("Geometry".into(), w);
                    self.geometry_panel.refresh = true;
                }
                A::Back | A::On => {
                    self.geometry_panel.tool = None;
                    self.geometry_panel.args.clear();
                    self.geometry_panel.selected = None;
                }
                _ => {}
            }
            self.dirty();
            return Some(vec![]);
        }
        let numeric = self.page == Some(Page::Numeric);
        let cursor = &mut self.memory.geometry_cursor;
        let book = self.memory.models.geometry_book.as_mut().unwrap();
        let (index, first, len) = if numeric {
            (
                &mut cursor.measurement,
                &mut cursor.first_measurement,
                book.measurements.len(),
            )
        } else {
            (
                &mut cursor.object,
                &mut cursor.first_object,
                book.objects.len(),
            )
        };
        *index = (*index).min(len);
        match action {
            A::History(d) => *index = (*index as i64 + i64::from(d)).clamp(0, len as i64) as usize,
            A::Edit(E::Left(_)) => cursor.visibility = true,
            A::Edit(E::Right(_)) => cursor.visibility = false,
            A::Evaluate if cursor.visibility => {
                if numeric {
                    if let Some(m) = book.measurements.get_mut(*index) {
                        m.visible = !m.visible;
                    }
                } else if let Some(o) = book.objects.get_mut(*index) {
                    o.visible = !o.visible;
                }
                self.geometry_changed();
                return Some(vec![]);
            }
            A::Evaluate => return Some(vec![Request::EditMath(self.geometry_target(None))]),
            A::Insert(text) => return self.geometry_text(text),
            A::Template(t) => {
                return Some(vec![
                    Request::EditMath(self.geometry_target(Some(String::new()))),
                    Request::InsertTemplate(t),
                ]);
            }
            A::Copy => {
                return Some(vec![Request::CopyText(if numeric {
                    book.measurements
                        .get(*index)
                        .map(|m| m.source.clone())
                        .unwrap_or_default()
                } else {
                    book.objects
                        .get(*index)
                        .map(|o| o.source.clone())
                        .unwrap_or_default()
                })]);
            }
            A::Edit(E::Backspace | E::Delete) => {
                if *index < len {
                    if numeric {
                        book.measurements.remove(*index);
                    } else {
                        book.objects.remove(*index);
                    }
                    self.geometry_changed();
                    return Some(vec![]);
                }
            }
            A::Back | A::On => return Some(vec![Request::Close]),
            _ => {}
        }
        if *index < *first {
            *first = *index;
        }
        if *index >= *first + 6 {
            *first = *index - 5;
        }
        self.dirty();
        Some(vec![])
    }
}
