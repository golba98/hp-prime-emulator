//! Dependency-aware Geometry definitions, evaluated off the UI thread.
//! HP Prime User Guide chapter9: Symbolic objects and Numeric measurements.
mod access;
mod arc;
mod conic;
mod ellipse;
mod evaluate;
mod function;
mod hyperbola;
mod parabola;
pub mod plot;
pub mod primitives;
mod transform;
use super::native_app_models::Geometry;
pub use super::native_app_models::Point2;
pub use arc::CircularArc;
pub use ellipse::Ellipse;
pub use hyperbola::Hyperbola;
pub use parabola::Parabola;
use ppl_vm::SessionSnapshot;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, atomic::AtomicBool},
};
pub const MAX_OBJECTS: usize = 3000;
pub(super) const TRACE_SAMPLE_LIMIT: usize = 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Object {
    pub name: String,
    pub source: String,
    pub visible: bool,
    pub label: bool,
    pub color: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub traced: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub trace_connected: bool,
    #[serde(skip_serializing_if = "VecDeque::is_empty")]
    pub trace_samples: VecDeque<Point2>,
}
impl Default for Object {
    fn default() -> Self {
        Self {
            name: String::new(),
            source: String::new(),
            visible: true,
            label: true,
            color: 0x000000,
            traced: false,
            trace_connected: false,
            trace_samples: VecDeque::new(),
        }
    }
}
pub(super) fn push_trace(samples: &mut VecDeque<Point2>, p: Point2) {
    if samples.back() == Some(&p) {
        return;
    }
    if samples.len() == TRACE_SAMPLE_LIMIT {
        samples.pop_front();
    }
    samples.push_back(p);
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Measurement {
    pub source: String,
    pub label: String,
    pub visible: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub objects: Vec<Object>,
    pub measurements: Vec<Measurement>,
    /// Firmware saves definitions, and rebuilds evaluated object values after
    /// reopening a view. Value-only PPL assignments share this transient cache.
    #[serde(skip)]
    pub current_values: BTreeMap<String, ppl_vm::PortableValue>,
    /// Snapshots of the curves that produced the current constrained points.
    #[serde(skip)]
    pub current_point_curves: BTreeMap<String, Shape>,
    /// Keep curve parameters alongside the evaluated PPL values until the next redraw.
    #[serde(skip)]
    pub current_curves: BTreeMap<String, Shape>,
    /// Last evaluated positions of armed points; restart establishes a fresh baseline.
    #[serde(skip)]
    pub(crate) last_trace_positions: BTreeMap<String, Point2>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Extent {
    Segment,
    Ray,
    Line,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    EmptyConic(String),
    Undefined,
    Point(Point2),
    Number {
        re: f64,
        im: f64,
    },
    Line {
        a: Point2,
        b: Point2,
        extent: Extent,
    },
    Lines(Vec<[Point2; 2]>),
    Circle {
        center: Point2,
        radius: f64,
    },
    Arc(CircularArc),
    Ellipse(Ellipse),
    Hyperbola(Hyperbola),
    Parabola(Parabola),
    Polygon(Vec<Point2>),
    Points(Vec<Point2>),
    Function {
        expression: String,
        paths: Vec<Vec<Point2>>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneObject {
    pub name: String,
    pub value: Result<Shape, String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub objects: Vec<SceneObject>,
    pub measurements: Vec<Result<Shape, String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    point_curves: BTreeMap<String, Shape>,
}
impl Scene {
    pub(super) fn is_point_on_curve(&self, name: &str) -> bool {
        self.point_curves.contains_key(name)
    }
    pub(super) fn is_point(&self, name: &str) -> bool {
        self.objects
            .iter()
            .any(|o| o.name == name && matches!(o.value, Ok(Shape::Point(_))))
    }
    fn curve(&self, reference: &str) -> Result<&Shape, String> {
        let name = reference
            .strip_prefix("Geometry.")
            .unwrap_or(reference)
            .strip_prefix('G')
            .ok_or("Select a named curve")?;
        self.objects
            .iter()
            .find(|object| object.name == name)
            .ok_or("The selected curve is unavailable")?
            .value
            .as_ref()
            .map_err(Clone::clone)
    }
    pub fn point_on_source(&self, reference: &str, cursor: Point2) -> Result<String, String> {
        let parameter = primitives::element_parameter(self.curve(reference)?, cursor)?;
        Ok(format!("element({reference},{})", canonical(parameter)))
    }
}
pub fn default_window() -> super::native_apps::PlotWindow {
    let [xmin, xmax, ymin, ymax] = ppl_vm::GeometryPlotSettings::default()
        .derived_bounds()
        .map(|value| value.to_f64());
    super::native_apps::PlotWindow {
        xmin,
        xmax,
        ymin,
        ymax,
        axes: true,
        ..Default::default()
    }
}
pub fn number(v: f64) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err("Geometry value exceeds numerical range".into())
    }
}
pub fn point(p: Point2) -> Result<Point2, String> {
    number(p.x)?;
    number(p.y)?;
    Ok(p)
}
pub fn canonical(n: f64) -> String {
    ppl_math::Real::from_f64(n)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| n.to_string())
}
impl Book {
    pub fn validate(&self) -> Result<(), String> {
        if self.objects.len() > MAX_OBJECTS || self.measurements.len() > MAX_OBJECTS {
            return Err("Geometry exceeds object limit".into());
        }
        let mut names = BTreeSet::new();
        for o in &self.objects {
            if !valid_name(&o.name) || !names.insert(&o.name) {
                return Err("Geometry object needs a unique letter-led name".into());
            }
            source(&o.source)?;
            if o.trace_samples.len() > TRACE_SAMPLE_LIMIT {
                return Err("Geometry trace exceeds the sample limit".into());
            }
            for &p in &o.trace_samples {
                point(p)?;
            }
        }
        for m in &self.measurements {
            source(&m.source)?;
            if m.label.len() > 256 {
                return Err("Geometry label exceeds256bytes".into());
            }
        }
        Ok(())
    }
    pub fn from_legacy(old: &Geometry) -> Result<Self, String> {
        let mut result = Self::default();
        for (name, p) in &old.points {
            point(*p)?;
            result.objects.push(Object {
                name: name.clone(),
                source: format!("point({},{})", canonical(p.x), canonical(p.y)),
                ..Default::default()
            });
        }
        for [a, b] in &old.segments {
            let name = result.next_name();
            result.objects.push(Object {
                name,
                source: format!("segment(G{a},G{b})"),
                ..Default::default()
            });
        }
        result.validate()?;
        Ok(result)
    }
    pub fn next_name(&self) -> String {
        for index in 0..=MAX_OBJECTS {
            let mut n = index;
            let mut name = String::new();
            loop {
                name.insert(0, (b'A' + (n % 26) as u8) as char);
                if n < 26 {
                    break;
                }
                n = n / 26 - 1;
            }
            if self.objects.iter().all(|o| o.name != name) {
                return name;
            }
        }
        "OBJECT_LIMIT".into()
    }
    pub fn calculate(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Scene, String> {
        self.validate()?;
        evaluate::scene(self, snapshot, cancel)
    }
    pub fn enter(
        &self,
        name: &str,
        definition: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<(Self, Scene), String> {
        source(definition)?;
        let mut next = self.clone();
        if let Some(object) = next.objects.iter_mut().find(|o| o.name == name) {
            object.source = definition.into();
        } else {
            next.objects.push(Object {
                name: name.into(),
                source: definition.into(),
                ..Default::default()
            });
        }
        let scene = next.calculate(snapshot, cancel)?;
        scene
            .objects
            .iter()
            .find(|o| o.name == name)
            .ok_or("Missing constructed object")?
            .value
            .as_ref()
            .map_err(Clone::clone)?;
        Ok((next, scene))
    }
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        let entered = new.trim();
        let new = entered.strip_prefix('G').unwrap_or(entered);
        if !valid_name(new) || self.objects.iter().any(|o| o.name == new && o.name != old) {
            return Err("Choose a unique Geometry name".into());
        }
        let mut next = self.clone();
        next.objects
            .iter_mut()
            .find(|o| o.name == old)
            .ok_or("Unknown Geometry object")?
            .name = new.into();
        fn replace(source: &str, old: &str, new: &str) -> String {
            let mut diags = ppl_syntax::Diagnostics::default();
            let tokens = ppl_syntax::lexer::lex(source, &mut diags);
            let mut out = source.to_string();
            let prefixed = format!("G{old}");
            for token in tokens.iter().rev() {
                if let ppl_syntax::lexer::Tok::Ident(name) = &token.tok {
                    if name == &prefixed {
                        out.replace_range(
                            token.span.start as usize..token.span.end as usize,
                            &format!("G{new}"),
                        );
                    }
                }
            }
            out
        }
        for object in &mut next.objects {
            object.source = replace(&object.source, old, new);
        }
        for m in &mut next.measurements {
            m.source = replace(&m.source, old, new);
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub(super) fn can_move_point(&self, name: &str) -> bool {
        self.objects
            .iter()
            .any(|object| object.name == name && evaluate::point_movement(&object.source).is_some())
    }
    pub(super) fn trace_position(
        &self,
        name: &str,
        cursor: Point2,
        scene: Option<&Scene>,
    ) -> Option<Point2> {
        let object = self
            .objects
            .iter()
            .find(|object| object.name == name && object.traced)?;
        match evaluate::point_movement(&object.source)? {
            evaluate::PointMovement::Free => point(cursor).ok(),
            evaluate::PointMovement::OnCurve(reference) => {
                primitives::curve_projection(scene?.curve(&reference).ok()?, cursor).ok()
            }
        }
    }
    pub fn move_point(
        &mut self,
        name: &str,
        to: Point2,
        scene: Option<&Scene>,
    ) -> Result<(), String> {
        point(to)?;
        let object = self
            .objects
            .iter_mut()
            .find(|o| o.name == name)
            .ok_or("Unknown point")?;
        object.source = match evaluate::point_movement(&object.source) {
            Some(evaluate::PointMovement::Free) => {
                format!("point({},{})", canonical(to.x), canonical(to.y))
            }
            Some(evaluate::PointMovement::OnCurve(reference)) => scene
                .ok_or("Calculate the construction before moving the point")?
                .point_on_source(&reference, to)?,
            None => return Err("This point is constrained by its definition".into()),
        };
        Ok(())
    }
}
fn valid_name(name: &str) -> bool {
    let mut c = name.bytes();
    name.len() <= 24
        && c.next().is_some_and(|c| c.is_ascii_alphabetic())
        && c.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn source(source: &str) -> Result<(), String> {
    if source.is_empty() || source.len() > 4096 {
        return Err("Geometry expression needs1to4096bytes".into());
    }
    let mut depth = 0_i32;
    for c in source.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                if depth > 64 {
                    return Err("Geometry expression nesting exceeded".into());
                }
            }
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use ppl_vm::{PortableValue, SessionContext};
    fn book(definitions: &[(&str, &str)]) -> Book {
        Book {
            objects: definitions
                .iter()
                .map(|(name, source)| Object {
                    name: (*name).into(),
                    source: (*source).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn quadratic_constructions_follow_original_geometry_parameters_and_signed_areas() {
        let cases = [
            ("x^2+y^2-81", 254.469004941, None),
            ("x^2+4*y^2-4", 6.28318530718, Some((0., 1.))),
            (
                "2*x^2+2*x*y+2*y^2-6",
                10.8827961854,
                Some((-3_f64.sqrt(), 3_f64.sqrt())),
            ),
            ("x^2-4*y", -2304., Some((1., 0.25))),
            ("x^2+4*y", 2304., Some((1., -0.25))),
            ("y^2-4*x", -4608., Some((0.25, 1.))),
            ("(x-2)^2-4*(y-3)", -2496., Some((1., 3.25))),
            ("x^2-y^2-1", -32.4501189211, Some((1., 0.))),
            ("x^2+y^2+1", 0., None),
            ("x^2+y^2", 0., Some((0., 0.))),
        ];
        for (expression, area, element) in cases {
            let result = scene(&book(&[
                ("A", &format!("conic({expression})")),
                ("B", "area(GA)"),
                ("C", "element(GA,1)"),
            ]));
            let Shape::Number { re, .. } = shape(&result, "B") else {
                panic!("{expression}");
            };
            assert!((re - area).abs() < 1e-8, "{expression}: {re}");
            if let Some((x, y)) = element {
                pt(&result, "C", Point2 { x, y });
            }
        }
        let result = scene(&book(&[
            ("A", "conic(x^2-1)"),
            ("B", "conic(x^2-y^2)"),
            ("C", "conic(x^3-1)"),
            ("D", "conic(SIN(x)+y)"),
        ]));
        for name in ["A", "B"] {
            assert!(matches!(shape(&result,name),Shape::Lines(v) if v.len()==2));
        }
        assert!(result.objects[2..].iter().all(|o| o.value.is_err()));
    }
    #[test]
    fn original_ellipse_and_arc_tangents_use_curve_parameters() {
        let construction = book(&[
            ("A", "ellipse(point(-2,0),point(2,0),3)"),
            ("B", "circle(point(0,0),2,0,2)"),
            ("C", "slope(tangent(GA,1))"),
            ("D", "slope(tangent(GB,1))"),
            ("E", "element(GA,1)"),
            ("F", "slope(tangent(GE))"),
        ]);
        let scene = scene(&construction);
        for (name, expected) in [("C", 2. / 9.), ("D", -1. / 1_f64.tan()), ("F", 2. / 9.)] {
            let Shape::Number { re, .. } = shape(&scene, name) else {
                panic!("{name}")
            };
            assert!((re - expected).abs() < 1e-10, "{name}: {re}");
        }
    }

    #[test]
    fn similarities_transform_whole_objects_and_reject_a_point_as_displacement() {
        let construction = book(&[
            ("A", "square(point(0,0),point(2,0))"),
            ("B", "translation(1+2*i,GA)"),
            ("C", "similarity(0,2,0,GA)"),
            ("D", "reflection(line(point(0,0),point(1,0)),GA)"),
            ("E", "translation(point(1,2),GA)"),
            ("F", "rotation(point(0,0),0,segment(point(1,2),point(3,4)))"),
            ("G", "homothety(point(1,1),2,circle(point(1,1),2))"),
        ]);
        let scene = scene(&construction);
        for (name, last) in [("B", (1., 4.)), ("C", (0., 4.)), ("D", (0., -2.))] {
            let Shape::Polygon(vertices) = shape(&scene, name) else {
                panic!("{name}")
            };
            assert_eq!(
                vertices[3],
                Point2 {
                    x: last.0,
                    y: last.1
                }
            );
        }
        assert!(scene.objects[4].value.is_err());
        assert!(matches!(
            shape(&scene, "F"),
            Shape::Line {
                extent: Extent::Segment,
                ..
            }
        ));
        assert_eq!(
            shape(&scene, "G"),
            Shape::Circle {
                center: Point2 { x: 1., y: 1. },
                radius: 4.
            }
        );
    }

    #[test]
    fn constructed_polygons_use_ordered_vertices_and_follow_their_parents() {
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(2,0)"),
            ("C", "point(3,4)"),
            ("D", "square(GA,GB)"),
            ("E", "equilateral_triangle(GA,GB)"),
            ("F", "parallelogram(GA,GB,GC)"),
            ("G", "rectangle(GA,GB,GC)"),
            ("H", "rectangle(GA,GB,2)"),
            ("I", "right_triangle(GA,GB,2)"),
            ("J", "quadrilateral(GA,GB,GC,point(0,4))"),
            ("K", "isopolygon(GA,GB,6)"),
            ("L", "isopolygon(GA,GB,2)"),
            ("M", "isopolygon(GA,GB,2.5)"),
            ("N", "isosceles_triangle(GA,GB,angle(GA,GB,point(0,1)))"),
            ("O", "rhombus(GA,GB,angle(GA,GB,point(0,1)))"),
        ]);
        let initial = scene(&construction);
        for (name, count, area) in [
            ("D", 4, 4.),
            ("E", 3, 3_f64.sqrt()),
            ("F", 4, 8.),
            ("G", 4, 8.),
            ("H", 4, 8.),
            ("I", 3, 4.),
            ("J", 4, 10.),
            ("K", 6, 6. * 3_f64.sqrt()),
            ("N", 3, 2.),
            ("O", 4, 4.),
        ] {
            let Shape::Polygon(vertices) = shape(&initial, name) else {
                panic!("{name}")
            };
            assert_eq!(vertices.len(), count);
            assert!(
                (crate::ide::native_app_models::Geometry::polygon_area(&vertices).unwrap() - area)
                    .abs()
                    < 1e-10,
                "{name}"
            );
        }
        assert!(matches!(
            shape(&initial, "L"),
            Shape::Line {
                extent: Extent::Segment,
                ..
            }
        ));
        assert!(initial.objects[12].value.is_err());
        construction
            .move_point("B", Point2 { x: 0., y: -2. }, Some(&initial))
            .unwrap();
        let Shape::Polygon(square) = shape(&scene(&construction), "D") else {
            panic!()
        };
        assert_eq!(
            square,
            vec![
                Point2 { x: 0., y: 0. },
                Point2 { x: 0., y: -2. },
                Point2 { x: 2., y: -2. },
                Point2 { x: 2., y: 0. }
            ]
        );
    }

    #[test]
    fn original_function_tangent_forms_use_the_evaluated_slope() {
        let construction = book(&[
            ("A", "plotfunc(x^2)"),
            ("B", "element(GA,2)"),
            ("C", "tangent(GB)"),
            ("D", "tangent(GA,GB)"),
            ("E", "tangent(GA,2)"),
        ]);
        let scene = scene(&construction);
        for object in &scene.objects[2..] {
            let Ok(Shape::Line {
                a,
                b,
                extent: Extent::Line,
            }) = object.value
            else {
                panic!("{object:?}");
            };
            assert_eq!(a, Point2 { x: 2., y: 4. });
            assert_eq!((b.y - a.y) / (b.x - a.x), 4.);
        }
    }

    #[test]
    fn trace_records_evaluated_dependent_points_only_when_they_move() {
        let mut construction = book(&[
            ("A", "point(1,2)"),
            ("B", "point(abscissa(GA)+3,ordinate(GA))"),
            ("C", "point(8,9)"),
        ]);
        construction.objects[1].traced = true;
        construction.objects[2].traced = true;
        let initial = scene(&construction);
        construction.cache_scene(&initial).unwrap();
        assert!(!construction.can_move_point("B"));
        construction
            .move_point("A", Point2 { x: 2., y: 3. }, Some(&initial))
            .unwrap();
        let moved = scene(&construction);
        construction.cache_scene(&moved).unwrap();
        construction.cache_scene(&moved).unwrap();
        assert_eq!(
            construction.objects[1].trace_samples,
            [Point2 { x: 4., y: 2. }, Point2 { x: 5., y: 3. }]
        );
        assert!(construction.objects[2].trace_samples.is_empty());
        let restored: Book =
            serde_json::from_value(serde_json::to_value(&construction).unwrap()).unwrap();
        assert_eq!(
            restored.objects[1].trace_samples,
            construction.objects[1].trace_samples
        );
        assert!(restored.last_trace_positions.is_empty());
    }

    #[test]
    fn a_traced_constrained_point_previews_the_same_projection_without_moving() {
        let mut construction = book(&[
            ("A", "segment(point(0,0),point(4,0))"),
            ("B", "element(GA,0.5)"),
        ]);
        construction.objects[1].traced = true;
        let initial = scene(&construction);
        let cursor = Point2 { x: 3., y: 7. };
        let trace = construction.trace_position("B", cursor, Some(&initial));
        assert_eq!(trace, Some(Point2 { x: 3., y: 0. }));
        assert_eq!(construction.objects[1].source, "element(GA,0.5)");
        construction
            .move_point("B", cursor, Some(&initial))
            .unwrap();
        assert_eq!(construction.objects[1].source, "element(GA,0.75)");
        pt(&scene(&construction), "B", trace.unwrap());
    }

    #[test]
    fn arc_intersections_follow_the_g1_branch_cut_and_clip_straight_extents() {
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut construction = book(&[
            ("A", "circle(point(0,0),4,0,π/2)"),
            ("B", "line(point(-5,2),point(5,2))"),
            ("C", "line(point(4,-6),point(4,6))"),
            ("D", "line(point(-6,0),point(6,0))"),
            ("E", "circle(point(0,0),4,-π/4,π/4)"),
            ("F", "line(point(-5,1),point(5,-1))"),
            ("G", "circle(point(0,0),4,π/4,-π/4)"),
            ("H", "line(point(0,-5),point(0,5))"),
            ("I", "circle(point(0,0),4,π/4,π/4)"),
            ("J", "circle(point(0,0),4,2*π,0)"),
            ("K", "circle(point(0,0),0,0,π/2)"),
            ("L", "single_inter(GE,GD)"),
            ("M", "half_line(point(0,2),point(-5,2))"),
            ("N", "segment(point(-2,2),point(2,2))"),
        ]);
        let initial = scene(&construction);
        let diagonal = 20. / 26_f64.sqrt();
        for (a, b, expected) in [
            (
                "A",
                "B",
                vec![Point2 {
                    x: 12_f64.sqrt(),
                    y: 2.,
                }],
            ),
            ("A", "C", vec![Point2 { x: 4., y: 0. }]),
            ("A", "M", vec![]),
            ("A", "N", vec![]),
            ("E", "D", vec![]),
            (
                "E",
                "F",
                vec![Point2 {
                    x: diagonal,
                    y: -diagonal / 5.,
                }],
            ),
            ("G", "D", vec![Point2 { x: -4., y: 0. }]),
            (
                "G",
                "H",
                vec![Point2 { x: 0., y: 4. }, Point2 { x: 0., y: -4. }],
            ),
            ("I", "D", vec![Point2 { x: -4., y: 0. }]),
            (
                "J",
                "D",
                vec![Point2 { x: 4., y: 0. }, Point2 { x: -4., y: 0. }],
            ),
            ("K", "D", vec![]),
        ] {
            let hits = primitives::intersections(&shape(&initial, a), &shape(&initial, b)).unwrap();
            assert_eq!(hits.len(), expected.len(), "{a} with {b}");
            assert!(
                hits.iter()
                    .zip(expected)
                    .all(|(p, q)| p.distance(q).unwrap() < 1e-8)
            );
            assert_eq!(
                hits,
                primitives::intersections(&shape(&initial, b), &shape(&initial, a)).unwrap()
            );
        }
        assert_eq!(shape(&initial, "L"), Shape::Undefined);
        construction.cache_scene(&initial).unwrap();
        assert_eq!(
            construction
                .current_expression("single_inter(GE,GD)", &snapshot, cancel)
                .unwrap(),
            PortableValue::Symbolic(ppl_cas::CasValue::Undefined)
        );
    }

    #[test]
    fn ellipse_intersections_order_hits_and_use_the_first_in_the_two_argument_form() {
        let construction = book(&[
            ("A", "ellipse(point(8,0),point(12,0),point(10,3))"),
            ("B", "line(point(-4,0),point(18,0))"),
            ("C", "line(point(18,0),point(-4,0))"),
            ("D", "line(point(10,5),point(10,-5))"),
            ("E", "line(point(-4,3),point(18,3))"),
            ("F", "line(point(-4,3.001),point(18,3.001))"),
            ("G", "single_inter(GA,GB)"),
            ("H", "single_inter(GC,GA,point(0,0))"),
            ("I", "single_inter(GA,GF)"),
            ("J", "ellipse(point(-1,-1),point(1,1),point(2,0))"),
            ("K", "line(point(5,-5),point(-5,5))"),
        ]);
        let result = scene(&construction);
        let major = 13_f64.sqrt();
        let first = Point2 {
            x: 10. + major,
            y: 0.,
        };
        let second = Point2 {
            x: 10. - major,
            y: 0.,
        };
        pt(&result, "G", first);
        pt(&result, "H", second);
        assert_eq!(shape(&result, "I"), Shape::Undefined);
        for line in ["B", "C"] {
            let hits =
                primitives::intersections(&shape(&result, line), &shape(&result, "A")).unwrap();
            assert_eq!(hits.len(), 2);
            assert!(hits[0].distance(first).unwrap() < 1e-8);
            assert!(hits[1].distance(second).unwrap() < 1e-8);
        }
        let hits = primitives::intersections(&shape(&result, "A"), &shape(&result, "D")).unwrap();
        assert_eq!(hits.len(), 2);
        for (point, ordinate) in hits.iter().zip([3., -3.]) {
            assert!(
                point
                    .distance(Point2 {
                        x: 10.,
                        y: ordinate
                    })
                    .unwrap()
                    < 1e-8
            );
        }
        let hits = primitives::intersections(&shape(&result, "E"), &shape(&result, "A")).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].distance(Point2 { x: 10., y: 3. }).unwrap() < 1e-8);
        let hits = primitives::intersections(&shape(&result, "J"), &shape(&result, "K")).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].x > 0. && hits[0].y < 0.);
        assert!(hits[1].x < 0. && hits[1].y > 0.);
    }

    #[test]
    fn arc_measurements_keep_signed_angles_and_element_uses_the_supporting_circle() {
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(0,4)"),
            ("C", "circle(GA,GB,π/2,0)"),
            ("D", "circle(GA,GB-GA,7*π/4,π/4)"),
            ("E", "circle(GA,2,π/4,π/4)"),
            ("F", "circle(GA,0,0,π/2)"),
            ("G", "circle(GA,2,4*π,0)"),
            ("H", "element(GC,-π/2)"),
            ("I", "element(GC)"),
            ("J", "circle(GA,GB)"),
        ]);
        let initial = scene(&construction);
        let pi = std::f64::consts::PI;
        pt(&initial, "H", Point2 { x: 0., y: 0. });
        pt(
            &initial,
            "I",
            Point2 {
                x: 2. * 0.5_f64.cos(),
                y: 2. + 2. * 0.5_f64.sin(),
            },
        );
        assert!(initial.is_point_on_curve("H"));
        construction.cache_scene(&initial).unwrap();
        for (expression, expected) in [
            ("perimeter(Geometry.GC)", -pi),
            ("area(Geometry.GC)", -pi),
            ("abscissa(center(Geometry.GC))", 0.),
            ("ordinate(center(Geometry.GC))", 2.),
            ("radius(Geometry.GC)", 2.),
            ("perimeter(Geometry.GD)", -6. * pi),
            ("area(Geometry.GD)", -12. * pi),
            ("perimeter(Geometry.GE)", 0.),
            ("area(Geometry.GF)", 0.),
            ("perimeter(Geometry.GG)", -8. * pi),
            ("perimeter(Geometry.GJ)", 4. * pi),
            ("area(circle(point(0,0),-2,0,π/2))", pi),
        ] {
            let PortableValue::Real(result) = construction
                .current_expression(expression, &snapshot, cancel.clone())
                .unwrap()
            else {
                panic!("{expression} did not return a real value");
            };
            assert!((result.to_f64() - expected).abs() < 1e-8, "{expression}");
        }

        construction
            .move_point("B", Point2 { x: 0., y: 6. }, Some(&initial))
            .unwrap();
        let updated = scene(&construction);
        let Shape::Arc(arc) = shape(&updated, "C") else {
            panic!("arc did not redraw");
        };
        assert_eq!(arc.center, Point2 { x: 0., y: 3. });
        assert_eq!(arc.radius, 3.);
        assert!((arc.perimeter() + 3. * pi / 2.).abs() < 1e-9);
        assert_eq!(construction.objects[7].source, "element(GC,-π/2)");
        let restored: Book =
            serde_json::from_str(&serde_json::to_string(&construction).unwrap()).unwrap();
        assert_eq!(scene(&restored), updated);
    }

    #[test]
    fn arc_plot_wraps_counterclockwise_and_closes_at_a_full_turn() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
        let center = Point2 { x: 0., y: 0. };
        // Virtual G1 framebuffer controls cover forward, reversed, wrapped,
        // coincident endpoint, and full-turn renders of both signs.
        for (start, end, quadrants) in [
            (0., FRAC_PI_2, [true, false, false, false]),
            (FRAC_PI_2, 0., [false, true, true, true]),
            (7. * FRAC_PI_4, FRAC_PI_4, [true, false, false, true]),
            (FRAC_PI_4, FRAC_PI_4, [true, false, false, false]),
            (TAU, 0., [true, true, true, true]),
            (0., 4. * PI, [true, true, true, true]),
            (9. * FRAC_PI_4, 0., [true, true, true, true]),
        ] {
            let arc = CircularArc::new(center, 2., start, end).unwrap();
            let vertices: Vec<_> = arc.vertices().collect();
            for (quadrant, expected) in quadrants.into_iter().enumerate() {
                let angle = (2 * quadrant + 1) as f64 * FRAC_PI_4;
                let diagonal = primitives::circle_element(center, 2., angle);
                assert_eq!(
                    vertices
                        .iter()
                        .any(|p| p.distance(diagonal).unwrap() < 1e-9),
                    expected,
                    "start={start} end={end} quadrant={quadrant}",
                );
            }
        }
    }

    #[test]
    fn ellipse_elements_use_world_chord_slopes_and_survive_redraw() {
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut construction = book(&[
            ("A", "point(-2,0)"),
            ("B", "point(2,0)"),
            ("C", "point(0,3)"),
            ("D", "ellipse(GA,GB,GC)"),
            ("E", "element(GD,1/2)"),
            ("F", "ellipse(point(0,-2),point(0,2),point(3,0))"),
            ("G", "element(GF,1)"),
            ("H", "element(ellipse(GA,GB,-4),0)"),
            ("I", "element(ellipse(GA,GA,4),0)"),
            ("J", "element(ellipse(GA,GB,point(0,0)),0.5)"),
            ("K", "element(ellipse(GA,GB,-2),0.5)"),
        ]);
        let initial = scene(&construction);
        // Virtual G1, both horizontal and vertical foci, including negative
        // semi-major, coincident foci, and a witness between the foci.
        for (name, x, y) in [
            ("E", -156. / 49., 69. / 49.),
            ("G", -6. / 11., -39. / 11.),
            ("H", 4., 0.),
            ("I", -2., 4.),
            ("J", 0., 0.),
            ("K", -2., 0.),
        ] {
            pt(&initial, name, Point2 { x, y });
        }
        construction.cache_scene(&initial).unwrap();
        for (expression, expected) in [
            ("area(Geometry.GD)", 33.9815198898),
            ("perimeter(Geometry.GD)", 20.7955738173),
            ("radius(Geometry.GD)", 0.),
            ("area(ellipse(GA,GB,-4))", -43.5311847416),
            ("perimeter(ellipse(GA,GB,-4))", 23.4793953494),
            ("area(ellipse(GA,GB,point(0,0)))", 0.),
            ("perimeter(ellipse(GA,GB,point(0,0)))", 8.),
        ] {
            let PortableValue::Real(result) = construction
                .current_expression(expression, &snapshot, cancel.clone())
                .unwrap()
            else {
                panic!("{expression} did not return a real value");
            };
            assert!((result.to_f64() - expected).abs() < 1e-6, "{expression}");
        }

        let pointer = Point2 { x: 0., y: -3. };
        construction.objects[4].traced = true;
        let trace = construction
            .trace_position("E", pointer, Some(&initial))
            .unwrap();
        assert!(trace.distance(pointer).unwrap() < 1e-6);
        construction
            .move_point("E", pointer, Some(&initial))
            .unwrap();
        let moved = scene(&construction);
        pt(&moved, "E", trace);
        construction
            .move_point("B", Point2 { x: 4., y: 0. }, Some(&moved))
            .unwrap();
        let updated = scene(&construction);
        let Shape::Ellipse(ellipse) = shape(&updated, "D") else {
            panic!("ellipse did not redraw");
        };
        assert!((ellipse.area() - 41.6943579508).abs() < 1e-5);
        let restored: Book =
            serde_json::from_str(&serde_json::to_string(&construction).unwrap()).unwrap();
        assert_eq!(scene(&restored), updated);
    }

    #[test]
    fn function_instructions_keep_typed_points_and_refresh_dependencies() {
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut construction = book(&[
            ("A", "plotfunc(x^2/4+abscissa(GC))"),
            ("B", "element(GA,2)"),
            ("C", "point(1,0)"),
            ("D", "plotfunc(2*x+1)"),
            ("E", "element(GD,2)"),
            ("F", "plotfunc(1/x)"),
        ]);
        let scene = construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        let Shape::Function { paths, .. } = shape(&scene, "A") else {
            panic!("Expected evaluated function")
        };
        assert_eq!(paths.len(), 1);
        assert!(
            paths[0]
                .iter()
                .all(|p| (p.y - p.x * p.x / 4. - 1.).abs() < 1e-9)
        );
        pt(&scene, "B", Point2 { x: 2., y: 2. });
        pt(&scene, "E", Point2 { x: 24., y: 49. });
        let Shape::Function { paths, .. } = shape(&scene, "F") else {
            panic!("Expected reciprocal graph")
        };
        assert_eq!(paths.len(), 2);
        assert!(paths[0].iter().all(|p| p.x < 0.) && paths[1].iter().all(|p| p.x > 0.));
        for (expression, expected) in [
            ("element(GA)", "point(0.5,1.0625)"),
            ("element(Geometry.GA,-30)", "point(-30,226)"),
            ("element(GF,2)", "point(2,0.5)"),
        ] {
            assert_eq!(
                construction
                    .current_expression(expression, &snapshot, cancel.clone())
                    .unwrap(),
                access::symbolic(expected),
                "{expression}",
            );
        }

        construction.objects[2].source = "point(3,0)".into();
        assert_eq!(
            construction
                .current_expression("element(GA,2)", &snapshot, cancel.clone())
                .unwrap(),
            access::symbolic("point(2,2)"),
        );
        let redrawn = construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        pt(&redrawn, "B", Point2 { x: 2., y: 4. });
        let stored = serde_json::to_vec(&construction).unwrap();
        let mut reopened: Book = serde_json::from_slice(&stored).unwrap();
        let restored = reopened.refresh_values(&snapshot, cancel).unwrap();
        pt(&restored, "B", Point2 { x: 2., y: 4. });
    }

    #[test]
    fn function_point_on_projects_in_world_coordinates_and_keeps_its_definition() {
        let mut construction = book(&[("A", "plotfunc(x^2/4+1)"), ("B", "element(GA,2)")]);
        let first = scene(&construction);
        construction
            .move_point("B", Point2 { x: 2.3, y: 2.2 }, Some(&first))
            .unwrap();
        assert!(construction.objects[1].source.starts_with("element(GA,"));
        let moved = scene(&construction);
        let Shape::Point(p) = shape(&moved, "B") else {
            panic!("Expected constrained point")
        };
        // The original projects (2.3,2.2) to x=2.23959320696. A sampled
        // projection may differ by a small fraction of one LCD pixel.
        assert!((p.x - 2.23959320696).abs() < 0.001);
        assert!((p.y - (p.x * p.x / 4. + 1.)).abs() < 1e-10);
    }

    #[test]
    fn original_g1_tangent_cardinality_order_and_straight_extents() {
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut construction = book(&[
            ("A", "circle(point(0,0),2)"),
            ("B", "tangent(GA,point(4,0))"),
            ("C", "tangent(GA,point(1,0))"),
            ("D", "segment(point(3,4),point(4,5))"),
            ("E", "half_line(point(3,4),point(4,5))"),
            ("F", "line(point(3,4),point(4,5))"),
            ("G", "tangent(GA,point(2.00000001,0))"),
            ("H", "element(GA,1/2)"),
        ]);
        let scene = construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        let Shape::Lines(lines) = shape(&scene, "B") else {
            panic!("An exterior point should yield two tangents")
        };
        assert_eq!(lines.len(), 2);
        for ([anchor, contact], side) in lines.iter().zip([-1., 1.]) {
            assert_eq!(*anchor, Point2 { x: 4., y: 0. });
            assert!((contact.x - 1.).abs() <= 1e-12);
            assert!((contact.y - side * 3_f64.sqrt()).abs() <= 1e-12);
        }
        assert!(
            matches!(&construction.current_values["B"], PortableValue::List(items)
            if items.len() == 2 && items.iter().all(|item| matches!(item, PortableValue::Symbolic(_))))
        );
        assert_eq!(
            construction.current_values["C"],
            PortableValue::List(vec![])
        );
        assert!(matches!(shape(&scene, "G"), Shape::Lines(lines) if lines.len() == 2));
        for (expression, expected) in [
            ("tangent(GA,0)", "line(point(2,0),point(2,2))"),
            ("tangent(GA,2*i)", "line(point(0,2),point(-2,2))"),
            ("tangent(GD,point(0,0))", "segment(point(3,4),point(4,5))"),
            ("tangent(GE,1/2)", "half_line(point(3,4),point(4,5))"),
            ("tangent(GF,point(0,0))", "line(point(3,4),point(4,5))"),
        ] {
            assert_eq!(
                construction
                    .current_expression(expression, &snapshot, cancel.clone())
                    .unwrap(),
                access::symbolic(expected),
                "{expression}"
            );
        }
        for expression in ["slope(tangent(GH))", "slope(tangent(GA,GH))"] {
            let value = construction
                .current_expression(expression, &snapshot, cancel.clone())
                .unwrap();
            assert!(
                matches!(value, PortableValue::Real(slope)
                if (slope.to_f64() + 1.83048772171).abs() < 1e-10),
                "{expression}"
            );
        }
        let slope = construction
            .current_expression("slope(tangent(GA,π/2))", &snapshot, cancel.clone())
            .unwrap();
        assert!(matches!(slope, PortableValue::Real(slope) if slope.to_f64().abs() < 1e-10));
        for expression in ["tangent(GA)", "tangent(point(0,2),GA)", "tangent(GA,0,1)"] {
            assert_eq!(
                construction
                    .current_expression(expression, &snapshot, cancel.clone())
                    .unwrap(),
                PortableValue::String(format!("{expression} \n Error: Bad Argument Value")),
            );
        }
    }

    #[test]
    fn unary_tangent_uses_the_last_evaluated_constraint_until_redraw() {
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "circle(GA,2)"),
            ("C", "element(GB,π/2)"),
            ("D", "tangent(GC)"),
        ]);
        let vertical = access::symbolic("line(point(2,0),point(2,2))");
        construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        let first = construction.current_values["D"].clone();
        let slope = construction
            .current_expression("slope(tangent(GC))", &snapshot, cancel.clone())
            .unwrap();
        assert!(matches!(slope, PortableValue::Real(n) if n.to_f64().abs() < 1e-10));
        construction.objects[2].source = "element(GB,0)".into();
        assert_eq!(construction.current_values["D"], first);
        assert_eq!(
            construction
                .current_expression("slope(tangent(GC))", &snapshot, cancel.clone())
                .unwrap(),
            slope,
        );
        construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        assert_eq!(construction.current_values["D"], vertical);
        assert_eq!(
            construction
                .current_expression("tangent(GC)", &snapshot, cancel.clone())
                .unwrap(),
            vertical,
        );

        let json = serde_json::to_string(&construction).unwrap();
        let mut reopened: Book = serde_json::from_str(&json).unwrap();
        assert!(reopened.current_point_curves.is_empty());
        reopened.refresh_values(&snapshot, cancel.clone()).unwrap();
        assert_eq!(
            reopened
                .current_expression("tangent(Geometry.GC)", &snapshot, cancel)
                .unwrap(),
            vertical,
        );
    }

    #[test]
    fn original_g1_element_parameters_cover_affine_bounds_and_circle_radians() {
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "segment(GA,GB)"),
            ("D", "line(GA,GB)"),
            ("E", "half_line(GA,GB)"),
            ("F", "circle(point(6,4),2)"),
        ]);
        let mut context = SessionContext::default();
        context.settings.angle = ppl_math::AngleMode::Degrees;
        context.settings.numeric_version = ppl_math::NumericVersion::Current15751;
        let snapshot = context.snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        // Original virtual G1 `EXPR("element(...)")` returned these
        // symbolic points. The circle values were identical in both modes.
        for (expression, expected) in [
            ("element(GC)", "point(2,0)"),
            ("element(GC,-1/2)", "point(0,0)"),
            ("element(GC,1/4)", "point(1,0)"),
            ("element(GC,3/2)", "point(4,0)"),
            ("element(GD,-1/2)", "point(-2,0)"),
            ("element(GD,2)", "point(8,0)"),
            ("element(GE,-1/2)", "point(0,0)"),
            ("element(GE,3/2)", "point(6,0)"),
            ("element(GF)", "point(7.75516512378,4.95885107721)"),
            ("element(GF,0)", "point(8,4)"),
            ("element(GF,π/2)", "point(6,6)"),
            ("element(GF,90)", "point(5.10385276774,5.7879933272)"),
            ("element(GA,1/2)", "point(0,0)"),
        ] {
            let value = construction
                .current_expression(expression, &snapshot, cancel.clone())
                .unwrap();
            assert_eq!(
                value,
                PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(expected.into())),
                "{expression}"
            );
        }
    }

    #[test]
    fn point_on_curve_drag_and_reopen_preserve_the_constructed_dependency() {
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "segment(GA,GB)"),
            ("D", "element(GC,0.525)"),
            ("E", "element(GC,A)"),
            ("F", "circle(point(6,4),2)"),
            ("H", "element(GF,0)"),
        ]);
        let snapshot = SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let scene = construction.calculate(&snapshot, cancel.clone()).unwrap();
        let drag_to = Point2 { x: 3., y: 2. };
        assert!(construction.move_point("E", drag_to, Some(&scene)).is_err());
        assert_eq!(construction.objects[4].source, "element(GC,A)");
        construction.move_point("D", drag_to, Some(&scene)).unwrap();
        assert_eq!(construction.objects[3].source, "element(GC,0.75)");
        construction
            .move_point("H", Point2 { x: 5., y: 4. }, Some(&scene))
            .unwrap();
        assert!(construction.objects[6].source.starts_with("element(GF,"));
        construction
            .move_point("B", Point2 { x: 8., y: 0. }, None)
            .unwrap();
        let mut restored: Book =
            serde_json::from_value(serde_json::to_value(construction).unwrap()).unwrap();
        let scene = restored.refresh_values(&snapshot, cancel).unwrap();
        assert_eq!(
            scene.objects[3].value,
            Ok(Shape::Point(Point2 { x: 6., y: 0. }))
        );
        let Ok(Shape::Point(p)) = scene.objects[6].value else {
            panic!("circle point was not reconstructed");
        };
        assert!(p.distance(Point2 { x: 4., y: 4. }).unwrap() < 1e-10);
        assert_eq!(restored.objects[3].source, "element(GC,0.75)");
    }
    fn scene(book: &Book) -> Scene {
        book.calculate(
            &ppl_vm::SessionContext::default().snapshot(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }
    fn shape(scene: &Scene, name: &str) -> Shape {
        scene
            .objects
            .iter()
            .find(|o| o.name == name)
            .unwrap()
            .value
            .clone()
            .unwrap()
    }
    fn pt(scene: &Scene, name: &str, expected: Point2) {
        match shape(scene, name) {
            Shape::Point(p) => assert!(
                (p.x - expected.x).abs() < 1e-9 && (p.y - expected.y).abs() < 1e-9,
                "{p:?}"
            ),
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn actual_g1_point_distance_control_and_dependent_coordinate_updates() {
        let mut b = book(&[
            ("A", "point(3,4)"),
            ("B", "point(7,6)"),
            ("C", "midpoint(GA,GB)"),
            ("D", "point(abscissa(GA),ordinate(GB))"),
        ]);
        b.measurements.push(Measurement {
            source: "distance(GA,0)".into(),
            ..Default::default()
        });
        let s = scene(&b);
        pt(&s, "C", Point2 { x: 5., y: 5. });
        pt(&s, "D", Point2 { x: 3., y: 6. });
        assert!(matches!(
            s.measurements[0],
            Ok(Shape::Number { re: 5., im: 0. })
        ));
        b.move_point("A", Point2 { x: 1., y: 2. }, None).unwrap();
        let s = scene(&b);
        pt(&s, "C", Point2 { x: 4., y: 4. });
        pt(&s, "D", Point2 { x: 1., y: 6. });
        assert!(b.move_point("C", Point2 { x: 0., y: 0. }, None).is_err());
        let saved = serde_json::to_string(&b).unwrap();
        let restored: Book = serde_json::from_str(&saved).unwrap();
        pt(&scene(&restored), "C", Point2 { x: 4., y: 4. });
    }
    #[test]
    fn hyperbola_retains_its_secant_witness_and_exposes_two_branches() {
        let b = book(&[
            ("A", "point(-2,0)"),
            ("B", "point(2,0)"),
            ("C", "point(1,0)"),
            ("D", "hyperbola(GA,GB,GC)"),
            ("E", "element(GD,1)"),
            ("F", "element(GD,2)"),
            ("G", "area(hyperbola(GA,GB,GC))"),
            ("H", "perimeter(GD)"),
            ("K", "area(GD)"),
            ("L", "slope(tangent(GE))"),
            ("I", "element(hyperbola(GA,GB,-1),1)"),
            ("J", "element(hyperbola(GA,GB,point(2,3)),1)"),
        ]);
        let s = scene(&b);
        for (name, p) in [
            ("E", Point2 { x: -2., y: -3. }),
            ("F", Point2 { x: 7., y: 12. }),
            ("I", Point2 { x: 2., y: 3. }),
            ("J", Point2 { x: -1., y: 0. }),
        ] {
            pt(&s, name, p);
        }
        let Shape::Number { re, .. } = shape(&s, "G") else {
            panic!("missing area")
        };
        assert!((re + 338.985132275).abs() < 1e-9);
        assert!(matches!(shape(&s, "H"), Shape::Undefined));
        for (name, expected) in [("K", 0.), ("L", 2.)] {
            let Shape::Number { re, .. } = shape(&s, name) else {
                panic!("missing scalar")
            };
            assert!((re - expected).abs() < 1e-10);
        }
        let v = access::shape_value(&shape(&s, "D")).unwrap();
        assert!(matches!(v,ppl_vm::PortableValue::List(items) if items.len()==2));
    }
    #[test]
    fn focal_parabola_matches_parameters_measurements_and_intersections() {
        let b = book(&[
            ("A", "point(0,1)"),
            ("B", "point(-1,-1)"),
            ("C", "point(1,-1)"),
            ("D", "line(GB,GC)"),
            ("E", "parabola(GA,GD)"),
            ("F", "element(GE,2)"),
            ("G", "slope(tangent(GE,GF))"),
            ("H", "area(GE)"),
            ("I", "perimeter(GE)"),
            ("J", "element(parabola(point(2,3),-2),2)"),
        ]);
        let s = scene(&b);
        pt(&s, "F", Point2 { x: 2., y: 1. });
        pt(&s, "J", Point2 { x: 4., y: 2.5 });
        for (name, value) in [("G", 1.), ("H", -288.), ("I", 77.9767100689)] {
            let Shape::Number { re, .. } = shape(&s, name) else {
                panic!("missing measurement")
            };
            assert!((re - value).abs() < 1e-9);
        }
        let Shape::Parabola(curve) = shape(&s, "E") else {
            panic!("missing curve")
        };
        let mut hits = curve
            .crossings(Point2 { x: 0., y: 1. }, Point2 { x: 1., y: 1. })
            .unwrap();
        hits.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert_eq!(hits, [Point2 { x: -2., y: 1. }, Point2 { x: 2., y: 1. }]);
        assert!(Parabola::vertical(Point2 { x: 0., y: 0. }, 0.).is_err());
    }
    #[test]
    fn triangle_excircles_use_the_first_vertex_to_choose_the_opposite_side() {
        let b = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "point(0,3)"),
            ("D", "excircle(GA,GB,GC)"),
            ("E", "excircle(GB,GC,GA)"),
            ("F", "excircle(GA,GB,GA)"),
        ]);
        let s = scene(&b);
        for (name, expected, r) in [
            ("D", Point2 { x: 6., y: 6. }, 6.),
            ("E", Point2 { x: -2., y: 2. }, 2.),
        ] {
            let Shape::Circle { center, radius } = shape(&s, name) else {
                panic!("missing excircle")
            };
            assert!(center.distance(expected).unwrap() < 1e-10);
            assert!((radius - r).abs() < 1e-10);
        }
        assert!(
            s.objects
                .iter()
                .find(|o| o.name == "F")
                .unwrap()
                .value
                .is_err()
        );
    }
    #[test]
    fn circles_polygons_and_intersections_have_real_geometry() {
        let b = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "point(0,3)"),
            ("D", "triangle(GA,GB,GC)"),
            ("E", "circumcircle(GA,GB,GC)"),
            ("F", "incircle(GA,GB,GC)"),
            ("H", "circle(GA,GB)"),
            ("I", "circle(GA,GB-GA)"),
            ("J", "line(point(-5,0),point(5,0))"),
            ("K", "inter(GJ,GI)"),
            ("L", "single_inter(GJ,GI,point(5,0))"),
        ]);
        let s = scene(&b);
        assert!(matches!(
            shape(&s, "E"),
            Shape::Circle {
                center: Point2 { x: 2., y: 1.5 },
                radius: 2.5
            }
        ));
        assert!(matches!(
            shape(&s, "F"),
            Shape::Circle {
                center: Point2 { x: 1., y: 1. },
                radius: 1.
            }
        ));
        assert!(matches!(
            shape(&s, "H"),
            Shape::Circle {
                center: Point2 { x: 2., y: 0. },
                radius: 2.
            }
        ));
        assert!(matches!(
            shape(&s, "I"),
            Shape::Circle {
                center: Point2 { x: 0., y: 0. },
                radius: 4.
            }
        ));
        match shape(&s, "K") {
            Shape::Points(points) => assert_eq!(
                points,
                vec![Point2 { x: -4., y: 0. }, Point2 { x: 4., y: 0. }]
            ),
            v => panic!("{v:?}"),
        }
        pt(&s, "L", Point2 { x: 4., y: 0. });
    }
    #[test]
    fn line_constraints_measurements_and_bounded_intersections() {
        let mut b = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "point(1,3)"),
            ("D", "segment(GA,GB)"),
            ("E", "perpendicular(GC,GD)"),
            ("F", "single_inter(GD,GE)"),
            ("H", "reflection(GD,GC)"),
            ("I", "perpen_bisector(GA,GB)"),
        ]);
        b.measurements.push(Measurement {
            source: "distance(point(6,3),GD)".into(),
            ..Default::default()
        });
        let s = scene(&b);
        pt(&s, "F", Point2 { x: 1., y: 0. });
        pt(&s, "H", Point2 { x: 1., y: -3. });
        match &s.measurements[0] {
            Ok(Shape::Number { re, .. }) => assert!((re - 13_f64.sqrt()).abs() < 1e-10),
            v => panic!("{v:?}"),
        }
        b.objects.push(Object {
            name: "J".into(),
            source: "single_inter(GD,line(point(6,0),point(6,3)))".into(),
            ..Default::default()
        });
        assert!(scene(&b).objects.last().unwrap().value.is_err());
    }
    #[test]
    fn bad_dependencies_and_degeneracy_never_reuse_old_coordinates() {
        let mut b = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "line(GA,GB)"),
        ]);
        b.move_point("B", Point2 { x: 0., y: 0. }, None).unwrap();
        assert!(scene(&b).objects[2].value.is_err());
        let cycle = book(&[
            ("A", "midpoint(GB,point(0))"),
            ("B", "midpoint(GA,point(4))"),
        ]);
        assert!(
            scene(&cycle).objects.iter().all(|o| o
                .value
                .as_ref()
                .unwrap_err()
                .contains("Circular"))
        );
        let dangling = book(&[("A", "midpoint(GMISSING,point(0))")]);
        assert!(scene(&dangling).objects[0].value.is_err());
        assert!(
            b.calculate(
                &ppl_vm::SessionContext::default().snapshot(),
                Arc::new(AtomicBool::new(true))
            )
            .is_err()
        );
    }
    #[test]
    fn numeric_subexpressions_use_real_vm_app_settings_and_reject_memory_writes() {
        let mut memory = ppl_vm::SessionContext::default();
        memory
            .store_program("GEOFN", "EXPORT GEOFN(X) BEGIN RETURN X+2; END;".into())
            .unwrap();
        memory
            .store_program("GEOBAD", "EXPORT GEOBAD() BEGIN Z:=99;RETURN 1;END;".into())
            .unwrap();
        let mut snapshot = memory.snapshot();
        snapshot.app_settings.insert(
            "Geometry".into(),
            ppl_vm::AppSettings {
                angle: 2,
                ..Default::default()
            },
        );
        let b = book(&[
            ("A", "point(GEOFN(3),SIN(30))"),
            ("B", "point(GEOBAD(),2)"),
            ("C", "point(1E-400,2)"),
        ]);
        let s = b
            .calculate(&snapshot, Arc::new(AtomicBool::new(false)))
            .unwrap();
        pt(&s, "A", Point2 { x: 5., y: 0.5 });
        assert!(
            s.objects[1]
                .value
                .as_ref()
                .unwrap_err()
                .contains("cannot change")
        );
        assert!(s.objects[2].value.is_err());
        assert!(
            !memory.snapshot().globals.contains_key("Z")
                || memory.snapshot().globals["Z"] == snapshot.globals["Z"]
        );
    }
    #[test]
    fn renaming_rewrites_geometry_references_without_changing_home_identifiers() {
        let mut memory = ppl_vm::SessionContext::default();
        memory
            .set_variable(
                "A",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(3)),
            )
            .unwrap();
        let mut b = book(&[("A", "point(A,4)"), ("B", "midpoint(GA,point(7,6))")]);
        b.measurements.push(Measurement {
            source: "distance(GA,0)".into(),
            ..Default::default()
        });
        b.rename("A", "GK").unwrap();
        assert_eq!(b.objects[0].name, "K");
        assert_eq!(b.objects[0].source, "point(A,4)");
        assert_eq!(b.objects[1].source, "midpoint(GK,point(7,6))");
        assert_eq!(b.measurements[0].source, "distance(GK,0)");
        let s = b
            .calculate(&memory.snapshot(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        pt(&s, "K", Point2 { x: 3., y: 4. });
        pt(&s, "B", Point2 { x: 5., y: 5. });
        let before = serde_json::to_value(&b).unwrap();
        assert!(b.rename("K", "GB").is_err());
        assert_eq!(serde_json::to_value(&b).unwrap(), before);
    }
    #[test]
    fn explicit_geometry_vectors_work_when_automatic_complex_results_are_disabled() {
        let b = book(&[
            ("A", "point(0,0)"),
            ("B", "point(3,4)"),
            ("C", "circle(GA,GB-GA)"),
        ]);
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.settings.complex_results = false;
        let s = b
            .calculate(&snapshot, Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert!(matches!(shape(&s, "C"), Shape::Circle { radius: 5., .. }));
        assert!(!snapshot.settings.complex_results);
    }
    #[test]
    fn empty_book_is_valid_and_failed_edits_are_atomic() {
        assert!(scene(&Book::default()).objects.is_empty());
        let b = book(&[("A", "point(3,4)")]);
        let before = serde_json::to_value(&b).unwrap();
        assert!(
            b.enter(
                "A",
                "midpoint(GA,point(0))",
                &ppl_vm::SessionContext::default().snapshot(),
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(&b).unwrap(), before);
        let legacy = Geometry {
            points: std::collections::BTreeMap::from([
                ("P1".into(), Point2 { x: 0., y: 0. }),
                ("P2".into(), Point2 { x: 3., y: 4. }),
            ]),
            segments: vec![["P1".into(), "P2".into()]],
        };
        let imported = Book::from_legacy(&legacy).unwrap();
        assert_eq!(imported.objects.len(), 3);
        assert!(scene(&imported).objects.iter().all(|o| o.value.is_ok()));
    }
}
