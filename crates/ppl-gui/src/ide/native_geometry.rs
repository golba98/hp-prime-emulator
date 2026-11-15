//! Dependency-aware Geometry definitions, evaluated off the UI thread.
//! HP Prime User Guide chapter9: Symbolic objects and Numeric measurements.
mod evaluate;
pub mod plot;
pub mod primitives;
use super::native_app_models::Geometry;
pub use super::native_app_models::Point2;
use ppl_vm::SessionSnapshot;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};
pub const MAX_OBJECTS: usize = 3000;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Object {
    pub name: String,
    pub source: String,
    pub visible: bool,
    pub label: bool,
    pub color: u32,
}
impl Default for Object {
    fn default() -> Self {
        Self {
            name: String::new(),
            source: String::new(),
            visible: true,
            label: true,
            color: 0x000000,
        }
    }
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
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Extent {
    Segment,
    Ray,
    Line,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
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
    Circle {
        center: Point2,
        radius: f64,
    },
    Polygon(Vec<Point2>),
    Points(Vec<Point2>),
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
}
pub fn default_window() -> super::native_apps::PlotWindow {
    super::native_plot_controls::preset(
        &super::native_apps::PlotWindow::default(),
        super::native_plot_controls::Preset::Decimal,
        ppl_math::AngleMode::Radians,
        super::native_plot_controls::PresetCenter::Origin,
    )
    .expect("valid Geometry decimal window")
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
    pub fn move_point(&mut self, name: &str, to: Point2) -> Result<(), String> {
        point(to)?;
        let object = self
            .objects
            .iter_mut()
            .find(|o| o.name == name)
            .ok_or("Unknown point")?;
        if !evaluate::free_point(&object.source) {
            return Err("This point is constrained by its definition".into());
        }
        object.source = format!("point({},{})", canonical(to.x), canonical(to.y));
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
        b.move_point("A", Point2 { x: 1., y: 2. }).unwrap();
        let s = scene(&b);
        pt(&s, "C", Point2 { x: 4., y: 4. });
        pt(&s, "D", Point2 { x: 1., y: 6. });
        assert!(b.move_point("C", Point2 { x: 0., y: 0. }).is_err());
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
        b.move_point("B", Point2 { x: 0., y: 0. }).unwrap();
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
    fn similarity_transforms_preserve_shapes_and_recalculate_dependencies() {
        let mut b = book(&[
            ("A", "point(0,0)"),
            ("B", "point(3,4)"),
            ("C", "segment(GA,GB)"),
            ("D", "translation(2+i,GC)"),
            ("E", "circle(GA,2)"),
            ("F", "homothety(GA,-3,GE)"),
            ("H", "reflection(line(point(0),point(1)),GE)"),
            (
                "I",
                "rotation(GA,π/2,polygon(point(0),point(2),point(2,1)))",
            ),
            ("J", "translation(2+i,inter(GE,line(point(-3),point(3))))"),
        ]);
        let s = scene(&b);
        assert_eq!(
            shape(&s, "D"),
            Shape::Line {
                a: Point2 { x: 2., y: 1. },
                b: Point2 { x: 5., y: 5. },
                extent: Extent::Segment,
            }
        );
        assert_eq!(
            shape(&s, "F"),
            Shape::Circle {
                center: Point2 { x: 0., y: 0. },
                radius: 6.,
            }
        );
        assert_eq!(
            shape(&s, "H"),
            Shape::Circle {
                center: Point2 { x: 0., y: 0. },
                radius: 2.,
            }
        );
        match shape(&s, "I") {
            Shape::Polygon(p) => {
                assert_eq!(p.len(), 3);
                assert!((p[1].x).abs() < 1e-9 && (p[1].y - 2.).abs() < 1e-9);
                assert!((p[2].x + 1.).abs() < 1e-9 && (p[2].y - 2.).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            shape(&s, "J"),
            Shape::Points(vec![Point2 { x: 0., y: 1. }, Point2 { x: 4., y: 1. },])
        );
        b.move_point("B", Point2 { x: 5., y: 6. }).unwrap();
        assert_eq!(
            shape(&scene(&b), "D"),
            Shape::Line {
                a: Point2 { x: 2., y: 1. },
                b: Point2 { x: 7., y: 7. },
                extent: Extent::Segment,
            }
        );
        let restored: Book = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(scene(&restored), scene(&b));
    }

    #[test]
    fn transforms_use_geometry_angle_settings_and_refuse_collapsed_constructions() {
        let b = book(&[
            ("A", "point(0)"),
            ("B", "rotation(GA,90,half_line(point(1),point(2)))"),
            ("C", "homothety(GA,0,line(point(0),point(1)))"),
            ("D", "homothety(GA,0,circle(GA,1))"),
            ("E", "translation(1E499,point(1))"),
            ("F", "reflection(point(1,2),segment(point(0),point(3)))"),
        ]);
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.app_settings.insert(
            "Geometry".into(),
            ppl_vm::AppSettings {
                angle: 2,
                ..Default::default()
            },
        );
        let s = b
            .calculate(&snapshot, Arc::new(AtomicBool::new(false)))
            .unwrap();
        match shape(&s, "B") {
            Shape::Line {
                a,
                b,
                extent: Extent::Ray,
            } => {
                assert!(a.x.abs() < 1e-9 && (a.y - 1.).abs() < 1e-9);
                assert!(b.x.abs() < 1e-9 && (b.y - 2.).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
        for name in ["C", "D", "E"] {
            assert!(
                s.objects
                    .iter()
                    .find(|o| o.name == name)
                    .unwrap()
                    .value
                    .is_err()
            );
        }
        assert_eq!(
            shape(&s, "F"),
            Shape::Line {
                a: Point2 { x: 2., y: 4. },
                b: Point2 { x: -1., y: 4. },
                extent: Extent::Segment,
            }
        );
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
