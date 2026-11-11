//! Reference-distribution and confidence-interval views (HP Prime User Guide
//! chapter13, pp.271–272). Critical values, estimates, and intervals come from
//! the same inference calculation as Numeric view. Closed Line series mark
//! p-value/rejection/interval regions for the shared plot renderer.
//! Chi-square/F/regression requests are Native extensions rather than extra
//! HP Inference menu entries; documented app choices remain the six Z/t forms.
use super::native_inference::{
    Alternative, Distribution, InferenceMode, InferenceRequest, InferenceResult,
};
use super::native_model_plots::{PlotData, PlotStyle};
use std::sync::atomic::{AtomicBool, Ordering};
const SAMPLES: usize = 577; // leaves room for exact boundaries; every curve <=601
#[derive(Clone, Copy)]
struct Axis {
    location: f64,
    scale: f64,
}
impl Axis {
    fn x(self, value: f64) -> f64 {
        self.location + self.scale * value
    }
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Inference plot interrupted".into())
    } else {
        Ok(())
    }
}
fn add(
    data: &mut PlotData,
    label: impl Into<String>,
    style: PlotStyle,
    points: Vec<[f64; 2]>,
) -> Result<(), String> {
    if points.len() > 601 || points.iter().flatten().any(|x| !x.is_finite()) {
        return Err("Inference plot exceeds its finite sample range".into());
    }
    data.curves.push(points);
    data.labels.push(label.into());
    data.styles.push(style);
    Ok(())
}
fn mode(request: &InferenceRequest) -> Option<&InferenceMode> {
    use InferenceRequest::*;
    match request {
        MeanZ { mode, .. }
        | TwoMeansZ { mode, .. }
        | MeanT { mode, .. }
        | TwoMeansT { mode, .. }
        | OneProportion { mode, .. }
        | TwoProportions { mode, .. }
        | VarianceRatio { mode, .. }
        | Regression { mode, .. } => Some(mode),
        _ => None,
    }
}
fn distribution(
    request: &InferenceRequest,
    result: &InferenceResult,
) -> Result<Distribution, String> {
    use InferenceRequest::*;
    let df = |index| {
        result
            .degrees_of_freedom
            .get(index)
            .copied()
            .ok_or_else(|| "Inference result omitted required degrees of freedom".to_owned())
    };
    Ok(match request {
        MeanZ { .. } | TwoMeansZ { .. } | OneProportion { .. } | TwoProportions { .. } => {
            Distribution::Normal
        }
        MeanT { .. } | TwoMeansT { .. } | Regression { .. } => Distribution::Student { df: df(0)? },
        GoodnessOfFit { .. } | Contingency { .. } => Distribution::ChiSquare { df: df(0)? },
        VarianceRatio { .. } | Anova { .. } => Distribution::F {
            numerator_df: df(0)?,
            denominator_df: df(1)?,
        },
    })
}
fn caption(distribution: Distribution) -> String {
    match distribution {
        Distribution::Normal => "Normal density".into(),
        Distribution::Student { df } => format!("Student t density (df={df:.6})"),
        Distribution::ChiSquare { df } => format!("Chi-square density (df={df:.6})"),
        Distribution::F {
            numerator_df,
            denominator_df,
        } => format!("F density (df={numerator_df:.6},{denominator_df:.6})"),
    }
}
fn statistic_axis(distribution: Distribution) -> String {
    match distribution {
        Distribution::Normal => "Z",
        Distribution::Student { .. } => "t",
        Distribution::ChiSquare { .. } => "χ²",
        Distribution::F { .. } => "F",
    }
    .into()
}
fn singular_zero(distribution: Distribution) -> bool {
    matches!(distribution,Distribution::ChiSquare{df} if df<2.)
        || matches!(distribution,Distribution::F{numerator_df,..} if numerator_df<2.)
}
fn density_curve(
    distribution: Distribution,
    axis: Axis,
    anchors: &[f64],
    cancel: &AtomicBool,
) -> Result<Vec<[f64; 2]>, String> {
    let mut positions = Vec::with_capacity(SAMPLES + anchors.len() + 1);
    for index in 0..SAMPLES {
        check(cancel)?;
        let tail = if index == SAMPLES / 2 {
            0.5
        } else {
            0.999 - 0.998 * index as f64 / (SAMPLES - 1) as f64
        };
        positions.push(distribution.inverse_upper_tail(tail)?);
    }
    positions.extend(anchors.iter().copied());
    if !singular_zero(distribution) {
        positions.push(0.);
    }
    positions.sort_by(f64::total_cmp);
    positions.dedup();
    let mut curve = Vec::with_capacity(positions.len());
    for x in positions {
        check(cancel)?;
        if x == 0. && singular_zero(distribution) {
            continue;
        }
        curve.push([axis.x(x), distribution.density(x)? / axis.scale]);
    }
    if curve.len() > 597 || curve.iter().flatten().any(|x| !x.is_finite()) {
        return Err("Inference density exceeds numerical plot range".into());
    }
    Ok(curve)
}
/// Region endpoints are reference-statistic values; None means an unbounded tail.
type Region = (Option<f64>, Option<f64>);
fn p_regions(
    distribution: Distribution,
    statistic: f64,
    alternative: Alternative,
) -> Result<Vec<Region>, String> {
    Ok(match alternative {
        Alternative::Less => vec![(None, Some(statistic))],
        Alternative::Greater => vec![(Some(statistic), None)],
        Alternative::TwoSided => {
            if matches!(
                distribution,
                Distribution::Normal | Distribution::Student { .. }
            ) {
                vec![
                    (None, Some(-statistic.abs())),
                    (Some(statistic.abs()), None),
                ]
            } else {
                let lower = distribution.lower_tail(statistic)?;
                let upper = distribution.upper_tail(statistic)?;
                if lower <= upper {
                    let mut regions = vec![(None, Some(statistic))];
                    if lower > 0. {
                        regions.push((Some(distribution.inverse_upper_tail(lower)?), None));
                    }
                    regions
                } else {
                    let mut regions = vec![(Some(statistic), None)];
                    if upper > 0. {
                        let lower = if let Distribution::F {
                            numerator_df,
                            denominator_df,
                        } = distribution
                        {
                            1. / Distribution::F {
                                numerator_df: denominator_df,
                                denominator_df: numerator_df,
                            }
                            .inverse_upper_tail(upper)?
                        } else {
                            distribution.inverse_upper_tail(1. - upper)?
                        };
                        regions.push((None, Some(lower)));
                    }
                    regions
                }
            }
        }
    })
}
fn region_curve(density: &[[f64; 2]], region: Region, axis: Axis) -> Vec<[f64; 2]> {
    let lo = region.0.map(|x| axis.x(x)).unwrap_or(f64::NEG_INFINITY);
    let hi = region.1.map(|x| axis.x(x)).unwrap_or(f64::INFINITY);
    let inside: Vec<_> = density
        .iter()
        .filter(|p| p[0] >= lo && p[0] <= hi)
        .copied()
        .collect();
    let Some(first) = inside.first() else {
        return Vec::new();
    };
    let mut outline = vec![[first[0], 0.]];
    outline.extend(inside.iter());
    outline.push([inside.last().unwrap()[0], 0.]);
    outline.push([first[0], 0.]);
    outline
}
/// Returns bounded geometry for the request's actual reference distribution.
/// Region curves are outlines, not numerical substitutes for tail probabilities.
pub fn plot(request: &InferenceRequest, cancel: &AtomicBool) -> Result<PlotData, String> {
    check(cancel)?;
    let result = request.calculate_with_cancel(cancel)?;
    let mut reference = distribution(request, &result)?;
    let mut data = PlotData {
        curves: Vec::new(),
        labels: Vec::new(),
        styles: Vec::new(),
        x_label: statistic_axis(reference),
        y_label: "Density".into(),
    };
    if let Some(interval) = result.interval {
        let estimate = result
            .estimate
            .ok_or("Confidence result omitted its estimate")?;
        let scale = if matches!(request, InferenceRequest::VarianceRatio { .. }) {
            let Distribution::F {
                numerator_df,
                denominator_df,
            } = reference
            else {
                unreachable!()
            };
            // If U~F(df1,df2), 1/U~F(df2,df1): the variance-ratio
            // confidence pivot is observed_ratio/U, not a Normal curve.
            reference = Distribution::F {
                numerator_df: denominator_df,
                denominator_df: numerator_df,
            };
            estimate
        } else {
            result
                .standard_error
                .ok_or("Confidence result omitted its standard error")?
        };
        data.x_label = if matches!(request, InferenceRequest::VarianceRatio { .. }) {
            "Variance ratio"
        } else {
            "Parameter"
        }
        .into();
        if scale > 0. && interval[0] < interval[1] {
            let axis = Axis {
                location: if matches!(request, InferenceRequest::VarianceRatio { .. }) {
                    0.
                } else {
                    estimate
                },
                scale,
            };
            let anchors = interval.map(|x| (x - axis.location) / axis.scale);
            let density = density_curve(reference, axis, &anchors, cancel)?;
            add(
                &mut data,
                caption(reference),
                PlotStyle::Line,
                density.clone(),
            )?;
            add(
                &mut data,
                "Confidence region",
                PlotStyle::Line,
                region_curve(&density, (Some(anchors[0]), Some(anchors[1])), axis),
            )?;
        }
        if data.curves.is_empty() {
            data.y_label = "Interval endpoints".into();
        }
        add(
            &mut data,
            "Confidence interval",
            PlotStyle::Line,
            vec![[interval[0], 0.], [interval[1], 0.]],
        )?;
        add(
            &mut data,
            "Estimate",
            PlotStyle::Scatter,
            vec![[estimate, 0.]],
        )?;
        return Ok(data);
    }
    let statistic = result
        .statistic
        .ok_or("Test result omitted its statistic")?;
    let (alternative, alpha) = match mode(request) {
        Some(InferenceMode::Test {
            alternative, alpha, ..
        }) => (*alternative, *alpha),
        _ => (
            Alternative::Greater,
            match request {
                InferenceRequest::GoodnessOfFit { alpha, .. }
                | InferenceRequest::Contingency { alpha, .. }
                | InferenceRequest::Anova { alpha, .. } => *alpha,
                _ => return Err("Test request omitted its significance".into()),
            },
        ),
    };
    let pvalue = result.p_value.ok_or("Test result omitted its p-value")?;
    let regions = p_regions(reference, statistic, alternative)?;
    let mut anchors = vec![statistic];
    anchors.extend(result.critical_values.iter().copied());
    anchors.extend(regions.iter().flat_map(|(lo, hi)| [*lo, *hi]).flatten());
    let axis = Axis {
        location: 0.,
        scale: 1.,
    };
    let density = density_curve(reference, axis, &anchors, cancel)?;
    let height = density.iter().map(|p| p[1]).fold(0., f64::max);
    add(
        &mut data,
        caption(reference),
        PlotStyle::Line,
        density.clone(),
    )?;
    for (index, region) in regions.into_iter().enumerate() {
        check(cancel)?;
        add(
            &mut data,
            format!("P-value region {} (p={pvalue:.6e})", index + 1),
            PlotStyle::Line,
            region_curve(&density, region, axis),
        )?;
    }
    let rejection = match alternative {
        Alternative::Less => vec![(None, result.critical_values.first().copied())],
        Alternative::Greater => vec![(result.critical_values.first().copied(), None)],
        Alternative::TwoSided => vec![
            (None, result.critical_values.first().copied()),
            (result.critical_values.last().copied(), None),
        ],
    };
    for (index, region) in rejection.into_iter().enumerate() {
        add(
            &mut data,
            format!("Rejection region {} (α={alpha:.6e})", index + 1),
            PlotStyle::Line,
            region_curve(&density, region, axis),
        )?;
    }
    add(
        &mut data,
        "Observed statistic",
        PlotStyle::Line,
        vec![[statistic, 0.], [statistic, height]],
    )?;
    for (index, critical) in result.critical_values.iter().enumerate() {
        add(
            &mut data,
            format!("Critical value {}", index + 1),
            PlotStyle::Line,
            vec![[*critical, 0.], [*critical, height]],
        )?;
    }
    check(cancel)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::super::native_inference::{MeanSummary, ProportionSample, RegressionTarget};
    use super::*;
    fn near(a: f64, b: f64, tolerance: f64) {
        assert!(
            (a - b).abs() <= tolerance * b.abs().max(1e-200),
            "{a:.17e}!={b:.17e}"
        );
    }
    fn test(null_value: f64) -> InferenceMode {
        InferenceMode::Test {
            null_value,
            alpha: 0.05,
            alternative: Alternative::TwoSided,
        }
    }
    fn summary(mean: f64) -> MeanSummary {
        MeanSummary {
            mean,
            sd: 2.,
            count: 20,
        }
    }
    fn requests() -> Vec<InferenceRequest> {
        vec![
            InferenceRequest::default(),
            InferenceRequest::TwoMeansZ {
                first: summary(10.),
                second: summary(9.),
                mode: test(0.),
            },
            InferenceRequest::MeanT {
                sample: summary(10.),
                mode: test(9.),
            },
            InferenceRequest::TwoMeansT {
                first: summary(10.),
                second: summary(9.),
                pooled: false,
                mode: test(0.),
            },
            InferenceRequest::OneProportion {
                sample: ProportionSample {
                    successes: 60,
                    total: 100,
                },
                mode: test(0.5),
            },
            InferenceRequest::TwoProportions {
                first: ProportionSample {
                    successes: 60,
                    total: 100,
                },
                second: ProportionSample {
                    successes: 45,
                    total: 100,
                },
                mode: test(0.),
            },
            InferenceRequest::VarianceRatio {
                sd1: 2.,
                n1: 10,
                sd2: 1.,
                n2: 12,
                mode: test(1.),
            },
            InferenceRequest::GoodnessOfFit {
                observed: vec![10., 20., 30.],
                expected: vec![20., 20., 20.],
                estimated_parameters: 0,
                alpha: 0.05,
            },
            InferenceRequest::Contingency {
                table: vec![vec![10., 20.], vec![30., 40.]],
                alpha: 0.05,
            },
            InferenceRequest::Anova {
                groups: vec![vec![1., 2., 3.], vec![3., 4., 5.], vec![5., 6., 7.]],
                alpha: 0.05,
            },
            InferenceRequest::Regression {
                pairs: vec![[1., 2.], [2., 4.], [3., 5.], [4., 4.], [5., 5.]],
                target: RegressionTarget::Slope,
                mode: test(0.),
            },
        ]
    }
    #[test]
    fn eleven_families_use_actual_distributions_and_exact_critical_markers() {
        for (request, expected) in requests()
            .iter()
            .zip(["Z", "Z", "t", "t", "Z", "Z", "F", "χ²", "χ²", "F", "t"])
        {
            let result = request.calculate().unwrap();
            let data = plot(request, &AtomicBool::new(false)).unwrap();
            assert_eq!(data.x_label, expected);
            assert_eq!(data.curves.len(), data.labels.len());
            assert_eq!(data.curves.len(), data.styles.len());
            assert!(
                data.curves.iter().all(
                    |curve| curve.len() <= 601 && curve.iter().flatten().all(|v| v.is_finite())
                )
            );
            let observed = data
                .labels
                .iter()
                .position(|label| label == "Observed statistic")
                .unwrap();
            assert_eq!(data.curves[observed][0][0], result.statistic.unwrap());
            for (number, value) in result.critical_values.iter().enumerate() {
                let index = data
                    .labels
                    .iter()
                    .position(|label| label == &format!("Critical value {}", number + 1))
                    .unwrap();
                assert_eq!(data.curves[index][0][0], *value);
            }
            if matches!(expected, "F" | "χ²") {
                assert!(data.curves[0].iter().all(|point| point[0] >= 0.));
            }
            serde_json::to_value(&data).unwrap();
        }
    }
    #[test]
    fn density_at_zero_and_large_degrees_match_analytical_and_independent_references() {
        for (distribution, x, value) in [
            (Distribution::Normal, 0., 1. / std::f64::consts::TAU.sqrt()),
            (
                Distribution::Student { df: 1. },
                0.,
                1. / std::f64::consts::PI,
            ),
            (
                Distribution::Student { df: 1. },
                1.,
                1. / std::f64::consts::TAU,
            ),
            (Distribution::ChiSquare { df: 2. }, 0., 0.5),
            (
                Distribution::ChiSquare { df: 2. },
                2.,
                (-1.0_f64).exp() / 2.,
            ),
            (
                Distribution::F {
                    numerator_df: 2.,
                    denominator_df: 2.,
                },
                0.,
                1.,
            ),
            (
                Distribution::F {
                    numerator_df: 2.,
                    denominator_df: 2.,
                },
                3.,
                1. / 16.,
            ),
            (Distribution::Student { df: 1e6 }, 0., 0.398942180665875),
            (
                Distribution::ChiSquare { df: 1e6 },
                1e6,
                0.0002820947447580834,
            ),
            (
                Distribution::ChiSquare { df: 1e12 },
                1_000_001_000_000.,
                2.196954616542793e-7,
            ),
            (
                Distribution::F {
                    numerator_df: 1e6,
                    denominator_df: 1e6,
                },
                1.,
                199.471_090_332_937_5,
            ),
            (
                Distribution::F {
                    numerator_df: 1e12,
                    denominator_df: 1e12,
                },
                1.000001,
                176032.50935366142,
            ),
            (
                Distribution::F {
                    numerator_df: 1e12,
                    denominator_df: 3.,
                },
                1.,
                0.462540989410614,
            ),
            (
                Distribution::F {
                    numerator_df: 3.,
                    denominator_df: 1e12,
                },
                1.,
                0.462540989410614,
            ),
        ] {
            near(distribution.density(x).unwrap(), value, 1e-8);
        }
        assert!(Distribution::ChiSquare { df: 1. }.density(0.).is_err());
        assert!(
            Distribution::F {
                numerator_df: 1.,
                denominator_df: 10.
            }
            .density(0.)
            .is_err()
        );
        assert!(Distribution::Student { df: 0. }.density(0.).is_err());
        assert_eq!(Distribution::ChiSquare { df: 2. }.density(-1.).unwrap(), 0.);
    }
    #[test]
    fn confidence_plot_uses_actual_estimate_and_interval_in_parameter_units() {
        let request = InferenceRequest::MeanZ {
            mean: 10.,
            sigma: 2.,
            count: 25,
            mode: InferenceMode::Interval { confidence: 0.95 },
        };
        let result = request.calculate().unwrap();
        let data = plot(&request, &AtomicBool::new(false)).unwrap();
        let interval = data
            .labels
            .iter()
            .position(|label| label == "Confidence interval")
            .unwrap();
        let expected = result.interval.unwrap();
        assert_eq!(
            data.curves[interval],
            vec![[expected[0], 0.], [expected[1], 0.]]
        );
        let estimate = data
            .labels
            .iter()
            .position(|label| label == "Estimate")
            .unwrap();
        assert_eq!(data.curves[estimate], vec![[10., 0.]]);
        let center = data.curves[0].iter().find(|point| point[0] == 10.).unwrap();
        near(center[1], 1. / (0.4 * std::f64::consts::TAU.sqrt()), 1e-12);
    }
    #[test]
    fn asymmetric_f_two_sided_regions_have_correct_combined_probability() {
        let distribution = Distribution::F {
            numerator_df: 9.,
            denominator_df: 11.,
        };
        for statistic in [0.1, 1., 4., 1e20] {
            let regions = p_regions(distribution, statistic, Alternative::TwoSided).unwrap();
            let mass = regions
                .into_iter()
                .map(|(lo, hi)| match (lo, hi) {
                    (None, Some(hi)) => distribution.lower_tail(hi).unwrap(),
                    (Some(lo), None) => distribution.upper_tail(lo).unwrap(),
                    _ => panic!("expected tails"),
                })
                .sum::<f64>();
            let expected = (2.
                * distribution
                    .lower_tail(statistic)
                    .unwrap()
                    .min(distribution.upper_tail(statistic).unwrap()))
            .min(1.);
            near(mass, expected, 1e-8);
        }
    }
    #[test]
    fn variance_ratio_interval_uses_scaled_reciprocal_f_distribution() {
        let request = InferenceRequest::VarianceRatio {
            sd1: 2.,
            n1: 10,
            sd2: 1.,
            n2: 12,
            mode: InferenceMode::Interval { confidence: 0.95 },
        };
        let result = request.calculate().unwrap();
        let data = plot(&request, &AtomicBool::new(false)).unwrap();
        assert!(data.labels[0].starts_with("F density (df=11.000000,9.000000)"));
        let interval = result.interval.unwrap();
        let reference = Distribution::F {
            numerator_df: 11.,
            denominator_df: 9.,
        };
        near(
            reference.lower_tail(interval[1] / 4.).unwrap()
                - reference.lower_tail(interval[0] / 4.).unwrap(),
            0.95,
            1e-10,
        );
        let index = data
            .labels
            .iter()
            .position(|label| label == "Confidence interval")
            .unwrap();
        assert_eq!(data.curves[index][0][0], interval[0]);
    }
    #[test]
    fn canceled_plot_returns_before_evaluating_and_degenerate_interval_is_not_fake_density() {
        assert_eq!(
            plot(&InferenceRequest::default(), &AtomicBool::new(true)).unwrap_err(),
            "Inference plot interrupted"
        );
        let request = InferenceRequest::OneProportion {
            sample: ProportionSample {
                successes: 0,
                total: 10,
            },
            mode: InferenceMode::Interval { confidence: 0.95 },
        };
        let data = plot(&request, &AtomicBool::new(false)).unwrap();
        assert_eq!(data.labels, vec!["Confidence interval", "Estimate"]);
        assert_eq!(data.curves[0], vec![[0., 0.], [0., 0.]]);
    }
    #[test]
    fn all_eight_interval_families_preserve_actual_calculated_bounds() {
        let mut count = 0;
        for mut request in requests() {
            use InferenceRequest::*;
            let mode = match &mut request {
                MeanZ { mode, .. }
                | TwoMeansZ { mode, .. }
                | MeanT { mode, .. }
                | TwoMeansT { mode, .. }
                | OneProportion { mode, .. }
                | TwoProportions { mode, .. }
                | VarianceRatio { mode, .. }
                | Regression { mode, .. } => mode,
                _ => continue,
            };
            *mode = InferenceMode::Interval { confidence: 0.9 };
            let expected = request.calculate().unwrap().interval.unwrap();
            let data = plot(&request, &AtomicBool::new(false)).unwrap();
            let index = data
                .labels
                .iter()
                .position(|label| label == "Confidence interval")
                .unwrap();
            assert_eq!(
                data.curves[index],
                vec![[expected[0], 0.], [expected[1], 0.]]
            );
            assert!(data.curves.iter().all(|curve| curve.len() <= 601));
            count += 1;
        }
        assert_eq!(count, 8);
    }
}
