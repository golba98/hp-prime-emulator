//! Geometry definitions and the transient values visible to Home/PPL.
use super::*;
use ppl_vm::PortableValue;

pub(crate) fn symbolic(source: impl Into<String>) -> PortableValue {
    PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source.into()))
}
pub(crate) fn undefined() -> PortableValue {
    PortableValue::Symbolic(ppl_cas::CasValue::Undefined)
}
pub(crate) fn scalar(value: f64) -> Result<PortableValue, String> {
    number(value)?;
    Ok(PortableValue::Real(
        ppl_math::Real::from_f64(value).map_err(|error| error.to_string())?,
    ))
}
pub(crate) fn shape_value(shape: &Shape) -> Result<PortableValue, String> {
    fn point_source(p: Point2) -> String {
        format!("point({},{})", canonical(p.x), canonical(p.y))
    }
    fn line_source(a: Point2, b: Point2, extent: Extent) -> PortableValue {
        let function = match extent {
            Extent::Segment => "segment",
            Extent::Ray => "half_line",
            Extent::Line => "line",
        };
        symbolic(format!(
            "{function}({},{})",
            point_source(a),
            point_source(b)
        ))
    }
    Ok(match shape {
        Shape::Undefined => undefined(),
        Shape::Number { re, im } if *im == 0. => scalar(*re)?,
        Shape::Number { re, im } => PortableValue::Complex(ppl_math::Complex::new(
            ppl_math::Real::from_f64(*re).map_err(|e| e.to_string())?,
            ppl_math::Real::from_f64(*im).map_err(|e| e.to_string())?,
        )),
        Shape::Point(p) => symbolic(point_source(*p)),
        Shape::Line { a, b, extent } => line_source(*a, *b, *extent),
        Shape::Lines(lines) => PortableValue::List(
            lines
                .iter()
                .map(|&[a, b]| line_source(a, b, Extent::Line))
                .collect(),
        ),
        Shape::Circle { center, radius } => symbolic(format!(
            "circle({},{})",
            point_source(*center),
            canonical(*radius)
        )),
        Shape::Arc(arc) => symbolic(format!(
            "circle({},{},{},{})",
            point_source(arc.center),
            canonical(arc.radius),
            canonical(arc.start),
            canonical(arc.end)
        )),
        Shape::Ellipse(ellipse) => {
            let [a, b] = ellipse.foci;
            let size = match ellipse.size {
                super::ellipse::Size::Witness(p) => point_source(p),
                super::ellipse::Size::SemiMajor(major) => canonical(major),
            };
            symbolic(format!(
                "ellipse({},{},{size})",
                point_source(a),
                point_source(b)
            ))
        }
        Shape::Polygon(points) => {
            let mut points = points.iter().copied().map(point_source).collect::<Vec<_>>();
            if let Some(first) = points.first().cloned() {
                points.push(first);
            }
            symbolic(format!("polygon({})", points.join(",")))
        }
        Shape::Points(points) => {
            PortableValue::List(points.iter().map(|p| symbolic(point_source(*p))).collect())
        }
        Shape::Function { expression, .. } => symbolic(format!("plotfunc({expression})")),
    })
}
impl Book {
    /// Include value-only assignments in the GUI's optimistic commit fence.
    pub fn access_fingerprint(&self) -> serde_json::Value {
        serde_json::json!({
            "definitions": self,
            "point_curves": self.current_point_curves,
            "values": self.current_values.iter().map(|(name, value)|
                (name.clone(), ppl_vm::session::encode_value(value))).collect::<std::collections::BTreeMap<_, _>>()
        })
    }
    pub fn cache_scene(&mut self, scene: &Scene) -> Result<(), String> {
        let mut values = std::collections::BTreeMap::new();
        for object in &scene.objects {
            if self.objects.iter().any(|o| o.name == object.name) {
                values.insert(
                    object.name.clone(),
                    match &object.value {
                        Ok(shape) => shape_value(shape)?,
                        Err(_) => undefined(),
                    },
                );
            }
        }
        // Compare successive evaluated scenes. This includes constructed points that
        // move because their inputs changed, without making their definitions draggable.
        let positions: BTreeMap<_, _> = scene
            .objects
            .iter()
            .filter_map(|object| {
                if let Ok(Shape::Point(p)) = object.value {
                    Some((object.name.as_str(), p))
                } else {
                    None
                }
            })
            .collect();
        let previous = std::mem::take(&mut self.last_trace_positions);
        for object in self.objects.iter_mut().filter(|object| object.traced) {
            if let Some(&position) = positions.get(object.name.as_str()) {
                if let Some(&old) = previous.get(&object.name)
                    && old != position
                    && !object.trace_samples.back().is_some_and(|p| {
                        canonical(p.x) == canonical(position.x)
                            && canonical(p.y) == canonical(position.y)
                    })
                {
                    push_trace(&mut object.trace_samples, old);
                    push_trace(&mut object.trace_samples, position);
                }
                self.last_trace_positions
                    .insert(object.name.clone(), position);
            }
        }
        self.current_values = values;
        self.current_point_curves.clone_from(&scene.point_curves);
        Ok(())
    }
    pub fn refresh_values(
        &mut self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Scene, String> {
        let scene = self.calculate(snapshot, cancel)?;
        self.cache_scene(&scene)?;
        Ok(scene)
    }
    /// Evaluate an expression using the current object values, never refreshing
    /// stored definitions implicitly. Redraw is the firmware refresh boundary.
    pub fn current_expression(
        &self,
        expression: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<PortableValue, String> {
        evaluate::current_expression(self, expression, snapshot, cancel)
    }
}
