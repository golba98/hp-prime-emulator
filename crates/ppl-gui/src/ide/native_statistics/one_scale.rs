//! Original G1 Statistics Autoscale. Padding is applied in reverse checked-row
//! order, retaining already padded bounds from later rows. Some equal-data
//! charts leave the preceding window intact. Histogram bin limits are separate
//! calculator settings and still change when no new window can be determined.
use super::{
    OneBook,
    one::Chart,
    one_data::{Data, Frequency},
    one_plots,
};
use crate::ide::native_application_access::geometry::ReadSnapshot;
use crate::ide::{
    native_apps::PlotWindow,
    native_inference::Distribution,
    native_model_plots::{HistogramSpec, PlotScaleBounds},
};
use ppl_vm::SessionSnapshot;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct ScaleResult {
    pub bounds: Option<PlotScaleBounds>,
    pub histogram: Option<HistogramSpec>,
}
#[derive(Clone, Copy)]
struct Bounds {
    x: [f64; 2],
    y: [f64; 2],
}
impl Bounds {
    fn join(self, other: Self) -> Self {
        Self {
            x: [self.x[0].min(other.x[0]), self.x[1].max(other.x[1])],
            y: [self.y[0].min(other.y[0]), self.y[1].max(other.y[1])],
        }
    }
    fn pad(&mut self, x: f64, ylo: f64, yhi: f64) {
        let dx = (self.x[1] - self.x[0]) * x;
        let dy = self.y[1] - self.y[0];
        self.x = [self.x[0] - dx, self.x[1] + dx];
        self.y = [self.y[0] - dy * ylo, self.y[1] + dy * yhi];
    }
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Statistics Autoscale interrupted".into())
    } else {
        Ok(())
    }
}
fn histogram_spec(data: &Data, width: ppl_math::Real) -> Result<HistogramSpec, String> {
    let mut values = data
        .values
        .iter()
        .zip(&data.weights)
        .filter(|(_, w)| !w.is_zero())
        .map(|(&x, _)| x);
    let first = values.next().ok_or("Insufficient statistics data")?;
    let (mut lo, mut hi) = (first, first);
    for value in values {
        if value.cmp_value(lo) == std::cmp::Ordering::Less {
            lo = value;
        }
        if value.cmp_value(hi) == std::cmp::Ordering::Greater {
            hi = value;
        }
    }
    let floor = |value: ppl_math::Real| {
        value
            .div(width)
            .map(|number| number.floor())
            .and_then(|number| number.mul(width))
            .map_err(|error| error.to_string())
    };
    Ok(HistogramSpec {
        minimum: floor(lo)?.to_f64(),
        maximum: floor(hi)?.add(width).map_err(|e| e.to_string())?.to_f64(),
        width: width.to_f64(),
    })
}
fn histogram_peak(data: &Data, spec: HistogramSpec, cancel: &AtomicBool) -> Result<f64, String> {
    let mut bins = BTreeMap::<i128, Frequency>::new();
    let membership = one_plots::HistogramBins::new(spec)?;
    for (value, &weight) in data.values.iter().zip(&data.weights) {
        check(cancel)?;
        let Some(bin) = membership.index(*value)? else {
            continue;
        };
        let total = bins.entry(bin).or_insert(Frequency::ZERO);
        *total = total.checked_add(weight)?;
    }
    Ok(bins.values().map(|count| count.to_f64()).fold(0., f64::max))
}
/// Computes an actual app-owned scale from checked definitions, without changing
/// calculator memory. The GUI commits window/bin limits under its normal fence.
pub fn autoscale(
    book: &OneBook,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<ScaleResult, String> {
    autoscale_with_geometry(book, snapshot, cancel, None)
}

pub(crate) fn autoscale_with_geometry(
    book: &OneBook,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
    geometry: ReadSnapshot,
) -> Result<ScaleResult, String> {
    check(&cancel)?;
    book.validate()?;
    let settings = snapshot
        .plot_settings_for("Statistics 1Var")
        .ok_or("Missing statistics Plot settings")?;
    let width = settings.hwidth;
    if width.is_zero() || width.is_negative() || !width.to_f64().is_finite() || width.to_f64() == 0.
    {
        return Err("Histogram width exceeds plot numerical range".into());
    }
    let mut analyses = Vec::new();
    for (index, analysis) in book
        .analyses
        .iter()
        .enumerate()
        .filter(|(_, analysis)| analysis.enabled)
    {
        check(&cancel)?;
        let data =
            Data::evaluate_with_geometry(analysis, snapshot, cancel.clone(), geometry.clone())?;
        let model = one_plots::projection(&data, analysis.chart)?;
        analyses.push((index, analysis, data, model));
    }
    if analyses.is_empty() {
        return Err("Select at least one H analysis".into());
    }
    let histogram = analyses
        .iter()
        .rev()
        .find(|(_, analysis, _, _)| analysis.chart == Chart::Histogram)
        .map(|(_, _, data, _)| histogram_spec(data, width))
        .transpose()?;
    let mut bounds: Option<Bounds> = None;
    let mut unbounded_y = false;
    for (index, analysis, data, model) in analyses.iter().rev() {
        check(&cancel)?;
        let values = &model.samples;
        let weighted_box = analysis.chart == Chart::BoxWhisker && analysis.option == 1;
        let observations = values
            .iter()
            .zip(&data.weights)
            .filter_map(|(&value, weight)| (!weighted_box || !weight.is_zero()).then_some(value));
        let lo = observations.clone().fold(f64::INFINITY, f64::min);
        let hi = observations.fold(f64::NEG_INFINITY, f64::max);
        let n = values.len() as f64;
        if matches!(
            analysis.chart,
            Chart::Histogram | Chart::BoxWhisker | Chart::NormalProbability | Chart::Control
        ) && lo == hi
        {
            continue;
        }
        let mut raw = match analysis.chart {
            Chart::Histogram => {
                let spec = histogram_spec(data, width)?;
                let span = (spec.maximum - spec.minimum).max(12. * spec.width);
                let center = spec.minimum / 2. + spec.maximum / 2.;
                let peak = histogram_peak(data, histogram.unwrap(), &cancel)?;
                // A checked histogram with no samples in the shared range
                // makes the firmware Autoscale result unbounded along Y.
                unbounded_y |= peak == 0.;
                let mut own = Bounds {
                    x: [center - span / 2., center + span / 2.],
                    y: [0., peak],
                };
                own.pad(0.05, 0.1, 0.1);
                bounds = Some(bounds.map_or(own, |prior| prior.join(own)));
                continue;
            }
            Chart::BoxWhisker => Bounds {
                x: [lo, hi],
                y: [2. * *index as f64, 2. * *index as f64 + 3.],
            },
            Chart::NormalProbability => {
                let z = Distribution::Normal.inverse_upper_tail(0.5 / n)?;
                Bounds {
                    x: [lo, hi],
                    y: [-z, z],
                }
            }
            Chart::Line | Chart::Bar => {
                let maximum = if analysis.chart == Chart::Line
                    && !analysis.frequency_expression().is_empty()
                {
                    data.weights
                        .iter()
                        .map(|count| count.to_f64())
                        .fold(0., f64::max)
                } else {
                    hi
                };
                let minimum = if analysis.chart == Chart::Line
                    && !analysis.frequency_expression().is_empty()
                {
                    0.
                } else {
                    lo.min(0.)
                };
                Bounds {
                    x: [0., n],
                    y: [minimum, maximum.max(0.)],
                }
            }
            Chart::Pareto => {
                if lo < 0. {
                    return Err("Pareto Autoscale requires nonnegative observations".into());
                }
                Bounds {
                    x: [0., n],
                    y: [0., values.iter().sum()],
                }
            }
            Chart::Control => {
                if values.len() < 2 {
                    return Err("Control charts need at least two observations".into());
                }
                let mr = values
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]).abs() / (n - 1.))
                    .sum::<f64>();
                let center = values.iter().map(|value| value / n).sum::<f64>();
                let y = if analysis.option == 2 {
                    [0., 3.267 * mr]
                } else {
                    [center - 2.66 * mr, center + 2.66 * mr]
                };
                Bounds { x: [1., n], y }
            }
            Chart::Dot => {
                let grouped = one_plots::dot_groups(data, &cancel)?;
                let peak = grouped
                    .iter()
                    .map(|(_, count)| count.to_f64())
                    .fold(0., f64::max);
                Bounds {
                    x: [lo - 0.5, hi + 0.5],
                    y: [0., peak + 0.5],
                }
            }
            Chart::StemLeaf => {
                let layout = one_plots::stem_layout(values, analysis.option, &cancel)?;
                let columns = layout
                    .rows
                    .iter()
                    .map(|(_, leaves)| leaves.len())
                    .max()
                    .unwrap_or(0);
                Bounds {
                    x: [-2., if lo == hi { 1. } else { columns as f64 + 1. }],
                    y: [-2., layout.rows.len() as f64 + 1.],
                }
            }
            Chart::Pie => {
                if lo < 0. || hi <= 0. {
                    return Err(
                        "Pie Autoscale requires nonnegative observations and a positive total"
                            .into(),
                    );
                }
                let total = values.iter().map(|value| value / hi).sum::<f64>();
                let radius = hi.sqrt() * (total / std::f64::consts::PI).sqrt();
                let own = Bounds {
                    x: [-1., 2. + 2. * radius],
                    y: [-1., 2. + 2. * radius],
                };
                bounds = Some(bounds.map_or(own, |prior| prior.join(own)));
                continue;
            }
        };
        raw = bounds.map_or(raw, |prior| prior.join(raw));
        match analysis.chart {
            Chart::BoxWhisker => raw.pad(0.05, 0.05, 0.1),
            Chart::Line | Chart::Bar | Chart::Pareto => raw.pad(0.05, 0.1, 0.1),
            _ => raw.pad(1. / 30., 1. / 30., 1. / 30.),
        }
        bounds = Some(raw);
    }
    let bounds = if let Some(mut bounds) = bounds {
        let uncorrected_left = bounds.x[0];
        let pixel_width = (bounds.x[1] - bounds.x[0]) / 319.;
        bounds.x[0] += pixel_width;
        // Original Line Autoscale bases its X range on row count even with
        // explicit (data,frequency) coordinates, then pans to the first trace.
        let (_, first, _, model) = &analyses[0];
        if first.chart == Chart::Line && !first.frequency_expression().is_empty() {
            if let Some(&x) = model.samples.first() {
                let cursor = ((x - uncorrected_left) / pixel_width).round();
                let shift = if cursor < 0. {
                    cursor * pixel_width
                } else if cursor > 319. {
                    (cursor - 319.) * pixel_width
                } else {
                    0.
                };
                bounds.x[0] += shift;
                bounds.x[1] += shift;
            }
        }
        if unbounded_y {
            if !bounds.x.iter().all(|x| x.is_finite()) || bounds.x[0] >= bounds.x[1] {
                return Err("Plot bounds must be finite and increasing".into());
            }
            Some(PlotScaleBounds::UnboundedY { x: bounds.x })
        } else {
            let [xmin, xmax] = bounds.x;
            let [ymin, ymax] = bounds.y;
            PlotWindow {
                xmin,
                xmax,
                ymin,
                ymax,
                ..Default::default()
            }
            .validate()?;
            Some(PlotScaleBounds::Finite([xmin, xmax, ymin, ymax]))
        }
    } else {
        None
    };
    check(&cancel)?;
    Ok(ScaleResult { bounds, histogram })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_math::{NumericVersion, Real};
    use ppl_vm::{PortableValue, SessionContext};

    fn finite_bounds(scale: &ScaleResult) -> [f64; 4] {
        match scale.bounds {
            Some(PlotScaleBounds::Finite(bounds)) => bounds,
            ref other => panic!("Expected finite Autoscale bounds: {other:?}"),
        }
    }

    #[test]
    fn original_box_options_use_raw_fences_or_weighted_extrema() {
        let mut context = SessionContext::default();
        let list = |values: &[i64]| {
            PortableValue::List(
                values
                    .iter()
                    .map(|&value| PortableValue::Real(Real::from_i64(value)))
                    .collect(),
            )
        };
        context
            .set_variable("D1", list(&[0, 1, 2, 4, 6, 8, 9, 10, -80, 90]))
            .unwrap();
        context
            .set_variable("D2", list(&[0, 0, 3, 3, 3, 3, 0, 0, 0, 0]))
            .unwrap();
        let snapshot = context.snapshot();
        let mut book = OneBook::default();
        let analysis = &mut book.analyses[0];
        analysis.chart = Chart::BoxWhisker;
        analysis.option = 2;
        analysis.frequency_source = Some("D2".into());
        let cancel = Arc::new(AtomicBool::new(false));
        let shown = one_plots::plot(&book, &snapshot, None, cancel.clone()).unwrap();
        let markers = |plot: &crate::ide::native_model_plots::PlotData| {
            plot.statistics_traces[0]
                .samples
                .iter()
                .map(|sample| (sample.label.clone().unwrap(), sample.x))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            markers(&shown),
            [
                ("Outlier", -80.),
                ("Min", 0.),
                ("Q1", 3.),
                ("Med", 5.),
                ("Q3", 7.),
                ("Max", 10.),
                ("Outlier", 90.),
            ]
            .map(|(label, x)| (label.to_owned(), x))
        );
        assert_eq!(shown.curves[0], [[-80., 1.5], [90., 1.5]]);
        let raw = finite_bounds(&autoscale(&book, &snapshot, cancel.clone()).unwrap());
        assert!((raw[0] + 87.9137931034).abs() < 1e-9);
        assert_eq!(raw[1], 98.5);

        book.analyses[0].option = 1;
        let weighted = finite_bounds(&autoscale(&book, &snapshot, cancel.clone()).unwrap());
        assert!((weighted[0] - 1.72068965517).abs() < 1e-11);
        assert_eq!(weighted[1], 8.3);
        let hidden = one_plots::plot(&book, &snapshot, None, cancel).unwrap();
        assert_eq!(
            markers(&hidden),
            [
                ("Min", 2.),
                ("Q1", 3.),
                ("Med", 5.),
                ("Q3", 7.),
                ("Max", 8.)
            ]
            .map(|(label, x)| (label.to_owned(), x))
        );
    }
    #[test]
    fn original_fractional_weights_scale_all_charts_and_preserve_plot_frequencies() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-fractional-controls.json"
        ))
        .unwrap();
        let list = |source: &[&str]| {
            PortableValue::List(
                source
                    .iter()
                    .map(|s| PortableValue::Real(Real::parse(s).unwrap()))
                    .collect(),
            )
        };
        let mut context = SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable("D1", list(&["1", "2", "4", "8"]))
            .unwrap();
        context
            .set_variable("D2", list(&["0.5", "1.25", "2.25", "1.5"]))
            .unwrap();
        let snapshot = context.snapshot();
        let mut book = OneBook::default();
        book.analyses[0].frequency_source = Some("D2".into());
        for control in fixture["autoscale_cases"].as_array().unwrap() {
            let chart = Chart::from_code(control["chart_code"].as_i64().unwrap()).unwrap();
            book.analyses[0].chart = chart;
            book.analyses[0].option = chart.default_option();
            let cancel = Arc::new(AtomicBool::new(false));
            let scale = autoscale(&book, &snapshot, cancel.clone()).unwrap();
            let actual = finite_bounds(&scale);
            let expected = control["bounds"].as_array().unwrap();
            for (actual, expected) in actual.into_iter().zip(expected) {
                let expected = expected.as_str().unwrap().parse::<f64>().unwrap();
                assert!(
                    (actual - expected).abs() < 2e-9,
                    "{chart:?}: {actual} != {expected}"
                );
            }
            let data = one_plots::plot(&book, &snapshot, scale.histogram, cancel).unwrap();
            let trace = &data.statistics_traces[0].samples;
            match chart {
                Chart::Histogram => {
                    let heights: Vec<_> = data.curves[0].chunks_exact(5).map(|p| p[1][1]).collect();
                    assert_eq!(heights, [0.5, 1.25, 0., 2.25, 0., 0., 0., 1.5]);
                    assert_eq!(trace[3].y, Some(2.25));
                }
                Chart::Line => {
                    assert_eq!(
                        data.curves[0],
                        [[1., 0.5], [2., 1.25], [4., 2.25], [8., 1.5]]
                    );
                    assert_eq!(trace[2].y, Some(2.25));
                }
                Chart::Dot => {
                    let disks: Vec<_> = data.dots.iter().map(|c| (c.count, c.fraction)).collect();
                    assert_eq!(
                        disks,
                        [
                            (0, Some(0.5)),
                            (1, Some(0.25)),
                            (2, Some(0.25)),
                            (1, Some(0.5))
                        ]
                    );
                    assert!(trace[2].frequency.is_none());
                    let count = trace[2].fractional_frequency.as_ref().unwrap();
                    assert_eq!(
                        ppl_vm::session::decode_value(count)
                            .unwrap()
                            .to_value()
                            .expect_real("frequency")
                            .unwrap()
                            .format(Default::default()),
                        "2.25"
                    );
                }
                _ => {}
            }
        }

        context.set_variable("D1", list(&["1", "2", "3"])).unwrap();
        context
            .set_variable("D2", list(&["0", "0.5", "1.5"]))
            .unwrap();
        book.analyses[0].chart = Chart::Dot;
        let data = one_plots::plot(
            &book,
            &context.snapshot(),
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(data.statistics_traces[0].samples[0].frequency, Some(0));
        assert_eq!(data.dots[0].count, 0);
        assert_eq!(data.dots[0].fraction, None);
        assert_eq!(data.dots[1].color, 0xa9a9f6);
    }

    #[test]
    fn original_all_zero_dot_autoscales_and_traces_each_distinct_observation() {
        let mut context = SessionContext::default();
        let integers = |values: &[i64]| {
            PortableValue::List(
                values
                    .iter()
                    .map(|&value| PortableValue::Real(Real::from_i64(value)))
                    .collect(),
            )
        };
        context
            .set_variable("D1", integers(&[8, 1, 4, -7, 1]))
            .unwrap();
        context.set_variable("D2", integers(&[0; 5])).unwrap();
        let snapshot = context.snapshot();
        let mut book = OneBook::default();
        book.analyses[0].frequency_source = Some("D2".into());
        book.analyses[0].chart = Chart::Dot;
        let cancel = Arc::new(AtomicBool::new(false));
        let bounds = finite_bounds(&autoscale(&book, &snapshot, cancel.clone()).unwrap());
        for (actual, expected) in bounds.into_iter().zip([
            -7.97983281087,
            9.03333333333,
            -0.0166666666667,
            0.516666666667,
        ]) {
            assert!((actual - expected).abs() < 1e-11);
        }
        let plot = one_plots::plot(&book, &snapshot, None, cancel.clone()).unwrap();
        assert_eq!(
            plot.statistics_traces[0]
                .samples
                .iter()
                .map(|sample| (sample.x, sample.y, sample.frequency))
                .collect::<Vec<_>>(),
            [-7., 1., 4., 8.].map(|x| (x, Some(0.), Some(0)))
        );
        assert!(
            plot.dots
                .iter()
                .all(|dot| dot.count == 0 && dot.fraction.is_none())
        );
        assert!(
            Data::evaluate(&book.analyses[0], &snapshot, cancel.clone())
                .unwrap()
                .summary(&cancel)
                .is_err()
        );
        book.analyses[0].chart = Chart::Histogram;
        assert!(one_plots::plot(&book, &snapshot, None, cancel.clone()).is_err());
        assert!(autoscale(&book, &snapshot, cancel).is_err());
    }

    #[test]
    fn thirty_single_and_ten_multi_original_autoscales_reproduce_independent_data_controls() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-autoscale-controls.json"
        ))
        .unwrap();
        let mut verified = 0;
        for case in fixture["cases"].as_array().unwrap() {
            for row in case["windows"].as_array().unwrap() {
                let chart = Chart::from_code(row[0].as_i64().unwrap()).unwrap();
                let mut context = SessionContext::default();
                context.settings.numeric_version = NumericVersion::Current15751;
                let mut book = OneBook::default();
                for definition in &mut book.analyses {
                    definition.enabled = false;
                }
                for (index, values) in case["data"].as_array().unwrap().iter().enumerate() {
                    let data = values
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| PortableValue::Real(Real::from_i64(value.as_i64().unwrap())))
                        .collect();
                    context
                        .set_variable(&format!("D{}", index + 1), PortableValue::List(data))
                        .unwrap();
                    let definition = &mut book.analyses[index];
                    definition.enabled = true;
                    definition.data = Some(index);
                    definition.chart = chart;
                    definition.option = chart.default_option();
                    definition.frequency_source = Some(case["frequency"].to_string());
                }
                let result =
                    autoscale(&book, &context.snapshot(), Arc::new(AtomicBool::new(false)))
                        .unwrap();
                if row[3].is_string() {
                    assert_eq!(row[3], "-Inf");
                    assert_eq!(row[4], "+Inf");
                    let Some(PlotScaleBounds::UnboundedY { x }) = result.bounds else {
                        panic!("Expected unbounded Y, got {:?}", result.bounds);
                    };
                    for (actual, expected) in x.into_iter().zip(&row.as_array().unwrap()[1..3]) {
                        assert!((actual - expected.as_f64().unwrap()).abs() < 1e-9);
                    }
                    let histogram = result.histogram.unwrap();
                    assert_eq!(histogram.minimum, row[5].as_f64().unwrap());
                    assert_eq!(histogram.maximum, row[6].as_f64().unwrap());
                    assert_eq!(histogram.width, row[7].as_f64().unwrap());
                    verified += 1;
                    continue;
                }
                let values = case["data"][0].as_array().unwrap();
                let equal = values.iter().all(|value| value == &values[0]);
                if equal
                    && matches!(
                        chart,
                        Chart::Histogram
                            | Chart::BoxWhisker
                            | Chart::NormalProbability
                            | Chart::Control
                    )
                {
                    assert!(result.bounds.is_none(), "{} {chart:?}", case["name"]);
                } else {
                    for (actual, expected) in finite_bounds(&result).into_iter().zip(
                        row.as_array().unwrap()[1..5]
                            .iter()
                            .map(|value| value.as_f64().unwrap()),
                    ) {
                        assert!(
                            (actual - expected).abs() <= 1e-9 * expected.abs().max(1.),
                            "{} {chart:?}: {actual} != {expected}",
                            case["name"]
                        );
                    }
                }
                if chart == Chart::Histogram {
                    let histogram = result.histogram.unwrap();
                    assert_eq!(histogram.minimum, row[5].as_f64().unwrap());
                    assert_eq!(histogram.maximum, row[6].as_f64().unwrap());
                    assert_eq!(histogram.width, row[7].as_f64().unwrap());
                }
                verified += 1;
            }
        }
        assert_eq!(verified, 40);
    }
    #[test]
    fn original_line_trace_pans_by_integral_pixels_at_the_left_edge() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-autoscale-controls.json"
        ))
        .unwrap();
        let control = &fixture["line_left_pan_observations"];
        let mut context = SessionContext::default();
        context
            .set_variable(
                "D1",
                PortableValue::List(
                    control["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|n| PortableValue::Real(Real::from_i64(n.as_i64().unwrap())))
                        .collect(),
                ),
            )
            .unwrap();
        let mut book = OneBook::default();
        book.analyses[0].chart = Chart::Line;
        book.analyses[0].frequency_source = Some(control["frequency"].to_string());
        let scale =
            autoscale(&book, &context.snapshot(), Arc::new(AtomicBool::new(false))).unwrap();
        for (value, expected) in finite_bounds(&scale).into_iter().zip(
            control["bounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap()),
        ) {
            assert!((value - expected).abs() <= 1e-9 * expected.abs().max(1.));
        }
    }
    #[test]
    fn original_fractional_histogram_edges_keep_distinct_bins_in_both_autoscale_and_plot() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-autoscale-controls.json"
        ))
        .unwrap();
        let control = &fixture["fractional_histogram_observations"];
        let mut context = SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable(
                "D1",
                PortableValue::List(
                    control["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| {
                            PortableValue::Real(
                                Real::parse_versioned(
                                    value.as_str().unwrap(),
                                    NumericVersion::Current15751,
                                )
                                .unwrap(),
                            )
                        })
                        .collect(),
                ),
            )
            .unwrap();
        let mut settings = ppl_vm::AppPlotSettings::for_app("Statistics 1Var").unwrap();
        settings.hwidth = Real::parse_versioned("0.1", NumericVersion::Current15751)
            .unwrap()
            .decimal();
        context
            .app_plot_settings
            .insert("Statistics 1Var".into(), settings);
        let snapshot = context.snapshot();
        let mut book = OneBook::default();
        book.analyses[0].frequency_source = Some(control["frequency"].to_string());
        let scale = autoscale(&book, &snapshot, Arc::new(AtomicBool::new(false))).unwrap();
        for (value, expected) in finite_bounds(&scale).into_iter().zip(
            control["bounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap()),
        ) {
            assert!((value - expected).abs() < 1e-10);
        }
        let histogram = scale.histogram.unwrap();
        let plot = one_plots::plot(
            &book,
            &snapshot,
            Some(histogram),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let frequencies = plot.curves[0]
            .chunks_exact(5)
            .map(|points| points[1][1])
            .collect::<Vec<_>>();
        let expected: Vec<f64> = serde_json::from_value(control["bins"].clone()).unwrap();
        assert_eq!(frequencies, expected);
    }
    #[test]
    fn cancellation_and_invalid_definitions_never_propose_a_window() {
        let context = SessionContext::default();
        let mut book = OneBook::default();
        assert!(autoscale(&book, &context.snapshot(), Arc::new(AtomicBool::new(true))).is_err());
        book.analyses[0].enabled = false;
        assert!(autoscale(&book, &context.snapshot(), Arc::new(AtomicBool::new(false))).is_err());
    }
}
