//! Dependency-aware Geometry definitions, evaluated off the UI thread.
//! HP Prime User Guide chapter9: Symbolic objects and Numeric measurements.
mod access;
mod arc;
mod ellipse;
mod evaluate;
mod function;
pub mod plot;
pub mod primitives;
use super::native_app_models::Geometry;
pub use super::native_app_models::Point2;
pub use arc::CircularArc;
pub use ellipse::Ellipse;
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
    pub(crate) current_values: BTreeMap<String, CachedObject>,
    /// Snapshots of the curves that produced the current constrained points.
    #[serde(skip)]
    pub current_point_curves: BTreeMap<String, Shape>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CachedObject {
    Portable(ppl_vm::PortableValue),
    // Native curve data can contain details beyond the calculator-visible
    // symbolic representation.
    Curve(Box<Shape>),
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Extent {
    Segment,
    Ray,
    Line,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
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
impl Shape {
    pub(super) fn forms_arc_intersection_pair(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Arc(_), Self::Circle { .. } | Self::Arc(_))
                | (Self::Circle { .. }, Self::Arc(_))
        )
    }
}
impl Scene {
    pub(super) fn forms_arc_intersection_pair(&self, a: &str, b: &str) -> bool {
        let find = |name| {
            self.objects
                .iter()
                .find(|object| object.name == name)
                .and_then(|object| object.value.as_ref().ok())
        };
        find(a)
            .zip(find(b))
            .is_some_and(|(a, b)| a.forms_arc_intersection_pair(b))
    }
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
    pub(super) fn named_point_curve(&self, name: &str) -> Option<String> {
        let object = self.objects.iter().find(|object| object.name == name)?;
        evaluate::named_element_curve(&object.source)
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
pub(super) fn source(source: &str) -> Result<(), String> {
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
    fn reflecting_circles_and_segments_keeps_their_geometry_and_live_dependencies() {
        let mut construction = book(&[
            ("A", "line(point(-8,0),point(8,0))"),
            ("B", "point(2,3)"),
            ("C", "circle(GB,2)"),
            ("D", "reflection(GA,GC)"),
            ("E", "reflection(point(0,0),GC)"),
            ("G", "reflection(GA,segment(point(2,1),point(4,1)))"),
            ("H", "element(GG,0)"),
            ("I", "element(GG,1)"),
        ]);
        for (source, reflected) in [
            ("point(2,3)", [[2., -3.], [-2., -3.]]),
            ("point(3,4)", [[3., -4.], [-3., -4.]]),
        ] {
            construction.objects[1].source = source.into();
            let result = scene(&construction);
            for (name, [x, y]) in ["D", "E"].into_iter().zip(reflected) {
                let object = result
                    .objects
                    .iter()
                    .find(|object| object.name == name)
                    .unwrap();
                assert!(matches!(object.value, Ok(Shape::Circle { center, radius })
                    if center == Point2{x,y} && radius == 2.));
            }
            pt(&result, "H", Point2 { x: 2., y: -1. });
            pt(&result, "I", Point2 { x: 4., y: -1. });
        }
    }

    #[test]
    fn reflecting_a_polygon_preserves_vertex_order_and_signed_area() {
        let mut construction = book(&[
            ("A", "line(point(-8,0),point(8,0))"),
            ("B", "point(3,3)"),
            ("C", "triangle(point(1,1),point(1,3),GB)"),
            ("D", "reflection(GA,GC)"),
            ("E", "reflection(point(0,0),GC)"),
            ("F", "area(GC)"),
            ("G", "area(GD)"),
            ("H", "area(GE)"),
        ]);
        for (coordinate, area) in [(3., 2.), (4., 3.)] {
            construction.objects[1].source = format!("point({coordinate},{coordinate})");
            let objects = scene(&construction).objects;
            let expected = [
                Point2 { x: 1., y: -1. },
                Point2 { x: 1., y: -3. },
                Point2 {
                    x: coordinate,
                    y: -coordinate,
                },
            ];
            assert!(matches!(&objects[3].value, Ok(Shape::Polygon(points)) if points == &expected));
            for (index, signed_area) in [(5, -area), (6, area), (7, -area)] {
                assert!(
                    matches!(objects[index].value, Ok(Shape::Number { re, im: 0. })
                    if re == signed_area)
                );
            }
        }
    }

    #[test]
    fn quadrilateral_preserves_point_order_and_four_point_values() {
        let mut construction = book(&[
            ("A", "point(-6,-3)"),
            ("B", "point(0,-1)"),
            ("C", "point(2,3)"),
            ("D", "point(-3,5)"),
            ("E", "point(6,1)"),
            ("F", "quadrilateral(GA,GB,GC,GD)"),
            ("G", "quadrilateral(GA,GD,GC,GB)"),
            ("H", "quadrilateral(GA,GB,GE,GA)"),
            ("I", "quadrilateral(GA,GB,GC)"),
            ("J", "triangle(GA,GB,GE)"),
            ("K", "polygon(GA,GB,GE)"),
        ]);
        for source in ["area(GF)", "perimeter(GF)", "area(GG)", "area(GH)"] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        assert_eq!(
            shape(&result, "F"),
            Shape::Polygon(vec![
                Point2 { x: -6., y: -3. },
                Point2 { x: 0., y: -1. },
                Point2 { x: 2., y: 3. },
                Point2 { x: -3., y: 5. },
            ])
        );
        for (actual, expected) in result
            .measurements
            .iter()
            .zip([33., 24.7258598278, -33., 0.])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
        assert!(
            result.objects[8..]
                .iter()
                .all(|object| object.value.is_err())
        );

        let PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(exported)) =
            access::shape_value(&shape(&result, "H")).unwrap()
        else {
            panic!("Expected a closed polygon literal");
        };
        let value = construction.current_expression(
            &format!("area({exported})"),
            &SessionContext::default().snapshot(),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(value, Ok(PortableValue::Real(ppl_math::Real::ZERO)));

        construction.objects[2].source = "point(4,2)".into();
        let moved = scene(&construction);
        for (actual, expected) in moved
            .measurements
            .iter()
            .zip([37.5, 27.4843321715, -37.5, 0.])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
    }

    #[test]
    fn rectangle_projects_the_height_and_preserves_signed_measurements() {
        let mut construction = book(&[
            ("A", "point(-5,-2)"),
            ("B", "point(1,0)"),
            ("C", "point(4,4)"),
            ("D", "point(2,-4)"),
            ("E", "point(7,2)"),
            ("F", "rectangle(GA,GB,GC)"),
            ("G", "rectangle(GA,GB,GD)"),
            ("H", "rectangle(GA,GB,GE)"),
            ("I", "rectangle(GA,GB,2)"),
            ("J", "rectangle(GA,GA,GC)"),
            ("K", "rectangle(GA,GB)"),
        ]);
        for source in [
            "area(GF)",
            "perimeter(GF)",
            "area(GG)",
            "perimeter(GG)",
            "area(GH)",
            "area(GI)",
        ] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        let Shape::Polygon(vertices) = shape(&result, "F") else {
            panic!("Expected a rectangle with ordered vertices");
        };
        assert_eq!(vertices.len(), 4);
        for (p, (x, y)) in vertices
            .into_iter()
            .zip([(-5., -2.), (1., 0.), (0.1, 2.7), (-5.9, 0.7)])
        {
            assert!(p.distance(Point2 { x, y }).unwrap() < 1e-10);
        }
        assert!(
            result.objects[9..]
                .iter()
                .all(|object| object.value.is_err())
        );
        for (actual, expected) in
            result
                .measurements
                .iter()
                .zip([18., 18.341210429, -26., 20.8710325571, 0., 80.])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
        let PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(exported)) =
            access::shape_value(&shape(&result, "H")).unwrap()
        else {
            panic!("Expected a closed zero-height rectangle");
        };
        assert_eq!(
            construction.current_expression(
                &format!("area({exported})"),
                &SessionContext::default().snapshot(),
                Arc::new(AtomicBool::new(false)),
            ),
            Ok(PortableValue::Real(ppl_math::Real::ZERO))
        );
        construction.objects[2].source = "point(5,6)".into();
        for (actual, expected) in scene(&construction)
            .measurements
            .iter()
            .zip([28., 21.5034880891])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
    }

    #[test]
    fn parallelogram_keeps_corner_order_and_updates_its_derived_vertex() {
        let mut construction = book(&[
            ("A", "point(-5,-2)"),
            ("B", "point(1,0)"),
            ("C", "point(4,4)"),
            ("D", "point(2,-4)"),
            ("E", "point(7,2)"),
            ("F", "parallelogram(GA,GB,GC)"),
            ("G", "vertices(GF,3)"),
            ("H", "parallelogram(GA,GB,GD)"),
            ("I", "parallelogram(GA,GB,GE)"),
            ("J", "parallelogram(GA,GA,GC)"),
            ("K", "parallelogram(GA,GB)"),
            ("L", "vertices(GF,-1)"),
            ("M", "vertices(GF,1.5)"),
        ]);
        for source in [
            "area(GF)",
            "perimeter(GF)",
            "area(GH)",
            "perimeter(GH)",
            "area(GI)",
            "area(GJ)",
        ] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        assert_eq!(
            shape(&result, "F"),
            Shape::Polygon(vec![
                Point2 { x: -5., y: -2. },
                Point2 { x: 1., y: 0. },
                Point2 { x: 4., y: 4. },
                Point2 { x: -2., y: 2. },
            ])
        );
        pt(&result, "G", Point2 { x: -2., y: 2. });
        assert!(
            result.objects[10..]
                .iter()
                .all(|object| object.value.is_err())
        );
        for (actual, expected) in
            result
                .measurements
                .iter()
                .zip([18., 22.6491106407, -26., 20.8953218919, 0., 0.])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
        construction.objects[2].source = "point(5,6)".into();
        let moved = scene(&construction);
        pt(&moved, "G", Point2 { x: -1., y: 4. });
        for (actual, expected) in moved.measurements.iter().zip([28., 27.0713157425]) {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
    }

    #[test]
    fn polygon_keeps_two_or_more_vertices_and_round_trips_the_pair() {
        let mut construction = book(&[
            ("A", "point(-7,-3)"),
            ("B", "point(-1,-3)"),
            ("C", "point(3,1)"),
            ("D", "point(0,5)"),
            ("E", "point(-6,4)"),
            ("F", "polygon(GA,GB,GC,GD,GE)"),
            ("G", "polygon(GA,GB)"),
            ("H", "polygon(GA,GB,GA)"),
            ("I", "polygon(GA)"),
        ]);
        for source in ["area(GF)", "perimeter(GF)", "area(GG)", "perimeter(GG)"] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        assert_eq!(
            shape(&result, "F"),
            Shape::Polygon(vec![
                Point2 { x: -7., y: -3. },
                Point2 { x: -1., y: -3. },
                Point2 { x: 3., y: 1. },
                Point2 { x: 0., y: 5. },
                Point2 { x: -6., y: 4. },
            ])
        );
        assert_eq!(shape(&result, "G"), shape(&result, "H"));
        assert!(result.objects.last().unwrap().value.is_err());
        for (actual, expected) in result
            .measurements
            .iter()
            .zip([58.5, 29.8106845917, 0., 12.])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
        let PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(exported)) =
            access::shape_value(&shape(&result, "H")).unwrap()
        else {
            panic!("Expected a closed polygon literal");
        };
        assert_eq!(exported, "polygon(point(-7,-3),point(-1,-3),point(-7,-3))");
        construction.objects[2].source = "point(4,0)".into();
        for (actual, expected) in scene(&construction)
            .measurements
            .iter()
            .zip([63., 31.3879064744])
        {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
    }

    #[test]
    fn rotation_preserves_shape_measurements_and_live_references() {
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "circle(point(5,2),2)"),
            ("C", "triangle(point(1,1),point(1,3),point(3,3))"),
            ("D", "ellipse(point(-6,-3),point(-2,-3),4)"),
            ("E", "ellipse(point(0,2),point(4,2),point(2,5))"),
            ("F", "rotation(GA,π/2,GB)"),
            ("G", "rotation(point(0,0),π/2,GC)"),
            ("H", "rotation(GA,π/2,GD)"),
            ("I", "rotation(GA,-π/2,GE)"),
            ("J", "center(GF)"),
            ("K", "element(GH,0)"),
            ("L", "element(GI,0)"),
            ("M", "rotation(GA,π/2,point(5,2))"),
        ]);
        for source in [
            "radius(GF)",
            "area(GF)",
            "perimeter(GF)",
            "area(GG)",
            "perimeter(GG)",
            "area(GH)",
            "perimeter(GH)",
            "area(GI)",
            "perimeter(GI)",
        ] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        pt(&result, "J", Point2 { x: -2., y: 3. });
        pt(&result, "K", Point2 { x: -8., y: -3. });
        pt(&result, "L", Point2 { x: 2., y: 5. });
        pt(&result, "M", Point2 { x: -2., y: 3. });
        let Shape::Polygon(vertices) = shape(&result, "G") else {
            panic!("Expected a rotated polygon");
        };
        for (p, (x, y)) in vertices.into_iter().zip([(-1., 1.), (-3., 1.), (-3., 3.)]) {
            assert!(p.distance(Point2 { x, y }).unwrap() < 1e-10);
        }
        let Shape::Ellipse(ellipse) = shape(&result, "H") else {
            panic!("Expected a rotated ellipse");
        };
        for (p, y) in ellipse.foci.into_iter().zip([-8., -4.]) {
            assert!(p.distance(Point2 { x: 3., y }).unwrap() < 1e-10);
        }
        for (actual, expected) in result.measurements.into_iter().zip([
            2.,
            12.5663706144,
            12.5663706144,
            -2.,
            6.82842712475,
            43.5311847416,
            23.4793953494,
            33.9815201974,
            20.7955739243,
        ]) {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. })
                if (re - expected).abs() < 1e-7),
                "{actual:?}"
            );
        }
        construction.objects[0].source = "point(2,-1)".into();
        pt(&scene(&construction), "J", Point2 { x: -1., y: 2. });
    }

    #[test]
    fn rotation_preserves_straight_extents_and_circular_arc_bounds() {
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "segment(point(3,1),point(5,4))"),
            ("C", "half_line(point(3,1),point(5,4))"),
            ("D", "circle(point(1,3),point(5,3),-π/4,π/4)"),
            ("E", "circle(point(1,3),point(5,3),π/4,-π/4)"),
            ("F", "rotation(GA,π/2,GB)"),
            ("G", "rotation(GA,π/2,GC)"),
            ("H", "rotation(GA,π/2,GD)"),
            ("I", "rotation(GA,-π/2,GE)"),
            ("J", "element(GH,0)"),
            ("K", "center(GH)"),
            ("L", "element(GF,0)"),
        ]);
        let result = scene(&construction);
        for (name, expected_extent) in [("F", Extent::Segment), ("G", Extent::Ray)] {
            let Shape::Line { a, b, extent } = shape(&result, name) else {
                panic!("Expected a rotated straight object");
            };
            assert_eq!(extent, expected_extent);
            assert!(a.distance(Point2 { x: -1., y: 1. }).unwrap() < 1e-10);
            assert!(b.distance(Point2 { x: -4., y: 3. }).unwrap() < 1e-10);
        }
        for (before, after, sign) in [("D", "H", 1.), ("E", "I", -1.)] {
            let Shape::Arc(before) = shape(&result, before) else {
                panic!("Expected the source arc");
            };
            let Shape::Arc(after) = shape(&result, after) else {
                panic!("Expected a rotated arc");
            };
            assert_eq!(after.start, before.start);
            assert_eq!(after.end, before.end);
            assert_eq!(after.radius, before.radius);
            let measurement = sign * std::f64::consts::PI;
            assert!((after.perimeter() - measurement).abs() < 1e-10);
            assert!((after.area() - measurement).abs() < 1e-10);
        }
        pt(&result, "J", Point2 { x: -1., y: 1. });
        pt(&result, "K", Point2 { x: -3., y: 1. });
        pt(&result, "L", Point2 { x: -1., y: 1. });
        construction.objects[0].source = "point(2,-1)".into();
        let moved = scene(&construction);
        pt(&moved, "K", Point2 { x: -2., y: 0. });
        pt(&moved, "L", Point2 { x: 0., y: 0. });
    }

    #[test]
    fn rotation_keeps_infinite_line_extent_and_dependent_intersections() {
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "point(3,1)"),
            ("C", "line(GB,point(5,4))"),
            ("D", "line(point(-2,-5),point(-2,10))"),
            ("E", "rotation(GA,1,GC)"),
            ("F", "single_inter(GE,GD)"),
            ("G", "rotation(GA,-1,GC)"),
            ("H", "element(GE,2)"),
            ("I", "slope(GE)"),
            ("J", "slope(GG)"),
        ]);
        let original = scene(&construction);
        for name in ["E", "G"] {
            assert!(matches!(
                shape(&original, name),
                Shape::Line {
                    extent: Extent::Line,
                    ..
                }
            ));
        }
        for (name, expected) in [("I", -2.28828771073), ("J", -0.0172079749607)] {
            assert!(matches!(
                shape(&original, name),
                Shape::Number { re, im: 0. } if (re - expected).abs() < 1e-10
            ));
        }
        pt(
            &original,
            "H",
            Point2 {
                x: -2.48995404325,
                y: 8.37124435579,
            },
        );
        pt(
            &original,
            "F",
            Point2 {
                x: -2.,
                y: 7.25008853979,
            },
        );
        construction.objects[1].source = "point(4,-1)".into();
        pt(
            &scene(&construction),
            "F",
            Point2 {
                x: -2.,
                y: 5.98897730986,
            },
        );
    }

    #[test]
    fn translation_preserves_arc_bounds_radius_and_a_dependent_point() {
        let mut construction = book(&[
            ("A", "point(0,-5)"),
            ("B", "point(3,-3)"),
            ("C", "segment(GA,GB)"),
            ("D", "circle(point(-8,3),point(-4,3),-π/4,π/4)"),
            ("E", "translation(GC,GD)"),
            ("F", "element(GE,0)"),
            ("G", "circle(point(-8,3),point(-4,3),π/4,-π/4)"),
            ("H", "translation(GC,GG)"),
            ("I", "translation(affix(GA)-affix(GB),GD)"),
            ("J", "translation(GD,GC)"),
        ]);
        let result = scene(&construction);
        for (source, moved, center, sector) in [
            ("D", "E", Point2 { x: -3., y: 5. }, std::f64::consts::PI),
            ("G", "H", Point2 { x: -3., y: 5. }, -std::f64::consts::PI),
            ("D", "I", Point2 { x: -9., y: 1. }, std::f64::consts::PI),
        ] {
            let Shape::Arc(source) = shape(&result, source) else {
                panic!("Expected a source arc");
            };
            let Shape::Arc(arc) = shape(&result, moved) else {
                panic!("Expected a translated arc");
            };
            assert_eq!(arc.center, center);
            assert_eq!(arc.radius, source.radius);
            assert_eq!((arc.start, arc.end), (source.start, source.end));
            assert!((arc.area() - sector).abs() < 1e-10);
            assert!((arc.perimeter() - sector).abs() < 1e-10);
        }
        assert!(result.objects.last().unwrap().value.is_err());
        pt(&result, "F", Point2 { x: -1., y: 5. });
        construction.objects[1].source = "point(5,-6)".into();
        pt(&scene(&construction), "F", Point2 { x: 1., y: 2. });
    }

    #[test]
    fn dilation_preserves_arc_bounds_and_scales_sector_measurements() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "circle(point(-8,3),point(-4,3),-π/4,π/4)"),
            ("C", "homothety(GA,-0.5,GB)"),
            ("D", "element(GC,0)"),
            ("E", "homothety(GA,2,GB)"),
            ("F", "circle(point(-8,3),point(-4,3),π/4,-π/4)"),
            ("G", "homothety(GA,-0.5,GF)"),
            ("H", "homothety(GA,0,GB)"),
        ]);
        let before = scene(&construction);
        for (source, dilated, center, radius, area, perimeter) in [
            (
                "B",
                "C",
                Point2 { x: 4.5, y: -3. },
                1.,
                FRAC_PI_4,
                FRAC_PI_2,
            ),
            ("B", "E", Point2 { x: -13., y: 7. }, 4., 4. * PI, TAU),
            (
                "F",
                "G",
                Point2 { x: 4.5, y: -3. },
                1.,
                -FRAC_PI_4,
                -FRAC_PI_2,
            ),
        ] {
            let Shape::Arc(source) = shape(&before, source) else {
                panic!("Expected a source arc");
            };
            let Shape::Arc(arc) = shape(&before, dilated) else {
                panic!("Expected a dilated arc");
            };
            assert_eq!(arc.center, center);
            assert_eq!(arc.radius, radius);
            assert_eq!((arc.start, arc.end), (source.start, source.end));
            assert!((arc.area() - area).abs() < 1e-10);
            assert!((arc.perimeter() - perimeter).abs() < 1e-10);
        }
        assert!(before.objects.last().unwrap().value.is_err());
        pt(&before, "D", Point2 { x: 5.5, y: -3. });
        construction.objects[1].source = "circle(point(-8,5),point(-4,5),-π/4,π/4)".into();
        pt(&scene(&construction), "D", Point2 { x: 5.5, y: -4. });
    }

    #[test]
    fn dilation_preserves_an_infinite_line_and_recomputes_its_intersection() {
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "point(3,1)"),
            ("C", "line(GB,point(5,4))"),
            ("D", "line(point(-2,-5),point(-2,10))"),
            ("E", "homothety(GA,-0.5,GC)"),
            ("F", "homothety(GA,2,GC)"),
            ("G", "single_inter(GE,GD)"),
            ("H", "element(GE,-1)"),
            ("I", "homothety(GA,0,GC)"),
        ]);
        let before = scene(&construction);
        assert_eq!(
            shape(&before, "E"),
            Shape::Line {
                a: Point2 { x: 0., y: -2. },
                b: Point2 { x: -1., y: -3.5 },
                extent: Extent::Line,
            }
        );
        assert_eq!(
            shape(&before, "F"),
            Shape::Line {
                a: Point2 { x: 5., y: 3. },
                b: Point2 { x: 9., y: 9. },
                extent: Extent::Line,
            }
        );
        pt(&before, "G", Point2 { x: -2., y: -5. });
        pt(&before, "H", Point2 { x: 1., y: -0.5 });
        assert_eq!(
            before.objects.last().unwrap().value,
            Err("A line needs two distinct points".into())
        );

        construction.objects[1].source = "point(4,-1)".into();
        let moved = scene(&construction);
        pt(&moved, "G", Point2 { x: -2., y: -8.5 });
        pt(&moved, "H", Point2 { x: 0., y: 1.5 });
    }

    #[test]
    fn dilation_preserves_ordered_segment_and_ray_endpoints() {
        let construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "segment(point(3,1),point(5,4))"),
            ("C", "half_line(point(3,1),point(5,4))"),
            ("D", "homothety(GA,-0.5,GB)"),
            ("E", "homothety(GA,-0.5,GC)"),
            ("F", "homothety(GA,2,GC)"),
            ("G", "homothety(GA,0,GB)"),
            ("H", "homothety(GA,0,GC)"),
        ]);
        let result = scene(&construction);
        for (name, extent) in [("D", Extent::Segment), ("E", Extent::Ray)] {
            assert_eq!(
                shape(&result, name),
                Shape::Line {
                    a: Point2 { x: 0., y: -2. },
                    b: Point2 { x: -1., y: -3.5 },
                    extent,
                }
            );
        }
        assert_eq!(
            shape(&result, "F"),
            Shape::Line {
                a: Point2 { x: 5., y: 3. },
                b: Point2 { x: 9., y: 9. },
                extent: Extent::Ray,
            }
        );
        for object in &result.objects[6..] {
            assert_eq!(object.value, Err("A line needs two distinct points".into()));
        }
    }

    #[test]
    fn dilation_scales_dependent_shapes_and_preserves_zero_factor_results() {
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "point(5,2)"),
            ("C", "circle(GB,2)"),
            ("D", "triangle(point(1,1),point(1,3),point(3,3))"),
            ("E", "ellipse(point(-6,-3),point(-2,-3),4)"),
            ("F", "ellipse(point(0,2),point(4,2),point(2,5))"),
            ("G", "homothety(GA,-0.5,GC)"),
            ("H", "homothety(GA,-0.5,GD)"),
            ("I", "homothety(GA,-0.5,GE)"),
            ("J", "homothety(GA,2,GF)"),
            ("K", "homothety(GA,2,GB)"),
            ("L", "center(GG)"),
            ("M", "element(GI,0)"),
            ("N", "element(GJ,0)"),
            ("O", "homothety(GA,0,GC)"),
            ("P", "homothety(GA,0,GD)"),
            ("Q", "homothety(GA,0,GE)"),
            ("R", "homothety(GA,0,GB)"),
            ("S", "homothety(point(0,0),2,GP)"),
            ("T", "homothety(point(0,0),1E308,circle(point(0,0),2))"),
        ]);
        for source in [
            "radius(GG)",
            "area(GG)",
            "perimeter(GG)",
            "area(GH)",
            "perimeter(GH)",
            "area(GI)",
            "perimeter(GI)",
            "area(GJ)",
            "perimeter(GJ)",
            "area(GP)",
            "perimeter(GP)",
            "area(polygon(point(1,-1),point(1,-1),point(1,-1),point(1,-1)))",
        ] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        pt(&result, "K", Point2 { x: 9., y: 5. });
        pt(&result, "L", Point2 { x: -1., y: -2.5 });
        pt(&result, "M", Point2 { x: -8., y: -3. });
        pt(&result, "N", Point2 { x: 2., y: 5. });
        let center = Point2 { x: 1., y: -1. };
        for name in ["O", "Q", "R"] {
            pt(&result, name, center);
        }
        assert_eq!(shape(&result, "P"), Shape::Polygon(vec![center; 3]));
        assert_eq!(
            shape(&result, "H"),
            Shape::Polygon(vec![
                Point2 { x: 1., y: -2. },
                Point2 { x: 1., y: -3. },
                Point2 { x: 0., y: -3. },
            ])
        );
        assert_eq!(
            result.objects.last().unwrap().value,
            Err("Geometry value exceeds numerical range".into())
        );
        assert_eq!(
            shape(&result, "S"),
            Shape::Polygon(vec![Point2 { x: 2., y: -2. }; 3])
        );
        for (actual, expected) in result.measurements.into_iter().zip([
            1.,
            std::f64::consts::PI,
            std::f64::consts::TAU,
            -0.5,
            3.41421356237,
            10.8827961854,
            11.7396976747,
            135.9260807896,
            41.5911478486,
            0.,
            0.,
            0.,
        ]) {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. }) if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
        construction.objects[1].source = "point(7,4)".into();
        let moved = scene(&construction);
        pt(&moved, "K", Point2 { x: 13., y: 9. });
        pt(&moved, "L", Point2 { x: -2., y: -3.5 });
    }

    #[test]
    fn translation_uses_the_segment_direction_and_preserves_shape_measurements() {
        let mut construction = book(&[
            ("A", "point(1,-1)"),
            ("B", "point(4,1)"),
            ("C", "circle(point(5,2),2)"),
            ("D", "segment(GA,GB)"),
            ("E", "polygon(point(-5,1),point(-5,3),point(-3,3))"),
            ("F", "ellipse(point(-6,-3),point(-2,-3),4)"),
            ("G", "ellipse(point(0,2),point(4,2),point(2,5))"),
            ("H", "translation(GD,GC)"),
            ("I", "translation(GB-GA,point(5,2))"),
            ("J", "translation(GD,GE)"),
            ("K", "translation(GD,GF)"),
            ("L", "translation(GD,GG)"),
            ("M", "center(GH)"),
            ("N", "element(GK,0)"),
            ("O", "translation(0,GC)"),
            ("P", "translation(GA,GC)"),
            ("Q", "translation(GC,GB-GA)"),
            ("R", "translation(affix(GB)-affix(GA),point(5,2))"),
        ]);
        for source in [
            "radius(GH)",
            "area(GJ)",
            "perimeter(GJ)",
            "area(GK)",
            "perimeter(GK)",
            "area(GL)",
            "perimeter(GL)",
        ] {
            construction.measurements.push(Measurement {
                source: source.into(),
                ..Default::default()
            });
        }
        let result = scene(&construction);
        for name in ["I", "M", "R"] {
            pt(&result, name, Point2 { x: 8., y: 4. });
        }
        pt(&result, "N", Point2 { x: -8., y: -3. });
        assert_eq!(shape(&result, "O"), shape(&result, "C"));
        assert_eq!(
            shape(&result, "J"),
            Shape::Polygon(vec![
                Point2 { x: -2., y: 3. },
                Point2 { x: -2., y: 5. },
                Point2 { x: 0., y: 5. },
            ])
        );
        let Shape::Ellipse(ellipse) = shape(&result, "L") else {
            panic!("Expected a translated ellipse");
        };
        assert_eq!(
            ellipse.size,
            ellipse::Size::Witness(Point2 { x: 5., y: 7. })
        );
        for name in ["P", "Q"] {
            let value = &result
                .objects
                .iter()
                .find(|object| object.name == name)
                .unwrap()
                .value;
            assert!(value.is_err(), "{value:?}");
        }
        for (actual, expected) in result.measurements.into_iter().zip([
            2.,
            -2.,
            6.82842712475,
            43.5311847416,
            23.4793953494,
            33.9815201974,
            20.7955739243,
        ]) {
            assert!(
                matches!(actual, Ok(Shape::Number { re, im: 0. }) if (re - expected).abs() < 1e-8),
                "{actual:?}"
            );
        }
        construction.objects[1].source = "point(2,4)".into();
        let moved = scene(&construction);
        for name in ["I", "M", "R"] {
            pt(&moved, name, Point2 { x: 6., y: 7. });
        }
    }

    #[test]
    fn translation_preserves_line_ray_and_segment_extents() {
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(3,2)"),
            ("C", "segment(GA,GB)"),
            ("D", "line(point(5,-4),point(7,0))"),
            ("E", "line(point(12,-9),point(12,9))"),
            ("F", "segment(point(-9,1),point(-7,2))"),
            ("G", "half_line(point(-5,-5),point(-3,-4))"),
            ("H", "translation(GC,GD)"),
            ("I", "translation(GC,GF)"),
            ("J", "translation(GC,GG)"),
            ("K", "single_inter(GH,GE)"),
            ("L", "element(GH,2)"),
            ("M", "element(GI,2)"),
            ("N", "element(GJ,-1)"),
            ("O", "element(GJ,2)"),
            ("P", "translation(GD,GF)"),
        ]);
        let initial = scene(&construction);
        for (name, extent, (ax, ay), (bx, by)) in [
            ("H", Extent::Line, (8., -2.), (10., 2.)),
            ("I", Extent::Segment, (-6., 3.), (-4., 4.)),
            ("J", Extent::Ray, (-2., -3.), (0., -2.)),
        ] {
            assert_eq!(
                shape(&initial, name),
                Shape::Line {
                    a: Point2 { x: ax, y: ay },
                    b: Point2 { x: bx, y: by },
                    extent,
                }
            );
        }
        assert!(initial.objects.last().unwrap().value.is_err());
        for (name, x, y) in [
            ("K", 12., 6.),
            ("L", 12., 6.),
            ("M", -4., 4.),
            ("N", -2., -3.),
            ("O", 2., -1.),
        ] {
            pt(&initial, name, Point2 { x, y });
        }
        construction.objects[1].source = "point(1,1)".into();
        let moved = scene(&construction);
        for (name, x, y) in [("K", 12., 9.), ("M", -6., 3.), ("O", 0., -2.)] {
            pt(&moved, name, Point2 { x, y });
        }
    }

    #[test]
    fn reflected_arcs_keep_the_measured_angles_and_signed_sweeps() {
        let construction = book(&[
            ("A", "line(point(0,0),point(4,4))"),
            ("B", "circle(point(1,5),point(5,5),π/4,3*π/4)"),
            ("C", "reflection(GA,GB)"),
            ("D", "reflection(point(0,0),GB)"),
        ]);
        let scene = scene(&construction);
        let arc = |index: usize| match &scene.objects[index].value {
            Ok(Shape::Arc(arc)) => *arc,
            value => panic!("Expected a circular arc, got {value:?}"),
        };
        let original = arc(1);
        let mirror = arc(2);
        let middle = arc(3);
        assert!(mirror.center.distance(Point2 { x: 5., y: 3. }).unwrap() < 1e-12);
        assert_eq!(middle.center, Point2 { x: -3., y: -5. });
        assert_eq!((mirror.start, mirror.end), (-original.start, -original.end));
        assert_eq!((middle.start, middle.end), (original.start, original.end));
        assert_eq!(mirror.radius, original.radius);
        assert_eq!(middle.radius, original.radius);
        assert_eq!(mirror.perimeter(), -original.perimeter());
        assert_eq!(middle.area(), original.area());
        assert!(mirror.vertices().all(|point| point.y > 1.5));
    }

    #[test]
    fn reflected_ellipses_preserve_their_source_secants_and_signed_area() {
        let setup = book(&[
            ("A", "line(point(0,-5),point(0,5))"),
            ("B", "ellipse(point(0,2),point(4,2),point(2,5))"),
            ("C", "reflection(GA,GB)"),
            ("D", "reflection(point(0,0),GB)"),
            ("E", "reflection(GA,GC)"),
        ]);
        let scene = scene(&setup);
        let ellipse = |index: usize| match &scene.objects[index].value {
            Ok(Shape::Ellipse(ellipse)) => ellipse,
            value => panic!("Expected an ellipse, got {value:?}"),
        };
        let source = ellipse(1);
        let cursor = Point2 { x: -2., y: 5. };
        for (index, sign) in [(2, -1.), (3, 1.), (4, 1.)] {
            let image = ellipse(index);
            assert_eq!(image.area(), sign * source.area());
            assert_eq!(image.perimeter(), source.perimeter());
            assert_eq!(image.element(0.).unwrap(), Point2 { x: 2., y: 5. });
            assert_eq!(
                image.parameter(cursor).unwrap(),
                source.parameter(cursor).unwrap()
            );
        }
        let left = ellipse(2)
            .vertices()
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min);
        assert!(left < -5.6 && left > -5.61);
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
    fn tangent_follows_a_constrained_point_on_an_algebraic_function_graph() {
        let mut construction = book(&[
            ("A", "plotfunc(x^2/4+1)"),
            ("B", "element(GA,2)"),
            ("C", "tangent(GB)"),
            ("D", "tangent(GA,2)"),
            ("E", "tangent(GA,0)"),
        ]);
        for (source, slope, intercept) in [
            ("plotfunc(x^2/4+1)", 1., 0.),
            ("plotfunc(x^2/2+1)", 2., -1.),
            ("plotfunc(1/x)", -0.25, 1.),
        ] {
            construction.objects[0].source = source.into();
            let current = scene(&construction);
            assert!(current.is_point_on_curve("B"));
            for name in ["C", "D"] {
                let Shape::Line {
                    a,
                    b,
                    extent: Extent::Line,
                } = shape(&current, name)
                else {
                    panic!("{name} should be a tangent line");
                };
                let measured = (b.y - a.y) / (b.x - a.x);
                assert!((measured - slope).abs() < 1e-10);
                assert!((a.y - a.x * measured - intercept).abs() < 1e-10);
            }
        }
        let rational = scene(&construction);
        assert!(rational.objects[4].value.is_err());
        construction.cache_scene(&rational).unwrap();
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        for expression in ["tangent(GA,0)", "tangent(element(GA,0))"] {
            assert!(matches!(
                construction.current_expression(expression, &snapshot, Arc::new(AtomicBool::new(false))),
                Ok(PortableValue::String(error)) if error.contains("Bad Argument Value")
            ));
        }
    }

    #[test]
    fn circular_trig_tangents_follow_named_points_and_effective_angle() {
        use ppl_math::AngleMode;

        let mut construction = book(&[
            ("A", "plotfunc(SIN(x))"),
            ("B", "element(GA,1)"),
            ("C", "tangent(GB)"),
            ("D", "tangent(GA,1)"),
        ]);
        let mut context = SessionContext::default();
        context.active_app = "Geometry".into();
        let cancel = Arc::new(AtomicBool::new(false));

        let check = |scene: &Scene, x, y, slope: f64, intercept: f64| {
            pt(scene, "B", Point2 { x, y });
            for name in ["C", "D"] {
                let Shape::Line {
                    a,
                    b,
                    extent: Extent::Line,
                } = shape(scene, name)
                else {
                    panic!("{name} should be a tangent line");
                };
                let measured = (b.y - a.y) / (b.x - a.x);
                assert!((measured - slope).abs() < 1e-10);
                assert!((a.y - a.x * measured - intercept).abs() < 1e-10);
            }
        };

        let snapshot = context.snapshot();
        let initial = construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        check(&initial, 1., 0.841470984808, 0.540302305868, 0.30116867894);
        construction.objects[0].source = "plotfunc(2*cos(2*x)+1)".into();
        let changed = construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        check(&changed, 1., 0.167706326906, -3.6371897073, 3.80489603421);

        construction.objects[1].source = "element(GA,1/2)".into();
        construction.objects[3].source = "tangent(GA,1/2)".into();
        let controls = [
            (
                AngleMode::Degrees,
                1,
                2.08060461174,
                -3.36588393923,
                3.76354658135,
            ),
            (
                AngleMode::Radians,
                2,
                2.99969539031,
                -0.00121840781891,
                3.00030459422,
            ),
            (
                AngleMode::Radians,
                3,
                2.99975326496,
                -0.000986919853489,
                3.00024672489,
            ),
            (
                AngleMode::Degrees,
                0,
                2.99969539031,
                -0.00121840781891,
                3.00030459422,
            ),
        ];
        for (home, app, y, slope, intercept) in controls {
            context.settings.angle = home;
            context.app_settings.insert(
                "Geometry".into(),
                ppl_vm::AppSettings {
                    angle: app,
                    ..Default::default()
                },
            );
            let scene = construction
                .refresh_values(&context.snapshot(), cancel.clone())
                .unwrap();
            check(&scene, 0.5, y, slope, intercept);
        }
        let serialized = serde_json::to_string(&construction).unwrap();
        let mut reopened: Book = serde_json::from_str(&serialized).unwrap();
        let scene = reopened
            .refresh_values(&context.snapshot(), cancel)
            .unwrap();
        let (_, _, y, slope, intercept) = controls[3];
        check(&scene, 0.5, y, slope, intercept);
    }

    #[test]
    fn tangent_function_keeps_finite_neighbors_and_refuses_trig_poles() {
        let mut construction = book(&[
            ("A", "plotfunc(tan(x))"),
            ("B", "element(GA,1/4)"),
            ("C", "tangent(GB)"),
        ]);
        let mut context = SessionContext::default();
        context.active_app = "Geometry".into();
        let cancel = Arc::new(AtomicBool::new(false));
        let snapshot = context.snapshot();
        let scene = construction
            .refresh_values(&snapshot, cancel.clone())
            .unwrap();
        pt(
            &scene,
            "B",
            Point2 {
                x: 0.25,
                y: 0.255341921221,
            },
        );
        for (expression, expected) in [
            ("slope(tangent(GB))", 1.06519949673),
            ("slope(tangent(GA,1.5707))", 107771959.94),
            ("slope(tangent(GA,1.5709))", 93039410.7723),
        ] {
            let value = construction
                .current_expression(expression, &snapshot, cancel.clone())
                .unwrap();
            assert!(
                matches!(value, PortableValue::Real(actual)
                    if (actual.to_f64() / expected - 1.).abs() < 1e-7),
                "{expression}: {value:?}"
            );
        }
        for version in [
            ppl_math::NumericVersion::Legacy15157,
            ppl_math::NumericVersion::Current15751,
        ] {
            context.settings.numeric_version = version;
            let radians = context.snapshot();
            construction
                .refresh_values(&radians, cancel.clone())
                .unwrap();
            for expression in ["tangent(GA,π/2)", "tangent(element(GA,π/2))"] {
                assert!(
                    matches!(
                        construction.current_expression(expression, &radians, cancel.clone()).unwrap(),
                        PortableValue::String(error) if error.contains("Bad Argument Value")
                    ),
                    "{version:?}: {expression}"
                );
            }
        }

        context.settings.angle = ppl_math::AngleMode::Degrees;
        let degrees = context.snapshot();
        construction
            .refresh_values(&degrees, cancel.clone())
            .unwrap();
        assert!(matches!(
            construction.current_expression("slope(tangent(GA,89))", &degrees, cancel.clone()).unwrap(),
            PortableValue::Real(slope) if (slope.to_f64() - 57.3015976317).abs() < 1e-8
        ));
        assert!(matches!(
            construction.current_expression("tangent(GA,90)", &degrees, cancel).unwrap(),
            PortableValue::String(error) if error.contains("Bad Argument Value")
        ));
    }

    #[test]
    fn two_arc_intersection_clips_both_sweeps_before_choosing_a_hit() {
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "circle(GA,4,0,π/2)"),
            ("D", "circle(GB,4,π/2,π)"),
            ("E", "circle(GA,4,π/6,11*π/6)"),
            ("F", "circle(GB,4,π,3*π/2)"),
            ("G", "circle(GB,4,π/2,3*π/2)"),
            ("H", "point(2,-4)"),
            ("I", "single_inter(GC,GD,GH)"),
            ("J", "single_inter(GE,GG)"),
            ("K", "single_inter(GG,GE,GH)"),
            ("L", "single_inter(GC,GF,GH)"),
            ("M", "circle(point(8,0),4,π/2,π)"),
        ]);
        let initial = scene(&construction);
        let upper = Point2 {
            x: 2.,
            y: 12_f64.sqrt(),
        };
        let lower = Point2 { x: 2., y: -upper.y };
        for (a, b, expected) in [
            ("C", "D", vec![upper]),
            ("C", "F", vec![]),
            ("E", "F", vec![lower]),
            ("E", "G", vec![upper, lower]),
            ("C", "M", vec![Point2 { x: 4., y: 0. }]),
        ] {
            let a = shape(&initial, a);
            let b = shape(&initial, b);
            assert_eq!(primitives::intersections(&a, &b).unwrap(), expected);
            assert_eq!(primitives::intersections(&b, &a).unwrap(), expected);
        }
        assert_eq!(shape(&initial, "I"), Shape::Point(upper));
        assert_eq!(shape(&initial, "J"), Shape::Point(upper));
        assert_eq!(shape(&initial, "K"), Shape::Point(lower));
        assert_eq!(shape(&initial, "L"), Shape::Undefined);

        construction.objects[1].source = "point(6,0)".into();
        assert_eq!(
            shape(&scene(&construction), "I"),
            Shape::Point(Point2 {
                x: 3.,
                y: 7_f64.sqrt()
            })
        );

        construction.objects[1].source = "point(3,1)".into();
        let diagonal = scene(&construction);
        let wide_left = shape(&diagonal, "E");
        let wide_right = shape(&diagonal, "G");
        let hits = primitives::intersections(&wide_left, &wide_right).unwrap();
        assert_eq!(hits.len(), 2);
        let expected = [
            Point2 {
                x: 2.66189500386,
                y: -2.98568501159,
            },
            Point2 {
                x: 0.338104996138,
                y: 3.98568501159,
            },
        ];
        let reversed = primitives::intersections(&wide_right, &wide_left).unwrap();
        assert_eq!(reversed.len(), expected.len());
        for ((forward, reverse), expected) in hits.iter().zip(reversed).zip(expected) {
            assert!(forward.distance(expected).unwrap() < 1e-9);
            assert!(reverse.distance(expected).unwrap() < 1e-9);
        }
        assert_eq!(shape(&diagonal, "J"), Shape::Point(hits[0]));
    }

    #[test]
    fn circle_arc_intersection_orders_clips_and_updates_named_points() {
        let mut construction = book(&[
            ("A", "point(0,0)"),
            ("B", "point(4,0)"),
            ("C", "circle(GA,4,0,π/2)"),
            ("D", "circle(GB,4)"),
            ("E", "circle(GA,4,π/6,11*π/6)"),
            ("F", "point(2,-4)"),
            ("G", "single_inter(GC,GD,GF)"),
            ("H", "single_inter(GD,GE)"),
            ("I", "single_inter(GE,GD,GF)"),
            ("J", "inter(GC,circle(point(8,0),4))"),
            ("K", "inter(GC,circle(point(9,0),4))"),
            ("L", "single_inter(GC,circle(point(9,0),4))"),
        ]);
        let initial = scene(&construction);
        let upper = Point2 {
            x: 2.,
            y: 12_f64.sqrt(),
        };
        let lower = Point2 { x: 2., y: -upper.y };
        for (arc, expected) in [("C", vec![upper]), ("E", vec![upper, lower])] {
            let arc = shape(&initial, arc);
            let circle = shape(&initial, "D");
            assert_eq!(primitives::intersections(&arc, &circle).unwrap(), expected);
            assert_eq!(primitives::intersections(&circle, &arc).unwrap(), expected);
        }
        assert_eq!(shape(&initial, "G"), Shape::Point(upper));
        assert_eq!(shape(&initial, "H"), Shape::Point(upper));
        assert_eq!(shape(&initial, "I"), Shape::Point(lower));
        assert_eq!(
            shape(&initial, "J"),
            Shape::Points(vec![Point2 { x: 4., y: 0. }])
        );
        assert_eq!(shape(&initial, "K"), Shape::Points(vec![]));
        assert_eq!(shape(&initial, "L"), Shape::Undefined);

        construction.objects[1].source = "point(6,0)".into();
        assert_eq!(
            shape(&scene(&construction), "G"),
            Shape::Point(Point2 {
                x: 3.,
                y: 7_f64.sqrt()
            })
        );
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
            matches!(construction.current_values["B"].portable().unwrap(), PortableValue::List(items)
            if items.len() == 2 && items.iter().all(|item| matches!(item, PortableValue::Symbolic(_))))
        );
        assert_eq!(
            construction.current_values["C"].portable().unwrap(),
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
        assert_eq!(
            construction.current_values["D"].portable().unwrap(),
            vertical
        );
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
