//! Geometry for the selected one-variable chart. Frequencies are retained and
//! zero-frequency observations never influence a graph or its scale.
use super::super::native_app_models::Statistics1;
use super::super::native_inference::Distribution;
use super::super::native_model_plots::{PlotData, PlotStyle};
use super::one::Chart;
use std::sync::atomic::AtomicBool;

pub(super) fn render(
    model: &Statistics1,
    chart: Chart,
    option: u8,
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    let summary = model.calculate()?;
    super::check(cancel)?;
    let mut values = model
        .samples
        .iter()
        .enumerate()
        .filter_map(|(i, &x)| {
            let weight = model.frequencies.get(i).copied().unwrap_or(1);
            (weight > 0).then_some((x, weight))
        })
        .collect::<Vec<_>>();
    if values.len() > 10_000 {
        return Err("Plot accepts at most 10000 observations".into());
    }
    if matches!(chart, Chart::Line | Chart::Control) {
        if summary.count > 10_000 {
            return Err("This chart accepts at most 10000 weighted observations".into());
        }
        let mut expanded = Vec::with_capacity(summary.count as usize);
        for &(x, w) in &values {
            super::check(cancel)?;
            expanded.extend(std::iter::repeat_n((x, 1), w as usize));
        }
        values = expanded;
    }
    let mut plot = PlotData::new("Observation", "Value");
    let mut push = |name: &str, style, points| plot.push(name, style, points);
    match chart {
        Chart::Line => {
            push(
                "Line",
                PlotStyle::Line,
                values
                    .iter()
                    .enumerate()
                    .map(|(i, &(x, _))| [(i + 1) as f64, x])
                    .collect(),
            )?;
        }
        Chart::Control => {
            let samples = if option == 2 {
                values
                    .windows(2)
                    .map(|w| (w[1].0 - w[0].0).abs())
                    .collect::<Vec<_>>()
            } else {
                values.iter().map(|p| p.0).collect()
            };
            if samples.is_empty() {
                return Err("Moving range needs at least two observations".into());
            }
            let mean = samples.iter().sum::<f64>() / samples.len() as f64;
            let moving_range = values
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).abs())
                .sum::<f64>()
                / (values.len().saturating_sub(1).max(1)) as f64;
            let (low, high) = if option == 2 {
                (0., 3.267 * mean)
            } else {
                (mean - 2.66 * moving_range, mean + 2.66 * moving_range)
            };
            push(
                "Control observations",
                PlotStyle::Line,
                samples
                    .iter()
                    .enumerate()
                    .map(|(i, &x)| [(i + 1) as f64, x])
                    .collect(),
            )?;
            for (label, value) in [
                ("Center", mean),
                ("Lower control limit", low),
                ("Upper control limit", high),
            ] {
                push(
                    label,
                    PlotStyle::Line,
                    vec![[1., value], [samples.len().max(2) as f64, value]],
                )?;
            }
        }
        Chart::BoxWhisker => {
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            let rank = |rank: u64| {
                let mut total = 0;
                for &(x, w) in &values {
                    total += w;
                    if rank < total {
                        return x;
                    }
                }
                values.last().unwrap().0
            };
            let median = |start: u64, n: u64| {
                if n % 2 == 0 {
                    rank(start + n / 2 - 1) / 2. + rank(start + n / 2) / 2.
                } else {
                    rank(start + n / 2)
                }
            };
            let n = summary.count;
            let (q1, q3) = if n == 1 {
                (rank(0), rank(0))
            } else {
                (median(0, n / 2), median(n.div_ceil(2), n / 2))
            };
            let fence = 1.5 * (q3 - q1);
            let included = values
                .iter()
                .filter(|(x, _)| option != 2 || (*x >= q1 - fence && *x <= q3 + fence))
                .map(|p| p.0)
                .collect::<Vec<_>>();
            let lo = included[0];
            let hi = *included.last().unwrap();
            push(
                "Box",
                PlotStyle::Line,
                vec![[q1, 0.], [q1, 1.], [q3, 1.], [q3, 0.], [q1, 0.]],
            )?;
            push(
                "Median",
                PlotStyle::Line,
                vec![[summary.median, 0.], [summary.median, 1.]],
            )?;
            push("Lower whisker", PlotStyle::Line, vec![[lo, 0.5], [q1, 0.5]])?;
            push("Upper whisker", PlotStyle::Line, vec![[q3, 0.5], [hi, 0.5]])?;
            if option == 2 {
                push(
                    "Outliers",
                    PlotStyle::Scatter,
                    values
                        .iter()
                        .filter(|(x, _)| *x < lo || *x > hi)
                        .map(|p| [p.0, 0.5])
                        .collect(),
                )?;
            }
        }
        Chart::NormalProbability | Chart::Dot => {
            if summary.count > 10_000 {
                return Err("This chart accepts at most 10000 weighted observations".into());
            }
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut points = Vec::new();
            let mut rank = 0;
            let mut previous = None;
            let mut height = 0;
            for (x, weight) in values {
                super::check(cancel)?;
                if previous != Some(x) {
                    height = 0;
                }
                for _ in 0..weight {
                    rank += 1;
                    height += 1;
                    points.push(if chart == Chart::Dot {
                        [x, height as f64]
                    } else {
                        let p = (rank as f64 - 0.5) / summary.count as f64;
                        [x, Distribution::Normal.inverse_upper_tail(1. - p)?]
                    });
                }
                previous = Some(x);
            }
            push(chart.label(), PlotStyle::Scatter, points)?;
        }
        Chart::Bar | Chart::Pareto | Chart::Pie => {
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut categories: Vec<(f64, u64)> = Vec::new();
            for (x, w) in values {
                if let Some(last) = categories.last_mut().filter(|last| last.0 == x) {
                    last.1 += w;
                } else {
                    categories.push((x, w));
                }
            }
            if chart == Chart::Pareto {
                categories.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.total_cmp(&b.0)));
            }
            let mut cumulative = 0.;
            for (i, (x, w)) in categories.iter().enumerate() {
                super::check(cancel)?;
                if chart == Chart::Pie {
                    let start = cumulative;
                    cumulative += *w as f64 / summary.count as f64 * std::f64::consts::TAU;
                    let steps = ((cumulative - start) * 32.).ceil().max(1.) as usize;
                    let mut sector = vec![[0., 0.]];
                    for j in 0..=steps {
                        let a = start + (cumulative - start) * j as f64 / steps as f64;
                        sector.push([a.cos(), a.sin()]);
                    }
                    sector.push([0., 0.]);
                    push(&format!("{x}: {w}"), PlotStyle::Line, sector)?;
                } else {
                    let a = i as f64 + 0.6;
                    let b = a + 0.8;
                    push(
                        &format!("{x}: {w}"),
                        PlotStyle::Line,
                        vec![[a, 0.], [a, *w as f64], [b, *w as f64], [b, 0.], [a, 0.]],
                    )?;
                }
            }
            if chart == Chart::Pareto {
                let mut total = 0u64;
                push(
                    "Cumulative frequency",
                    PlotStyle::Line,
                    categories
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            total += p.1;
                            [(i + 1) as f64, total as f64]
                        })
                        .collect(),
                )?;
            }
        }
        Chart::StemLeaf => {
            if summary.count > 10_000 {
                return Err("Stem and leaf accepts at most 10000 weighted observations".into());
            }
            let magnitude = summary.maximum.abs().max(summary.minimum.abs());
            let unit = if magnitude == 0. {
                1.
            } else {
                10_f64.powf((magnitude.log10().floor() - 1.).min(0.))
            };
            // Values are sorted numerically. Preserve that order when grouping:
            // -0 must precede +0, and a negative split stem visits leaves9..5
            // before4..0 rather than sorting its unsigned split code.
            let mut rows: Vec<((i64, bool, u8), String)> = Vec::new();
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (x, w) in values {
                super::check(cancel)?;
                let value = (x / unit).round();
                if !value.is_finite() || value.abs() > i64::MAX as f64 {
                    return Err("Stem and leaf value exceeds its integer range".into());
                }
                let integer = value as i64;
                let leaf = integer.unsigned_abs() % 10;
                let stem = integer / 10;
                let split = u8::from(option == 2 && leaf >= 5);
                let key = (stem, integer < 0, split);
                if rows.last().is_none_or(|row| row.0 != key) {
                    rows.push((key, String::new()));
                }
                let leaves = &mut rows.last_mut().unwrap().1;
                for _ in 0..w {
                    leaves.push(char::from(b'0' + leaf as u8));
                }
            }
            push(&format!("StemLeaf unit {unit}"), PlotStyle::Scatter, vec![])?;
            for ((stem, negative, split), leaves) in rows {
                push(
                    &format!(
                        "StemLeaf {}{stem}{} | {leaves}",
                        if negative && stem == 0 { "-" } else { "" },
                        if option == 2 && split == 0 {
                            "*"
                        } else if option == 2 {
                            "."
                        } else {
                            ""
                        }
                    ),
                    PlotStyle::Scatter,
                    vec![],
                )?;
            }
        }
        Chart::Histogram => unreachable!("histograms use the configured bin geometry"),
    }
    let (x, y) = match chart {
        Chart::BoxWhisker => ("Value", "Analysis"),
        Chart::NormalProbability => ("Value", "Normal score"),
        Chart::Dot => ("Value", "Count"),
        Chart::Bar | Chart::Pareto => ("Category", "Frequency"),
        Chart::Pie => ("", ""),
        Chart::StemLeaf => ("Stem", "Leaf"),
        _ => ("Observation", "Value"),
    };
    plot.x_label = x.into();
    plot.y_label = y.into();
    Ok(plot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_stems_preserve_numeric_order_across_negative_zero() {
        let model = Statistics1 {
            samples: vec![
                -19., -16., -14., -11., -9., -6., -4., -1., 1., 4., 6., 9., 12.,
            ],
            frequencies: vec![],
        };
        let plot = render(&model, Chart::StemLeaf, 2, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            plot.labels,
            vec![
                "StemLeaf unit 1",
                "StemLeaf -1. | 96",
                "StemLeaf -1* | 41",
                "StemLeaf -0. | 96",
                "StemLeaf -0* | 41",
                "StemLeaf 0* | 14",
                "StemLeaf 0. | 69",
                "StemLeaf 1* | 2",
            ]
        );
    }
    #[test]
    fn stem_leaf_keeps_signed_zero_stems_and_weighted_repetitions() {
        let model = Statistics1 {
            samples: vec![-3., 3., 12.],
            frequencies: vec![2, 1, 1],
        };
        let plot = render(&model, Chart::StemLeaf, 1, &AtomicBool::new(false)).unwrap();
        assert!(plot.labels.iter().any(|s| s == "StemLeaf -0 | 33"));
        assert!(plot.labels.iter().any(|s| s == "StemLeaf 0 | 3"));
        assert!(plot.labels.iter().any(|s| s == "StemLeaf 1 | 2"));
    }
    #[test]
    fn weighted_categories_dot_and_pie_do_not_include_zero_frequency_values() {
        let model = Statistics1 {
            samples: vec![1., 1., 2., 999.],
            frequencies: vec![2, 1, 3, 0],
        };
        let cancel = AtomicBool::new(false);
        let dots = render(&model, Chart::Dot, 0, &cancel).unwrap();
        assert_eq!(
            dots.curves[0],
            vec![[1., 1.], [1., 2.], [1., 3.], [2., 1.], [2., 2.], [2., 3.]]
        );
        let bars = render(&model, Chart::Bar, 0, &cancel).unwrap();
        assert_eq!(bars.labels, vec!["1: 3", "2: 3"]);
        assert_eq!(bars.curves[0][1][1], 3.);
        let pie = render(&model, Chart::Pie, 0, &cancel).unwrap();
        assert_eq!(pie.curves.len(), 2);
        assert!(
            pie.curves
                .iter()
                .flatten()
                .all(|p| p[0].abs() <= 1. && p[1].abs() <= 1.)
        );
    }
    #[test]
    fn pareto_orders_frequency_and_accumulates_total() {
        let model = Statistics1 {
            samples: vec![10., 20., 30.],
            frequencies: vec![1, 5, 3],
        };
        let plot = render(&model, Chart::Pareto, 0, &AtomicBool::new(false)).unwrap();
        assert_eq!(&plot.labels[..3], &["20: 5", "30: 3", "10: 1"]);
        assert_eq!(plot.curves[3], vec![[1., 5.], [2., 8.], [3., 9.]]);
    }
    #[test]
    fn box_option_marks_outliers_and_normal_scores_are_symmetric() {
        let model = Statistics1 {
            samples: vec![1., 2., 3., 4., 100.],
            frequencies: vec![],
        };
        let plot = render(&model, Chart::BoxWhisker, 2, &AtomicBool::new(false)).unwrap();
        // Tukey's exclusive median halves place Q3 at52; no false outlier.
        assert_eq!(plot.labels[0], "Box");
        assert_eq!(plot.curves[1], vec![[3., 0.], [3., 1.]]);
        let plot = render(&model, Chart::NormalProbability, 0, &AtomicBool::new(false)).unwrap();
        let ys = plot.curves[0].iter().map(|p| p[1]).collect::<Vec<_>>();
        assert!((ys[0] + ys[4]).abs() < 1e-9);
        assert!(ys[2].abs() < 1e-9);
        assert!(ys.windows(2).all(|w| w[0] < w[1]));
    }
    #[test]
    fn moving_range_chart_uses_successive_differences_and_limits() {
        let model = Statistics1 {
            samples: vec![2., 5., 4.],
            frequencies: vec![],
        };
        let plot = render(&model, Chart::Control, 2, &AtomicBool::new(false)).unwrap();
        assert_eq!(plot.curves[0], vec![[1., 3.], [2., 1.]]);
        assert_eq!(plot.curves[1], vec![[1., 2.], [2., 2.]]);
        assert_eq!(plot.curves[2], vec![[1., 0.], [2., 0.]]);
    }
    #[test]
    fn charts_reject_cancellation_and_unbounded_mass_without_expansion() {
        let model = Statistics1 {
            samples: vec![1.],
            frequencies: vec![u64::from(u32::MAX)],
        };
        assert!(
            render(&model, Chart::Dot, 0, &AtomicBool::new(false))
                .unwrap_err()
                .contains("10000")
        );
        assert!(render(&model, Chart::Pie, 0, &AtomicBool::new(true)).is_err());
    }
}
