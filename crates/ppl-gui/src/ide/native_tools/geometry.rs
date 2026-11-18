//! Geometry's LCD forms and interactive construction state.
mod view;
use super::{MathTarget, NativeTools, Page, Request};
use crate::ide::{
    native::expression::EditCommand as E,
    native::keys::KeyAction as A,
    native_geometry::{self as geo, Book, Object, Point2, Scene, Shape},
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const TRACE_MENU: usize = 8;
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
    plot_minified: bool,
    texture: Option<egui::TextureHandle>,
    menu: Option<Menu>,
    tool: Option<Command>,
    command_text: CommandText,
    args: Vec<String>,
    selected: Option<String>,
    hover: Option<String>,
    pub(super) refresh: bool,
    busy: bool,
    drag: Option<String>,
    drag_trace: VecDeque<Point2>,
    angle_turn: f64,
    pub(super) memory_revision: Option<u64>,
    pub(super) scene_generation: Option<u64>,
    pub(super) live_context: Option<(u64, ppl_vm::SessionSnapshot)>,
    settings: Option<ppl_vm::GeometryPlotSettings>,
    setup_index: usize,
}
#[derive(Default)]
struct CommandText {
    source: Option<String>,
    automatic: bool,
    started: f64,
    offset: f32,
}
impl CommandText {
    fn update(&mut self, source: Option<String>, automatic: bool, now: f64) {
        if self.source != source || self.automatic != automatic {
            *self = Self {
                source,
                automatic,
                started: now,
                offset: 0.,
            };
        }
    }
}
#[derive(Clone, Copy)]
struct Menu {
    category: Option<usize>,
    index: usize,
    first: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    Transformable,
    Mirror,
    Point,
    PointOnCurve,
    Line,
    Circle,
    Shape,
    Curve,
}
impl Need {
    fn accepts(self, command: Command, shape: &Shape) -> bool {
        match self {
            Self::Mirror => matches!(shape, Shape::Point(_) | Shape::Line { .. }),
            Self::Transformable => !matches!(
                shape,
                Shape::Undefined
                    | Shape::EmptyConic(_)
                    | Shape::Function { .. }
                    | Shape::Number { .. }
            ),
            Self::Point | Self::PointOnCurve => matches!(shape, Shape::Point(_)),
            Self::Line => matches!(shape, Shape::Line { .. }),
            Self::Circle => matches!(shape, Shape::Circle { .. } | Shape::Arc(_)),
            Self::Curve => {
                matches!(
                    shape,
                    Shape::Line { .. }
                        | Shape::Circle { .. }
                        | Shape::Arc(_)
                        | Shape::Ellipse(_)
                        | Shape::Parabola(_)
                        | Shape::Hyperbola(_)
                ) || command.function == "element" && matches!(shape, Shape::Function { .. })
            }
            Self::Shape => matches!(
                shape,
                Shape::Line { .. }
                    | Shape::Circle { .. }
                    | Shape::Arc(_)
                    | Shape::Ellipse(_)
                    | Shape::Parabola(_)
                    | Shape::Hyperbola(_)
                    | Shape::Polygon(_)
            ),
        }
    }
}
#[derive(Debug, Clone, Copy)]
struct Command {
    label: &'static str,
    function: &'static str,
    needs: &'static [Need],
}
impl Command {
    fn operand(self, index: usize) -> Option<&'static Need> {
        if self.function == "polygon" {
            Some(&Need::Point)
        } else {
            self.needs.get(index)
        }
    }
    fn pick_filter(self) -> Option<Need> {
        match self.function {
            "element" => Some(Need::Curve),
            "tangent" => Some(Need::PointOnCurve),
            "polygon" => Some(Need::Point),
            _ => None,
        }
    }
}
const POINTS: &[Command] = &[
    Command {
        label: "Point",
        function: "point",
        needs: &[],
    },
    Command {
        label: "Point On",
        function: "element",
        needs: &[Need::Curve],
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
        label: "Tangent",
        function: "tangent",
        needs: &[Need::PointOnCurve],
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
const POLYGONS: &[Command] = &[
    Command {
        label: "Triangle",
        function: "triangle",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Equilateral Triangle",
        function: "equilateral_triangle",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Isosceles Triangle",
        function: "isosceles_triangle",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Right Triangle",
        function: "right_triangle",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Quadrilateral",
        function: "quadrilateral",
        needs: &[Need::Point, Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Parallelogram",
        function: "parallelogram",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Rhombus",
        function: "rhombus",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Rectangle",
        function: "rectangle",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Polygon",
        function: "polygon",
        needs: &[],
    },
    Command {
        label: "Regular Polygon",
        function: "isopolygon",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Square",
        function: "square",
        needs: &[Need::Point, Need::Point],
    },
];
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
    Command {
        label: "Function Graph",
        function: "plotfunc",
        needs: &[],
    },
    Command {
        label: "Ellipse",
        function: "ellipse",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Excircle",
        function: "excircle",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Parabola",
        function: "parabola",
        needs: &[Need::Point, Need::Line],
    },
    Command {
        label: "Hyperbola",
        function: "hyperbola",
        needs: &[Need::Point, Need::Point, Need::Point],
    },
    Command {
        label: "Conic",
        function: "conic",
        needs: &[],
    },
];
const TRANSFORMS: &[Command] = &[
    Command {
        label: "Reflection",
        function: "reflection",
        needs: &[Need::Mirror, Need::Transformable],
    },
    Command {
        label: "Rotation",
        function: "rotation",
        needs: &[Need::Point, Need::Transformable],
    },
    Command {
        label: "Dilation",
        function: "homothety",
        needs: &[Need::Point, Need::Transformable],
    },
    Command {
        label: "Translation",
        function: "translation",
        needs: &[Need::Transformable, Need::Point, Need::Point],
    },
    Command {
        label: "Similarity",
        function: "similarity",
        needs: &[Need::Point, Need::Transformable],
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
const SETUP_FIELDS: [&str; 10] = [
    "Xmin",
    "Ymin",
    "PixSize",
    "Xtick",
    "Ytick",
    "Axes",
    "Labels",
    "GridDots",
    "GridLines",
    "ScrollText",
];
fn point_source(p: Point2) -> String {
    format!("point({},{})", geo::canonical(p.x), geo::canonical(p.y))
}
impl NativeTools {
    pub fn geometry_settings_for_legacy_window(
        window: &crate::ide::native_apps::PlotWindow,
    ) -> Result<ppl_vm::GeometryPlotSettings, String> {
        window.validate()?;
        let pixel = ((window.xmax - window.xmin) / 318.).max((window.ymax - window.ymin) / 218.);
        if !pixel.is_finite() || pixel <= 0.001 || pixel > 10000. {
            return Err(
                "Saved Geometry window has an unsupported pixel scale; original window retained"
                    .into(),
            );
        }
        let cx = window.xmin / 2. + window.xmax / 2.;
        let cy = window.ymin / 2. + window.ymax / 2.;
        let mut settings = ppl_vm::GeometryPlotSettings {
            axes: !window.axes,
            ..Default::default()
        };
        for (name, value) in [
            ("Xmin", cx - 160. * pixel),
            ("Ymin", cy - 110. * pixel),
            ("PixSize", pixel),
        ] {
            let value = ppl_math::Real::from_f64(value).map_err(|e| e.to_string())?;
            // Migration must preserve the old camera, rather than silently
            // clamping its minima to a different location.
            if name != "PixSize" && value.abs() > ppl_math::Real::from_i64(10000) {
                return Err("Saved Geometry window is outside the scoped camera range; original window retained".into());
            }
            settings
                .set(name, &ppl_vm::Value::Real(value))
                .map_err(|e| e.to_string())?;
        }
        Ok(settings)
    }
    pub fn geometry_settings_changed(&mut self, settings: &ppl_vm::GeometryPlotSettings) {
        if self.geometry_panel.settings.as_ref() == Some(settings) {
            return;
        }
        let bounds = settings.derived_bounds();
        let axes = !settings.axes;
        self.memory.model_windows.insert(
            "Geometry".into(),
            crate::ide::native_apps::PlotWindow {
                xmin: bounds[0].to_f64(),
                xmax: bounds[1].to_f64(),
                ymin: bounds[2].to_f64(),
                ymax: bounds[3].to_f64(),
                axes,
                ..Default::default()
            },
        );
        self.geometry_panel.settings = Some(settings.clone());
        self.geometry_panel.refresh = true;
        if self
            .model_job
            .as_ref()
            .is_some_and(|job| job.name == "Geometry")
        {
            self.model_job = None;
        }
        self.memory.models.results.remove("Geometry:plot");
        self.dirty();
    }
    pub fn geometry_plot_configuration(&self) -> Option<ppl_vm::GeometryPlotSettings> {
        self.geometry_panel.settings.clone()
    }
    pub fn geometry_plot_field_source(&self, name: &str) -> String {
        self.geometry_panel
            .settings
            .clone()
            .unwrap_or_default()
            .get(name)
            .map(|v| v.string_form(ppl_vm::value::default_format()))
            .unwrap_or_default()
    }
    fn geometry_setup_activate(&mut self) -> Vec<Request> {
        let index = self.geometry_panel.setup_index;
        let name = SETUP_FIELDS[index];
        if index < 5 {
            return vec![Request::EditMath(MathTarget::GeometryPlotField(
                name.into(),
            ))];
        }
        let mut settings = self.geometry_panel.settings.clone().unwrap_or_default();
        let value = settings.get(name).unwrap().expect_real(name).unwrap();
        if let Err(error) = settings.set(name, &ppl_vm::Value::from_bool(value.is_zero())) {
            self.error = Some(error.to_string());
            return vec![];
        }
        self.geometry_queue_configuration(settings);
        vec![]
    }
    fn geometry_queue_configuration(&mut self, settings: ppl_vm::GeometryPlotSettings) {
        self.geometry_settings_changed(&settings);
        self.pending_actions
            .retain(|request| !matches!(request, Request::GeometryPlotConfiguration(_)));
        self.pending_actions
            .push(Request::GeometryPlotConfiguration(settings));
    }
    fn geometry_zoom(&mut self, factor: f64) {
        let settings = self.geometry_panel.settings.clone().unwrap_or_default();
        let result = (|| {
            let cursor = self
                .memory
                .geometry_cursor
                .pointer
                .map(ppl_math::Real::from_f64);
            let cursor = [
                cursor[0].map_err(|e| e.to_string())?,
                cursor[1].map_err(|e| e.to_string())?,
            ];
            settings
                .zoom(
                    cursor,
                    ppl_math::Real::from_f64(factor).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(settings) => self.geometry_queue_configuration(settings),
            Err(error) => self.error = Some(error),
        }
    }
    pub fn is_geometry_page(&self) -> bool {
        self.memory.selected_app == "Geometry"
            && matches!(
                self.page,
                Some(Page::Plot | Page::PlotSetup | Page::Symbolic | Page::Numeric)
            )
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
    pub(super) fn geometry_sync_context(&mut self, snapshot: &ppl_vm::SessionSnapshot) {
        if self.geometry_panel.memory_revision != Some(snapshot.generation) {
            self.geometry_panel.memory_revision = Some(snapshot.generation);
            self.geometry_panel.live_context = None;
            self.geometry_panel.scene_generation = Some(snapshot.generation);
            self.memory.models.results.remove("Geometry");
            self.memory.models.results.remove("Geometry:plot");
            self.geometry_panel.refresh = true;
        }
    }
    fn geometry_context(&mut self, snapshot: &ppl_vm::SessionSnapshot) -> ppl_vm::SessionSnapshot {
        self.geometry_sync_context(snapshot);
        self.geometry_panel
            .live_context
            .as_ref()
            .filter(|(committed, _)| *committed == snapshot.generation)
            .map_or_else(|| snapshot.clone(), |(_, context)| context.clone())
    }
    pub fn geometry_busy(&mut self, busy: bool) {
        if busy
            && !self.geometry_panel.busy
            && self
                .model_job
                .as_ref()
                .is_some_and(|job| job.name == "Geometry")
        {
            // A view refresh started before PPL owns the session must not
            // replace current values between two app-access requests. Explicit
            // STARTAPP/STARTVIEW redraws start after this transition and are
            // awaited by their caller.
            self.model_job = None;
            self.geometry_panel.refresh = true;
        }
        self.geometry_panel.busy = busy;
    }
    pub fn reset_geometry(&mut self, settings: &ppl_vm::GeometryPlotSettings) {
        self.memory.models.geometry = Default::default();
        self.memory.models.geometry_book = Some(Book::default());
        self.memory.geometry_cursor = Default::default();
        self.memory.geometry_legacy_window = None;
        self.geometry_panel = Default::default();
        self.pending_actions
            .retain(|request| !matches!(request, Request::GeometryPlotConfiguration(_)));
        self.geometry_settings_changed(settings);
        self.geometry_changed();
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
        if self.is_geometry_page() && self.page == Some(Page::PlotSetup) {
            if self.geometry_panel.busy {
                return Some(vec![]);
            }
            if self.geometry_panel.setup_index < 5 {
                return Some(vec![
                    Request::EditMath(MathTarget::GeometryPlotField(
                        SETUP_FIELDS[self.geometry_panel.setup_index].into(),
                    )),
                    Request::TypeText(text.into()),
                ]);
            }
            return Some(vec![]);
        }
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
    fn trace_selected(&self, scene: Option<&Scene>) -> bool {
        self.geometry_panel
            .selected
            .as_deref()
            .is_some_and(|name| scene.is_some_and(|scene| scene.is_point(name)))
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
            Some(TRACE_MENU) => {
                let object = self.memory.models.geometry_book.as_ref().and_then(|book| {
                    book.objects.iter().find(|object| {
                        self.geometry_panel.selected.as_deref() == Some(&object.name)
                    })
                });
                let mut items = vec!["Start Trace".into()];
                if let Some(object) = object.filter(|object| object.traced) {
                    items[0] = "Stop Trace".into();
                    items.push(
                        if object.trace_connected {
                            "Show Trace Dots"
                        } else {
                            "Connect Trace"
                        }
                        .into(),
                    );
                }
                items.push("Clear Trace".into());
                items
            }
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
        if category == TRACE_MENU {
            if let Some(name) = self.geometry_panel.selected.as_deref()
                && let Some(object) = self
                    .memory
                    .models
                    .geometry_book
                    .as_mut()
                    .and_then(|book| book.objects.iter_mut().find(|object| object.name == name))
            {
                match menu.index {
                    0 => object.traced = !object.traced,
                    1 if object.traced => object.trace_connected = !object.trace_connected,
                    1 | 2 => object.trace_samples.clear(),
                    _ => {}
                }
                self.geometry_changed();
            }
            return vec![];
        }
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
            match menu.index {
                0 => self.geometry_zoom(0.5),
                1 => self.geometry_zoom(2.),
                2 => {
                    self.geometry_panel.refresh = true;
                    self.auto_scale_pending = true;
                    self.dirty();
                    return vec![];
                }
                _ => self.geometry_queue_configuration(ppl_vm::GeometryPlotSettings::default()),
            }
            self.geometry_panel.refresh = true;
            self.dirty();
            return vec![];
        }
        let Some(command) = commands(category).get(menu.index).copied() else {
            return vec![];
        };
        if self.page == Some(Page::Plot) && matches!(command.function, "plotfunc" | "conic") {
            self.geometry_panel.tool = None;
            self.geometry_panel.args.clear();
            let name = self
                .memory
                .models
                .geometry_book
                .as_ref()
                .unwrap()
                .next_name();
            self.dirty();
            return vec![Request::EditMath(MathTarget::GeometryObject {
                name,
                preset: Some(format!("{}()", command.function)),
                insert: None,
            })];
        }
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
    fn construction_pointer(&self) -> Point2 {
        Point2 {
            x: self.memory.geometry_cursor.pointer[0],
            y: self.memory.geometry_cursor.pointer[1],
        }
    }
    fn geometry_hovered_operand<'a>(
        &self,
        command: Command,
        p: Point2,
        scene: Option<&'a Scene>,
    ) -> Option<(String, &'a Shape)> {
        let scene = scene?;
        let hit = if let Some(filter) = command.pick_filter() {
            let w = self.trace_window();
            view::hit(
                scene,
                self.memory.models.geometry_book.as_ref()?,
                view::project(p, &w),
                &w,
                Some(filter),
            )
        } else {
            self.geometry_panel.hover.clone()
        }?;
        let object = scene.objects.iter().find(|object| object.name == hit)?;
        Some((hit, object.value.as_ref().ok()?))
    }
    fn geometry_command_source(
        &self,
        command: Command,
        args: &[String],
        p: Point2,
        scene: Option<&Scene>,
    ) -> Result<String, String> {
        Ok(match (command.function, args) {
            ("point", _) => point_source(p),
            ("element", [curve]) => {
                return scene.ok_or("Select a curve")?.point_on_source(curve, p);
            }
            ("single_inter", [a, b]) => format!("single_inter({a},{b},{})", point_source(p)),
            ("circle", [center, edge]) => format!("circle({center},{edge}-{center})"),
            ("rotation", [shape, center]) => {
                let angle = match self.geometry_panel.angle_turn {
                    360. => "90",
                    400. => "100",
                    _ => "π/2",
                };
                format!("rotation({shape},{angle},{center})")
            }
            ("isosceles_triangle" | "rhombus", [a, b]) => {
                let angle = match self.geometry_panel.angle_turn {
                    360. => "60",
                    400. => "200/3",
                    _ => "π/3",
                };
                format!("{}({a},{b},{angle})", command.function)
            }
            ("right_triangle", [a, b]) => format!("right_triangle({a},{b},1)"),
            ("isopolygon", [a, b]) => format!("isopolygon({a},{b},6)"),
            ("translation", [object, a, b]) => format!("translation({b}-{a},{object})"),
            ("similarity", [center, object]) => format!("similarity({center},2,0,{object})"),
            ("homothety", [shape, center]) => format!("homothety({shape},2,{center})"),
            (name, _) => format!("{name}({})", args.join(",")),
        })
    }
    fn geometry_command_preview(&self, scene: Option<&Scene>) -> Option<String> {
        let command = self.geometry_panel.tool?;
        let p = self.construction_pointer();
        let mut args: Vec<_> = self
            .geometry_panel
            .args
            .iter()
            .map(|n| format!("G{n}"))
            .collect();
        if let Some(need) = command.operand(args.len()) {
            if let Some((name, shape)) = self.geometry_hovered_operand(command, p, scene)
                && need.accepts(command, shape)
            {
                args.push(format!("G{name}"));
            } else if *need == Need::Point {
                args.push(point_source(p));
            }
        }
        self.geometry_command_source(command, &args, p, scene).ok()
    }
    fn geometry_confirm(&mut self) -> Vec<Request> {
        let p = self.construction_pointer();
        if let Some(command) = self.geometry_panel.tool {
            if command.function == "point" {
                self.geometry_add_point(p);
                self.geometry_panel.args.clear();
                return vec![];
            }
            let index = self.geometry_panel.args.len();
            let Some(need) = command.operand(index) else {
                return vec![];
            };
            let scene = self.geometry_scene();
            let hovered = self.geometry_hovered_operand(command, p, scene.as_ref());
            if *need == Need::PointOnCurve && hovered.is_none() {
                self.error = Some("Select a point on a curve".into());
                self.dirty();
                return vec![];
            }
            let name = if let Some((name, shape)) = hovered {
                if !need.accepts(command, shape) {
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
            let finish = if command.function == "polygon" {
                let vertices = &mut self.geometry_panel.args;
                if vertices.first() == Some(&name) {
                    if vertices.len() < 3 {
                        self.error = Some(
                            "Select at least three vertices before closing the polygon".into(),
                        );
                        self.dirty();
                        return vec![];
                    }
                    true
                } else {
                    if vertices.last() != Some(&name) {
                        vertices.push(name);
                    }
                    false
                }
            } else {
                self.geometry_panel.args.push(name);
                self.geometry_panel.args.len() == command.needs.len()
            };
            if finish {
                let args = self
                    .geometry_panel
                    .args
                    .iter()
                    .map(|n| format!("G{n}"))
                    .collect::<Vec<_>>();
                let source = self.geometry_command_source(command, &args, p, scene.as_ref());
                self.geometry_panel.args.clear();
                let source = match source {
                    Ok(source) => source,
                    Err(error) => {
                        self.error = Some(error);
                        self.dirty();
                        return vec![];
                    }
                };
                let name = self
                    .memory
                    .models
                    .geometry_book
                    .as_ref()
                    .unwrap()
                    .next_name();
                if matches!(
                    command.function,
                    "rotation"
                        | "similarity"
                        | "homothety"
                        | "isosceles_triangle"
                        | "rhombus"
                        | "right_triangle"
                        | "isopolygon"
                ) {
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
            source: point_source(p),
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
        if self.page == Some(Page::PlotSetup) {
            match action {
                A::History(d) => {
                    self.geometry_panel.setup_index =
                        (self.geometry_panel.setup_index as i64 + i64::from(d))
                            .rem_euclid(SETUP_FIELDS.len() as i64) as usize
                }
                A::Edit(E::Left(_)) => {
                    self.geometry_panel.setup_index =
                        (self.geometry_panel.setup_index + SETUP_FIELDS.len() - 1)
                            % SETUP_FIELDS.len()
                }
                A::Edit(E::Right(_)) => {
                    self.geometry_panel.setup_index =
                        (self.geometry_panel.setup_index + 1) % SETUP_FIELDS.len()
                }
                A::Evaluate => return Some(self.geometry_setup_activate()),
                A::Back | A::On => self.close(),
                _ => {}
            }
            self.dirty();
            return Some(vec![]);
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
            let old_pointer = self.memory.geometry_cursor.pointer;
            match action {
                A::Evaluate => return Some(self.geometry_confirm()),
                A::History(d) => {
                    self.memory.geometry_cursor.pointer[1] -=
                        f64::from(d) * (w.ymax - w.ymin) / 219.
                }
                A::Edit(E::Left(_)) => {
                    self.memory.geometry_cursor.pointer[0] -= (w.xmax - w.xmin) / 319.
                }
                A::Edit(E::Right(_)) => {
                    self.memory.geometry_cursor.pointer[0] += (w.xmax - w.xmin) / 319.
                }
                A::Insert("+" | "-") => {
                    self.geometry_zoom(if action == A::Insert("+") { 0.5 } else { 2. });
                }
                A::Back | A::On => {
                    self.geometry_panel.tool = None;
                    self.geometry_panel.args.clear();
                    self.geometry_panel.selected = None;
                }
                _ => {}
            }
            if self.memory.geometry_cursor.pointer != old_pointer
                && let Some(command) = self.geometry_panel.tool
            {
                let p = view::project(self.construction_pointer(), &w);
                self.geometry_panel.hover = self.geometry_scene().and_then(|scene| {
                    view::hit(
                        &scene,
                        self.memory.models.geometry_book.as_ref()?,
                        p,
                        &w,
                        command.pick_filter(),
                    )
                });
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
