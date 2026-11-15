//! Each selected analysis contributes its own data/fit to the shared plot.
use super::{OneBook, TwoBook, check};
use crate::ide::native_model_plots::{self as plots, HistogramSpec, PlotData, PlotStyle};
use std::sync::atomic::AtomicBool;
pub fn one(
    book: &OneBook,
    spec: Option<HistogramSpec>,
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    one_with(book, spec, cancel, |index| book.model(index))
}
/// Evaluate definition expressions in the same pinned app memory used by the
/// worker. Scalar frequency expressions apply to every observation.
pub fn one_in(
    book: &OneBook,
    spec: Option<HistogramSpec>,
    snapshot: &ppl_vm::SessionSnapshot,
    cancel: std::sync::Arc<AtomicBool>,
) -> Result<PlotData, String> {
    one_with(book, spec, &cancel, |index| {
        let data =
            super::one_data::Data::evaluate(&book.analyses[index], snapshot, cancel.clone())?;
        let observations = data
            .values
            .iter()
            .zip(data.weights)
            .filter(|(_, weight)| *weight > 0)
            .map(|(x, weight)| {
                let n = x.to_f64();
                if !n.is_finite() || (!x.is_zero() && n == 0.) {
                    Err("Statistics plot value exceeds numerical range".to_owned())
                } else {
                    Ok((n, weight))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (samples, frequencies) = observations.into_iter().unzip();
        Ok(super::Statistics1 {
            samples,
            frequencies,
        })
    })
}
fn one_with(
    book: &OneBook,
    spec: Option<HistogramSpec>,
    cancel: &AtomicBool,
    model: impl Fn(usize) -> Result<super::Statistics1, String>,
) -> Result<PlotData, String> {
    book.validate()?;
    let enabled = book
        .analyses
        .iter()
        .filter(|a| a.enabled)
        .collect::<Vec<_>>();
    if enabled
        .iter()
        .any(|a| a.chart == super::one::Chart::StemLeaf)
        && enabled
            .iter()
            .any(|a| a.chart != super::one::Chart::StemLeaf)
    {
        return Err("Display stem and leaf separately from graphical charts".into());
    }
    let mut result = PlotData::new("Value", "Frequency");
    for (index, _) in book.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
        check(cancel)?;
        let model = model(index)?;
        let data = if book.analyses[index].chart != super::one::Chart::Histogram {
            super::charts::render(
                &model,
                book.analyses[index].chart,
                book.analyses[index].option,
                cancel,
            )?
        } else if let Some(spec) = spec {
            plots::histogram(&model, &spec, cancel)?
        } else {
            plots::histogram_auto(&model, cancel)?
        };
        if result.curves.is_empty() {
            result.x_label = data.x_label.clone();
            result.y_label = data.y_label.clone();
        } else {
            if result.x_label != data.x_label {
                result.x_label = "Mixed chart coordinates".into();
            }
            if result.y_label != data.y_label {
                result.y_label = "Mixed chart coordinates".into();
            }
        }
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
    cancel: &AtomicBool,
) -> Result<PlotData, String> {
    book.validate()?;
    let mut result = PlotData::new("X", "Y");
    for (index, _) in book.analyses.iter().enumerate().filter(|(_, a)| a.enabled) {
        check(cancel)?;
        let fit = book.fit(index, snapshot, cancel)?;
        let (model, frequencies) = book.model(index)?;
        let observations = model
            .pairs
            .iter()
            .enumerate()
            .filter(|(row, _)| frequencies.get(*row).copied().unwrap_or(1) > 0)
            .map(|(_, p)| *p)
            .collect::<Vec<_>>();
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
                .globals
                .get(name)?
                .to_value()
                .as_real()
                .map(|n| n.to_f64())
                .filter(|n| n.is_finite())
        };
        let (min, max) = match (bound("Xmin"), bound("Xmax")) {
            (Some(a), Some(b)) if a < b => (a, b),
            _ if low < high => (low, high),
            _ => (low - 1., high + 1.),
        };
        result.push(
            format!("S{} observations", index + 1),
            PlotStyle::Scatter,
            observations,
        )?;
        let label = format!("S{} {} fit", index + 1, fit.kind.label());
        let mut line = Vec::new();
        let mut previous: Option<f64> = None;
        for sample in 0..=256 {
            check(cancel)?;
            let x = min + (max - min) * sample as f64 / 256.;
            let pole = fit.kind == crate::ide::native_regression::FitKind::Inverse
                && previous.is_some_and(|p| {
                    p != 0. && x != 0. && p.is_sign_negative() != x.is_sign_negative()
                });
            if pole && !line.is_empty() {
                result.push(&label, PlotStyle::Line, std::mem::take(&mut line))?;
            }
            match fit.predict(x) {
                Ok(y) => line.push([x, y]),
                Err(_) => {
                    if !line.is_empty() {
                        result.push(&label, PlotStyle::Line, std::mem::take(&mut line))?;
                    }
                }
            }
            previous = Some(x);
        }
        if !line.is_empty() {
            result.push(&label, PlotStyle::Line, line)?;
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
    #[test]
    fn zero_frequency_extreme_values_do_not_break_projection_or_axis_labels() {
        let mut book = OneBook::default();
        book.analyses[0].data_source = Some("{1,1E400}".into());
        book.analyses[0].frequency_source = Some("{1,0}".into());
        book.analyses[0].chart = super::super::one::Chart::Dot;
        let plot = one_in(
            &book,
            None,
            &ppl_vm::SessionContext::default().snapshot(),
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(plot.curves[0], vec![[1., 1.]]);
        assert_eq!(
            (plot.x_label.as_str(), plot.y_label.as_str()),
            ("Value", "Count")
        );
    }
    #[test]
    fn plot_uses_live_definition_expressions_and_scalar_frequency() {
        let mut book = OneBook::default();
        book.analyses[0].data_source = Some("{1+3,2+3}".into());
        book.analyses[0].frequency_source = Some("2".into());
        book.analyses[0].chart = super::super::one::Chart::Dot;
        let plot = one_in(
            &book,
            None,
            &ppl_vm::SessionContext::default().snapshot(),
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(plot.curves[0], vec![[4., 1.], [4., 2.], [5., 1.], [5., 2.]]);
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
        let plot = one(&book, None, &AtomicBool::new(false)).unwrap();
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
        let plot = two(&book, &snapshot, &AtomicBool::new(false)).unwrap();
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
