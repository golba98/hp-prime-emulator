//! Each selected analysis contributes its own data/fit to the shared plot.
use super::{OneBook, TwoBook, check};
use crate::ide::native_model_plots::{
    self as plots, HistogramSpec, PlotData, PlotScale, PlotScaleBounds, PlotStyle,
};
use std::sync::{Arc, atomic::AtomicBool};
pub fn one(
    book: &OneBook,
    spec: Option<HistogramSpec>,
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    book.validate()?;
    let mut result = PlotData::new("Value", "Frequency");
    for (index, _) in book.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
        check(cancel)?;
        let model = book.model(index)?;
        let data = if let Some(spec) = spec {
            plots::histogram(&model, &spec, cancel)?
        } else {
            plots::histogram_auto(&model, cancel)?
        };
        for ((label, style), curve) in data.labels.into_iter().zip(data.styles).zip(data.curves) {
            result.push(format!("H{} {label}", index + 1), style, curve)?;
        }
    }
    if result.curves.is_empty() {
        return Err("Select at least one H analysis".into());
    }
    Ok(result)
}
pub fn two(
    book: &TwoBook,
    snapshot: &ppl_vm::SessionSnapshot,
    cancel: &Arc<AtomicBool>,
) -> Result<PlotData, String> {
    two_plot(book, snapshot, cancel, None)
}

pub fn two_autoscale(
    book: &TwoBook,
    snapshot: &ppl_vm::SessionSnapshot,
    cancel: &Arc<AtomicBool>,
) -> Result<PlotData, String> {
    book.validate()?;
    let bounds = observation_bounds(book, snapshot, cancel)?;
    let (mut data, bounds) = match bounds {
        Some([xmin, xmax, ymin, ymax])
            if xmin < xmax
                && ymin < ymax
                && [xmin, xmax, ymin, ymax].iter().all(|v| v.is_finite()) =>
        {
            let data = two_plot(book, snapshot, cancel, Some([xmin, xmax]))?;
            (
                data,
                Some(PlotScaleBounds::Finite([xmin, xmax, ymin, ymax])),
            )
        }
        Some(_) => return two(book, snapshot, cancel),
        None => (PlotData::new("X", "Y"), None),
    };
    data.scale = Some(PlotScale {
        bounds,
        histogram: None,
    });
    Ok(data)
}

fn observation_bounds(
    book: &TwoBook,
    snapshot: &ppl_vm::SessionSnapshot,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<[f64; 4]>, String> {
    let mut bounds = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut has_observations = false;
    for (index, _) in book.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
        check(cancel)?;
        let (model, frequencies) = book.model(index, snapshot, cancel)?;
        if model.pairs.is_empty()
            || (!frequencies.is_empty() && frequencies.iter().all(|&w| w == 0))
        {
            continue;
        }
        // The original fits the data rectangle of each checked analysis in
        // order. Zero-frequency rows still extend that rectangle when the
        // analysis has positive mass.
        for [x, y] in model.pairs {
            bounds = [
                bounds[0].min(x),
                bounds[1].max(x),
                bounds[2].min(y),
                bounds[3].max(y),
            ];
        }
        for axis in [0, 2] {
            let margin = (bounds[axis + 1] - bounds[axis]) / 30.;
            bounds[axis] -= margin;
            bounds[axis + 1] += margin;
        }
        has_observations = true;
    }
    if !has_observations {
        return Ok(None);
    }
    bounds[0] += (bounds[1] - bounds[0]) / 319.;
    Ok(Some(bounds))
}

fn two_plot(
    book: &TwoBook,
    snapshot: &ppl_vm::SessionSnapshot,
    cancel: &Arc<AtomicBool>,
    autoscaled_x: Option<[f64; 2]>,
) -> Result<PlotData, String> {
    book.validate()?;
    let mut result = PlotData::new("X", "Y");
    for (index, _) in book.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
        check(cancel)?;
        let (model, frequencies) = book.model(index, snapshot, cancel)?;
        let observations = model
            .pairs
            .iter()
            .enumerate()
            .filter(|(row, _)| frequencies.get(*row).copied().unwrap_or(1) > 0)
            .map(|(_, p)| *p)
            .collect::<Vec<_>>();
        if autoscaled_x.is_some() && observations.is_empty() {
            continue;
        }
        let fit = book.fit_model(index, snapshot, cancel, &model, &frequencies)?;
        result.statistics_two_fits.push((
            index,
            fit.equation(),
            snapshot.statistics2_columns_revision,
        ));
        let low = observations
            .iter()
            .map(|p| p[0])
            .fold(f64::INFINITY, f64::min);
        let high = observations
            .iter()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        let bound = |name: &str| {
            snapshot
                .plot_variable_for("Statistics 2Var", name)?
                .to_value()
                .as_real()
                .map(|n| n.to_f64())
                .filter(|n| n.is_finite())
        };
        let [min, max] = autoscaled_x.unwrap_or_else(|| match (bound("Xmin"), bound("Xmax")) {
            (Some(a), Some(b)) if a < b => [a, b],
            _ if low < high => [low, high],
            _ => [low - 1., high + 1.],
        });
        result.push(
            format!("S{} observations", index + 1),
            PlotStyle::Scatter,
            observations,
        )?;
        result
            .colors
            .push(book.analyses[index].observation_color(index));
        let label = format!("S{} {} fit", index + 1, book.analyses[index].fit.label());
        let fit_color = book.analyses[index].fit_color(index);
        let mut line = Vec::new();
        let mut previous: Option<f64> = None;
        for sample in 0..=256 {
            check(cancel)?;
            let x = min + (max - min) * sample as f64 / 256.;
            let pole = fit.kind == crate::ide::native_regression::FitKind::Inverse
                && previous.is_some_and(|p| p * x < 0.);
            if pole && !line.is_empty() {
                result.push(&label, PlotStyle::Line, std::mem::take(&mut line))?;
                result.colors.push(fit_color);
            }
            match fit.predict(x) {
                Ok(y) => line.push([x, y]),
                Err(_) => {
                    if !line.is_empty() {
                        result.push(&label, PlotStyle::Line, std::mem::take(&mut line))?;
                        result.colors.push(fit_color);
                    }
                }
            }
            previous = Some(x);
        }
        if !line.is_empty() {
            result.push(&label, PlotStyle::Line, line)?;
            result.colors.push(fit_color);
        }
    }
    if result.curves.is_empty() {
        return Err("Select at least one S analysis".into());
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::{
        native_regression::FitKind,
        native_statistics::{OneAnalysis, TwoAnalysis},
    };
    fn assert_autoscale(plot: &PlotData, expected: [f64; 4]) {
        let Some(PlotScaleBounds::Finite(actual)) = &plot.scale.as_ref().unwrap().bounds else {
            panic!("missing finite scale: {:?}", plot.scale);
        };
        for (&actual, expected) in actual.iter().zip(expected) {
            // The original reports the bounds with 12 significant decimal digits.
            let tolerance = expected.abs().max(1.) * 1e-11;
            assert!(
                (actual - expected).abs() < tolerance,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn statistics_two_autoscale_expands_observations_in_analysis_order() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![1., 2., 3.];
        book.columns.0[1] = vec![3., 5., 7.];
        book.columns.0[3] = vec![-5., 0., 10.];
        book.columns.0[4] = vec![-12., 10., 4.];
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let plot = two_autoscale(&book, &snapshot, &cancel).unwrap();
        assert_autoscale(
            &plot,
            [0.940020898642, 3.06666666666, 2.86666666666, 7.13333333333],
        );
        book.analyses[1] = TwoAnalysis {
            enabled: true,
            x: Some(3),
            y: Some(4),
            ..Default::default()
        };
        let plot = two_autoscale(&book, &snapshot, &cancel).unwrap();
        assert_autoscale(&plot, [-5.44984326019, 10.5, -12.7333333333, 10.7333333333]);
        assert!(plot.labels.iter().any(|label| label == "S2 observations"));
        book.analyses.swap(0, 1);
        let plot = two_autoscale(&book, &snapshot, &cancel).unwrap();
        assert_autoscale(
            &plot,
            [-5.97983281087, 11.0333333333, -13.5155555555, 11.5155555555],
        );
    }

    #[test]
    fn statistics_two_autoscale_keeps_zero_weight_extrema_when_the_fit_has_mass() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![1., 2., 3., 99.];
        book.columns.0[1] = vec![3., 5., 7., -80.];
        book.columns.0[2] = vec![1., 2., 1., 0.];
        book.analyses[0].frequency = Some(2);
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let plot = two_autoscale(&book, &snapshot, &cancel).unwrap();
        assert_autoscale(&plot, [-1.93897596656, 102.266666666, -82.9, 9.9]);
        assert_eq!(plot.curves[0], [[1., 3.], [2., 5.], [3., 7.]]);
        book.columns.0[2].fill(0.);
        let empty = two_autoscale(&book, &snapshot, &cancel).unwrap();
        assert!(empty.scale.unwrap().bounds.is_none());
        assert!(empty.curves.is_empty());
        book.analyses[0].enabled = false;
        let unselected = two_autoscale(&book, &snapshot, &cancel).unwrap();
        assert!(unselected.scale.unwrap().bounds.is_none());
    }

    #[test]
    fn statistics_two_autoscale_bounds_ignore_the_inverse_fit_pole() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![-4., -2., 1., 3.];
        book.columns.0[1] = vec![-2., -1., 5., 8.];
        book.analyses[0].fit = FitKind::Inverse;
        let plot = two_autoscale(
            &book,
            &ppl_vm::SessionContext::default().snapshot(),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_autoscale(
            &plot,
            [-4.20992685475, 3.23333333333, -2.33333333333, 8.33333333333],
        );
        let fit_segments: Vec<_> = plot
            .styles
            .iter()
            .zip(&plot.curves)
            .filter_map(|(style, curve)| (*style == PlotStyle::Line).then_some(curve))
            .collect();
        assert_eq!(fit_segments.len(), 2);
        assert!(fit_segments[0].iter().all(|p| p[0] < 0.));
        assert!(fit_segments[1].iter().all(|p| p[0] > 0.));
    }

    #[test]
    fn selected_histograms_use_their_own_data_and_frequency_columns() {
        let mut book = OneBook::default();
        book.columns.0[0] = vec![1., 2., 3.];
        book.columns.0[1] = vec![2., 3., 4.];
        book.analyses[0].frequency = Some(1);
        book.analyses[1] = OneAnalysis {
            enabled: true,
            data: Some(1),
            frequency: None,
            ..Default::default()
        };
        let plot = one(&book, None, &Arc::new(AtomicBool::new(false))).unwrap();
        assert!(plot.labels.iter().any(|name| name.starts_with("H1 ")));
        assert!(plot.labels.iter().any(|name| name.starts_with("H2 ")));
        assert!(plot.curves.iter().flatten().any(|p| p[1] >= 4.));
    }
    #[test]
    fn inverse_fit_splits_its_pole_and_excludes_zero_frequency_observations() {
        let mut book = TwoBook::default();
        book.columns.0[0] = vec![-2., -1., 0., 1., 2.];
        book.columns.0[1] = vec![0.5, -1., 99., 5., 3.5];
        book.columns.0[2] = vec![1., 1., 0., 1., 1.];
        book.analyses[0] = TwoAnalysis {
            enabled: true,
            x: Some(0),
            y: Some(1),
            frequency: Some(2),
            fit: FitKind::Inverse,
            ..Default::default()
        };
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.globals.insert(
            "Xmin".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(-3)),
        );
        snapshot.globals.insert(
            "Xmax".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(2)),
        );
        let plot = two(&book, &snapshot, &Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(plot.curves[0].len(), 4);
        let lines = plot
            .curves
            .iter()
            .zip(&plot.styles)
            .filter(|(_, style)| **style == PlotStyle::Line)
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        for (line, _) in lines {
            assert!(line.iter().all(|p| p[0] < 0.) || line.iter().all(|p| p[0] > 0.));
        }
    }
}
