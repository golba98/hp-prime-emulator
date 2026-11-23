//! Scene sampling for existing asynchronous model plot jobs.
use super::*;
use crate::ide::{
    native_apps::PlotWindow,
    native_model_plots::{PlotData, PlotStyle},
};
use std::sync::atomic::Ordering;
pub fn build(
    book: &Book,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<PlotData, String> {
    let scene = book.calculate(snapshot, cancel.clone())?;
    let [xmin, xmax, ymin, ymax] = snapshot.geometry_plot_settings().derived_bounds();
    let window = PlotWindow {
        xmin: xmin.to_f64(),
        xmax: xmax.to_f64(),
        ymin: ymin.to_f64(),
        ymax: ymax.to_f64(),
        ..Default::default()
    };
    window.validate()?;
    if !(window.xmax - window.xmin).is_finite() || !(window.ymax - window.ymin).is_finite() {
        return Err("Geometry plot span exceeds numerical range".into());
    }
    let mut data = PlotData::new("X", "Y");
    for object in &scene.objects {
        if cancel.load(Ordering::Acquire) {
            return Err("Geometry plot interrupted".into());
        }
        if !book
            .objects
            .iter()
            .any(|o| o.name == object.name && o.visible)
        {
            continue;
        }
        let Ok(shape) = &object.value else { continue };
        add_shape(&mut data, &object.name, shape, &window)?;
    }
    data.geometry = Some(Box::new(scene));
    Ok(data)
}
fn add_shape(
    data: &mut PlotData,
    name: &str,
    shape: &Shape,
    window: &PlotWindow,
) -> Result<(), String> {
    let (style, points) = match shape {
        Shape::Objects(objects) => {
            for object in objects {
                add_shape(data, name, object, window)?;
            }
            return Ok(());
        }
        Shape::Point(p) => (PlotStyle::Scatter, vec![[p.x, p.y]]),
        Shape::Points(points) => (
            PlotStyle::Scatter,
            points.iter().map(|p| [p.x, p.y]).collect(),
        ),
        Shape::Line { a, b, extent } => {
            let Some([a, b]) = clip(*a, *b, *extent, window) else {
                return Ok(());
            };
            (PlotStyle::Line, vec![[a.x, a.y], [b.x, b.y]])
        }
        Shape::Lines(lines) => {
            for &[a, b] in lines {
                if let Some([a, b]) = clip(a, b, Extent::Line, window) {
                    data.push(
                        format!("G{name}"),
                        PlotStyle::Line,
                        vec![[a.x, a.y], [b.x, b.y]],
                    )?;
                }
            }
            return Ok(());
        }
        Shape::Circle { center, radius } => {
            let points = (0..=192)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / 192.;
                    [center.x + radius * a.cos(), center.y + radius * a.sin()]
                })
                .collect();
            (PlotStyle::Line, points)
        }
        Shape::Arc(arc) => (
            PlotStyle::Line,
            arc.vertices().map(|p| [p.x, p.y]).collect(),
        ),
        Shape::Hyperbola(curve) => {
            for sign in [1., -1.] {
                data.push(
                    format!("G{name}"),
                    PlotStyle::Line,
                    curve.branch(sign).map(|p| [p.x, p.y]).collect(),
                )?;
            }
            return Ok(());
        }
        Shape::Parabola(curve) => (
            PlotStyle::Line,
            curve.vertices().map(|p| [p.x, p.y]).collect(),
        ),
        Shape::Ellipse(ellipse) => (
            PlotStyle::Line,
            ellipse.vertices().map(|p| [p.x, p.y]).collect(),
        ),
        Shape::Polyline(points) => (PlotStyle::Line, points.iter().map(|p| [p.x, p.y]).collect()),
        Shape::Polygon(points) => {
            let mut points = points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>();
            if let Some(p) = points.first().copied() {
                points.push(p);
            }
            (PlotStyle::Line, points)
        }
        Shape::Sequence(seq) => {
            let label = format!("G{name}");
            if let Some([a, b]) = clip(
                Point2 { x: 0., y: 0. },
                Point2 { x: 1., y: 1. },
                Extent::Line,
                window,
            ) {
                data.push(label.clone(), PlotStyle::Line, vec![[a.x, a.y], [b.x, b.y]])?;
            }
            for path in &seq.paths {
                data.push(
                    label.clone(),
                    PlotStyle::Line,
                    path.iter().map(|p| [p.x, p.y]).collect(),
                )?;
            }
            return Ok(());
        }
        Shape::Function { paths, .. }
        | Shape::Parametric(crate::ide::native_geometry::Parametric { paths, .. }) => {
            for path in paths {
                data.push(
                    format!("G{name}"),
                    PlotStyle::Line,
                    path.iter().map(|p| [p.x, p.y]).collect(),
                )?;
            }
            return Ok(());
        }
        Shape::Parameter(_) | Shape::Number { .. } | Shape::EmptyConic(_) | Shape::Undefined => {
            return Ok(());
        }
    };
    data.push(format!("G{name}"), style, points)?;
    Ok(())
}
pub fn clip(a: Point2, b: Point2, extent: Extent, w: &PlotWindow) -> Option<[Point2; 2]> {
    let d = primitives::delta(a, b);
    let (mut lo, mut hi) = match extent {
        Extent::Segment => (0., 1.),
        Extent::Ray => (0., f64::INFINITY),
        Extent::Line => (f64::NEG_INFINITY, f64::INFINITY),
    };
    for (p, d, min, max) in [(a.x, d.x, w.xmin, w.xmax), (a.y, d.y, w.ymin, w.ymax)] {
        if d == 0. {
            if p < min || p > max {
                return None;
            }
            continue;
        }
        let t0 = (min - p) / d;
        let t1 = (max - p) / d;
        lo = lo.max(t0.min(t1));
        hi = hi.min(t0.max(t1));
    }
    if lo > hi || !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    let p = point(Point2 {
        x: a.x + lo * d.x,
        y: a.y + lo * d.y,
    })
    .ok()?;
    let q = point(Point2 {
        x: a.x + hi * d.x,
        y: a.y + hi * d.y,
    })
    .ok()?;
    Some([p, q])
}
