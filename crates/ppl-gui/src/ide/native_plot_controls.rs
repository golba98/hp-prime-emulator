//! Pure plot presets, autoscaling and sampled trace navigation.
//!
//! HP Prime User Guide pp.79–80 defines units per pixel, axes changed by each
//! preset, and first-selected-curve autoscaling. It does not specify whether
//! these presets retain the previous center: callers must choose that policy
//! explicitly. Sampling/evaluation remains in native_apps workers.
use super::{
    native_apps::{CurveSamples, GraphKind, PlotSamples, PlotWindow},
    native_model_plots::PlotData,
};
use ppl_vm::AngleMode;
use serde::{Deserialize, Serialize};

/// Distances between the extreme Cartesian plot pixels, confirmed by genuine
/// G1 C→PX. Bitmap dimensions remain 320x240; they are not the plot spans.
pub const X_PIXEL_SPAN: f64 = 318.;
pub const Y_PIXEL_SPAN: f64 = 218.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Decimal,
    Integer,
    Trig,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetCenter {
    /// Keep the current mathematical center when changing scales.
    Preserve,
    /// Center changed axes on zero; distinct from PlotWindow::default().
    Origin,
}
/// A preset changes scale, not parameter range or axes visibility. Integer
/// leaves the complete vertical window unchanged even with Origin centering.
pub fn preset(
    window: &PlotWindow,
    choice: Preset,
    angle: AngleMode,
    center: PresetCenter,
) -> Result<PlotWindow, String> {
    window.validate()?;
    let mut next = window.clone();
    let (cx, cy) = match center {
        PresetCenter::Preserve => (
            window.xmin / 2. + window.xmax / 2.,
            window.ymin / 2. + window.ymax / 2.,
        ),
        PresetCenter::Origin => (0., 0.),
    };
    let x_step = match choice {
        Preset::Decimal => 0.1,
        Preset::Integer => 1.,
        Preset::Trig => match angle {
            AngleMode::Radians => std::f64::consts::PI / 24.,
            AngleMode::Degrees => 7.5,
            AngleMode::Gradians => 25. / 3.,
        },
    };
    let half_x = X_PIXEL_SPAN * x_step / 2.;
    next.xmin = cx - half_x;
    next.xmax = cx + half_x;
    if choice != Preset::Integer {
        let half_y = Y_PIXEL_SPAN * 0.1 / 2.;
        next.ymin = cy - half_y;
        next.ymax = cy + half_y;
    }
    next.validate()?;
    Ok(next)
}

/// Uses only the first supplied curve: callers supply selected curves in their
/// displayed order. Invalid first-curve samples do not cause a different curve
/// to determine the scale. Function/implicit x bounds remain unchanged;
/// Parametric/Polar/Sequence scale both axes. The 8% margin (minimum 1% of the
/// coordinate magnitude) is the native fit policy, not a claimed HP algorithm.
pub fn autoscale(
    kind: GraphKind,
    window: &PlotWindow,
    curves: &[CurveSamples],
) -> Result<Option<PlotWindow>, String> {
    let both = matches!(
        kind,
        GraphKind::Parametric | GraphKind::Polar | GraphKind::Sequence
    );
    fit_selected(window, curves, both)
}

/// Statistics/model plots fit both axes while retaining the first selected
/// data curve policy. This does not evaluate or synthesize mathematical data.
pub fn autoscale_model(window: &PlotWindow, data: &PlotData) -> Result<Option<PlotWindow>, String> {
    fit_selected(window, &model_samples(data), true)
}

fn fit_selected(
    window: &PlotWindow,
    curves: &[CurveSamples],
    both: bool,
) -> Result<Option<PlotWindow>, String> {
    window.validate()?;
    let Some(curve) = curves.first() else {
        return Ok(None);
    };
    let points = curve
        .iter()
        .flatten()
        .copied()
        .filter(|point| finite(*point))
        .filter(|point| both || (window.xmin <= point[0] && point[0] <= window.xmax))
        .collect::<Vec<_>>();
    if points.is_empty() {
        return Ok(None);
    }
    let bounds = |axis: usize| -> Result<(f64, f64), String> {
        let low = points
            .iter()
            .map(|point| point[axis])
            .fold(f64::INFINITY, f64::min);
        let high = points
            .iter()
            .map(|point| point[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        let margin = ((high - low) * 0.08).max(low.abs().max(high.abs()).max(1.) * 0.01);
        let bounds = (low - margin, high + margin);
        if !bounds.0.is_finite() || !bounds.1.is_finite() || bounds.0 >= bounds.1 {
            return Err("Autoscale exceeds numerical plot range".into());
        }
        Ok(bounds)
    };
    let mut next = window.clone();
    if both {
        (next.xmin, next.xmax) = bounds(0)?;
    }
    (next.ymin, next.ymax) = bounds(1)?;
    next.validate()?;
    Ok(Some(next))
}

/// The GUI's model plot data contains dense samples. Convert once when its
/// result is installed, then use the same trace/painter path as function apps.
pub fn model_samples(data: &PlotData) -> PlotSamples {
    data.curves
        .iter()
        .map(|curve| curve.iter().copied().map(Some).collect())
        .collect()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceDirection {
    Left,
    Right,
    Up,
    Down,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TracePoint {
    pub curve: usize,
    pub sample: usize,
    /// Exactly a computed sample, never a straight-line interpolation.
    pub point: [f64; 2],
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceState {
    pub enabled: bool,
    pub curve: usize,
    pub sample: usize,
}
impl TraceState {
    pub fn current(&self, curves: &[CurveSamples]) -> Option<TracePoint> {
        if !self.enabled {
            return None;
        }
        let point = curves
            .get(self.curve)?
            .get(self.sample)?
            .filter(|point| finite(*point))?;
        Some(TracePoint {
            curve: self.curve,
            sample: self.sample,
            point,
        })
    }
    /// Keep an existing valid position; otherwise select the first finite
    /// sample of the first curve containing one. Empty curves are skipped.
    pub fn enable(&mut self, curves: &[CurveSamples]) -> Option<TracePoint> {
        self.enabled = true;
        if let Some(point) = self.current(curves) {
            return Some(point);
        }
        for (curve, points) in curves.iter().enumerate() {
            if let Some(sample) = points.iter().position(|point| point.is_some_and(finite)) {
                self.curve = curve;
                self.sample = sample;
                return self.current(curves);
            }
        }
        None
    }
    pub fn disable(&mut self) {
        self.enabled = false;
    }
    /// Left/right stops at sample boundaries and skips gaps/nonfinite points.
    /// Up/down cycles curves once, preserving the nearest valid sample index
    /// (shared parameter grid). It never substitutes an invented coordinate.
    pub fn step(
        &mut self,
        direction: TraceDirection,
        curves: &[CurveSamples],
    ) -> Option<TracePoint> {
        if !self.enabled {
            return None;
        }
        if self.current(curves).is_none() {
            return self.enable(curves);
        }
        match direction {
            TraceDirection::Left | TraceDirection::Right => {
                let curve = &curves[self.curve];
                let sample = if direction == TraceDirection::Left {
                    (0..self.sample)
                        .rev()
                        .find(|index| curve[*index].is_some_and(finite))
                } else {
                    (self.sample + 1..curve.len()).find(|index| curve[*index].is_some_and(finite))
                };
                if let Some(sample) = sample {
                    self.sample = sample;
                }
            }
            TraceDirection::Up | TraceDirection::Down => {
                let count = curves.len();
                for distance in 1..count {
                    let curve = if direction == TraceDirection::Up {
                        (self.curve + count - distance) % count
                    } else {
                        (self.curve + distance) % count
                    };
                    let sample = curves[curve]
                        .iter()
                        .enumerate()
                        .filter(|(_, point)| point.is_some_and(finite))
                        .min_by_key(|(index, _)| index.abs_diff(self.sample))
                        .map(|(index, _)| index);
                    if let Some(sample) = sample {
                        self.curve = curve;
                        self.sample = sample;
                        break;
                    }
                }
            }
        }
        self.current(curves)
    }
    /// Mathematical pointer position, normalized by window ranges then
    /// measured in the actual 318x218 plot pixel spans. Numeric unit changes
    /// cannot bias the distance, and the rectangular LCD aspect is preserved. Ties
    /// retain displayed curve/sample ordering. Invalid input preserves state.
    pub fn nearest(
        &mut self,
        position: [f64; 2],
        window: &PlotWindow,
        curves: &[CurveSamples],
    ) -> Result<Option<TracePoint>, String> {
        self.nearest_in_view(position, window, curves, [X_PIXEL_SPAN, Y_PIXEL_SPAN])
    }
    /// Pointer distance in the actual rendered plot rectangle. A shortened
    /// native LCD canvas must not select points using the full-screen ratio.
    pub fn nearest_in_view(
        &mut self,
        position: [f64; 2],
        window: &PlotWindow,
        curves: &[CurveSamples],
        size: [f64; 2],
    ) -> Result<Option<TracePoint>, String> {
        if size.into_iter().any(|span| !span.is_finite() || span <= 0.) {
            return Err("Trace viewport dimensions must be positive and finite".into());
        }
        window.validate()?;
        if !finite(position) {
            return Err("Trace position must be finite".into());
        }
        let dx = window.xmax - window.xmin;
        let dy = window.ymax - window.ymin;
        if !dx.is_finite() || !dy.is_finite() {
            return Err("Trace window exceeds numerical range".into());
        }
        let mut best: Option<(f64, TracePoint)> = None;
        for (curve, points) in curves.iter().enumerate() {
            for (sample, point) in points.iter().enumerate() {
                let Some(point) = point.filter(|point| finite(*point)) else {
                    continue;
                };
                let distance = ((point[0] - position[0]) / dx * size[0])
                    .hypot((point[1] - position[1]) / dy * size[1]);
                if distance.is_finite() && best.as_ref().is_none_or(|(old, _)| distance < *old) {
                    best = Some((
                        distance,
                        TracePoint {
                            curve,
                            sample,
                            point,
                        },
                    ));
                }
            }
        }
        Ok(best.map(|(_, point)| {
            self.enabled = true;
            self.curve = point.curve;
            self.sample = point.sample;
            point
        }))
    }
}
fn finite(point: [f64; 2]) -> bool {
    point.into_iter().all(f64::is_finite)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-11, "{actual} != {expected}");
    }
    fn window() -> PlotWindow {
        PlotWindow {
            xmin: 10.,
            xmax: 30.,
            ymin: 50.,
            ymax: 70.,
            tmin: 1.,
            tmax: 3.,
            axes: false,
        }
    }
    #[test]
    fn decimal_matches_documented_tenth_unit_per_genuine_plot_pixel() {
        let w = preset(
            &window(),
            Preset::Decimal,
            AngleMode::Radians,
            PresetCenter::Origin,
        )
        .unwrap();
        close(w.xmin, -15.9);
        close(w.xmax, 15.9);
        close(w.ymin, -10.9);
        close(w.ymax, 10.9);
        assert_eq!((w.tmin, w.tmax, w.axes), (1., 3., false));
    }
    #[test]
    fn preserving_center_is_explicit_and_integer_keeps_entire_y_range() {
        let w = preset(
            &window(),
            Preset::Integer,
            AngleMode::Degrees,
            PresetCenter::Preserve,
        )
        .unwrap();
        assert_eq!((w.xmin, w.xmax, w.ymin, w.ymax), (-139., 179., 50., 70.));
        let origin = preset(
            &window(),
            Preset::Integer,
            AngleMode::Degrees,
            PresetCenter::Origin,
        )
        .unwrap();
        assert_eq!(
            (origin.xmin, origin.xmax, origin.ymin, origin.ymax),
            (-159., 159., 50., 70.)
        );
        let decimal = preset(
            &window(),
            Preset::Decimal,
            AngleMode::Radians,
            PresetCenter::Preserve,
        )
        .unwrap();
        close(decimal.xmin, 4.1);
        close(decimal.xmax, 35.9);
        close(decimal.ymin, 49.1);
        close(decimal.ymax, 70.9);
    }
    #[test]
    fn trig_uses_actual_angle_unit_not_sine_range_or_default_window() {
        let rad = preset(
            &window(),
            Preset::Trig,
            AngleMode::Radians,
            PresetCenter::Origin,
        )
        .unwrap();
        close(rad.xmin, -6.625 * std::f64::consts::PI);
        close(rad.xmax, 6.625 * std::f64::consts::PI);
        let deg = preset(
            &window(),
            Preset::Trig,
            AngleMode::Degrees,
            PresetCenter::Origin,
        )
        .unwrap();
        assert_eq!((deg.xmin, deg.xmax), (-1192.5, 1192.5));
        let grad = preset(
            &window(),
            Preset::Trig,
            AngleMode::Gradians,
            PresetCenter::Origin,
        )
        .unwrap();
        assert_eq!((grad.xmin, grad.xmax), (-1325., 1325.));
        close(grad.ymin, -10.9);
        close(grad.ymax, 10.9);
    }
    #[test]
    fn autoscale_first_curve_only_function_y_and_parametric_both_axes() {
        let w = PlotWindow::default();
        let samples = vec![
            vec![Some([-1., 2.]), None, Some([1., 6.])],
            vec![Some([-1000., -1e9]), Some([1000., 1e9])],
        ];
        let function = autoscale(GraphKind::Function, &w, &samples)
            .unwrap()
            .unwrap();
        assert_eq!((function.xmin, function.xmax), (-10., 10.));
        close(function.ymin, 1.68);
        close(function.ymax, 6.32);
        for kind in [GraphKind::Parametric, GraphKind::Polar, GraphKind::Sequence] {
            let fitted = autoscale(kind, &w, &samples).unwrap().unwrap();
            close(fitted.xmin, -1.16);
            close(fitted.xmax, 1.16);
            close(fitted.ymin, 1.68);
            close(fitted.ymax, 6.32);
        }
    }
    #[test]
    fn autoscale_ignores_nonfinite_and_outside_x_but_never_uses_second_curve() {
        let samples = vec![vec![
            Some([0., 5.]),
            Some([100., 1e9]),
            Some([f64::NAN, 7.]),
        ]];
        let fit = autoscale(GraphKind::Function, &PlotWindow::default(), &samples)
            .unwrap()
            .unwrap();
        close(fit.ymin, 4.95);
        close(fit.ymax, 5.05);
        assert!(
            autoscale(
                GraphKind::Function,
                &PlotWindow::default(),
                &[vec![None], vec![Some([0., 5.])]]
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn trace_steps_skip_discontinuities_and_stop_at_boundaries() {
        let curves = vec![vec![
            None,
            Some([-2., 4.]),
            None,
            Some([1., 1.]),
            Some([f64::NAN, 3.]),
        ]];
        let mut trace = TraceState::default();
        assert_eq!(trace.enable(&curves).unwrap().point, [-2., 4.]);
        assert_eq!(trace.step(TraceDirection::Left, &curves).unwrap().sample, 1);
        assert_eq!(
            trace.step(TraceDirection::Right, &curves).unwrap().point,
            [1., 1.]
        );
        assert_eq!(
            trace.step(TraceDirection::Right, &curves).unwrap().sample,
            3
        );
        assert_eq!(trace.step(TraceDirection::Left, &curves).unwrap().sample, 1);
        trace.disable();
        assert!(trace.step(TraceDirection::Right, &curves).is_none());
        assert_eq!(trace.sample, 1);
    }
    #[test]
    fn trace_switches_curves_preserving_parameter_index_and_skips_empty_curves() {
        let curves = vec![
            vec![Some([0., 0.]), None, Some([2., 4.])],
            vec![None],
            vec![Some([7., 8.]), Some([9., 10.]), None],
        ];
        let mut trace = TraceState {
            enabled: true,
            curve: 0,
            sample: 2,
        };
        assert_eq!(
            trace.step(TraceDirection::Down, &curves).unwrap(),
            TracePoint {
                curve: 2,
                sample: 1,
                point: [9., 10.]
            }
        );
        assert_eq!(trace.step(TraceDirection::Down, &curves).unwrap().curve, 0);
        assert_eq!(trace.step(TraceDirection::Up, &curves).unwrap().curve, 2);
    }
    #[test]
    fn pointer_uses_normalized_distance_and_exact_samples_without_interpolation() {
        let w = PlotWindow {
            xmin: 0.,
            xmax: 1000.,
            ymin: 0.,
            ymax: 1.,
            ..PlotWindow::default()
        };
        let curves = vec![vec![Some([100., 0.]), None], vec![Some([0., 0.2])]];
        let mut trace = TraceState::default();
        assert_eq!(
            trace.nearest([0., 0.], &w, &curves).unwrap().unwrap(),
            TracePoint {
                curve: 0,
                sample: 0,
                point: [100., 0.]
            }
        );
        let curve = vec![vec![Some([0., 0.]), Some([10., 100.])]];
        assert_eq!(
            trace
                .nearest(
                    [4., 40.],
                    &PlotWindow {
                        xmin: 0.,
                        xmax: 10.,
                        ymin: 0.,
                        ymax: 100.,
                        ..PlotWindow::default()
                    },
                    &curve
                )
                .unwrap()
                .unwrap()
                .point,
            [0., 0.]
        );
    }
    #[test]
    fn invalid_windows_pointer_data_and_stale_indices_are_safe() {
        let mut w = window();
        w.xmax = w.xmin;
        assert!(
            preset(
                &w,
                Preset::Decimal,
                AngleMode::Radians,
                PresetCenter::Origin
            )
            .is_err()
        );
        let mut trace = TraceState {
            enabled: true,
            curve: 99,
            sample: 99,
        };
        let old = trace;
        assert!(
            trace
                .nearest([f64::NAN, 0.], &PlotWindow::default(), &[])
                .is_err()
        );
        assert_eq!(trace, old);
        assert!(trace.enable(&[]).is_none());
        assert_eq!(
            trace.enable(&[vec![None, Some([3., 4.])]]).unwrap().point,
            [3., 4.]
        );
    }
    #[test]
    fn dense_model_adapter_preserves_original_sample_order_and_values() {
        let data = PlotData {
            curves: vec![vec![[3., 7.], [1., 2.]], vec![]],
            labels: vec![],
            styles: vec![],
            x_label: String::new(),
            y_label: String::new(),
        };
        let samples = model_samples(&data);
        assert_eq!(samples, vec![vec![Some([3., 7.]), Some([1., 2.])], vec![]]);
        let mut trace = TraceState::default();
        assert_eq!(trace.enable(&samples).unwrap().point, [3., 7.]);
        assert_eq!(
            trace.step(TraceDirection::Right, &samples).unwrap().point,
            [1., 2.]
        );
    }
    #[test]
    fn model_autoscale_uses_first_data_curve_and_both_axes() {
        let data = PlotData {
            curves: vec![vec![[1., 2.], [3., 7.]], vec![[1e9, -1e9]]],
            labels: vec![],
            styles: vec![],
            x_label: String::new(),
            y_label: String::new(),
        };
        let fit = autoscale_model(&PlotWindow::default(), &data)
            .unwrap()
            .unwrap();
        close(fit.xmin, 0.84);
        close(fit.xmax, 3.16);
        close(fit.ymin, 1.6);
        close(fit.ymax, 7.4);
    }
    #[test]
    fn pointer_distance_accounts_for_rectangular_lcd_plot_pixels() {
        let curves = vec![vec![Some([0.1, 0.])], vec![Some([0., 0.12])]];
        let w = PlotWindow {
            xmin: 0.,
            xmax: 1.,
            ymin: 0.,
            ymax: 1.,
            ..PlotWindow::default()
        };
        let mut trace = TraceState::default();
        //31.8 horizontal pixels is farther than 26.16 vertical pixels.
        assert_eq!(
            trace.nearest([0., 0.], &w, &curves).unwrap().unwrap().curve,
            1
        );
    }
}
