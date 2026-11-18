//! Sequence construction returns its function, diagonal and two open polygonal paths.
use super::{
    Extent, Parametric, Point2, Shape, access, canonical, function, point, primitives as g,
};
use crate::ide::native_apps::Evaluator;
use ppl_vm::{PortableValue, SessionSnapshot};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencePlot {
    pub(crate) paths: Vec<Vec<Point2>>,
    pub(super) function: Box<Shape>,
    iterates: Vec<Point2>,
    end: [Point2; 2],
}
impl SequencePlot {
    pub(super) fn build(
        expression: String,
        variable: String,
        [start, lo, hi]: [f64; 3],
        n: i64,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let n = n.unsigned_abs();
        if hi <= lo || n > 4096 {
            return Err("Invalid sequence interval or too many iterations".into());
        }
        let mut e = Evaluator::geometry_curve(&expression, &variable, snapshot, cancel.clone())?;
        let mut at = |x| {
            e.at(x)?
                .ok_or_else(|| String::from("Undefined sequence term"))
        };
        let mut paths = Vec::new();
        let (tree, d) = ppl_syntax::parse_expression(&expression);
        if d.has_errors() {
            return Err("Invalid sequence expression".into());
        }
        let function = if function::degree_in(&tree, &variable).is_some_and(|n| n <= 1) {
            let a = point(Point2 { x: lo, y: at(lo)? })?;
            let b = point(Point2 { x: hi, y: at(hi)? })?;
            paths.push(vec![a, b]);
            g::line(a, b, Extent::Segment)?
        } else {
            let mut c = Parametric::new(
                format!("{variable}+i*({expression})"),
                variable,
                [lo, hi],
                None,
            )?;
            c.trace(snapshot, cancel)?;
            paths.extend(c.paths.iter().cloned());
            Shape::Parametric(c)
        };
        let mut x = start;
        let mut iterates = vec![point(Point2 { x, y: lo })?];
        for _ in 0..n {
            let y = at(x)?;
            iterates.push(point(Point2 { x, y })?);
            x = y;
            iterates.push(Point2 { x, y: x });
        }
        let end = [Point2 { x, y: x }, Point2 { x, y: lo }];
        paths.push(iterates.clone());
        paths.push(end.to_vec());
        Ok(Self {
            paths,
            function: Box::new(function),
            iterates,
            end,
        })
    }
    pub(super) fn value(&self) -> Result<PortableValue, String> {
        let polygon = |v: &[Point2]| {
            let points = v
                .iter()
                .map(|p| format!("point({},{})", canonical(p.x), canonical(p.y)))
                .collect::<Vec<_>>();
            access::symbolic(format!("polygon({})", points.join(",")))
        };
        Ok(PortableValue::List(vec![
            access::shape_value(&self.function)?,
            access::symbolic("line(point(0,0),point(1,1))"),
            polygon(&self.iterates),
            polygon(&self.end),
        ]))
    }
}
