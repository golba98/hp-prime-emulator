//! Native app plot geometry, computed off the UI thread.
//! Contracts: HP Prime User Guide, Statistics 1Var pp.238/241 (half-open
//! histogram bins), Statistics 2Var p.254 (least-squares linear fit), Finance
//! pp.329–331 (payment groups and amortization). Triangle outlines visualize
//! the solver's side/angle convention; this is a native geometric projection.
use super::native_app_models::{Finance, Geometry, Statistics1, Statistics2, Triangle};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
const MAX_INPUT: usize = 10_000;
const MAX_BINS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlotStyle {
    Line,
    Scatter,
    PointLine,
    CrossLine,
    TraceOnly,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotAnnotation {
    pub point: [f64; 2],
    pub text: String,
    pub color: u32,
    #[serde(default = "annotation_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub right_aligned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<usize>,
}
fn annotation_font_size() -> f32 {
    12.
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotFill {
    pub points: Vec<[f64; 2]>,
    pub color: u32,
}
pub(crate) const DOT_FILL_ALPHA: u8 = 86;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotColumn {
    pub x: f64,
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    /// Fill composited over white. Other backgrounds use the same opacity.
    pub color: u32,
    pub analysis: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlotScaleBounds {
    Finite([f64; 4]),
    UnboundedY { x: [f64; 2] },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotScale {
    pub bounds: Option<PlotScaleBounds>,
    pub histogram: Option<HistogramSpec>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatisticsTraceSample {
    pub x: f64,
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auxiliary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fractional_frequency: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatisticsTraceSeries {
    pub analysis: usize,
    pub chart: super::native_statistics::one::Chart,
    pub samples: Vec<StatisticsTraceSample>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotData {
    pub curves: Vec<Vec<[f64; 2]>>,
    pub labels: Vec<String>,
    pub styles: Vec<PlotStyle>,
    pub x_label: String,
    pub y_label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<PlotAnnotation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<PlotFill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dots: Vec<DotColumn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statistics_traces: Vec<StatisticsTraceSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<PlotScale>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statistics_two_fits: Vec<(usize, String, u64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Box<super::native_geometry::Scene>>,
}
impl PlotData {
    pub(super) fn new(x_label: &str, y_label: &str) -> Self {
        Self {
            curves: Vec::new(),
            labels: Vec::new(),
            styles: Vec::new(),
            x_label: x_label.into(),
            y_label: y_label.into(),
            geometry: None,
            colors: Vec::new(),
            annotations: Vec::new(),
            fills: Vec::new(),
            dots: Vec::new(),
            statistics_traces: Vec::new(),
            scale: None,
            statistics_two_fits: Vec::new(),
        }
    }
    pub(super) fn push(
        &mut self,
        label: impl Into<String>,
        style: PlotStyle,
        points: Vec<[f64; 2]>,
    ) -> Result<(), String> {
        finite(points.iter().flatten().copied())?;
        self.curves.push(points);
        self.labels.push(label.into());
        self.styles.push(style);
        Ok(())
    }
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Plot interrupted".into())
    } else {
        Ok(())
    }
}
fn finite(values: impl IntoIterator<Item = f64>) -> Result<(), String> {
    if values.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err("Plot values must be finite".into())
    }
}
fn bounded(count: usize) -> Result<(), String> {
    if count > MAX_INPUT {
        Err(format!(
            "Plot accepts at most {MAX_INPUT} observations or objects"
        ))
    } else {
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HistogramSpec {
    pub minimum: f64,
    pub maximum: f64,
    pub width: f64,
}
impl HistogramSpec {
    /// An autoscale starting view, with approximately sqrt(n) equal-width bins.
    /// Explicit HRange/HWidth should use histogram() directly.
    pub fn for_data(model: &Statistics1) -> Result<Self, String> {
        bounded(model.samples.len())?;
        let summary = model.calculate()?;
        Self::for_observations(
            model.samples.len(),
            summary.minimum,
            summary.maximum,
            summary.count as f64,
        )
    }

    pub(super) fn for_observations(
        rows: usize,
        minimum: f64,
        maximum: f64,
        count: f64,
    ) -> Result<Self, String> {
        bounded(rows)?;
        finite([count, minimum, maximum])?;
        let span = maximum - minimum;
        if !span.is_finite() {
            return Err("Histogram data span exceeds numerical range".into());
        }
        let bins = count.sqrt().ceil().clamp(1., (MAX_BINS - 1) as f64);
        let width = if span == 0. {
            maximum.abs().max(1.) * 0.1
        } else {
            span / bins
        };
        // The upper edge is exclusive, so include a maximum on a bin boundary
        // by reserving one additional bin rather than changing bin membership.
        let maximum = maximum + width;
        finite([width, maximum])?;
        if width <= 0. || maximum <= minimum {
            return Err("Histogram bin width is below numerical precision".into());
        }
        Ok(Self {
            minimum,
            maximum,
            width,
        })
    }
}
pub fn histogram_auto(model: &Statistics1, cancel: &AtomicBool) -> Result<PlotData, String> {
    check(cancel)?;
    let spec = HistogramSpec::for_data(model)?;
    histogram(model, &spec, cancel)
}
pub fn histogram(
    model: &Statistics1,
    spec: &HistogramSpec,
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    check(cancel)?;
    bounded(model.samples.len())?;
    model.calculate()?; // validates frequencies, finite data, and nonempty mass
    finite([spec.minimum, spec.maximum, spec.width])?;
    if spec.width <= 0. || spec.minimum >= spec.maximum {
        return Err("Histogram range must increase and width must be positive".into());
    }
    let ratio = (spec.maximum - spec.minimum) / spec.width;
    let integer = ratio.round();
    let bins = if (ratio - integer).abs() <= 16. * f64::EPSILON * ratio.abs().max(1.) {
        integer
    } else {
        ratio.ceil()
    };
    if !bins.is_finite() || bins < 1. || bins >= usize::MAX as f64 {
        return Err("Histogram bin count exceeds its addressable range".into());
    }
    let bins = bins as usize;
    // Native emits five contour vertices, a fill polygon and a typed trace
    // record per bin. Reserve a conservative 1 KiB per bin across those
    // structures. This is a host rendering budget, not a firmware bin limit;
    // check before zero-filling to avoid Linux virtual-memory overcommit OOM.
    const RENDER_BUDGET_BYTES: usize = 64 * 1024 * 1024;
    const BYTES_PER_RENDERED_BIN: usize = 1024;
    if bins > RENDER_BUDGET_BYTES / BYTES_PER_RENDERED_BIN {
        return Err("Histogram geometry exceeds the 64 MiB Native rendering budget".into());
    }
    check(cancel)?;
    let mut frequencies = Vec::new();
    frequencies
        .try_reserve_exact(bins)
        .map_err(|_| "Insufficient memory for histogram bins")?;
    frequencies.resize(bins, 0u64);
    for (i, &value) in model.samples.iter().enumerate() {
        check(cancel)?;
        if value < spec.minimum || value >= spec.maximum {
            continue;
        }
        let bin =
            (((value - spec.minimum) / spec.width).floor() as usize).min(frequencies.len() - 1);
        let frequency = model.frequencies.get(i).copied().unwrap_or(1);
        if let Some(total) = frequencies.get_mut(bin) {
            *total = total
                .checked_add(frequency)
                .ok_or("Histogram frequency overflow")?;
        }
    }
    let points = frequencies
        .len()
        .checked_mul(5)
        .ok_or("Histogram point count overflow")?;
    let mut bars = Vec::new();
    bars.try_reserve_exact(points)
        .map_err(|_| "Insufficient memory for histogram geometry")?;
    for (bin, frequency) in frequencies.into_iter().enumerate() {
        check(cancel)?;
        let left = spec.minimum + bin as f64 * spec.width;
        let right = (left + spec.width).min(spec.maximum);
        if left >= right {
            return Err("Histogram edges exceed numerical precision".into());
        }
        bars.extend([
            [left, 0.],
            [left, frequency as f64],
            [right, frequency as f64],
            [right, 0.],
            [left, 0.],
        ]);
    }
    let mut result = PlotData::new("Observation", "Frequency");
    result.push("Histogram", PlotStyle::Line, bars)?;
    Ok(result)
}
pub fn statistics2(model: &Statistics2, cancel: &AtomicBool) -> Result<PlotData, String> {
    check(cancel)?;
    bounded(model.pairs.len())?;
    let fit = model.fit()?;
    check(cancel)?;
    let xmin = model
        .pairs
        .iter()
        .map(|p| p[0])
        .fold(f64::INFINITY, f64::min);
    let xmax = model
        .pairs
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let mut plot = PlotData::new("X", "Y");
    plot.push("Observations", PlotStyle::Scatter, model.pairs.clone())?;
    plot.push(
        format!("{} fit", model.fit_kind.label()),
        PlotStyle::Line,
        (0..=128)
            .map(|i| {
                check(cancel)?;
                let x = xmin + (xmax - xmin) * i as f64 / 128.;
                Ok([x, fit.predict(x)?])
            })
            .collect::<Result<Vec<_>, String>>()?,
    )?;
    Ok(plot)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmortizationRow {
    pub first_payment: usize,
    pub last_payment: usize,
    pub principal: f64,
    pub interest: f64,
    /// Balance immediately after this group's last payment. For beginning
    /// payments, the final period's unpaid accrual is reported separately.
    pub balance: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmortizationSchedule {
    pub groups: Vec<AmortizationRow>,
    pub final_accrued_interest: f64,
    pub terminal_balance: f64,
    /// terminal_balance + contractual FV; zero for a consistent TVM solution.
    pub residual: f64,
    pub decimal_places: Option<u8>,
}
fn monetary(value: f64, places: Option<u8>) -> Result<f64, String> {
    finite([value])?;
    if let Some(places) = places {
        if places > 12 {
            return Err("Amortization precision must be between 0 and 12 decimal places".into());
        }
        let scale = 10f64.powi(i32::from(places));
        let rounded = (value * scale).round() / scale;
        finite([rounded])?;
        Ok(rounded)
    } else {
        Ok(value)
    }
}
pub fn amortization_schedule(
    model: &Finance,
    group_size: usize,
    decimal_places: Option<u8>,
    cancel: &AtomicBool,
) -> Result<AmortizationSchedule, String> {
    check(cancel)?;
    finite([
        model.periods,
        model.annual_rate,
        model.present_value,
        model.payment,
        model.future_value,
        model.payments_per_year,
        model.compounds_per_year,
    ])?;
    if model.periods.fract() != 0. || model.periods < 1. || model.periods > MAX_INPUT as f64 {
        return Err(format!(
            "Amortization requires an integer number of 1 to {MAX_INPUT} payments"
        ));
    }
    if group_size == 0 || group_size > MAX_INPUT {
        return Err("Payment group size must be between 1 and 10000".into());
    }
    if model.payments_per_year <= 0. || model.compounds_per_year <= 0. {
        return Err("Payment and compounding frequencies must be positive".into());
    }
    let base = 1. + model.annual_rate / (100. * model.compounds_per_year);
    if base <= 0. {
        return Err("Compounding interest base must be positive".into());
    }
    let rate = (base.ln() * model.compounds_per_year / model.payments_per_year).exp_m1();
    finite([rate])?;
    if rate <= -1. {
        return Err("Per-payment interest is below numerical precision".into());
    }
    let payment = monetary(model.payment, decimal_places)?;
    let mut balance = monetary(model.present_value, decimal_places)?;
    let mut groups = Vec::new();
    let mut principal = 0.;
    let mut interest = 0.;
    let mut first = 1;
    for number in 1..=model.periods as usize {
        check(cancel)?;
        let charge = if model.begin && number == 1 {
            0.
        } else {
            monetary(balance * rate, decimal_places)?
        };
        let repaid = monetary(-payment - charge, decimal_places)?;
        balance = monetary(balance - repaid, decimal_places)?;
        principal = monetary(principal + repaid, decimal_places)?;
        interest = monetary(interest + charge, decimal_places)?;
        if number % group_size == 0 || number == model.periods as usize {
            groups.push(AmortizationRow {
                first_payment: first,
                last_payment: number,
                principal,
                interest,
                balance,
            });
            first = number + 1;
            principal = 0.;
            interest = 0.;
        }
    }
    let final_accrued_interest = if model.begin {
        monetary(balance * rate, decimal_places)?
    } else {
        0.
    };
    let terminal_balance = monetary(balance + final_accrued_interest, decimal_places)?;
    let residual = terminal_balance + model.future_value;
    finite([residual])?;
    Ok(AmortizationSchedule {
        groups,
        final_accrued_interest,
        terminal_balance,
        residual,
        decimal_places,
    })
}
pub fn amortization(
    model: &Finance,
    group_size: usize,
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    let schedule = amortization_schedule(model, group_size, Some(2), cancel)?;
    let mut plot = PlotData::new("Payment group", "Amount");
    let mut balances = Vec::new();
    let mut principal = Vec::new();
    let mut interest = Vec::new();
    for (i, row) in schedule.groups.iter().enumerate() {
        check(cancel)?;
        let group = i as f64 + 1.;
        balances.extend([
            [group - 0.4, 0.],
            [group - 0.4, row.balance],
            [group + 0.4, row.balance],
            [group + 0.4, 0.],
            [group - 0.4, 0.],
        ]);
        principal.push([group, row.principal]);
        interest.push([group, row.interest]);
    }
    plot.push("Balance after payment group", PlotStyle::Line, balances)?;
    plot.push("Principal paid in group", PlotStyle::Line, principal)?;
    plot.push("Interest paid in group", PlotStyle::Line, interest)?;
    Ok(plot)
}
pub fn triangle(model: &Triangle, cancel: &AtomicBool) -> Result<PlotData, String> {
    check(cancel)?;
    let solutions = model.solve()?;
    let mut plot = PlotData::new("X", "Y");
    for (index, solution) in solutions.iter().enumerate() {
        check(cancel)?;
        let [_, b, c] = solution.sides;
        let angle = solution.angles_degrees[0].to_radians();
        let vertices = vec![
            [0., 0.],
            [c, 0.],
            [b * angle.cos(), b * angle.sin()],
            [0., 0.],
        ];
        plot.push(format!("Triangle {}", index + 1), PlotStyle::Line, vertices)?;
    }
    Ok(plot)
}
pub fn geometry(model: &Geometry, cancel: &AtomicBool) -> Result<PlotData, String> {
    check(cancel)?;
    bounded(model.points.len())?;
    bounded(model.segments.len())?;
    if model.points.is_empty() {
        return Err("Create at least one geometry point".into());
    }
    let mut plot = PlotData::new("X", "Y");
    let mut points = Vec::new();
    for point in model.points.values() {
        check(cancel)?;
        points.push([point.x, point.y]);
    }
    plot.push("Points", PlotStyle::Scatter, points)?;
    for [a, b] in &model.segments {
        check(cancel)?;
        let first = model
            .points
            .get(a)
            .ok_or_else(|| format!("Unknown segment point {a}"))?;
        let last = model
            .points
            .get(b)
            .ok_or_else(|| format!("Unknown segment point {b}"))?;
        plot.push(
            format!("{a}–{b}"),
            PlotStyle::Line,
            vec![[first.x, first.y], [last.x, last.y]],
        )?;
    }
    Ok(plot)
}

#[cfg(test)]
mod tests {
    use super::super::native_app_models::{FinanceUnknown, Point2};
    use super::*;
    fn near(a: f64, b: f64, tolerance: f64) {
        assert!((a - b).abs() < tolerance, "{a}!={b}");
    }
    #[test]
    fn histogram_uses_weighted_half_open_bins_and_explicit_range() {
        let model = Statistics1 {
            samples: vec![0., 0.5, 1., 2., 3.],
            frequencies: vec![2, 3, 5, 7, 11],
        };
        let plot = histogram(
            &model,
            &HistogramSpec {
                minimum: 0.,
                maximum: 3.,
                width: 1.,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(plot.curves[0].len(), 15);
        assert_eq!(
            (0..3)
                .map(|i| plot.curves[0][i * 5 + 1][1])
                .collect::<Vec<_>>(),
            vec![5., 5., 7.]
        );
        assert_eq!(plot.curves[0][0], [0., 0.]);
        assert_eq!(plot.curves[0][12], [3., 7.]);
        assert!(
            histogram(
                &model,
                &HistogramSpec {
                    minimum: 0.,
                    maximum: 3.,
                    width: 0.
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
    #[test]
    fn scatter_and_regression_use_same_observations() {
        let model = Statistics2 {
            fit_kind: Default::default(),
            pairs: vec![[1., 3.], [2., 5.], [3., 7.]],
        };
        let plot = statistics2(&model, &AtomicBool::new(false)).unwrap();
        assert_eq!(plot.styles, vec![PlotStyle::Scatter, PlotStyle::Line]);
        assert_eq!(plot.curves[0], model.pairs);
        assert_eq!(plot.curves[1].len(), 129);
        for &[x, y] in &plot.curves[1] {
            assert!((y - (2. * x + 1.)).abs() < 1e-10);
        }
        assert!(
            statistics2(
                &Statistics2 {
                    fit_kind: Default::default(),
                    pairs: vec![[1., 2.], [1., 3.]]
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
    #[test]
    fn mortgage_group_matches_documented_cent_rounded_amortization() {
        let model = Finance {
            periods: 120.,
            annual_rate: 6.5,
            present_value: 150000.,
            payment: -948.10,
            ..Finance::default()
        };
        let schedule = amortization_schedule(&model, 12, Some(2), &AtomicBool::new(false)).unwrap();
        assert_eq!(schedule.groups.len(), 10);
        near(schedule.groups[0].principal, 1676.57, 1e-8);
        near(schedule.groups[0].interest, 9700.63, 1e-8);
        near(schedule.groups[9].balance, 127164.47, 1e-8);
        let plot = amortization(&model, 12, &AtomicBool::new(false)).unwrap();
        assert_eq!(plot.curves[0][1][1], schedule.groups[0].balance);
    }
    #[test]
    fn end_and_begin_payments_preserve_contractual_future_value() {
        for begin in [false, true] {
            let mut model = Finance {
                periods: 24.,
                annual_rate: 8.,
                present_value: 10000.,
                future_value: -2500.,
                begin,
                ..Finance::default()
            };
            model.payment = model.solve(FinanceUnknown::Payment).unwrap();
            let schedule = amortization_schedule(&model, 5, None, &AtomicBool::new(false)).unwrap();
            assert_eq!(schedule.groups.last().unwrap().last_payment, 24);
            near(schedule.terminal_balance, 2500., 1e-8);
            near(schedule.residual, 0., 1e-8);
            if begin {
                assert!(schedule.final_accrued_interest > 0.);
            } else {
                assert_eq!(schedule.final_accrued_interest, 0.);
            }
        }
        let model = Finance {
            periods: 2.,
            annual_rate: 120.,
            present_value: 100.,
            payment: -50.,
            begin: true,
            ..Finance::default()
        };
        let schedule = amortization_schedule(&model, 1, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(schedule.groups[0].interest, 0.);
        assert_eq!(schedule.groups[0].principal, 50.);
    }
    #[test]
    fn triangle_outlines_preserve_side_lengths_for_both_ambiguous_solutions() {
        let model = Triangle {
            sides: [Some(7.), Some(10.), None],
            angles_degrees: [Some(30.), None, None],
        };
        let solutions = model.solve().unwrap();
        let plot = triangle(&model, &AtomicBool::new(false)).unwrap();
        assert_eq!(plot.curves.len(), 2);
        for (curve, solution) in plot.curves.iter().zip(solutions) {
            let point = |index: usize| Point2 {
                x: curve[index][0],
                y: curve[index][1],
            };
            near(
                point(0).distance(point(1)).unwrap(),
                solution.sides[2],
                1e-10,
            );
            near(
                point(1).distance(point(2)).unwrap(),
                solution.sides[0],
                1e-10,
            );
            near(
                point(2).distance(point(0)).unwrap(),
                solution.sides[1],
                1e-10,
            );
            assert_eq!(curve[0], curve[3]);
        }
    }
    #[test]
    fn geometry_resolves_real_named_points_and_rejects_broken_segments() {
        let mut model = Geometry {
            points: std::collections::BTreeMap::from([
                ("A".into(), Point2 { x: 0., y: 0. }),
                ("B".into(), Point2 { x: 3., y: 4. }),
            ]),
            segments: vec![["A".into(), "B".into()]],
        };
        let plot = geometry(&model, &AtomicBool::new(false)).unwrap();
        assert_eq!(plot.curves[1], vec![[0., 0.], [3., 4.]]);
        assert_eq!(plot.labels[1], "A–B");
        model.segments[0][1] = "Missing".into();
        assert!(geometry(&model, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn canceled_and_unbounded_work_is_rejected_before_computation() {
        let canceled = AtomicBool::new(true);
        assert_eq!(
            histogram_auto(&Statistics1::default(), &canceled).unwrap_err(),
            "Plot interrupted"
        );
        assert!(
            amortization(
                &Finance {
                    periods: 10001.,
                    ..Finance::default()
                },
                12,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(
            histogram_auto(
                &Statistics1 {
                    samples: vec![1.; 10001],
                    frequencies: vec![]
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let model = Statistics1 {
            samples: vec![1., 2.],
            frequencies: vec![9_000_000, 1],
        };
        assert!(
            histogram_auto(&model, &AtomicBool::new(false)).is_ok(),
            "autoscale must honor bin limit with large weighted count"
        );
    }
}
