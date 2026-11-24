//! Geometry's LCD forms and interactive construction state.
mod slider;
mod view;
mod wizard;
use super::{MathTarget, NativeTools, Page, Request};
use crate::ide::{
    native::expression::EditCommand as E,
    native::keys::KeyAction as A,
    native_geometry::{self as geo, Book, Object, Point2, Scene, Shape},
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const TRACE_MENU: usize = 8;
const ARC_PICKER: usize = 9;
const TANGENT_PICKER: usize = 10;

fn is_object_picker(category: Option<usize>) -> bool {
    matches!(category, Some(ARC_PICKER | TANGENT_PICKER))
}
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
    picker_candidates: Vec<String>,
    tangent_curve: Option<String>,
    wizard: Option<wizard::Slopefield>,
    sliders: slider::Controls,
    field_solution: Option<(String, String, String)>,
    tool: Option<Command>,
    command_text: CommandText,
    args: Vec<String>,
    polygon_last_pointer: Option<Point2>,
    selected: Option<String>,
    hover: Option<String>,
    pub(super) refresh: bool,
    busy: bool,
    drag: Option<String>,
    drag_trace: VecDeque<Point2>,
    transform_parameter: Option<String>,
    pub(super) memory_revision: Option<u64>,
    pub(super) scene_generation: Option<u64>,
    pub(super) live_context: Option<(u64, ppl_vm::SessionSnapshot)>,
    angle_turn: f64,
    settings: Option<ppl_vm::GeometryPlotSettings>,
    setup_index: usize,
}
impl Panel {
    fn dismiss_object_picker(&mut self) {
        if self
            .menu
            .is_some_and(|menu| is_object_picker(menu.category))
        {
            self.menu = None;
        }
        self.picker_candidates.clear();
    }
    fn clear_operands(&mut self) {
        self.dismiss_object_picker();
        self.args.clear();
        self.tangent_curve = None;
        self.polygon_last_pointer = None;
        self.transform_parameter = None;
    }
}
#[derive(Debug, Clone, Copy)]
pub enum TransformParameter {
    Angle,
    ScaleFactor,
}
impl TransformParameter {
    pub fn prompt(self) -> &'static str {
        match self {
            Self::Angle => "Enter the angle",
            Self::ScaleFactor => "Enter a scale factor",
        }
    }
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
    ReflectionAxis,
    ReflectionObject,
    DilationObject,
    RotationObject,
    TranslationObject,
    TranslationSegment,
    Line,
    Circle,
    Arc,
    Shape,
    Curve,
}
impl Need {
    fn allows_new_point(self) -> bool {
        matches!(
            self,
            Self::Point
                | Self::ReflectionAxis
                | Self::ReflectionObject
                | Self::DilationObject
                | Self::RotationObject
                | Self::TranslationObject
        )
    }
    fn accepts(self, shape: &Shape, allow_sampled_curve: bool) -> bool {
        match self {
            Self::Mirror => matches!(shape, Shape::Point(_) | Shape::Line { .. }),
            Self::Transformable => !matches!(
                shape,
                Shape::Undefined
                    | Shape::EmptyConic(_)
                    | Shape::Sequence(_)
                    | Shape::Parametric(_)
                    | Shape::Function { .. }
                    | Shape::Number { .. }
            ),
            Self::Point | Self::PointOnCurve => matches!(shape, Shape::Point(_)),
            Self::ReflectionAxis => matches!(shape, Shape::Point(_) | Shape::Line { .. }),
            Self::ReflectionObject
            | Self::DilationObject
            | Self::RotationObject
            | Self::TranslationObject => Self::Transformable.accepts(shape, false),
            Self::TranslationSegment => matches!(
                shape,
                Shape::Line {
                    extent: geo::Extent::Segment,
                    ..
                }
            ),
            Self::Line => matches!(shape, Shape::Line { .. }),
            Self::Circle => matches!(shape, Shape::Circle { .. } | Shape::Arc(_)),
            Self::Arc => matches!(shape, Shape::Arc(_)),
            Self::Curve => {
                matches!(
                    shape,
                    Shape::Line { .. }
                        | Shape::Circle { .. }
                        | Shape::Arc(_)
                        | Shape::Ellipse(_)
                        | Shape::Parabola(_)
                        | Shape::Hyperbola(_)
                ) || allow_sampled_curve
                    && matches!(shape, Shape::Function { .. } | Shape::Parametric(_))
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
    fn need(self, operand: usize) -> Option<Need> {
        if self.function == "polygon" {
            Some(Need::Point)
        } else {
            self.needs.get(operand).copied()
        }
    }
    fn pick_filter(self, operand: usize) -> Option<Need> {
        match self.function {
            "element" => Some(Need::Curve),
            "tangent" => Some(Need::PointOnCurve),
            "reflection"
            | "rotation"
            | "homothety"
            | "translation"
            | "similarity"
            | "projection"
            | "inversion"
            | "reciprocation"
            | "polygon"
            | "rectangle"
            | "parallelogram"
            | "isosceles_triangle"
            | "right_triangle"
            | "rhombus"
            | "square"
            | "equilateral_triangle"
            | "isopolygon" => self.need(operand),
            _ => None,
        }
    }
    fn transform_parameter(self) -> Option<TransformParameter> {
        match self.function {
            "rotation" => Some(TransformParameter::Angle),
            "homothety" => Some(TransformParameter::ScaleFactor),
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
    Command {
        label: "Parametric",
        function: "plotparam",
        needs: &[],
    },
    Command {
        label: "Polar",
        function: "plotpolar",
        needs: &[],
    },
    Command {
        label: "Locus",
        function: "locus",
        needs: &[Need::Point, Need::PointOnCurve],
    },
    Command {
        label: "Sequence",
        function: "plotseq",
        needs: &[],
    },
    Command {
        label: "Implicit",
        function: "plotimplicit",
        needs: &[],
    },
    Command {
        label: "Slopefield",
        function: "plotfield",
        needs: &[],
    },
    Command {
        label: "ODE",
        function: "plotode",
        needs: &[],
    },
    Command {
        label: "List",
        function: "plotlist",
        needs: &[],
    },
];
const PLOT_CATEGORY: usize = 11;
const PLOTS: &[Command] = &[
    Command {
        label: "Function",
        function: "plotfunc",
        needs: &[],
    },
    Command {
        label: "Parametric",
        function: "plotparam",
        needs: &[],
    },
    Command {
        label: "Polar",
        function: "plotpolar",
        needs: &[],
    },
    Command {
        label: "Sequence",
        function: "plotseq",
        needs: &[],
    },
    Command {
        label: "Implicit",
        function: "plotimplicit",
        needs: &[],
    },
    Command {
        label: "Slopefield",
        function: "plotfield",
        needs: &[],
    },
    Command {
        label: "ODE",
        function: "plotode",
        needs: &[],
    },
    Command {
        label: "List",
        function: "plotlist",
        needs: &[],
    },
    Command {
        label: "Slider",
        function: "parameter",
        needs: &[],
    },
];
const TRANSFORMS: &[Command] = &[
    Command {
        label: "Reflection",
        function: "reflection",
        needs: &[Need::ReflectionAxis, Need::ReflectionObject],
    },
    Command {
        label: "Rotation",
        function: "rotation",
        needs: &[Need::Point, Need::RotationObject],
    },
    Command {
        label: "Dilation",
        function: "homothety",
        needs: &[Need::Point, Need::DilationObject],
    },
    Command {
        label: "Translation",
        function: "translation",
        needs: &[Need::TranslationObject, Need::TranslationSegment],
    },
    Command {
        label: "Similarity",
        function: "similarity",
        needs: &[Need::Point, Need::Transformable],
    },
    Command {
        label: "Projection",
        function: "projection",
        needs: &[Need::Curve, Need::Point],
    },
    Command {
        label: "Inversion",
        function: "inversion",
        needs: &[Need::Point, Need::Point],
    },
    Command {
        label: "Reciprocation",
        function: "reciprocation",
        needs: &[Need::Circle, Need::Mirror],
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
        PLOT_CATEGORY => PLOTS,
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
        self.geometry_panel.angle_turn = context
            .app_settings
            .get("Geometry")
            .cloned()
            .unwrap_or_default()
            .effective(&context.settings)
            .angle
            .full_turn();
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
        self.geometry_panel.angle_turn = snapshot
            .app_settings
            .get("Geometry")
            .cloned()
            .unwrap_or_default()
            .effective(&snapshot.settings)
            .angle
            .full_turn();
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
        self.geometry_panel.dismiss_object_picker();
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
        if self
            .geometry_panel
            .args
            .iter()
            .chain(&self.geometry_panel.tangent_curve)
            .any(|n| {
                !self
                    .memory
                    .models
                    .geometry_book
                    .as_ref()
                    .is_some_and(|b| b.objects.iter().any(|o| &o.name == n))
            })
        {
            self.geometry_panel.clear_operands();
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
        if self.is_geometry_page()
            && let Some(wizard) = &self.geometry_panel.wizard
        {
            return Some(vec![
                Request::EditMath(MathTarget::Slopefield {
                    index: wizard.selected,
                    clear: true,
                }),
                Request::TypeText(text.into()),
            ]);
        }
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
            Some(Page::Symbolic) => vec![1, 2, 3, 4, PLOT_CATEGORY, 5],
            Some(Page::Numeric) => (1..=6).collect(),
            _ => vec![0, 1, 2, 3, 4, PLOT_CATEGORY, 5, 6],
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
                .map(|c| {
                    if c == PLOT_CATEGORY {
                        "Plot"
                    } else {
                        CATEGORIES[c]
                    }
                    .into()
                })
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
            Some(ARC_PICKER | TANGENT_PICKER) => self
                .geometry_panel
                .picker_candidates
                .iter()
                .map(|name| {
                    let source = self
                        .memory
                        .models
                        .geometry_book
                        .as_ref()
                        .and_then(|book| book.objects.iter().find(|object| &object.name == name))
                        .map_or("", |object| object.source.as_str());
                    format!("G{name}  {source}")
                })
                .collect(),
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
        if is_object_picker(Some(category)) {
            let candidates = std::mem::take(&mut self.geometry_panel.picker_candidates);
            let Some(name) = candidates.get(menu.index) else {
                return vec![];
            };
            if category == TANGENT_PICKER
                && self.geometry_scene().is_some_and(|scene| {
                    scene.objects.iter().any(|object| {
                        object.name == *name && matches!(object.value, Ok(Shape::Function { .. }))
                    })
                })
            {
                self.geometry_panel.tangent_curve = Some(name.clone());
                self.error = None;
                self.dirty();
                return vec![];
            }
            return self.geometry_confirm(Some(name));
        }
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
        self.geometry_panel.field_solution = None;
        if category == PLOT_CATEGORY && menu.index == 8 && self.page == Some(Page::Plot) {
            self.geometry_new_slider();
            self.dirty();
            return vec![];
        }
        if category == PLOT_CATEGORY && menu.index == 5 && self.page == Some(Page::Plot) {
            self.geometry_panel.tool = None;
            self.geometry_panel.wizard = Some(wizard::Slopefield::new(&self.trace_window()));
            self.dirty();
            return vec![];
        }
        let Some(command) = commands(category).get(menu.index).copied() else {
            return vec![];
        };
        if self.page == Some(Page::Plot)
            && matches!(
                command.function,
                "plotfunc"
                    | "conic"
                    | "plotparam"
                    | "plotpolar"
                    | "plotseq"
                    | "plotimplicit"
                    | "plotfield"
                    | "plotode"
                    | "plotlist"
            )
        {
            self.geometry_panel.tool = None;
            self.geometry_panel.clear_operands();
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
            self.geometry_panel.clear_operands();
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
    fn geometry_needs_intersection_point(&self, command: Command, scene: Option<&Scene>) -> bool {
        command.function == "single_inter"
            && matches!(
                self.geometry_panel.args.as_slice(),
                [a, b] if scene.is_some_and(|scene| scene.forms_arc_intersection_pair(a, b))
            )
    }
    fn geometry_operand_filter(&self, command: Command, scene: Option<&Scene>) -> Option<Need> {
        if self.geometry_needs_intersection_point(command, scene) {
            Some(Need::Point)
        } else {
            command.pick_filter(self.geometry_panel.args.len())
        }
    }
    fn geometry_plot_hit(
        &self,
        scene: &Scene,
        book: &Book,
        p: [f64; 2],
        window: &crate::ide::native_apps::PlotWindow,
        filter: Option<Need>,
    ) -> Option<String> {
        if let (Some(Need::PointOnCurve), Some(curve)) =
            (filter, self.geometry_panel.tangent_curve.as_deref())
        {
            view::function_tangent_point_hit(scene, book, p, window, curve)
        } else {
            view::hit(scene, book, p, window, filter)
        }
    }
    fn geometry_hovered_operand<'a>(
        &self,
        command: Command,
        p: Point2,
        scene: Option<&'a Scene>,
    ) -> Option<(String, &'a Shape)> {
        let scene = scene?;
        let filter = self.geometry_operand_filter(command, Some(scene));
        let hit = if filter.is_some() {
            let w = self.trace_window();
            self.geometry_plot_hit(
                scene,
                self.memory.models.geometry_book.as_ref()?,
                view::project(p, &w),
                &w,
                filter,
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
        if let Some(parameter) = command.transform_parameter() {
            let name = command.function;
            let value = self.geometry_panel.transform_parameter.as_deref();
            return Ok(match args {
                [center, object] => {
                    let value = value.ok_or(parameter.prompt())?;
                    format!("{name}({center},{value},{object})")
                }
                [center] => match value {
                    Some(value) => format!("{name}({center},{value})"),
                    None => format!("{name}({center})"),
                },
                _ => format!("{name}({})", args.join(",")),
            });
        }
        Ok(match (command.function, args) {
            ("point", _) => point_source(p),
            ("element", [curve]) => {
                return scene.ok_or("Select a curve")?.point_on_source(curve, p);
            }
            ("single_inter", [a, b]) => format!("single_inter({a},{b},{})", point_source(p)),
            ("circle", [center, edge]) => format!("circle({center},{edge}-{center})"),
            ("translation", [object, segment]) => format!("translation({segment},{object})"),
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
            ("similarity", [center, object]) => format!("similarity({center},2,0,{object})"),
            ("inversion", [center, object]) => format!("inversion({center},1,{object})"),
            (name, _) => format!("{name}({})", args.join(",")),
        })
    }
    fn geometry_references(&self) -> Vec<String> {
        self.geometry_panel
            .args
            .iter()
            .map(|n| format!("G{n}"))
            .collect()
    }
    fn geometry_command_preview(&self, scene: Option<&Scene>) -> Option<String> {
        let command = self.geometry_panel.tool?;
        let p = self.construction_pointer();
        let mut args = self.geometry_references();
        let need = if self.geometry_needs_intersection_point(command, scene) {
            Some(Need::Point)
        } else {
            command.need(args.len())
        };
        if let Some(need) = need {
            if let Some((name, shape)) = self.geometry_hovered_operand(command, p, scene)
                && need.accepts(shape, matches!(command.function, "element" | "projection"))
            {
                args.push(format!("G{name}"));
            } else if need.allows_new_point() {
                args.push(point_source(p));
            }
        }
        self.geometry_command_source(command, &args, p, scene).ok()
    }
    pub fn geometry_transform_parameter(
        &mut self,
        parameter: TransformParameter,
        source: Option<&str>,
    ) -> Result<(), String> {
        if let Some(source) = source {
            let source = source.trim();
            if source.is_empty() {
                return Err(parameter.prompt().into());
            }
            geo::source(source)?;
            let (_, diagnostics) = ppl_syntax::parse_expression(source);
            if let Some(error) = diagnostics.errors().next() {
                return Err(error.message.clone());
            }
            self.geometry_panel.transform_parameter = Some(source.into());
        } else {
            self.geometry_panel.clear_operands();
        }
        self.dirty();
        Ok(())
    }
    fn geometry_finish_command(
        &mut self,
        command: Command,
        scene: Option<&Scene>,
        new_intersection_point: Option<Point2>,
    ) -> Vec<Request> {
        let mut book = self.memory.models.geometry_book.as_ref().unwrap().clone();
        let mut args = self.geometry_references();
        if let Some(p) = new_intersection_point {
            if book.objects.len() + 2 > geo::MAX_OBJECTS || geo::point(p).is_err() {
                self.error = Some("Geometry point exceeds its limit".into());
                self.dirty();
                return vec![];
            }
            let name = book.next_name();
            args.push(format!("G{name}"));
            book.objects.push(Object {
                name,
                source: point_source(p),
                ..Default::default()
            });
        }
        let source =
            self.geometry_command_source(command, &args, self.construction_pointer(), scene);
        self.geometry_panel.clear_operands();
        let source = match source {
            Ok(source) => source,
            Err(error) => {
                self.error = Some(error);
                self.dirty();
                return vec![];
            }
        };
        let name = book.next_name();
        if matches!(
            command.function,
            "inversion"
                | "similarity"
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
        vec![Request::Calculate(
            crate::ide::native_app_ui::Calculation::GeometryEdit {
                book: Box::new(book),
                name,
                source,
                insert: None,
            },
        )]
    }
    fn geometry_open_object_picker(
        &mut self,
        command: Command,
        scene: Option<&Scene>,
        p: Point2,
    ) -> bool {
        let (Some(scene), Some(book)) = (scene, self.memory.models.geometry_book.as_ref()) else {
            return false;
        };
        let category = match (command.function, self.geometry_panel.args.as_slice()) {
            ("single_inter", [first])
                if scene.objects.iter().any(|object| {
                    &object.name == first && matches!(object.value, Ok(Shape::Arc(_)))
                }) =>
            {
                ARC_PICKER
            }
            ("tangent", []) if self.geometry_panel.tangent_curve.is_none() => TANGENT_PICKER,
            _ => return false,
        };
        let window = self.trace_window();
        let p = view::project(p, &window);
        let names = if category == ARC_PICKER {
            view::arc_hits(scene, book, p, &window)
        } else {
            view::function_tangent_hits(scene, book, p, &window)
        };
        if names.len() < 2 {
            return false;
        }
        self.geometry_panel.picker_candidates = names;
        self.geometry_panel.menu = Some(Menu {
            category: Some(category),
            index: 0,
            first: 0,
        });
        self.error = None;
        self.dirty();
        true
    }
    fn geometry_confirm(&mut self, picked_object: Option<&str>) -> Vec<Request> {
        if let Some((expr, x, y)) = &self.geometry_panel.field_solution {
            let p = self.construction_pointer();
            let source = format!(
                "plotode({expr},[{x},{y}],[{},{}])",
                geo::canonical(p.x),
                geo::canonical(p.y)
            );
            return self.create_wizard_object(source);
        }
        let p = self.construction_pointer();
        if let Some(command) = self.geometry_panel.tool {
            if command.function == "point" {
                self.geometry_add_point(p);
                self.geometry_panel.clear_operands();
                return vec![];
            }
            let index = self.geometry_panel.args.len();
            let scene = self.geometry_scene();
            let needs_intersection_point =
                self.geometry_needs_intersection_point(command, scene.as_ref());
            let need = if needs_intersection_point {
                Need::Point
            } else if let Some(need) = command.need(index) {
                need
            } else {
                return vec![];
            };
            if picked_object.is_none()
                && self.geometry_open_object_picker(command, scene.as_ref(), p)
            {
                return vec![];
            }
            let hovered = if let Some(name) = picked_object {
                scene.as_ref().and_then(|scene| {
                    let object = scene.objects.iter().find(|object| object.name == name)?;
                    Some((name.to_owned(), object.value.as_ref().ok()?))
                })
            } else {
                self.geometry_hovered_operand(command, p, scene.as_ref())
            };
            if command.function == "polygon" {
                if hovered
                    .as_ref()
                    .is_some_and(|(name, _)| self.geometry_panel.args.first() == Some(name))
                {
                    if index >= 3 {
                        return self.geometry_finish_command(command, scene.as_ref(), None);
                    }
                    self.error =
                        Some("Select at least three vertices before closing the polygon".into());
                    self.dirty();
                    return vec![];
                }
                let repeated = self.geometry_panel.polygon_last_pointer == Some(p)
                    || hovered
                        .as_ref()
                        .is_some_and(|(name, _)| self.geometry_panel.args.last() == Some(name));
                if repeated {
                    return if index >= 2 {
                        self.geometry_finish_command(command, scene.as_ref(), None)
                    } else {
                        vec![]
                    };
                }
                let name = hovered
                    .as_ref()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| {
                        self.memory
                            .models
                            .geometry_book
                            .as_ref()
                            .unwrap()
                            .next_name()
                    });
                let mut candidate = self.geometry_references();
                candidate.push(format!("G{name}"));
                if let Err(error) = self
                    .geometry_command_source(command, &candidate, p, scene.as_ref())
                    .and_then(|source| geo::source(&source))
                {
                    self.error = Some(error);
                    self.dirty();
                    return vec![];
                }
            }
            let required = match need {
                Need::PointOnCurve => Some("Select a point on a curve"),
                Need::TranslationSegment => Some("Select the displacement segment"),
                _ => None,
            };
            if hovered.is_none()
                && let Some(message) = required
            {
                self.error = Some(message.into());
                self.dirty();
                return vec![];
            }
            let name = if let Some((name, shape)) = hovered {
                if !need.accepts(shape, matches!(command.function, "element" | "projection")) {
                    self.error = Some(format!("Select a {need:?}"));
                    return vec![];
                }
                name
            } else if needs_intersection_point {
                return self.geometry_finish_command(command, scene.as_ref(), Some(p));
            } else if need.allows_new_point() {
                let Some(name) = self.geometry_add_point(p) else {
                    return vec![];
                };
                name
            } else {
                self.error = Some(format!("Select a {need:?}"));
                return vec![];
            };
            self.geometry_panel.args.push(name);
            if command.function == "polygon" {
                self.geometry_panel.polygon_last_pointer = Some(p);
                self.error = None;
            }
            if index == 0
                && let Some(parameter) = command.transform_parameter()
            {
                return vec![Request::EditMath(MathTarget::GeometryTransformParameter(
                    parameter,
                ))];
            }
            if command.need(self.geometry_panel.args.len()).is_none()
                && !self.geometry_needs_intersection_point(command, scene.as_ref())
            {
                return self.geometry_finish_command(command, scene.as_ref(), None);
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
        if let Some(result) = self.slider_key(action) {
            return Some(result);
        }
        if self.geometry_panel.wizard.is_some() {
            return self.slopefield_action(action);
        }
        if self.geometry_panel.field_solution.is_some() && action == A::Back {
            self.geometry_panel.field_solution = None;
            self.dirty();
            return Some(vec![]);
        }
        if action == A::Tool(crate::ide::native::keys::Tool::Menu) {
            self.geometry_panel.dismiss_object_picker();
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
            self.geometry_panel.dismiss_object_picker();
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
                    if is_object_picker(menu.category) {
                        self.geometry_panel.dismiss_object_picker();
                    } else if menu.category.is_some() {
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
                A::Evaluate => return Some(self.geometry_confirm(None)),
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
                A::Back
                    if self
                        .geometry_panel
                        .tool
                        .is_some_and(|tool| tool.function == "polygon")
                        && !self.geometry_panel.args.is_empty() =>
                {
                    self.geometry_panel.args.pop();
                    self.geometry_panel.polygon_last_pointer = None;
                    self.error = None;
                }
                A::Back | A::On => {
                    self.geometry_panel.tool = None;
                    self.geometry_panel.clear_operands();
                    self.geometry_panel.selected = None;
                }
                _ => {}
            }
            if self.memory.geometry_cursor.pointer != old_pointer
                && let Some(command) = self.geometry_panel.tool
            {
                let p = view::project(self.construction_pointer(), &w);
                self.geometry_panel.hover = self.geometry_scene().and_then(|scene| {
                    self.geometry_plot_hit(
                        &scene,
                        self.memory.models.geometry_book.as_ref()?,
                        p,
                        &w,
                        self.geometry_operand_filter(command, Some(&scene)),
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
