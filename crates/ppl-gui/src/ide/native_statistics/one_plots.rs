//! One-variable charts from immutable calculator memory. Chart meanings follow
//! HP Prime User Guide, chapter 11, pp.238–243. Geometric projection is the only
//! conversion to f64; definitions and the statistical result cache stay typed.
//!
//! Chart frequencies, raw-row series, midpoint normal scores, fill palette and
//! control limits are checked against an isolated original G1 firmware session.
//! The reference is firmware execution, not physical-device verification.
use super::{
    OneBook,
    one::Chart,
    one_data::{Data, Frequency},
};
use crate::ide::native_application_access::geometry::ReadSnapshot;
use crate::ide::{
    native_app_models::Statistics1,
    native_inference::Distribution,
    native_model_plots::{
        self as plots, DotColumn, HistogramSpec, PlotAnnotation, PlotData, PlotFill, PlotStyle,
    },
};
use ppl_vm::SessionSnapshot;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Statistics plot interrupted".into())
    } else {
        Ok(())
    }
}
pub(super) fn projection(data: &Data) -> Result<Statistics1, String> {
    let samples = data
        .values
        .iter()
        .map(|value| {
            let result = value.to_f64();
            if !result.is_finite() || (!value.is_zero() && result == 0.) {
                Err("Statistics observation exceeds plot numerical range".into())
            } else {
                Ok(result)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    if data.count.is_zero() {
        return Err("Insufficient statistics data".into());
    }
    Ok(Statistics1 {
        samples,
        // The shared histogram renderer supplies contour edges. Actual
        // frequencies are filled from the typed bin counts below.
        frequencies: Vec::new(),
    })
}
pub(super) struct HistogramBins {
    minimum: ppl_math::Real,
    maximum: ppl_math::Real,
    width: ppl_math::Real,
}
impl HistogramBins {
    pub(super) fn new(spec: HistogramSpec) -> Result<Self, String> {
        if !spec.minimum.is_finite()
            || !spec.maximum.is_finite()
            || !spec.width.is_finite()
            || spec.minimum >= spec.maximum
            || spec.width <= 0.
        {
            return Err("Histogram range must increase and width must be positive".into());
        }
        let real = |value: f64| {
            ppl_math::Real::parse_versioned(
                &value.to_string(),
                ppl_math::NumericVersion::Current15751,
            )
            .ok_or_else(|| "Histogram bound exceeds calculator numerical range".to_owned())
        };
        Ok(Self {
            minimum: real(spec.minimum)?,
            maximum: real(spec.maximum)?,
            width: real(spec.width)?,
        })
    }
    pub(super) fn edges(&self, index: usize) -> Result<(ppl_math::Real, ppl_math::Real), String> {
        let left = self
            .width
            .mul(ppl_math::Real::from_u64(index as u64))
            .and_then(|offset| self.minimum.add(offset))
            .map_err(|error| error.to_string())?;
        let right = left.add(self.width).map_err(|error| error.to_string())?;
        Ok((
            left,
            if right.cmp_value(self.maximum) == std::cmp::Ordering::Greater {
                self.maximum
            } else {
                right
            },
        ))
    }
    pub(super) fn index(&self, value: ppl_math::Real) -> Result<Option<i128>, String> {
        if value.cmp_value(self.minimum) == std::cmp::Ordering::Less
            || value.cmp_value(self.maximum) == std::cmp::Ordering::Greater
        {
            return Ok(None);
        }
        if value.cmp_value(self.maximum) == std::cmp::Ordering::Equal {
            return self
                .maximum
                .sub(self.minimum)
                .and_then(|span| span.div(self.width))
                .and_then(|bins| bins.ceil().sub(ppl_math::Real::ONE))
                .map_err(|error| error.to_string())?
                .to_i128()
                .map(Some)
                .ok_or_else(|| "Histogram index exceeds numerical range".into());
        }
        value
            .sub(self.minimum)
            .and_then(|value| value.div(self.width))
            .map_err(|error| error.to_string())?
            .floor()
            .to_i128()
            .map(Some)
            .ok_or_else(|| "Histogram index exceeds numerical range".into())
    }
}
pub(super) fn histogram_counts(
    data: &Data,
    spec: HistogramSpec,
    count: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Frequency>, String> {
    let bins = HistogramBins::new(spec)?;
    let mut frequencies = vec![Frequency::ZERO; count];
    for (&value, &weight) in data.values.iter().zip(&data.weights) {
        check(cancel)?;
        if let Some(bin) = bins.index(value)? {
            let bin = usize::try_from(bin)
                .ok()
                .filter(|&bin| bin < count)
                .ok_or("Histogram index exceeds its bin range")?;
            frequencies[bin] = frequencies[bin].checked_add(weight)?;
        }
    }
    Ok(frequencies)
}
fn typed_histogram(
    data: &Data,
    model: &Statistics1,
    spec: HistogramSpec,
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    // Reuse the existing bounded, validated edge geometry. Membership is typed
    // calculator arithmetic: binary .3-.2 must not fall in the .2 bin of width.1.
    let mut plot = plots::histogram(model, &spec, cancel)?;
    let count = plot.curves.first().map_or(0, |points| points.len() / 5);
    let frequencies = histogram_counts(data, spec, count, cancel)?;
    for (polygon, frequency) in plot.curves[0].chunks_exact_mut(5).zip(frequencies) {
        let height = frequency.to_f64();
        if !height.is_finite() || (!frequency.is_zero() && height == 0.) {
            return Err("Histogram frequency exceeds plot numerical range".into());
        }
        polygon[1][1] = height;
        polygon[2][1] = height;
    }
    Ok(plot)
}
fn curve(
    result: &mut PlotData,
    label: impl Into<String>,
    style: PlotStyle,
    points: Vec<[f64; 2]>,
    color: u32,
) -> Result<(), String> {
    result.push(label, style, points)?;
    result.colors.push(color);
    Ok(())
}
fn pale(color: u32) -> u32 {
    let channel = |shift: u32| 169u32 + (((color >> shift) & 255u32) * 86u32 / 255u32);
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
fn filled(
    result: &mut PlotData,
    label: impl Into<String>,
    points: Vec<[f64; 2]>,
    color: u32,
) -> Result<(), String> {
    curve(result, label, PlotStyle::Line, points.clone(), 0)?;
    result.fills.push(PlotFill {
        points,
        color: pale(color),
    });
    Ok(())
}
fn annotate(result: &mut PlotData, point: [f64; 2], text: String, color: u32) {
    result.annotations.push(PlotAnnotation {
        point,
        text,
        color,
        font_size: 12.,
        right_aligned: false,
        analysis: None,
    });
}
fn bars(
    result: &mut PlotData,
    values: &[f64],
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    for (index, &value) in values.iter().enumerate() {
        check(cancel)?;
        let x = index as f64 + 0.5;
        filled(
            result,
            format!("Row {}", index + 1),
            vec![
                [x - 0.25, 0.],
                [x - 0.25, value],
                [x + 0.25, value],
                [x + 0.25, 0.],
                [x - 0.25, 0.],
            ],
            color,
        )?;
    }
    Ok(())
}
fn box_whisker(
    result: &mut PlotData,
    data: &Data,
    model: &Statistics1,
    option: u8,
    index: usize,
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let summary = data.summary(cancel)?;
    let number = |name: &str| summary.finite(name).to_f64();
    let (q1, median, q3) = (number("Q₁"), number("MedVal"), number("Q₃"));
    let (mut low, mut high) = (number("MinVal"), number("MaxVal"));
    if option == 2 {
        let spread = q3 - q1;
        let (left, right) = (q1 - 1.5 * spread, q3 + 1.5 * spread);
        let inside = model
            .samples
            .iter()
            .zip(&data.weights)
            .filter(|(x, w)| !w.is_zero() && **x >= left && **x <= right)
            .map(|(&x, _)| x)
            .collect::<Vec<_>>();
        low = inside.iter().copied().fold(f64::INFINITY, f64::min);
        high = inside.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let outliers = model
            .samples
            .iter()
            .zip(&data.weights)
            .filter(|(x, w)| !w.is_zero() && (**x < left || **x > right))
            .map(|(&x, _)| [x, index as f64 * 2. + 1.5])
            .collect::<Vec<_>>();
        if !outliers.is_empty() {
            curve(result, "Outliers", PlotStyle::Scatter, outliers, color)?;
        }
    }
    let y = index as f64 * 2. + 1.5;
    result.fills.push(PlotFill {
        points: vec![[q1, y - 0.5], [q1, y + 0.5], [q3, y + 0.5], [q3, y - 0.5]],
        color: pale(color),
    });
    for (name, points) in [
        (
            "Box",
            vec![
                [q1, y - 0.5],
                [q1, y + 0.5],
                [q3, y + 0.5],
                [q3, y - 0.5],
                [q1, y - 0.5],
            ],
        ),
        ("Median", vec![[median, y - 0.5], [median, y + 0.5]]),
        ("Left whisker", vec![[low, y], [q1, y]]),
        ("Right whisker", vec![[q3, y], [high, y]]),
        ("Minimum", vec![[low, y - 0.25], [low, y + 0.25]]),
        ("Maximum", vec![[high, y - 0.25], [high, y + 0.25]]),
    ] {
        check(cancel)?;
        curve(result, name, PlotStyle::Line, points, 0)?;
    }
    Ok(())
}
fn probability(
    result: &mut PlotData,
    values: &mut [f64],
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    values.sort_by(f64::total_cmp);
    let count = values.len() as f64;
    let mut points = Vec::with_capacity(values.len());
    for (index, &value) in values.iter().enumerate() {
        check(cancel)?;
        // Midpoint ranks never evaluate a distribution quantile at 0 or 1.
        let probability = (index as f64 + 0.5) / count;
        let z = -Distribution::Normal.inverse_upper_tail(probability)?;
        points.push([value, z]);
    }
    curve(
        result,
        "Normal probability",
        PlotStyle::Scatter,
        points,
        color,
    )
}
fn control(
    result: &mut PlotData,
    values: &[f64],
    option: u8,
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if values.len() < 2 {
        return Err("Control charts need at least two observations".into());
    }
    let ranges = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .collect::<Vec<_>>();
    let moving_mean = ranges.iter().map(|x| x / ranges.len() as f64).sum::<f64>();
    let (center, lower, upper, points) = if option == 2 {
        (
            moving_mean,
            0.,
            3.267 * moving_mean,
            ranges
                .iter()
                .enumerate()
                .map(|(i, &y)| [i as f64 + 2., y])
                .collect::<Vec<_>>(),
        )
    } else {
        let mean = values.iter().map(|x| x / values.len() as f64).sum::<f64>();
        let width = 2.66 * moving_mean;
        (
            mean,
            mean - width,
            mean + width,
            values
                .iter()
                .enumerate()
                .map(|(i, &y)| [i as f64 + 1., y])
                .collect::<Vec<_>>(),
        )
    };
    let (left, right) = (points[0][0], points.last().unwrap()[0]);
    curve(
        result,
        if option == 2 {
            "Moving range"
        } else {
            "Individuals"
        },
        PlotStyle::CrossLine,
        points,
        color,
    )?;
    for (name, y) in [
        ("Mean", center),
        ("Lower limit", lower),
        ("Upper limit", upper),
    ] {
        check(cancel)?;
        curve(
            result,
            name,
            PlotStyle::Line,
            vec![[left, y], [right, y]],
            0xff0000,
        )?;
    }
    Ok(())
}
fn shares(values: &[f64]) -> Result<Vec<f64>, String> {
    if values.iter().any(|&value| value < 0.) {
        return Err("Pie and Pareto charts require nonnegative observations".into());
    }
    let scale = values.iter().copied().fold(0., f64::max);
    if scale == 0. {
        return Err("Pie and Pareto charts need a positive total".into());
    }
    let total = values.iter().map(|value| value / scale).sum::<f64>();
    Ok(values.iter().map(|value| (value / scale) / total).collect())
}
fn pareto(
    result: &mut PlotData,
    values: &mut [f64],
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    values.sort_by(|a, b| b.total_cmp(a));
    shares(values)?; // verifies the nonnegative, positive-total domain
    let mut cumulative = 0.;
    let mut points = Vec::new();
    for (index, &value) in values.iter().enumerate() {
        check(cancel)?;
        let x = index as f64;
        filled(
            result,
            format!("Row {}", index + 1),
            vec![[x, 0.], [x, value], [x + 1., value], [x + 1., 0.], [x, 0.]],
            color,
        )?;
        cumulative += value;
        points.push([x + 0.5, cumulative]);
    }
    curve(
        result,
        "Cumulative total",
        PlotStyle::CrossLine,
        points,
        color,
    )
}
fn pie(
    result: &mut PlotData,
    values: &[f64],
    _index: usize,
    _color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let percentages = shares(values)?;
    let mut start = std::f64::consts::FRAC_PI_2;
    let scale = values.iter().copied().fold(0., f64::max);
    let normalized_total = values.iter().map(|value| value / scale).sum::<f64>();
    // Original G1 sector areas equal the entered raw values, so the whole
    // circle's area is their sum. Factored roots avoid overflowing that sum.
    let radius = scale.sqrt() * (normalized_total / std::f64::consts::PI).sqrt();
    if !radius.is_finite() {
        return Err("Pie radius exceeds plot numerical range".into());
    }
    let center = [radius + 1., radius + 1.];
    const PALETTE: [u32; 5] = [0x0000ff, 0xff0000, 0x008000, 0xffa500, 0x800080];
    for (row, &percentage) in percentages.iter().enumerate().filter(|(_, p)| **p > 0.) {
        check(cancel)?;
        let end = start - std::f64::consts::TAU * percentage;
        let steps = (percentage * 180.).ceil().max(2.) as usize;
        let mut polygon = vec![center];
        polygon.extend((0..=steps).map(|i| {
            let angle = start + (end - start) * i as f64 / steps as f64;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        }));
        polygon.push(center);
        filled(
            result,
            format!("Row {}", row + 1),
            polygon,
            PALETTE[row % PALETTE.len()],
        )?;
        start = end;
    }
    Ok(())
}
pub(super) fn dot_groups(
    data: &Data,
    cancel: &AtomicBool,
) -> Result<Vec<(ppl_math::Real, Frequency)>, String> {
    let mut observations = data
        .values
        .iter()
        .copied()
        .zip(data.weights.iter().copied())
        .collect::<Vec<_>>();
    observations.sort_by(|a, b| a.0.cmp_value(b.0));
    let mut grouped: Vec<(ppl_math::Real, Frequency)> = Vec::new();
    for (value, count) in observations {
        check(cancel)?;
        if let Some((last, total)) = grouped.last_mut() {
            if last.cmp_value(value) == std::cmp::Ordering::Equal {
                *total = total.checked_add(count)?;
                continue;
            }
        }
        grouped.push((value, count));
    }
    Ok(grouped)
}
fn dot(
    result: &mut PlotData,
    data: &Data,
    index: usize,
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let grouped = dot_groups(data, cancel)?;
    let mut points = Vec::with_capacity(grouped.len());
    for (value, count) in grouped {
        check(cancel)?;
        let x = value.to_f64();
        let (whole, fraction) = count.split()?;
        result.dots.push(DotColumn {
            x,
            count: whole,
            fraction,
            color: pale(color),
            analysis: index + 1,
        });
        // Trace visits each distinct X once and uses the complete frequency.
        points.push([x, count.to_f64()]);
    }
    curve(result, "Dot", PlotStyle::TraceOnly, points, color)
}
pub(super) type StemRow = ((bool, i64, u8), Vec<u8>);
pub(super) struct StemLayout {
    pub unit: f64,
    pub rows: Vec<StemRow>,
}
pub(super) fn stem_layout(
    values: &[f64],
    option: u8,
    cancel: &AtomicBool,
) -> Result<StemLayout, String> {
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = high - low;
    if !span.is_finite() {
        return Err("Stem-and-leaf span exceeds plot numerical range".into());
    }
    let exponent = if span == 0. {
        -1
    } else {
        span.log10().floor() as i32 - 1
    };
    let unit = 10_f64.powi(exponent);
    if unit == 0. || !unit.is_finite() {
        return Err("Stem-and-leaf scale exceeds plot numerical range".into());
    }
    let mut stems: BTreeMap<(bool, i64, u8), Vec<u8>> = BTreeMap::new();
    for &value in values {
        check(cancel)?;
        let digits = (value.abs() / unit + 1e-10).floor() as i64;
        let stem = digits / 10;
        let leaf = (digits % 10) as u8;
        let split = if option == 2 { u8::from(leaf >= 5) } else { 0 };
        stems
            .entry((value < 0., stem, split))
            .or_default()
            .push(leaf);
    }
    let mut rows = stems.into_iter().collect::<Vec<_>>();
    rows.sort_by(|((an, as_, ap), _), ((bn, bs, bp), _)| match (an, bn) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (true, true) => (bs, bp).cmp(&(as_, ap)),
        (false, false) => (as_, ap).cmp(&(bs, bp)),
    });
    // Include empty stems between observed values, as on the original G1.
    if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
        if !first.0.0 && !last.0.0 {
            for stem in first.0.1..=last.0.1 {
                for split in 0..=u8::from(option == 2) {
                    if !rows
                        .iter()
                        .any(|((negative, s, p), _)| !*negative && *s == stem && *p == split)
                    {
                        rows.push(((false, stem, split), Vec::new()));
                    }
                }
            }
            rows.sort_by_key(|(key, _)| *key);
        }
    }
    for (_, leaves) in &mut rows {
        leaves.sort_unstable();
        leaves.dedup();
    }
    Ok(StemLayout { unit, rows })
}
fn stem_leaf(
    result: &mut PlotData,
    values: &[f64],
    option: u8,
    index: usize,
    color: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let StemLayout { unit, rows } = stem_layout(values, option, cancel)?;
    let _ = index;
    let left = 0.;
    let count = rows.len();
    let mut max_leaves = 0usize;
    for (row, ((negative, stem, split), mut leaves)) in rows.into_iter().enumerate() {
        check(cancel)?;
        leaves.sort_unstable();
        leaves.dedup();
        max_leaves = max_leaves.max(leaves.len());
        let sign = if negative { "-" } else { "" };
        let mark = if option == 2 {
            if split == 0 { "L" } else { "H" }
        } else {
            ""
        };
        let y = (count - row) as f64 - 0.5;
        annotate(
            result,
            [left - 0.4, y],
            format!("{sign}{stem}{mark}"),
            color,
        );
        let annotation = result.annotations.last_mut().unwrap();
        annotation.font_size = 22.;
        annotation.right_aligned = true;
        annotate(
            result,
            [left + 0.4, y],
            leaves
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(" "),
            color,
        );
        result.annotations.last_mut().unwrap().font_size = 22.;
    }
    let right = left + 1.2 + max_leaves as f64 * 0.18;
    let points = vec![[left - 1., count as f64], [right, count as f64]];
    curve(result, "Stem table top", PlotStyle::Line, points, color)?;
    curve(
        result,
        "Stem divider",
        PlotStyle::Line,
        vec![[left, count as f64], [left, 0.]],
        color,
    )?;
    curve(
        result,
        "Stem table bottom",
        PlotStyle::Line,
        vec![[left - 1., 0.], [left + 1., 0.]],
        color,
    )?;
    annotate(
        result,
        [left + 0.2, -0.5],
        format!("1 | 2 = {}", 12. * unit),
        color,
    );
    result.annotations.last_mut().unwrap().font_size = 22.;
    // Empty text anchors reserve margins for fixed-size labels and the key.
    annotate(result, [left - 1.5, -1.], String::new(), color);
    annotate(
        result,
        [right + 0.5, count as f64 + 0.5],
        String::new(),
        color,
    );
    Ok(())
}

/// Evaluate every checked definition using the same immutable session snapshot
/// as the GUI worker. A bad definition or interrupted calculation returns no
/// partial plot. Legacy profile column caches never override actual D values.
pub fn plot(
    book: &OneBook,
    snapshot: &SessionSnapshot,
    spec: Option<HistogramSpec>,
    cancel: Arc<AtomicBool>,
) -> Result<PlotData, String> {
    plot_with_geometry(book, snapshot, spec, cancel, None)
}

pub(crate) fn plot_with_geometry(
    book: &OneBook,
    snapshot: &SessionSnapshot,
    spec: Option<HistogramSpec>,
    cancel: Arc<AtomicBool>,
    geometry: ReadSnapshot,
) -> Result<PlotData, String> {
    check(&cancel)?;
    book.validate()?;
    let mut result = PlotData::new("Value", "Frequency");
    for (index, analysis) in book.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
        check(&cancel)?;
        let data =
            Data::evaluate_with_geometry(analysis, snapshot, cancel.clone(), geometry.clone())?;
        let model = projection(&data)?;
        let color = analysis.plot_color(index);
        let mut chart = PlotData::new("Value", "Frequency");
        let mut histogram_trace = None;
        match analysis.chart {
            Chart::Histogram => {
                let spec = if let Some(spec) = spec {
                    spec
                } else {
                    let mut positive =
                        model.samples.iter().copied().zip(&data.weights).filter_map(
                            |(sample, frequency)| (!frequency.is_zero()).then_some(sample),
                        );
                    let first = positive.next().ok_or("Insufficient statistics data")?;
                    let (min, max) = positive.fold((first, first), |(min, max), sample| {
                        (min.min(sample), max.max(sample))
                    });
                    HistogramSpec::for_observations(
                        model.samples.len(),
                        min,
                        max,
                        data.count.to_f64(),
                    )?
                };
                chart = typed_histogram(&data, &model, spec, &cancel)?;
                histogram_trace = Some(spec);
                chart.colors = vec![0; chart.curves.len()];
                // Histogram contours are closed rectangles separated by their
                // baseline. Filling each bar avoids fictitious diagonal edges.
                for points in chart.curves.iter().flat_map(|curve| curve.chunks(5)) {
                    chart.fills.push(PlotFill {
                        points: points.to_vec(),
                        color: pale(color),
                    });
                }
            }
            Chart::Dot => dot(&mut chart, &data, index, color, &cancel)?,
            Chart::BoxWhisker => box_whisker(
                &mut chart,
                &data,
                &model,
                analysis.option,
                index,
                color,
                &cancel,
            )?,
            _ => {
                let mut values = model.samples.clone();
                match analysis.chart {
                    Chart::NormalProbability => {
                        probability(&mut chart, &mut values, color, &cancel)?
                    }
                    Chart::Line => curve(
                        &mut chart,
                        "Line",
                        PlotStyle::PointLine,
                        {
                            if analysis.frequency_expression().is_empty() {
                                values
                                    .iter()
                                    .enumerate()
                                    .map(|(index, &y)| [index as f64 + 1., y])
                                    .collect()
                            } else {
                                values
                                    .iter()
                                    .zip(&data.weights)
                                    .map(|(&x, &y)| [x, y.to_f64()])
                                    .collect()
                            }
                        },
                        color,
                    )?,
                    Chart::Bar => bars(&mut chart, &values, color, &cancel)?,
                    Chart::Pareto => pareto(&mut chart, &mut values, color, &cancel)?,
                    Chart::Control => {
                        control(&mut chart, &values, analysis.option, color, &cancel)?
                    }
                    Chart::Dot => unreachable!(),
                    Chart::StemLeaf => {
                        stem_leaf(&mut chart, &values, analysis.option, index, color, &cancel)?
                    }
                    Chart::Pie => pie(&mut chart, &values, index, color, &cancel)?,
                    Chart::Histogram | Chart::BoxWhisker => unreachable!(),
                }
            }
        }
        chart.statistics_traces.push(super::one_trace::build(
            analysis,
            index,
            &data,
            histogram_trace,
            &chart,
            &cancel,
        )?);
        for annotation in &mut chart.annotations {
            annotation.analysis = Some(index + 1);
        }
        for label in &mut chart.labels {
            *label = format!("H{} {label}", index + 1);
        }
        result.curves.extend(chart.curves);
        result.styles.extend(chart.styles);
        result.labels.extend(chart.labels);
        result.colors.extend(chart.colors);
        result.annotations.extend(chart.annotations);
        result.fills.extend(chart.fills);
        result.dots.extend(chart.dots);
        result.statistics_traces.extend(chart.statistics_traces);
    }
    if result.curves.is_empty() {
        return Err("Select at least one H analysis".into());
    }
    check(&cancel)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_math::Real;
    use ppl_vm::{PortableValue, SessionContext};
    fn setup(values: &[i64], chart: Chart) -> (OneBook, SessionSnapshot) {
        let mut session = SessionContext::default();
        session
            .set_variable(
                "D1",
                PortableValue::List(
                    values
                        .iter()
                        .map(|&n| PortableValue::Real(Real::from_i64(n)))
                        .collect(),
                ),
            )
            .unwrap();
        let mut book = OneBook::default();
        book.analyses[0].chart = chart;
        book.analyses[0].option = chart.default_option();
        (book, session.snapshot())
    }
    fn run(book: &OneBook, snapshot: &SessionSnapshot) -> PlotData {
        plot(book, snapshot, None, Arc::new(AtomicBool::new(false))).unwrap()
    }
    #[test]
    fn all_ten_charts_use_live_typed_definitions_and_keep_selected_colors() {
        for chart in Chart::ALL {
            let (mut book, snapshot) = setup(&[1, 2, 4, 7, 9], chart);
            book.columns.0[0] = vec![900.]; // stale cache must not supply the chart
            book.analyses[0].data_source = Some("D1+1".into());
            book.analyses[0].color = Some(0x123456);
            book.analyses[0].frequency_source = Some("2".into());
            let plot = run(&book, &snapshot);
            assert!(!plot.curves.is_empty(), "{chart:?}");
            assert_eq!(plot.colors.len(), plot.curves.len());
            if chart != Chart::Pie {
                assert!(
                    plot.colors.contains(&0x123456)
                        || plot.fills.iter().any(|fill| fill.color == pale(0x123456))
                        || plot
                            .annotations
                            .iter()
                            .any(|annotation| annotation.color == 0x123456)
                );
            } else {
                assert_eq!(plot.fills[0].color, 0xa9a9ff);
            }
            assert_eq!(plot.labels.len(), plot.curves.len());
            assert!(
                plot.curves
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|x| x.is_finite())
            );
            assert!(
                matches!(&snapshot.globals["D1"], PortableValue::List(values) if values.len() == 5)
            );
        }
    }
    #[test]
    fn weighted_dot_positions_and_line_row_numbers_are_calculated_from_actual_data() {
        let (mut book, snapshot) = setup(&[3, 1, 3], Chart::Dot);
        book.analyses[0].frequency_source = Some("2".into());
        let dot = run(&book, &snapshot);
        assert_eq!(dot.curves[0], vec![[1., 2.], [3., 4.]]);
        assert_eq!(dot.styles[0], PlotStyle::TraceOnly);
        assert_eq!(
            dot.dots
                .iter()
                .map(|column| (column.x, column.count))
                .collect::<Vec<_>>(),
            vec![(1., 2), (3., 4)]
        );
        book.analyses[0].chart = Chart::Line;
        let line = run(&book, &snapshot);
        assert_eq!(line.curves[0], vec![[3., 2.], [1., 2.], [3., 2.]]);
    }
    #[test]
    fn original_factory_blank_frequency_line_uses_row_numbers_instead_of_frequency_coordinates() {
        let (mut book, snapshot) = setup(&[1, 2, 4], Chart::Line);
        assert_eq!(
            run(&book, &snapshot).curves[0],
            vec![[1., 1.], [2., 2.], [3., 4.]]
        );
        book.analyses[0].frequency_source = Some("2".into());
        assert_eq!(
            run(&book, &snapshot).curves[0],
            vec![[1., 2.], [2., 2.], [4., 2.]]
        );
    }
    #[test]
    fn box_outlier_option_changes_whiskers_and_preserves_median() {
        let (mut book, snapshot) = setup(&[1, 2, 3, 4, 5, 6, 7, 100], Chart::BoxWhisker);
        let inclusive = run(&book, &snapshot);
        let maximum = inclusive
            .labels
            .iter()
            .position(|label| label.ends_with("Maximum"))
            .unwrap();
        assert_eq!(inclusive.curves[maximum][0][0], 100.);
        book.analyses[0].option = 2;
        let outliers = run(&book, &snapshot);
        let maximum = outliers
            .labels
            .iter()
            .position(|label| label.ends_with("Maximum"))
            .unwrap();
        assert_eq!(outliers.curves[maximum][0][0], 7.);
        assert_eq!(outliers.curves[0], vec![[100., 1.5]]);
        let median = outliers
            .labels
            .iter()
            .position(|label| label.ends_with("Median"))
            .unwrap();
        assert_eq!(outliers.curves[median][0][0], 4.5);
    }
    #[test]
    fn normal_midpoint_quantiles_and_control_moving_ranges_have_independent_values() {
        let (mut book, snapshot) = setup(&[1, 3, 6], Chart::NormalProbability);
        let normal = run(&book, &snapshot);
        assert!((normal.curves[0][0][1] + 0.967421566101701).abs() < 1e-9);
        assert_eq!(normal.curves[0][1], [3., 0.]);
        assert!((normal.curves[0][2][1] - 0.967421566101701).abs() < 1e-9);
        book.analyses[0].chart = Chart::Control;
        book.analyses[0].option = 2;
        let control = run(&book, &snapshot);
        assert_eq!(control.curves[0], vec![[2., 2.], [3., 3.]]);
        assert_eq!(control.curves[1], vec![[2., 2.5], [3., 2.5]]);
        assert_eq!(control.curves[2], vec![[2., 0.], [3., 0.]]);
        assert!((control.curves[3][0][1] - 8.1675).abs() < 1e-10);
    }
    #[test]
    fn pareto_percentage_order_pie_area_and_split_stems_share_actual_observations() {
        let (mut book, snapshot) = setup(&[10, 30, 60], Chart::Pareto);
        let pareto = run(&book, &snapshot);
        assert!((pareto.curves[0][1][1] - 60.).abs() < 1e-10);
        assert!((pareto.curves[1][1][1] - 30.).abs() < 1e-10);
        assert!((pareto.curves[2][1][1] - 10.).abs() < 1e-10);
        assert_eq!(pareto.curves[3], vec![[0.5, 60.], [1.5, 90.], [2.5, 100.]]);
        book.analyses[0].chart = Chart::Pie;
        let pie = run(&book, &snapshot);
        let area = |points: &[[f64; 2]]| {
            points
                .windows(2)
                .map(|pair| pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1])
                .sum::<f64>()
                .abs()
                / 2.
        };
        let areas = pie
            .fills
            .iter()
            .map(|fill| area(&fill.points))
            .collect::<Vec<_>>();
        let total = areas.iter().sum::<f64>();
        assert!((areas[0] / total - 0.1).abs() < 0.001);
        assert!((areas[1] / total - 0.3).abs() < 0.001);
        assert!((areas[2] / total - 0.6).abs() < 0.001);
        let (mut book, snapshot) = setup(&[12, 15, 17, 21], Chart::StemLeaf);
        book.analyses[0].option = 2;
        let stems = run(&book, &snapshot);
        let visible = stems
            .annotations
            .iter()
            .filter(|annotation| !annotation.text.is_empty())
            .map(|annotation| annotation.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(&visible[..3], &["12L", "0", "12H"]);
        assert!(visible.contains(&"21L"));
        assert_eq!(
            stems
                .annotations
                .iter()
                .filter(|annotation| annotation.right_aligned)
                .count(),
            20
        );
        assert!(stems.annotations.iter().all(|a| a.analysis == Some(1)));
        assert!(
            stems
                .annotations
                .iter()
                .filter(|a| !a.text.is_empty())
                .all(|a| a.font_size == 22.)
        );
    }
    #[test]
    fn invalid_analysis_and_cancellation_are_atomic_while_large_dot_counts_remain_compact() {
        let (mut book, snapshot) = setup(&[1, 2, 3], Chart::Pie);
        book.analyses[1] = book.analyses[0].clone();
        book.analyses[1].data_source = Some("D1-10".into());
        assert!(plot(&book, &snapshot, None, Arc::new(AtomicBool::new(false))).is_err());
        assert!(plot(&book, &snapshot, None, Arc::new(AtomicBool::new(true))).is_err());
        book.analyses[1].enabled = false;
        book.analyses[0].frequency_source = Some("1000000000".into());
        book.analyses[0].chart = Chart::Dot;
        let large = plot(&book, &snapshot, None, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(large.dots.len(), 3);
        assert!(
            large
                .dots
                .iter()
                .all(|column| column.count == 1_000_000_000)
        );
        assert_eq!(large.curves[0].len(), 3);
        assert!(large.fills.is_empty());
        book.analyses[0].chart = Chart::Histogram;
        assert!(plot(&book, &snapshot, None, Arc::new(AtomicBool::new(false))).is_ok());
    }
    #[test]
    fn original_large_dot_trace_groups_are_exact_and_do_not_expand_repeated_observations() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-autoscale-controls.json"
        ))
        .unwrap();
        let original = &fixture["dot_trace_observations"];
        let (mut book, snapshot) = setup(&[3, 1, 3], Chart::Dot);
        book.analyses[0].frequency_source = Some(original["frequency"].to_string());
        let plot = run(&book, &snapshot);
        let expected: Vec<[f64; 2]> = serde_json::from_value(serde_json::Value::Array(
            original["right_steps"].as_array().unwrap()[..2].to_vec(),
        ))
        .unwrap();
        assert_eq!(plot.curves[0], expected);
        assert_eq!(
            plot.dots
                .iter()
                .map(|column| (column.x, column.count))
                .collect::<Vec<_>>(),
            vec![(1., 10001), (3., 20002)]
        );
        assert_eq!(plot.styles, vec![PlotStyle::TraceOnly]);
        assert!(plot.fills.is_empty());
        book.analyses[0].frequency_source = Some("9007199254740993".into());
        let mut current = snapshot.clone();
        current.settings.numeric_version = ppl_math::NumericVersion::Current15751;
        let plot = run(&book, &current);
        assert_eq!(plot.dots[0].count, 9_007_199_254_740_993);
        assert_eq!(plot.dots[1].count, 18_014_398_509_481_986);
        assert_eq!(plot.curves[0].len(), 2);
    }
    #[test]
    fn original_pie_area_controls_for_independent_data_sets_determine_circle_radius_and_position() {
        for values in [
            &[1, 2, 4][..],
            &[2, 5, 8, 11][..],
            &[10, 20, 50, 100, 200][..],
            &[3, 3, 3][..],
        ] {
            let (book, snapshot) = setup(values, Chart::Pie);
            let data = run(&book, &snapshot);
            let sum = values.iter().sum::<i64>() as f64;
            let radius = (sum / std::f64::consts::PI).sqrt();
            let center = [radius + 1., radius + 1.];
            assert_eq!(data.fills.len(), values.len());
            for polygon in &data.fills {
                assert!((polygon.points[0][0] - center[0]).abs() < 1e-10);
                assert!((polygon.points[0][1] - center[1]).abs() < 1e-10);
                for point in &polygon.points[1..polygon.points.len() - 1] {
                    assert!(
                        ((point[0] - center[0]).hypot(point[1] - center[1]) - radius).abs() < 1e-10
                    );
                }
            }
            let area = data
                .fills
                .iter()
                .flat_map(|fill| fill.points.windows(2))
                .map(|pair| pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1])
                .sum::<f64>()
                .abs()
                / 2.;
            assert!((area - sum).abs() / sum < 0.0003);
        }
    }
    #[test]
    fn original_chart_frame_coordinates_distinguish_weighted_and_raw_row_graphs() {
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-workflow-controls.json"
        ))
        .unwrap();
        let original = &controls["chart_observations"];
        for (chart, field) in [
            (Chart::NormalProbability, "normal_probability"),
            (Chart::Line, "line"),
            (Chart::Dot, "dot_trace"),
            (Chart::Control, "control_individual_points"),
        ] {
            let (mut book, snapshot) = setup(&[1, 2, 4], chart);
            book.analyses[0].frequency_source = Some("2".into());
            let data = run(&book, &snapshot);
            let expected: Vec<[f64; 2]> = serde_json::from_value(original[field].clone()).unwrap();
            assert_eq!(data.curves[0].len(), expected.len(), "{chart:?}");
            for (point, want) in data.curves[0].iter().zip(expected) {
                assert!(
                    (point[0] - want[0]).abs() < 1e-9 && (point[1] - want[1]).abs() < 1e-9,
                    "{chart:?}: {point:?} != {want:?}"
                );
            }
        }
        let (mut book, snapshot) = setup(&[1, 2, 4], Chart::Bar);
        book.analyses[0].frequency_source = Some("2".into());
        let bar = run(&book, &snapshot);
        for (polygon, want) in bar
            .curves
            .iter()
            .zip(original["bar_centers_and_heights"].as_array().unwrap())
        {
            assert_eq!(
                (polygon[0][0] + polygon[2][0]) / 2.,
                want[0].as_f64().unwrap()
            );
            assert_eq!(polygon[1][1], want[1].as_f64().unwrap());
        }
        book.analyses[0].chart = Chart::Pareto;
        let pareto = run(&book, &snapshot);
        let expected: Vec<[f64; 2]> =
            serde_json::from_value(original["pareto_cumulative"].clone()).unwrap();
        assert_eq!(pareto.curves[3], expected);
        book.analyses[0].chart = Chart::Pie;
        let pie = run(&book, &snapshot);
        let colors: Vec<u32> = serde_json::from_value(original["pale_fills"].clone()).unwrap();
        assert_eq!(
            pie.fills.iter().map(|fill| fill.color).collect::<Vec<_>>(),
            colors
        );
        assert_eq!(pie.fills.len(), 3); // scalar frequency2 must not double sectors
        assert!(pie.annotations.is_empty()); // percentages belong in the trace footer
    }
    #[test]
    fn metadata_roundtrip_and_legacy_plot_documents_keep_existing_profiles_loadable() {
        let (book, snapshot) = setup(&[1, 2, 3], Chart::Pie);
        let data = run(&book, &snapshot);
        let restored: PlotData =
            serde_json::from_value(serde_json::to_value(&data).unwrap()).unwrap();
        assert_eq!(restored, data);
        let legacy = r#"{"curves":[[[1.0,2.0]]],"labels":["old"],"styles":["Line"],"x_label":"X","y_label":"Y"}"#;
        let restored: PlotData = serde_json::from_str(legacy).unwrap();
        assert!(
            restored.colors.is_empty()
                && restored.annotations.is_empty()
                && restored.fills.is_empty()
        );
    }
    #[test]
    fn original_histogram_capacity_controls_keep_every_bin_beyond_512() {
        for count in [512, 513, 1000] {
            let mut context = ppl_vm::SessionContext::default();
            context
                .set_variable(
                    "D1",
                    PortableValue::List(
                        (1..=count)
                            .map(|n| PortableValue::Real(ppl_math::Real::from_i64(n)))
                            .collect(),
                    ),
                )
                .unwrap();
            let book = super::super::OneBook::default();
            let data = plot(
                &book,
                &context.snapshot(),
                Some(HistogramSpec {
                    minimum: 1.,
                    maximum: count as f64 + 1.,
                    width: 1.,
                }),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert_eq!(data.statistics_traces[0].samples.len(), count as usize);
            assert!(
                data.statistics_traces[0]
                    .samples
                    .iter()
                    .all(|sample| sample.frequency == Some(1))
            );
            assert_eq!(data.curves[0].len(), count as usize * 5);
        }
    }
    #[test]
    fn pathological_histogram_geometry_fails_before_allocating_or_iterating_bins() {
        let mut context = ppl_vm::SessionContext::default();
        context
            .set_variable(
                "D1",
                PortableValue::List(vec![
                    PortableValue::Real(ppl_math::Real::ONE),
                    PortableValue::Real(ppl_math::Real::from_i64(2)),
                ]),
            )
            .unwrap();
        let book = super::super::OneBook::default();
        let error = plot(
            &book,
            &context.snapshot(),
            Some(HistogramSpec {
                minimum: 0.,
                maximum: 100_000_000.,
                width: 1.,
            }),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(error.contains("Native rendering budget"), "{error}");
    }
}
