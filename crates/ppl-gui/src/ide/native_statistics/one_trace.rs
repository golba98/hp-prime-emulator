//! Statistical trace samples are calculator data, independent of paint contours.
//! Undefined moving-range entries use Option rather than a non-JSON NaN. Typed
//! values remain encoded calculator objects until the current display settings
//! format the footer; whole counts and fractional weights retain their types.
use super::{
    OneAnalysis,
    one::Chart,
    one_data::{Data, Frequency},
    one_plots,
};
use crate::ide::native_model_plots::{
    HistogramSpec, PlotData, StatisticsTraceSample as Sample, StatisticsTraceSeries,
};
use ppl_math::Real;
use ppl_vm::PortableValue;
use std::sync::atomic::{AtomicBool, Ordering};
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Statistics trace interrupted".into())
    } else {
        Ok(())
    }
}
fn encoded(value: Real) -> serde_json::Value {
    ppl_vm::session::encode_value(&PortableValue::Real(value))
}
fn approximate(value: f64) -> Result<Real, String> {
    Real::from_f64(value).map_err(|error| error.to_string())
}
fn sample(x: f64, y: Option<f64>, value: Option<Real>) -> Sample {
    Sample {
        x,
        y,
        value: value.map(encoded),
        auxiliary: None,
        frequency: None,
        fractional_frequency: None,
        row: None,
        label: None,
    }
}
fn set_frequency(sample: &mut Sample, frequency: Frequency) {
    match frequency {
        Frequency::Whole(count) => sample.frequency = Some(count),
        Frequency::Fractional(value) => sample.fractional_frequency = Some(encoded(value)),
    }
}
fn sorted(data: &Data) -> Vec<Real> {
    let mut values = data.values.clone();
    values.sort_by(|a, b| a.cmp_value(*b));
    values
}
fn sum(values: &[Real], cancel: &AtomicBool) -> Result<Real, String> {
    let mut sum = Real::ZERO;
    for &value in values {
        check(cancel)?;
        sum = sum.add(value).map_err(|error| error.to_string())?;
    }
    Ok(sum)
}
fn percent(value: Real, total: Real) -> Result<Real, String> {
    value
        .div(total)
        .and_then(|value| value.mul(Real::from_i64(100)))
        .map_err(|error| error.to_string())
}
/// Uses the same immutable observations as the paint computation. Coordinates
/// select the actual row/bin/statistic, never an arbitrary polygon vertex.
pub(super) fn build(
    analysis: &OneAnalysis,
    index: usize,
    data: &Data,
    histogram: Option<HistogramSpec>,
    geometry: &PlotData,
    cancel: &AtomicBool,
) -> Result<StatisticsTraceSeries, String> {
    check(cancel)?;
    let mut samples = Vec::new();
    match analysis.chart {
        Chart::Histogram => {
            let spec = histogram.ok_or("Missing histogram trace bins")?;
            let bins = one_plots::HistogramBins::new(spec)?;
            let count = geometry.curves.first().map_or(0, |curve| curve.len() / 5);
            let frequencies = one_plots::histogram_counts(data, spec, count, cancel)?;
            for (bin, frequency) in frequencies.into_iter().enumerate() {
                check(cancel)?;
                let (left, right) = bins.edges(bin)?;
                let mut point = sample(
                    left.to_f64() / 2. + right.to_f64() / 2.,
                    Some(frequency.to_f64()),
                    Some(left),
                );
                point.auxiliary = Some(encoded(right));
                set_frequency(&mut point, frequency);
                point.label = Some(if bin + 1 == count { "]" } else { ")" }.into());
                samples.push(point);
            }
        }
        Chart::BoxWhisker => {
            let data = one_plots::box_values(data, analysis.option, cancel)?;
            let markers = ["Min", "Q1", "Med", "Q3", "Max"]
                .into_iter()
                .zip(data.markers);
            for (label, value) in data
                .below
                .into_iter()
                .map(|value| ("Outlier", value))
                .chain(markers)
                .chain(data.above.into_iter().map(|value| ("Outlier", value)))
            {
                let mut point = sample(value.to_f64(), Some(index as f64 * 2. + 1.5), Some(value));
                point.label = Some(label.into());
                samples.push(point);
            }
        }
        Chart::NormalProbability => {
            let values = sorted(data);
            let points = geometry
                .curves
                .first()
                .ok_or("Missing normal probability points")?;
            for (value, point) in values.into_iter().zip(points) {
                check(cancel)?;
                let mut sample = sample(point[0], Some(point[1]), Some(value));
                sample.auxiliary = Some(encoded(approximate(point[1])?));
                samples.push(sample);
            }
        }
        Chart::Line => {
            let frequencies = !analysis.frequency_expression().is_empty();
            for (row, &value) in data.values.iter().enumerate() {
                check(cancel)?;
                let coordinate = if frequencies {
                    [value.to_f64(), data.weights[row].to_f64()]
                } else {
                    [row as f64 + 1., value.to_f64()]
                };
                let mut point = sample(coordinate[0], Some(coordinate[1]), Some(value));
                point.row = Some(row + 1);
                if frequencies {
                    set_frequency(&mut point, data.weights[row]);
                }
                samples.push(point);
            }
        }
        Chart::Bar => {
            for (row, &value) in data.values.iter().enumerate() {
                check(cancel)?;
                let polygon = geometry
                    .curves
                    .get(row)
                    .ok_or("Missing bar trace geometry")?;
                let mut point = sample(
                    polygon[0][0] / 2. + polygon[2][0] / 2.,
                    Some(value.to_f64()),
                    Some(value),
                );
                point.row = Some(row + 1);
                samples.push(point);
            }
        }
        Chart::Pareto => {
            let mut values = sorted(data);
            values.reverse();
            let total = sum(&values, cancel)?;
            for (row, value) in values.into_iter().enumerate() {
                check(cancel)?;
                let mut point = sample(row as f64 + 0.5, Some(value.to_f64()), Some(value));
                point.auxiliary = Some(encoded(percent(value, total)?));
                samples.push(point);
            }
        }
        Chart::Control => {
            for (row, &value) in data.values.iter().enumerate() {
                check(cancel)?;
                let value = if analysis.option == 2 {
                    if row == 0 {
                        None
                    } else {
                        Some(
                            value
                                .sub(data.values[row - 1])
                                .map_err(|error| error.to_string())?
                                .abs(),
                        )
                    }
                } else {
                    Some(value)
                };
                let mut point = sample(row as f64 + 1., value.map(Real::to_f64), value);
                point.row = Some(row + 1);
                samples.push(point);
            }
        }
        Chart::Dot => {
            for (value, count) in one_plots::dot_groups(data, cancel)? {
                let mut point = sample(value.to_f64(), Some(count.to_f64()), Some(value));
                set_frequency(&mut point, count);
                samples.push(point);
            }
        }
        Chart::StemLeaf => {
            // The table collapses equal leaves, but original trace retains
            // each sorted observation with its own one-based row number.
            let values = sorted(data);
            let layout = one_plots::stem_layout(
                &data
                    .values
                    .iter()
                    .map(|value| value.to_f64())
                    .collect::<Vec<_>>(),
                analysis.option,
                cancel,
            )?;
            for (row, value) in values.into_iter().enumerate() {
                check(cancel)?;
                let digits = (value.to_f64().abs() / layout.unit + 1e-10).floor() as i64;
                let stem = digits / 10;
                let leaf = (digits % 10) as u8;
                let split = if analysis.option == 2 {
                    u8::from(leaf >= 5)
                } else {
                    0
                };
                let position = layout
                    .rows
                    .iter()
                    .position(|((negative, s, p), _)| {
                        *negative == value.is_negative() && *s == stem && *p == split
                    })
                    .ok_or("Missing stem trace row")?;
                let leaves = &layout.rows[position].1;
                let column = leaves
                    .iter()
                    .position(|&candidate| candidate == leaf)
                    .ok_or("Missing stem trace leaf")?;
                let mut point = sample(
                    column as f64 + 0.5,
                    Some((layout.rows.len() - position) as f64 - 0.5),
                    Some(value),
                );
                point.row = Some(row + 1);
                samples.push(point);
            }
        }
        Chart::Pie => {
            let total = sum(&data.values, cancel)?;
            let maximum = data
                .values
                .iter()
                .map(|value| value.to_f64())
                .fold(0., f64::max);
            let normalized = data
                .values
                .iter()
                .map(|value| value.to_f64() / maximum)
                .sum::<f64>();
            let radius = maximum.sqrt() * (normalized / std::f64::consts::PI).sqrt();
            let mut angle = std::f64::consts::FRAC_PI_2;
            for (row, &value) in data.values.iter().enumerate() {
                check(cancel)?;
                let percentage = percent(value, total)?;
                let delta = std::f64::consts::TAU * percentage.to_f64() / 100.;
                let middle = angle - delta / 2.;
                let mut point = sample(
                    radius + 1. + radius * middle.cos(),
                    Some(radius + 1. + radius * middle.sin()),
                    Some(value),
                );
                point.auxiliary = Some(encoded(percentage));
                point.row = Some(row + 1);
                samples.push(point);
                angle -= delta;
            }
        }
    }
    check(cancel)?;
    if samples.is_empty() {
        return Err("No statistics trace samples".into());
    }
    if samples
        .iter()
        .any(|sample| !sample.x.is_finite() || sample.y.is_some_and(|y| !y.is_finite()))
    {
        return Err("Statistics trace exceeds plot numerical range".into());
    }
    Ok(StatisticsTraceSeries {
        analysis: index + 1,
        chart: analysis.chart,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_math::NumericVersion;
    use ppl_vm::SessionContext;
    #[test]
    fn original_stem_duplicates_keep_distinct_sorted_trace_rows_while_the_table_collapses_leaves() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-trace-controls.json"
        ))
        .unwrap();
        let original = &fixture["stem_duplicate_observations"];
        let mut context = SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable(
                "D1",
                PortableValue::List(
                    original["data"]
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
        let mut book = super::super::OneBook::default();
        book.analyses[0].chart = Chart::StemLeaf;
        book.analyses[0].option = 1;
        book.analyses[0].frequency_source = Some(original["frequency"].as_str().unwrap().into());
        let plot = one_plots::plot(
            &book,
            &context.snapshot(),
            None,
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let samples = &plot.statistics_traces[0].samples;
        assert_eq!(samples.len(), 3);
        for (sample, value) in samples.iter().zip(original["values"].as_array().unwrap()) {
            let real = ppl_vm::session::decode_value(sample.value.as_ref().unwrap())
                .unwrap()
                .to_value()
                .expect_real("stem value")
                .unwrap();
            assert_eq!(real.format(Default::default()), value.as_str().unwrap());
        }
        assert_eq!((samples[1].x, samples[1].y), (samples[2].x, samples[2].y));
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.row.unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
    #[test]
    fn original_manual_histogram_includes_the_upper_bound_only_in_its_last_closed_bin() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-trace-controls.json"
        ))
        .unwrap();
        let original = &fixture["histogram_upper_edge_observations"];
        let mut context = SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable(
                "D1",
                PortableValue::List(
                    original["data"]
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
        let mut book = super::super::OneBook::default();
        book.analyses[0].frequency_source = Some(original["frequency"].as_str().unwrap().into());
        let bounds = original["histogram"].as_array().unwrap();
        let histogram = HistogramSpec {
            minimum: bounds[0].as_f64().unwrap(),
            maximum: bounds[1].as_f64().unwrap(),
            width: bounds[2].as_f64().unwrap(),
        };
        let plot = one_plots::plot(
            &book,
            &context.snapshot(),
            Some(histogram),
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let expected: Vec<u64> = serde_json::from_value(original["frequencies"].clone()).unwrap();
        assert_eq!(
            plot.statistics_traces[0]
                .samples
                .iter()
                .map(|sample| sample.frequency.unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            plot.curves[0]
                .chunks_exact(5)
                .map(|polygon| polygon[1][1] as u64)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            plot.statistics_traces[0]
                .samples
                .last()
                .unwrap()
                .label
                .as_deref(),
            Some("]")
        );
        let bins = one_plots::HistogramBins::new(histogram).unwrap();
        assert!(bins.index(Real::from_i64(6)).unwrap().is_none());
    }
    #[test]
    fn all_original_unsorted_trace_footers_follow_real_rows_bins_statistics_and_chart_order() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-trace-controls.json"
        ))
        .unwrap();
        let mut context = SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable(
                "D1",
                PortableValue::List(
                    fixture["data"]
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
        let snapshot = context.snapshot();
        let real = |value: &serde_json::Value| {
            ppl_vm::session::decode_value(value)
                .unwrap()
                .to_value()
                .expect_real("trace")
                .unwrap()
        };
        for case in fixture["cases"].as_array().unwrap() {
            let mut book = super::super::OneBook::default();
            let definition = &mut book.analyses[0];
            definition.chart = Chart::from_code(case["chart"].as_i64().unwrap()).unwrap();
            definition.option = case["option"].as_u64().unwrap() as u8;
            definition.frequency_source = Some(fixture["frequency"].as_str().unwrap().into());
            let histogram = if definition.chart == Chart::Histogram {
                super::super::one_scale::autoscale(
                    &book,
                    &snapshot,
                    std::sync::Arc::new(AtomicBool::new(false)),
                )
                .unwrap()
                .histogram
            } else {
                None
            };
            let plot = one_plots::plot(
                &book,
                &snapshot,
                histogram,
                std::sync::Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert_eq!(plot.statistics_traces.len(), 1);
            let series = &plot.statistics_traces[0];
            assert_eq!(series.analysis, 1);
            assert_eq!(series.chart, book.analyses[0].chart);
            let observed = case["samples"].as_array().unwrap();
            assert!(series.samples.len() >= observed.len(), "{}", case["name"]);
            let tolerance = case["tolerance"].as_f64().unwrap_or(1e-10);
            for (sample, expected) in series.samples.iter().zip(observed) {
                if let Some(x) = expected.get("x") {
                    assert!(
                        (sample.x - x.as_f64().unwrap()).abs() < tolerance,
                        "{} X",
                        case["name"]
                    );
                }
                if let Some(y) = expected.get("y") {
                    if y.is_null() {
                        assert!(sample.y.is_none());
                    } else {
                        assert!(
                            (sample.y.unwrap() - y.as_f64().unwrap()).abs() < tolerance,
                            "{} Y",
                            case["name"]
                        );
                    }
                }
                if expected["value"].is_null() {
                    assert!(sample.value.is_none());
                } else {
                    assert_eq!(
                        real(sample.value.as_ref().unwrap()).format(Default::default()),
                        expected["value"].as_str().unwrap(),
                        "{} value",
                        case["name"]
                    );
                }
                if let Some(auxiliary) = expected.get("auxiliary") {
                    let value = real(sample.auxiliary.as_ref().unwrap());
                    if let Some(places) = case.get("auxiliary_display_places") {
                        assert_eq!(
                            format!("{:.*}", places.as_u64().unwrap() as usize, value.to_f64()),
                            auxiliary.as_str().unwrap(),
                            "{} percentage",
                            case["name"]
                        );
                    } else {
                        let wanted = auxiliary.as_str().unwrap().parse::<f64>().unwrap();
                        assert!(
                            (value.to_f64() - wanted).abs() < tolerance,
                            "{} auxiliary",
                            case["name"]
                        );
                    }
                } else {
                    assert!(sample.auxiliary.is_none());
                }
                assert_eq!(
                    sample.frequency,
                    expected.get("frequency").map(|v| v.as_u64().unwrap())
                );
                assert_eq!(
                    sample.row,
                    expected.get("row").map(|v| v.as_u64().unwrap() as usize)
                );
                assert_eq!(
                    sample.label.as_deref(),
                    expected.get("label").map(|v| v.as_str().unwrap())
                );
            }
            // Profile/restart transport retains typed values and explicit
            // undefined entries without ever serializing a nonfinite number.
            let restored: PlotData =
                serde_json::from_value(serde_json::to_value(&plot).unwrap()).unwrap();
            assert_eq!(restored.statistics_traces, plot.statistics_traces);
        }
        assert_eq!(context.snapshot().globals, snapshot.globals);
    }
    #[test]
    fn original_show_outliers_trace_uses_whisker_max_then_outlier() {
        let mut context = ppl_vm::SessionContext::default();
        context
            .set_variable(
                "D1",
                PortableValue::List(
                    [1, 2, 3, 4, 5, 6, 7, 100]
                        .into_iter()
                        .map(|n| PortableValue::Real(Real::from_i64(n)))
                        .collect(),
                ),
            )
            .unwrap();
        let mut book = super::super::OneBook::default();
        book.analyses[0].chart = Chart::BoxWhisker;
        book.analyses[0].option = 2;
        let plot = one_plots::plot(
            &book,
            &context.snapshot(),
            None,
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let samples = &plot.statistics_traces[0].samples;
        assert_eq!(samples.len(), 6);
        for (sample, (label, value)) in samples.iter().zip([
            ("Min", "1"),
            ("Q1", "2.5"),
            ("Med", "4.5"),
            ("Q3", "6.5"),
            ("Max", "7"),
            ("Outlier", "100"),
        ]) {
            assert_eq!(sample.label.as_deref(), Some(label));
            assert_eq!(
                ppl_vm::session::decode_value(sample.value.as_ref().unwrap())
                    .unwrap()
                    .to_value()
                    .expect_real("trace")
                    .unwrap(),
                Real::parse(value).unwrap()
            );
        }
    }
}
