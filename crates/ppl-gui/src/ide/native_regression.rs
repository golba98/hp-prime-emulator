//! Statistics 2Var fits (HP Prime User Guide, chapter 12).
//! Coefficients are ascending powers for polynomials, [b,m] otherwise.
use super::native_app_models::Statistics2;
use serde::{Deserialize, Serialize};
mod nonlinear;
mod user_curve;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitKind {
    #[default]
    Linear,
    Logarithmic,
    Exponential,
    Power,
    Exponent,
    Inverse,
    Quadratic,
    Cubic,
    Quartic,
    Logistic,
    Trigonometric,
    MedianMedian,
    UserDefined,
}
impl FitKind {
    pub const ALL: [Self; 13] = [
        Self::Linear,
        Self::Logarithmic,
        Self::Exponential,
        Self::Power,
        Self::Exponent,
        Self::Inverse,
        Self::Logistic,
        Self::Quadratic,
        Self::Cubic,
        Self::Quartic,
        Self::Trigonometric,
        Self::MedianMedian,
        Self::UserDefined,
    ];
    pub fn from_code(code: i64) -> Option<Self> {
        usize::try_from(code.checked_sub(1)?)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }
    pub fn code(self) -> i64 {
        Self::ALL.iter().position(|kind| *kind == self).unwrap() as i64 + 1
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Logarithmic => "Logarithmic",
            Self::Exponential => "Exponential",
            Self::Power => "Power",
            Self::Exponent => "Exponent",
            Self::Inverse => "Inverse",
            Self::Quadratic => "Quadratic",
            Self::Cubic => "Cubic",
            Self::Quartic => "Quartic",
            Self::Logistic => "Logistic",
            Self::Trigonometric => "Trigonometric",
            Self::MedianMedian => "Median-median",
            Self::UserDefined => "User Defined",
        }
    }
    fn degree(self) -> usize {
        match self {
            Self::Quadratic => 2,
            Self::Cubic => 3,
            Self::Quartic => 4,
            _ => 1,
        }
    }
}
/// Optional frequency column and app-specific fit inputs. Empty frequencies
/// mean one observation per row. Zero frequencies exclude a row completely.
pub struct FitOptions<'a> {
    pub frequencies: &'a [u64],
    /// None or zero computes L; a positive L fixes the logistic saturation.
    pub logistic_limit: Option<f64>,
    pub user_expression: Option<&'a str>,
    pub angle: ppl_math::AngleMode,
    pub cancel: Option<&'a std::sync::atomic::AtomicBool>,
}
impl Default for FitOptions<'_> {
    fn default() -> Self {
        Self {
            frequencies: &[],
            logistic_limit: None,
            user_expression: None,
            angle: ppl_math::AngleMode::Radians,
            cancel: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Regression {
    pub kind: FitKind,
    pub count: usize,
    pub coefficients: Vec<f64>,
    pub residual_sum_squares: f64,
    #[serde(default)]
    pub user_expression: Option<String>,
    #[serde(default)]
    pub angle_code: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSummary {
    pub mean: f64,
    pub sum: f64,
    pub sum_squares: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub median: f64,
    pub population_variance: f64,
    pub sample_variance: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedSummary {
    pub count: usize,
    pub x: ColumnSummary,
    pub y: ColumnSummary,
    pub sum_products: f64,
    pub population_covariance: f64,
    pub sample_covariance: Option<f64>,
    pub correlation: Option<f64>,
}
impl Regression {
    pub fn predict(&self, x: f64) -> Result<f64, String> {
        use FitKind::*;
        let c = &self.coefficients;
        let expected = match self.kind {
            UserDefined => 0,
            Logistic => 3,
            Trigonometric => 4,
            kind => kind.degree() + 1,
        };
        if !x.is_finite() || c.len() != expected || c.iter().any(|v| !v.is_finite()) {
            return Err("Invalid fit coefficients or x coordinate".into());
        }
        let y = match self.kind {
            Logarithmic if x > 0. => c[0] + c[1] * x.ln(),
            Power if x > 0. => c[0] * x.powf(c[1]),
            Inverse if x != 0. => c[0] + c[1] / x,
            Exponential => c[0] * (c[1] * x).exp(),
            Exponent => c[0] * c[1].powf(x),
            Logistic => nonlinear::logistic_value(c[0], c[1].ln() - c[2] * x),
            Trigonometric => c[0] * (c[1] * x + c[2]).sin() + c[3],
            UserDefined => user_curve::evaluate(
                self.user_expression
                    .as_deref()
                    .ok_or("Missing user fit expression")?,
                x,
                ppl_math::AngleMode::from_code(self.angle_code).ok_or("Invalid fit angle mode")?,
            )?,
            Linear | Quadratic | Cubic | Quartic | MedianMedian => {
                c.iter().rev().fold(0., |y, a| y * x + a)
            }
            _ => return Err("Fit is undefined at this x coordinate".into()),
        };
        if x.is_finite() && y.is_finite() {
            Ok(y)
        } else {
            Err("Fit exceeds numerical range".into())
        }
    }
}
impl Statistics2 {
    pub fn summary_with_frequencies(&self, frequencies: &[u64]) -> Result<PairedSummary, String> {
        let points = weighted_points(&self.pairs, frequencies)?;
        let total = points.iter().map(|p| p.2).sum::<f64>();
        let column = |axis: usize| -> ColumnSummary {
            let mut values: Vec<_> = points
                .iter()
                .map(|p| (if axis == 0 { p.0 } else { p.1 }, p.2))
                .collect();
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mean = values.iter().map(|(x, w)| x * (w / total)).sum::<f64>();
            let sum_squares = values.iter().map(|(x, w)| x * x * w).sum::<f64>();
            let m2 = values
                .iter()
                .map(|(x, w)| (x - mean).powi(2) * w)
                .sum::<f64>();
            let rank = |rank: f64| {
                let mut n = 0.;
                for &(x, w) in &values {
                    n += w;
                    if rank < n {
                        return x;
                    }
                }
                unreachable!()
            };
            ColumnSummary {
                mean,
                sum: values.iter().map(|(x, w)| x * w).sum(),
                sum_squares,
                minimum: values[0].0,
                maximum: values.last().unwrap().0,
                median: rank(((total - 1.) / 2.).floor()) / 2. + rank((total / 2.).floor()) / 2.,
                population_variance: m2 / total,
                sample_variance: (total > 1.).then(|| m2 / (total - 1.)),
            }
        };
        let x = column(0);
        let y = column(1);
        let covariance = points
            .iter()
            .map(|&(a, b, w)| (a - x.mean) * (b - y.mean) * (w / total))
            .sum::<f64>();
        let sum_products = points.iter().map(|&(a, b, w)| a * b * w).sum::<f64>();
        let correlation = (x.population_variance > 0. && y.population_variance > 0.).then(|| {
            (covariance / x.population_variance.sqrt() / y.population_variance.sqrt())
                .clamp(-1., 1.)
        });
        let result = PairedSummary {
            count: total as usize,
            x,
            y,
            sum_products,
            population_covariance: covariance,
            sample_covariance: (total > 1.).then(|| covariance * (total / (total - 1.))),
            correlation,
        };
        // Refuse nonfinite quantities before JSON can replace them with null.
        for column in [&result.x, &result.y] {
            if [
                column.mean,
                column.sum,
                column.sum_squares,
                column.population_variance,
            ]
            .iter()
            .any(|v| !v.is_finite())
            {
                return Err("Summary statistics exceed numerical range".into());
            }
        }
        if !covariance.is_finite() || !sum_products.is_finite() {
            return Err("Summary statistics exceed numerical range".into());
        }
        Ok(result)
    }
    pub fn fit(&self) -> Result<Regression, String> {
        self.fit_with_options(&FitOptions::default())
    }
    pub fn fit_with_options(&self, options: &FitOptions<'_>) -> Result<Regression, String> {
        use FitKind::*;
        check_cancel(options.cancel)?;
        let points = weighted_points(&self.pairs, options.frequencies)?;
        let degree = self.fit_kind.degree();
        if points.len() <= degree {
            return Err(format!(
                "{} fit needs at least {} points",
                self.fit_kind.label(),
                degree + 1
            ));
        }
        let mut x = Vec::new();
        let mut y = Vec::new();
        for &(a, b, _) in &points {
            if !a.is_finite() || !b.is_finite() {
                return Err("Values must be finite real numbers".into());
            }
            x.push(match self.fit_kind {
                Logarithmic | Power if a > 0. => a.ln(),
                Logarithmic | Power => return Err("This fit requires positive x values".into()),
                Inverse if a != 0. => 1. / a,
                Inverse => return Err("Inverse fit requires nonzero x values".into()),
                _ => a,
            });
            y.push(match self.fit_kind {
                Exponential | Exponent | Power if b > 0. => b.ln(),
                Exponential | Exponent | Power => {
                    return Err("This fit requires positive y values".into());
                }
                _ => b,
            });
        }
        let weights: Vec<_> = points.iter().map(|p| p.2).collect();
        let mut coefficients = match self.fit_kind {
            Logistic => nonlinear::logistic(&points, options.logistic_limit, options.cancel)?,
            Trigonometric => nonlinear::trigonometric(&points, options.cancel)?,
            MedianMedian => median_median(&points)?,
            UserDefined => {
                let expression = options
                    .user_expression
                    .ok_or("Enter a user fit expression in X")?;
                user_curve::validate(expression)?;
                vec![]
            }
            _ => polynomial_fit(&x, &y, &weights, degree)?,
        };
        match self.fit_kind {
            Exponential | Power => coefficients[0] = coefficients[0].exp(),
            Exponent => {
                coefficients[0] = coefficients[0].exp();
                coefficients[1] = coefficients[1].exp();
            }
            _ => {}
        }
        if !coefficients.iter().all(|c| c.is_finite()) {
            return Err("Fit exceeds numerical range".into());
        }
        let mut result = Regression {
            kind: self.fit_kind,
            count: points.iter().map(|p| p.2 as usize).sum(),
            coefficients,
            residual_sum_squares: 0.,
            user_expression: options
                .user_expression
                .map(str::to_owned)
                .filter(|_| self.fit_kind == UserDefined),
            angle_code: options.angle.code(),
        };
        for &(x, y, weight) in &points {
            check_cancel(options.cancel)?;
            result.residual_sum_squares += weight * (y - result.predict(x)?).powi(2);
        }
        if !result.residual_sum_squares.is_finite() {
            return Err("Fit residuals exceed numerical range".into());
        }
        Ok(result)
    }
}
fn check_cancel(cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn weighted_points(
    pairs: &[[f64; 2]],
    frequencies: &[u64],
) -> Result<Vec<(f64, f64, f64)>, String> {
    if !frequencies.is_empty() && frequencies.len() != pairs.len() {
        return Err("Frequency column must match the observation count".into());
    }
    let mut total = 0u64;
    let mut result = Vec::new();
    for (i, &[x, y]) in pairs.iter().enumerate() {
        let weight = frequencies.get(i).copied().unwrap_or(1);
        if weight == 0 {
            continue;
        }
        total = total
            .checked_add(weight)
            .filter(|v| *v <= (1u64 << 53).min(usize::MAX as u64))
            .ok_or("Frequency total exceeds exact numerical range")?;
        if !x.is_finite() || !y.is_finite() {
            return Err("Values must be finite real numbers".into());
        }
        result.push((x, y, weight as f64));
    }
    if result.is_empty() {
        return Err("Enter observations with positive frequencies".into());
    }
    Ok(result)
}
fn median_median(points: &[(f64, f64, f64)]) -> Result<Vec<f64>, String> {
    let mut points = points.to_vec();
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let count = points.iter().map(|p| p.2 as u64).sum::<u64>();
    if count < 3 {
        return Err("Median-median fit needs at least three observations".into());
    }
    let first = count / 3 + u64::from(count % 3 == 2);
    let middle = count - 2 * first;
    fn median(points: &[(f64, f64, f64)], start: u64, len: u64, axis: usize) -> f64 {
        let mut end = 0u64;
        let mut selected = Vec::new();
        for &(x, y, w) in points {
            let next = end + w as u64;
            let overlap = next.min(start + len).saturating_sub(end.max(start));
            if overlap > 0 {
                selected.push((if axis == 0 { x } else { y }, overlap));
            }
            end = next;
        }
        selected.sort_by(|a, b| a.0.total_cmp(&b.0));
        let value = |rank| {
            let mut n = 0;
            for &(v, w) in &selected {
                n += w;
                if rank < n {
                    return v;
                }
            }
            unreachable!()
        };
        value((len - 1) / 2) / 2. + value(len / 2) / 2.
    }
    let medians = [(0, first), (first, middle), (first + middle, first)]
        .map(|(start, n)| [median(&points, start, n, 0), median(&points, start, n, 1)]);
    let [a, _, c] = medians;
    if a[0] == c[0] {
        return Err("Median-median fit requires distinct outer x medians".into());
    }
    let slope = (c[1] - a[1]) / (c[0] - a[0]);
    let intercept = medians.iter().map(|p| (p[1] - slope * p[0]) / 3.).sum();
    Ok(vec![intercept, slope])
}

// Reorthogonalized modified Gram-Schmidt QR on a centered/scaled Vandermonde
// matrix avoids squaring the condition number through normal equations.
fn polynomial_fit(
    x: &[f64],
    y: &[f64],
    weights: &[f64],
    degree: usize,
) -> Result<Vec<f64>, String> {
    let total = weights.iter().sum::<f64>();
    let center = x
        .iter()
        .zip(weights)
        .map(|(x, w)| x * (w / total))
        .sum::<f64>();
    let scale = x.iter().map(|x| (x - center).abs()).fold(0., f64::max);
    if !scale.is_finite() || scale == 0. {
        return Err("Fit requires distinct x coordinates".into());
    }
    let z: Vec<_> = x.iter().map(|x| (x - center) / scale).collect();
    let n = degree + 1;
    let mut q: Vec<Vec<f64>> = Vec::new();
    let mut r = vec![vec![0.; n]; n];
    for (j, mut col) in (0..n)
        .map(|j| {
            z.iter()
                .zip(weights)
                .map(|(z, w)| z.powi(j as i32) * (w / total).sqrt())
                .collect::<Vec<_>>()
        })
        .enumerate()
    {
        for _ in 0..2 {
            for i in 0..j {
                let dot = q[i].iter().zip(&col).map(|(a, b)| a * b).sum::<f64>();
                r[i][j] += dot;
                for (v, a) in col.iter_mut().zip(&q[i]) {
                    *v -= dot * a;
                }
            }
        }
        let norm = col.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm <= 1e-12 * (x.len() as f64).sqrt() {
            return Err("Fit data are rank deficient".into());
        }
        r[j][j] = norm;
        q.push(col.into_iter().map(|v| v / norm).collect());
    }
    let mut a: Vec<_> = q
        .iter()
        .map(|c| {
            c.iter()
                .zip(y)
                .zip(weights)
                .map(|((a, b), w)| a * b * (w / total).sqrt())
                .sum::<f64>()
        })
        .collect();
    for j in (0..n).rev() {
        a[j] = (a[j] - ((j + 1)..n).map(|k| r[j][k] * a[k]).sum::<f64>()) / r[j][j];
    }
    // Horner composition p((x-center)/scale), returned as powers of x.
    let mut out = vec![a[degree]];
    for &value in a[..degree].iter().rev() {
        let mut next = vec![0.; out.len() + 1];
        for (i, c) in out.iter().enumerate() {
            next[i] -= c * center / scale;
            next[i + 1] += c / scale;
        }
        next[0] += value;
        out = next;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(kind: FitKind, f: impl Fn(f64) -> f64) -> Statistics2 {
        Statistics2 {
            fit_kind: kind,
            pairs: (1..=12)
                .map(|i| {
                    let x = i as f64 / 2.;
                    [x, f(x)]
                })
                .collect(),
        }
    }
    #[test]
    fn all_nine_fits_recover_independent_known_curves_and_predict_unseen_points() {
        type Curve = (FitKind, fn(f64) -> f64);
        let cases: [Curve; 9] = [
            (FitKind::Linear, |x| 2. * x - 3.),
            (FitKind::Logarithmic, |x| 3. * x.ln() + 2.),
            (FitKind::Exponential, |x| 4. * (0.3 * x).exp()),
            (FitKind::Power, |x| 2. * x.powf(1.5)),
            (FitKind::Exponent, |x| 3. * 1.2f64.powf(x)),
            (FitKind::Inverse, |x| 2. + 3. / x),
            (FitKind::Quadratic, |x| x * x - 3. * x + 2.),
            (FitKind::Cubic, |x| x * x * x - 2. * x * x + 3. * x - 1.),
            (FitKind::Quartic, |x| {
                x.powi(4) - 2. * x.powi(3) + 3. * x * x - x + 5.
            }),
        ];
        for (kind, f) in cases {
            let fit = model(kind, f).fit().unwrap();
            assert!(fit.residual_sum_squares < 1e-16, "{kind:?}: {fit:?}");
            for x in [0.75, 2.25, 5.75, 7.] {
                assert!(
                    (fit.predict(x).unwrap() - f(x)).abs() < 1e-8,
                    "{kind:?} at {x}"
                );
            }
        }
    }
    #[test]
    fn noisy_least_squares_matches_hand_computed_normal_equations() {
        let model = Statistics2 {
            pairs: vec![[0., 1.], [1., 2.], [2., 2.], [3., 4.]],
            ..Default::default()
        };
        let result = model.fit().unwrap();
        assert!((result.coefficients[0] - 0.9).abs() < 1e-12);
        assert!((result.coefficients[1] - 0.9).abs() < 1e-12);
        assert!((result.residual_sum_squares - 0.7).abs() < 1e-12);
    }
    #[test]
    fn invalid_domains_insufficient_points_and_rank_deficiency_are_errors() {
        for kind in FitKind::ALL {
            assert!(
                Statistics2 {
                    fit_kind: kind,
                    pairs: vec![[1., 2.]; 6]
                }
                .fit()
                .is_err()
            );
            assert!(
                Statistics2 {
                    fit_kind: kind,
                    pairs: vec![[1., 2.]]
                }
                .fit()
                .is_err()
            );
            assert!(
                Statistics2 {
                    fit_kind: kind,
                    pairs: vec![[1., 2.], [2., f64::INFINITY]]
                }
                .fit()
                .is_err()
            );
        }
        for kind in [FitKind::Logarithmic, FitKind::Power, FitKind::Inverse] {
            assert!(
                Statistics2 {
                    fit_kind: kind,
                    pairs: vec![[0., 2.], [2., 3.]]
                }
                .fit()
                .is_err()
            );
        }
        for kind in [FitKind::Exponential, FitKind::Exponent, FitKind::Power] {
            assert!(
                Statistics2 {
                    fit_kind: kind,
                    pairs: vec![[1., 0.], [2., 3.]]
                }
                .fit()
                .is_err()
            );
        }
        assert!(
            Statistics2 {
                fit_kind: FitKind::Quartic,
                pairs: vec![[1., 2.], [2., 3.], [3., 4.], [1., 2.], [2., 3.]]
            }
            .fit()
            .is_err()
        );
    }
    #[test]
    fn old_profiles_default_to_linear_and_selected_fit_round_trips() {
        let old: Statistics2 = serde_json::from_str(r#"{"pairs":[[1,2],[2,4]]}"#).unwrap();
        assert_eq!(old.fit_kind, FitKind::Linear);
        let next = model(FitKind::Power, |x| x * x);
        let restored: Statistics2 =
            serde_json::from_str(&serde_json::to_string(&next).unwrap()).unwrap();
        assert_eq!(restored.fit_kind, FitKind::Power);
        assert!((restored.fit().unwrap().predict(3.).unwrap() - 9.).abs() < 1e-10);
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    #[test]
    fn independent_original_firmware_predictions_and_residuals_match_native_fits() {
        for (index, kind) in FitKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.code(), index as i64 + 1);
            assert_eq!(FitKind::from_code(kind.code()), Some(kind));
        }
        assert!(FitKind::from_code(0).is_none());
        assert!(FitKind::from_code(14).is_none());
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-regression-controls.json"
        ))
        .unwrap();
        for case in controls["cases"].as_array().unwrap() {
            assert_eq!(
                ppl_engine::source::hash(case["source"].as_str().unwrap().as_bytes()),
                case["source_sha256"].as_str().unwrap()
            );
            let pairs: Vec<[f64; 2]> = case["pairs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| std::array::from_fn(|i| p[i].as_str().unwrap().parse().unwrap()))
                .collect();
            let frequencies: Vec<u64> =
                serde_json::from_value(case["frequencies"].clone()).unwrap();
            let model = Statistics2 {
                fit_kind: serde_json::from_value(case["kind"].clone()).unwrap(),
                pairs,
            };
            let fit = model
                .fit_with_options(&FitOptions {
                    frequencies: &frequencies,
                    logistic_limit: case["logistic_limit"].as_f64(),
                    user_expression: case["user_expression"].as_str(),
                    ..Default::default()
                })
                .unwrap();
            let query: f64 = case["query"].as_str().unwrap().parse().unwrap();
            let tolerance = case["absolute_tolerance"].as_f64().unwrap();
            if let Some(expected) = case["prediction"].as_str() {
                let expected: f64 = expected.parse().unwrap();
                assert!(
                    (fit.predict(query).unwrap() - expected).abs() < tolerance,
                    "{}: {fit:?}",
                    case["name"]
                );
            }
            for (point, residual) in model
                .pairs
                .iter()
                .zip(case["residuals"].as_array().unwrap())
            {
                let expected: f64 = residual.as_str().unwrap().parse().unwrap();
                assert!(
                    (point[1] - fit.predict(point[0]).unwrap() - expected).abs() < tolerance,
                    "{} at {point:?}",
                    case["name"]
                );
            }
        }
    }
    #[test]
    fn weighted_summary_matches_hand_computed_frequency_table() {
        let data = Statistics2 {
            pairs: vec![[0., 1.], [1., 2.], [2., 2.], [3., 4.]],
            ..Default::default()
        };
        let result = data.summary_with_frequencies(&[1, 2, 3, 4]).unwrap();
        assert_eq!(result.count, 10);
        assert!((result.x.mean - 2.).abs() < 1e-14);
        assert!((result.y.mean - 2.7).abs() < 1e-14);
        assert!((result.x.population_variance - 1.).abs() < 1e-14);
        assert!((result.y.population_variance - 1.21).abs() < 1e-14);
        assert!((result.sample_covariance.unwrap() - 10. / 9.).abs() < 1e-14);
        assert!((result.correlation.unwrap() - 10. / 11.).abs() < 1e-14);
        assert_eq!((result.x.median, result.y.median), (2., 2.));
        assert_eq!(result.sum_products, 64.);
        let fit = data
            .fit_with_options(&FitOptions {
                frequencies: &[1, 2, 3, 4],
                ..Default::default()
            })
            .unwrap();
        assert!((fit.coefficients[0] - 0.7).abs() < 1e-14);
        assert!((fit.coefficients[1] - 1.).abs() < 1e-14);
        assert!((fit.residual_sum_squares - 2.1).abs() < 1e-14);
    }
    #[test]
    fn fit_cancellation_and_unidentifiable_curves_are_errors() {
        let cancel = std::sync::atomic::AtomicBool::new(true);
        for kind in FitKind::ALL {
            let data = Statistics2 {
                fit_kind: kind,
                pairs: vec![[0., 1.], [1., 2.], [2., 4.]],
            };
            assert_eq!(
                data.fit_with_options(&FitOptions {
                    cancel: Some(&cancel),
                    ..Default::default()
                })
                .unwrap_err(),
                "Calculation cancelled"
            );
        }
        for kind in [FitKind::Logistic, FitKind::Trigonometric] {
            assert!(
                Statistics2 {
                    fit_kind: kind,
                    pairs: vec![[0., 2.], [1., 2.], [2., 2.], [3., 2.]]
                }
                .fit()
                .is_err()
            );
        }
    }
    #[test]
    fn weighted_linear_and_polynomial_match_expanded_observations_without_expansion() {
        for kind in [FitKind::Linear, FitKind::Quadratic, FitKind::Power] {
            let data = Statistics2 {
                fit_kind: kind,
                pairs: vec![[1., 2.], [2., 5.], [3., 8.], [4., 17.]],
            };
            let frequencies = [3, 2, 4, 1];
            let result = data
                .fit_with_options(&FitOptions {
                    frequencies: &frequencies,
                    ..Default::default()
                })
                .unwrap();
            let expanded = Statistics2 {
                fit_kind: kind,
                pairs: data
                    .pairs
                    .iter()
                    .zip(frequencies)
                    .flat_map(|(p, n)| std::iter::repeat_n(*p, n as usize))
                    .collect(),
            }
            .fit()
            .unwrap();
            assert_eq!(result.count, 10);
            for x in [0.5, 1.5, 3.5, 5.] {
                assert!((result.predict(x).unwrap() - expanded.predict(x).unwrap()).abs() < 1e-10);
            }
            assert!((result.residual_sum_squares - expanded.residual_sum_squares).abs() < 1e-10);
        }
    }
    #[test]
    fn logistic_recovers_ascending_descending_and_fixed_saturation_curves() {
        for b in [-1.2, 0.7] {
            let curve = |x: f64| 20. / (1. + 3. * (-b * x).exp());
            let data = Statistics2 {
                fit_kind: FitKind::Logistic,
                pairs: (-10..=20)
                    .map(|i| {
                        let x = i as f64 / 4.;
                        [x, curve(x)]
                    })
                    .collect(),
            };
            for limit in [None, Some(0.), Some(20.)] {
                let fit = data
                    .fit_with_options(&FitOptions {
                        logistic_limit: limit,
                        ..Default::default()
                    })
                    .unwrap();
                assert!(fit.residual_sum_squares < 1e-16, "{fit:?}");
                for x in [-2.7, 0.13, 2.23, 6.] {
                    assert!((fit.predict(x).unwrap() - curve(x)).abs() < 1e-8, "{fit:?}");
                }
            }
            assert!(
                data.fit_with_options(&FitOptions {
                    logistic_limit: Some(1.),
                    ..Default::default()
                })
                .is_err()
            );
        }
    }
    #[test]
    fn trigonometric_recovers_frequency_phase_offset_and_predicts_between_samples() {
        for frequency in [0.4, 1.7, 4.3] {
            let curve = |x: f64| 2.7 * (frequency * x + 0.63).sin() - 1.2;
            let data = Statistics2 {
                fit_kind: FitKind::Trigonometric,
                pairs: (0..45)
                    .map(|i| {
                        let x = i as f64 * 0.17;
                        [x, curve(x)]
                    })
                    .collect(),
            };
            let fit = data.fit().unwrap();
            assert!(fit.residual_sum_squares < 1e-16, "{fit:?}");
            for x in [0.031, 1.23, 3.59, 8.] {
                assert!((fit.predict(x).unwrap() - curve(x)).abs() < 1e-8, "{fit:?}");
            }
        }
    }
    #[test]
    fn nonlinear_weighted_noise_improves_over_known_generating_curve() {
        for (kind, curve) in [
            (
                FitKind::Logistic,
                (|x: f64| 10. / (1. + 2. * (-0.9 * x).exp())) as fn(f64) -> f64,
            ),
            (
                FitKind::Trigonometric,
                (|x: f64| 3. * (1.7 * x + 0.3).sin() + 2.) as fn(f64) -> f64,
            ),
        ] {
            let data = Statistics2 {
                fit_kind: kind,
                pairs: (0..36)
                    .map(|i| {
                        let x = i as f64 / 6.;
                        [x, curve(x) + (i as f64 * 2.3).sin() * 0.04]
                    })
                    .collect(),
            };
            let frequencies: Vec<_> = (0..36).map(|i| 1 + i % 5).collect();
            let fit = data
                .fit_with_options(&FitOptions {
                    frequencies: &frequencies,
                    ..Default::default()
                })
                .unwrap();
            let generating = data
                .pairs
                .iter()
                .zip(&frequencies)
                .map(|(p, w)| *w as f64 * (p[1] - curve(p[0])).powi(2))
                .sum::<f64>();
            assert!(
                fit.residual_sum_squares < generating,
                "{fit:?}, generating={generating}"
            );
        }
    }
    #[test]
    fn median_median_is_robust_to_middle_outlier_and_honors_frequencies() {
        let data = Statistics2 {
            fit_kind: FitKind::MedianMedian,
            pairs: vec![
                [0., 1.],
                [1., 3.],
                [2., 5.],
                [3., 7.],
                [4., 100.],
                [5., 11.],
                [6., 13.],
                [7., 15.],
                [8., 17.],
            ],
        };
        let fit = data.fit().unwrap();
        // Group medians (1,3), (4,11), (7,15): outer slope2;
        // intercept is the average of1,3,1, despite the large outlier.
        assert!((fit.coefficients[0] - 5. / 3.).abs() < 1e-12);
        assert!((fit.coefficients[1] - 2.).abs() < 1e-12);
        let frequencies = [1, 2, 1, 3, 0, 1, 1, 1, 2];
        let weighted = data
            .fit_with_options(&FitOptions {
                frequencies: &frequencies,
                ..Default::default()
            })
            .unwrap();
        let expanded = Statistics2 {
            fit_kind: FitKind::MedianMedian,
            pairs: data
                .pairs
                .iter()
                .zip(frequencies)
                .flat_map(|(p, n)| std::iter::repeat_n(*p, n as usize))
                .collect(),
        }
        .fit()
        .unwrap();
        assert_eq!(weighted.coefficients, expanded.coefficients);
    }
    #[test]
    fn user_curve_uses_real_compiler_angle_mode_and_survives_serialization() {
        let data = Statistics2 {
            fit_kind: FitKind::UserDefined,
            pairs: vec![[0., 2.], [30., 3.], [90., 4.]],
        };
        let fit = data
            .fit_with_options(&FitOptions {
                user_expression: Some("2*SIN(X)+2"),
                angle: ppl_math::AngleMode::Degrees,
                ..Default::default()
            })
            .unwrap();
        assert!(fit.residual_sum_squares < 1e-20);
        let saved = serde_json::to_string(&fit).unwrap();
        let restored: Regression = serde_json::from_str(&saved).unwrap();
        assert!((restored.predict(45.).unwrap() - 3.41421356237).abs() < 1e-10);
        for expression in [
            "Y+X",
            "X:=2",
            "RANDOM()",
            "WAIT(0)",
            "X; END; EXPORT INJECT() BEGIN RETURN 1",
        ] {
            assert!(
                data.fit_with_options(&FitOptions {
                    user_expression: Some(expression),
                    ..Default::default()
                })
                .is_err(),
                "{expression}"
            );
        }
    }
    #[test]
    fn zero_frequency_excludes_invalid_domain_and_totals_do_not_wrap() {
        let data = Statistics2 {
            fit_kind: FitKind::Logarithmic,
            pairs: vec![[-1., f64::NAN], [1., 2.], [2., 3.]],
        };
        assert!(
            data.fit_with_options(&FitOptions {
                frequencies: &[0, 1, 1],
                ..Default::default()
            })
            .is_ok()
        );
        for frequencies in [&[0, 0, 0][..], &[1, 2][..], &[u64::MAX, 1, 1][..]] {
            assert!(
                data.fit_with_options(&FitOptions {
                    frequencies,
                    ..Default::default()
                })
                .is_err()
            );
        }
    }
}
