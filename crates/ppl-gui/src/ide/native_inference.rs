//! Native statistical inference, independent of GUI state and external CAS.
//! The HP Prime Inference app's documented methods are mean/proportion Z/t
//! tests and intervals (HP Prime User Guide 813269-002, chapter13).
//! Chi-square, variance-ratio F, ANOVA and regression inference are separate
//! Native mathematical extensions, not additional documented HP menu entries.
//! Their distributions use regularized beta/gamma functions (NIST DLMF8.17,8.2).
pub use super::native_app_models::Alternative;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};
const ITERATIONS: usize = 10_000;
const EPS: f64 = 8. * f64::EPSILON;
const TINY: f64 = 1e-300;
const MAX_COUNT: u64 = 9_007_199_254_740_992;
fn finite(values: impl IntoIterator<Item = f64>) -> Result<(), String> {
    if values.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err("Inference inputs and results must be finite".into())
    }
}
fn probability(p: f64) -> Result<(), String> {
    if p.is_finite() && p > 0. && p < 1. {
        Ok(())
    } else {
        Err("Significance/confidence must be strictly between 0 and 1".into())
    }
}
fn count(n: u64, min: u64) -> Result<(), String> {
    if (min..=MAX_COUNT).contains(&n) {
        Ok(())
    } else {
        Err(format!(
            "Sample count must be between {min} and {MAX_COUNT}"
        ))
    }
}
fn positive(x: f64) -> Result<(), String> {
    if x.is_finite() && x > 0. {
        Ok(())
    } else {
        Err("Standard deviations and degrees of freedom must be positive and finite".into())
    }
}
fn cancel(flag: &AtomicBool) -> Result<(), String> {
    if flag.load(Ordering::Acquire) {
        Err("Inference cancelled".into())
    } else {
        Ok(())
    }
}
fn nonzero(x: f64) -> f64 {
    if x.abs() < TINY { TINY.copysign(x) } else { x }
}
fn log_gamma(z: f64) -> f64 {
    const COEFF: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if z < 0.5 {
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * z).sin().ln()
            - log_gamma(1. - z);
    }
    let z = z - 1.;
    let mut x = COEFF[0];
    for (i, c) in COEFF.iter().enumerate().skip(1) {
        x += c / (z + i as f64);
    }
    let t = z + 7.5;
    0.5 * std::f64::consts::TAU.ln() + (z + 0.5) * t.ln() - t + x.ln()
}
fn stirling_correction(x: f64) -> f64 {
    let r = 1. / x;
    let q = r * r;
    r * (1. / 12.
        + q * (-1. / 360.
            + q * (1. / 1260.
                + q * (-1. / 1680. + q * (1. / 1188. + q * (-691. / 360360. + q / 156.))))))
}
fn log_beta(a: f64, b: f64) -> f64 {
    if a >= 8. && b >= 8. {
        return 0.5 * std::f64::consts::TAU.ln()
            - (a - 0.5) * (b / a).ln_1p()
            - (b - 0.5) * (a / b).ln_1p()
            - 0.5 * (a + b).ln()
            + stirling_correction(a)
            + stirling_correction(b)
            - stirling_correction(a + b);
    }
    let (large, small) = if a > b { (a, b) } else { (b, a) };
    if large >= 8. {
        let ratio = small * large.ln() + (large + small - 0.5) * (small / large).ln_1p() - small
            + stirling_correction(large + small)
            - stirling_correction(large);
        log_gamma(small) - ratio
    } else {
        log_gamma(a) + log_gamma(b) - log_gamma(a + b)
    }
}
fn beta_fraction(a: f64, b: f64, x: f64) -> Result<f64, String> {
    let qab = a + b;
    let qap = a + 1.;
    let qam = a - 1.;
    let mut c = 1.;
    let mut d = 1. / nonzero(1. - qab * x / qap);
    let mut h = d;
    for m in 1..=ITERATIONS {
        let mf = m as f64;
        let m2 = 2. * mf;
        let aa = mf * (b - mf) * x / ((qam + m2) * (a + m2));
        d = 1. / nonzero(1. + aa * d);
        c = nonzero(1. + aa / c);
        h *= d * c;
        let aa = -(a + mf) * (qab + mf) * x / ((a + m2) * (qap + m2));
        d = 1. / nonzero(1. + aa * d);
        c = nonzero(1. + aa / c);
        let delta = d * c;
        h *= delta;
        if !h.is_finite() {
            return Err("Beta continued fraction overflow".into());
        }
        if (delta - 1.).abs() <= EPS {
            return Ok(h);
        }
    }
    Err("Beta continued fraction did not converge".into())
}
fn beta_i(a: f64, b: f64, x: f64) -> Result<f64, String> {
    positive(a)?;
    positive(b)?;
    if !x.is_finite() || !(0.0..=1.).contains(&x) {
        return Err("Invalid beta argument".into());
    }
    if x == 0. {
        return Ok(0.);
    }
    if x == 1. {
        return Ok(1.);
    }
    if a == b && x == 0.5 {
        return Ok(0.5);
    }
    let front = (a * x.ln() + b * (-x).ln_1p() - log_beta(a, b)).exp();
    let result = if x < (a + 1.) / (a + b + 2.) {
        front * beta_fraction(a, b, x)? / a
    } else {
        1. - front * beta_fraction(b, a, 1. - x)? / b
    };
    finite([result])?;
    Ok(result.clamp(0., 1.))
}
fn gamma_q(a: f64, x: f64) -> Result<f64, String> {
    positive(a)?;
    if !x.is_finite() || x < 0. {
        return Err("Invalid gamma argument".into());
    }
    if x == 0. {
        return Ok(1.);
    }
    let factor = (-x + a * x.ln() - log_gamma(a)).exp();
    if x < a + 1. {
        let mut ap = a;
        let mut term = 1. / a;
        let mut sum = term;
        for _ in 0..ITERATIONS {
            ap += 1.;
            term *= x / ap;
            sum += term;
            if term.abs() <= sum.abs() * EPS {
                let p = sum * factor;
                finite([p])?;
                return Ok((1. - p).clamp(0., 1.));
            }
        }
    } else {
        let mut b = x + 1. - a;
        let mut c = 1. / TINY;
        let mut d = 1. / nonzero(b);
        let mut h = d;
        for i in 1..=ITERATIONS {
            let f = i as f64;
            let an = -f * (f - a);
            b += 2.;
            d = 1. / nonzero(an * d + b);
            c = nonzero(b + an / c);
            let delta = d * c;
            h *= delta;
            if (delta - 1.).abs() <= EPS {
                let q = factor * h;
                finite([q])?;
                return Ok(q.clamp(0., 1.));
            }
        }
    }
    Err("Gamma evaluation did not converge".into())
}
fn gamma_p(a: f64, x: f64) -> Result<f64, String> {
    positive(a)?;
    if !x.is_finite() || x < 0. {
        return Err("Invalid gamma argument".into());
    }
    if x == 0. {
        return Ok(0.);
    }
    if x >= a + 1. {
        return gamma_q(a, x).map(|q| 1. - q);
    }
    let factor = (-x + a * x.ln() - log_gamma(a)).exp();
    let mut ap = a;
    let mut term = 1. / a;
    let mut sum = term;
    for _ in 0..ITERATIONS {
        ap += 1.;
        term *= x / ap;
        sum += term;
        if term.abs() <= sum.abs() * EPS {
            let p = sum * factor;
            finite([p])?;
            return Ok(p.clamp(0., 1.));
        }
    }
    Err("Gamma evaluation did not converge".into())
}

/// log(1+x)-x, retaining the quadratic term near zero.
fn log1p_minus(x: f64) -> f64 {
    if x.abs() >= 1e-4 {
        return x.ln_1p() - x;
    }
    let mut power = x * x;
    let mut sum = 0.;
    for n in 2..=12 {
        sum += if n % 2 == 0 {
            -power / n as f64
        } else {
            power / n as f64
        };
        power *= x;
    }
    sum
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "distribution", rename_all = "snake_case")]
pub enum Distribution {
    Normal,
    Student {
        df: f64,
    },
    ChiSquare {
        df: f64,
    },
    F {
        numerator_df: f64,
        denominator_df: f64,
    },
}
impl Distribution {
    fn validate(self) -> Result<(), String> {
        match self {
            Self::Normal => Ok(()),
            Self::Student { df } | Self::ChiSquare { df } => positive(df),
            Self::F {
                numerator_df,
                denominator_df,
            } => {
                positive(numerator_df)?;
                positive(denominator_df)
            }
        }
    }
    /// Reference densities from NIST/SEMATECH e-Handbook 1.3.6.6:
    /// https://www.itl.nist.gov/div898/handbook/eda/section3/eda3664.htm
    /// https://www.itl.nist.gov/div898/handbook/eda/section3/eda3665.htm
    /// https://www.itl.nist.gov/div898/handbook/eda/section3/eda3666.htm
    /// The value at a singular zero boundary is explicitly undefined rather
    /// than replacing an infinite density with a fabricated finite point.
    pub fn density(self, x: f64) -> Result<f64, String> {
        self.validate()?;
        finite([x])?;
        let log = match self {
            Self::Normal => -0.5 * x * x - 0.5 * std::f64::consts::TAU.ln(),
            Self::Student { df } => {
                let log_ratio = if x == 0. {
                    f64::NEG_INFINITY
                } else {
                    2. * x.abs().ln() - df.ln()
                };
                let log_scale = if log_ratio > 0. {
                    log_ratio + (-log_ratio).exp().ln_1p()
                } else {
                    log_ratio.exp().ln_1p()
                };
                -0.5 * df.ln() - log_beta(df / 2., 0.5) - (df + 1.) / 2. * log_scale
            }
            Self::ChiSquare { df } => {
                if x < 0. {
                    return Ok(0.);
                }
                let a = df / 2.;
                if x == 0. {
                    return if df > 2. {
                        Ok(0.)
                    } else if df == 2. {
                        Ok(0.5)
                    } else {
                        Err("Chi-square density is unbounded at zero".into())
                    };
                }
                if a >= 8. {
                    let delta = (x / 2. - a) / a;
                    let log_ratio = x.ln() - 2f64.ln() - a.ln();
                    -0.5 * (std::f64::consts::TAU.ln() + a.ln())
                        - 2f64.ln()
                        - stirling_correction(a)
                        + a * log1p_minus(delta)
                        - log_ratio
                } else {
                    (a - 1.) * x.ln() - x / 2. - a * 2f64.ln() - log_gamma(a)
                }
            }
            Self::F {
                numerator_df,
                denominator_df,
            } => {
                if x < 0. {
                    return Ok(0.);
                }
                if x == 0. {
                    return if numerator_df > 2. {
                        Ok(0.)
                    } else if numerator_df == 2. {
                        Ok(1.)
                    } else {
                        Err("F density is unbounded at zero".into())
                    };
                }
                let (a, b) = (numerator_df / 2., denominator_df / 2.);
                if a >= 8. && b >= 8. {
                    // Beta deviance about its mean. First-order terms cancel
                    // analytically, so large degrees of freedom never subtract
                    // two log-gamma values of order n*log(n).
                    let p = a / (a + b);
                    let q = b / (a + b);
                    let d = q + p * x;
                    let first = q * (x - 1.) / d;
                    let second = -p * (x - 1.) / d;
                    let left = if first.abs() < 0.5 {
                        log1p_minus(first)
                    } else {
                        (x / d).ln() - first
                    };
                    let right = if second.abs() < 0.5 {
                        log1p_minus(second)
                    } else {
                        -d.ln() - second
                    };
                    0.5 * (a.ln() + b.ln() - (a + b).ln() - std::f64::consts::TAU.ln())
                        - stirling_correction(a)
                        - stirling_correction(b)
                        + stirling_correction(a + b)
                        - x.ln()
                        + a * left
                        + b * right
                } else {
                    let u = a.ln() - b.ln() + x.ln();
                    let kernel = if u > 0. {
                        -b * u - (a + b) * (-u).exp().ln_1p()
                    } else {
                        a * u - (a + b) * u.exp().ln_1p()
                    };
                    kernel - x.ln() - log_beta(a, b)
                }
            }
        };
        let density = log.exp();
        finite([density])?;
        Ok(density)
    }
    pub fn upper_tail(self, x: f64) -> Result<f64, String> {
        self.validate()?;
        finite([x])?;
        match self {
            Self::Normal => {
                let q = if x.abs() > 40. {
                    0.
                } else {
                    0.5 * gamma_q(0.5, x * x / 2.)?
                };
                Ok(if x >= 0. { q } else { 1. - q })
            }
            Self::Student { df } => {
                if x == 0. {
                    return Ok(0.5);
                }
                let ratio = if x.abs() > f64::MAX.sqrt() {
                    0.
                } else {
                    df / (df + x * x)
                };
                let q = 0.5 * beta_i(df / 2., 0.5, ratio)?;
                Ok(if x > 0. { q } else { 1. - q })
            }
            Self::ChiSquare { df } => {
                if x <= 0. {
                    Ok(1.)
                } else {
                    gamma_q(df / 2., x / 2.)
                }
            }
            Self::F {
                numerator_df,
                denominator_df,
            } => {
                if x <= 0. {
                    Ok(1.)
                } else {
                    let ratio = 1. / (1. + numerator_df / denominator_df * x);
                    beta_i(denominator_df / 2., numerator_df / 2., ratio)
                }
            }
        }
    }
    pub fn lower_tail(self, x: f64) -> Result<f64, String> {
        self.validate()?;
        finite([x])?;
        match self {
            Self::Normal | Self::Student { .. } => self.upper_tail(-x),
            Self::ChiSquare { df } => {
                if x <= 0. {
                    Ok(0.)
                } else {
                    gamma_p(df / 2., x / 2.)
                }
            }
            Self::F {
                numerator_df,
                denominator_df,
            } => {
                if x <= 0. {
                    return Ok(0.);
                }
                let y = numerator_df / denominator_df * x;
                if y.is_infinite() {
                    Ok(1.)
                } else {
                    beta_i(numerator_df / 2., denominator_df / 2., y / (1. + y))
                }
            }
        }
    }
    pub fn inverse_upper_tail(self, p: f64) -> Result<f64, String> {
        self.validate()?;
        probability(p)?;
        if matches!(self, Self::Normal | Self::Student { .. }) {
            if p == 0.5 {
                return Ok(0.);
            }
            if p > 0.5 {
                return self.inverse_upper_tail(1. - p).map(|q| -q);
            }
        }
        let mut lo = 0.;
        let mut hi = 1.;
        let mut bracketed = false;
        for _ in 0..512 {
            if if p > 0.5 {
                self.lower_tail(hi)? >= 1. - p
            } else {
                self.upper_tail(hi)? <= p
            } {
                bracketed = true;
                break;
            }
            hi *= 2.;
            if !hi.is_finite() {
                break;
            }
        }
        if !bracketed {
            return Err("Distribution quantile exceeds numerical range".into());
        }
        for _ in 0..1200 {
            let mid = lo + (hi - lo) / 2.;
            if mid == lo || mid == hi {
                break;
            }
            if if p > 0.5 {
                self.lower_tail(mid)? < 1. - p
            } else {
                self.upper_tail(mid)? > p
            } {
                lo = mid;
            } else {
                hi = mid;
            }
            if hi - lo <= EPS * hi.abs().max(f64::MIN_POSITIVE) {
                break;
            }
        }
        Ok((lo + hi) / 2.)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanSummary {
    pub mean: f64,
    pub sd: f64,
    pub count: u64,
}
impl MeanSummary {
    fn validate(&self, minimum: u64) -> Result<(), String> {
        finite([self.mean])?;
        positive(self.sd)?;
        count(self.count, minimum)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProportionSample {
    pub successes: u64,
    pub total: u64,
}
impl ProportionSample {
    fn value(&self) -> Result<f64, String> {
        count(self.total, 1)?;
        if self.successes > self.total {
            return Err("Successes cannot exceed sample size".into());
        }
        Ok(self.successes as f64 / self.total as f64)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum InferenceMode {
    Test {
        null_value: f64,
        alpha: f64,
        alternative: Alternative,
    },
    Interval {
        confidence: f64,
    },
}
impl Default for InferenceMode {
    fn default() -> Self {
        Self::Test {
            null_value: 0.,
            alpha: 0.05,
            alternative: Alternative::TwoSided,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum RegressionTarget {
    Slope,
    Intercept,
    MeanResponse { x: f64 },
    Prediction { x: f64 },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InferenceRequest {
    MeanZ {
        mean: f64,
        sigma: f64,
        count: u64,
        mode: InferenceMode,
    },
    TwoMeansZ {
        first: MeanSummary,
        second: MeanSummary,
        mode: InferenceMode,
    },
    MeanT {
        sample: MeanSummary,
        mode: InferenceMode,
    },
    TwoMeansT {
        first: MeanSummary,
        second: MeanSummary,
        pooled: bool,
        mode: InferenceMode,
    },
    OneProportion {
        sample: ProportionSample,
        mode: InferenceMode,
    },
    TwoProportions {
        first: ProportionSample,
        second: ProportionSample,
        mode: InferenceMode,
    },
    VarianceRatio {
        sd1: f64,
        n1: u64,
        sd2: f64,
        n2: u64,
        mode: InferenceMode,
    },
    GoodnessOfFit {
        observed: Vec<f64>,
        expected: Vec<f64>,
        estimated_parameters: usize,
        alpha: f64,
    },
    Contingency {
        table: Vec<Vec<f64>>,
        alpha: f64,
    },
    Anova {
        groups: Vec<Vec<f64>>,
        alpha: f64,
    },
    Regression {
        pairs: Vec<[f64; 2]>,
        target: RegressionTarget,
        mode: InferenceMode,
    },
}
impl Default for InferenceRequest {
    fn default() -> Self {
        Self::MeanZ {
            mean: 10.,
            sigma: 2.,
            count: 100,
            mode: InferenceMode::Test {
                null_value: 9.6,
                alpha: 0.05,
                alternative: Alternative::TwoSided,
            },
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceResult {
    pub method: String,
    pub estimate: Option<f64>,
    pub statistic: Option<f64>,
    pub standard_error: Option<f64>,
    pub p_value: Option<f64>,
    pub upper_tail_probability: Option<f64>,
    pub interval: Option<[f64; 2]>,
    pub critical_values: Vec<f64>,
    pub critical_estimates: Vec<f64>,
    pub degrees_of_freedom: Vec<f64>,
    pub reject_null: Option<bool>,
    pub details: BTreeMap<String, f64>,
}
impl InferenceResult {
    fn validate(&self) -> Result<(), String> {
        finite(
            [
                self.estimate,
                self.statistic,
                self.standard_error,
                self.p_value,
                self.upper_tail_probability,
            ]
            .into_iter()
            .flatten()
            .chain(self.interval.into_iter().flatten())
            .chain(self.critical_values.iter().copied())
            .chain(self.critical_estimates.iter().copied())
            .chain(self.degrees_of_freedom.iter().copied())
            .chain(self.details.values().copied()),
        )?;
        if self.p_value.is_some_and(|p| !(0.0..=1.).contains(&p)) {
            return Err("Computed probability is outside [0,1]".into());
        }
        Ok(())
    }
}
fn selected_p(
    distribution: Distribution,
    statistic: f64,
    alternative: Alternative,
) -> Result<f64, String> {
    let upper = distribution.upper_tail(statistic)?;
    let lower = distribution.lower_tail(statistic)?;
    let p = match alternative {
        Alternative::Greater => upper,
        Alternative::Less => lower,
        Alternative::TwoSided => 2. * upper.min(lower),
    };
    Ok(p.clamp(0., 1.))
}

fn location(
    method: &str,
    estimate: f64,
    se: f64,
    distribution: Distribution,
    mode: &InferenceMode,
    df: Vec<f64>,
) -> Result<InferenceResult, String> {
    finite([estimate, se])?;
    if se < 0. {
        return Err("Standard error cannot be negative".into());
    }
    let mut result = InferenceResult {
        method: method.into(),
        estimate: Some(estimate),
        standard_error: Some(se),
        degrees_of_freedom: df,
        ..Default::default()
    };
    match *mode {
        InferenceMode::Test {
            null_value,
            alpha,
            alternative,
        } => {
            finite([null_value])?;
            probability(alpha)?;
            if se == 0. {
                return Err(
                    "Test is undefined because the estimated standard error is zero".into(),
                );
            }
            let statistic = (estimate - null_value) / se;
            finite([statistic])?;
            let upper = distribution.upper_tail(statistic)?;
            let p = selected_p(distribution, statistic, alternative)?;
            let critical = match alternative {
                Alternative::TwoSided => {
                    let q = distribution.inverse_upper_tail(alpha / 2.)?;
                    vec![-q, q]
                }
                Alternative::Greater => vec![distribution.inverse_upper_tail(alpha)?],
                Alternative::Less => vec![-distribution.inverse_upper_tail(alpha)?],
            };
            result.statistic = Some(statistic);
            result.p_value = Some(p);
            result.upper_tail_probability = Some(upper);
            result.reject_null = Some(p <= alpha);
            result.critical_estimates = critical.iter().map(|q| null_value + q * se).collect();
            result.critical_values = critical;
        }
        InferenceMode::Interval { confidence } => {
            probability(confidence)?;
            let q = distribution.inverse_upper_tail((1. - confidence) / 2.)?;
            let interval = [estimate - q * se, estimate + q * se];
            finite(interval)?;
            result.interval = Some(interval);
            result.critical_values = vec![-q, q];
        }
    }
    Ok(result)
}
fn upper_test(
    method: &str,
    statistic: f64,
    distribution: Distribution,
    alpha: f64,
    df: Vec<f64>,
) -> Result<InferenceResult, String> {
    probability(alpha)?;
    finite([statistic])?;
    let p = distribution.upper_tail(statistic)?;
    Ok(InferenceResult {
        method: method.into(),
        statistic: Some(statistic),
        p_value: Some(p),
        upper_tail_probability: Some(p),
        critical_values: vec![distribution.inverse_upper_tail(alpha)?],
        degrees_of_freedom: df,
        reject_null: Some(p <= alpha),
        ..Default::default()
    })
}
impl InferenceRequest {
    pub fn calculate(&self) -> Result<InferenceResult, String> {
        self.calculate_with_cancel(&AtomicBool::new(false))
    }
    pub fn calculate_with_cancel(&self, flag: &AtomicBool) -> Result<InferenceResult, String> {
        cancel(flag)?;
        let result = match self {
            Self::MeanZ {
                mean,
                sigma,
                count: n,
                mode,
            } => {
                finite([*mean])?;
                positive(*sigma)?;
                count(*n, 1)?;
                location(
                    "One-mean Z",
                    *mean,
                    *sigma / (*n as f64).sqrt(),
                    Distribution::Normal,
                    mode,
                    vec![],
                )
            }
            Self::TwoMeansZ {
                first,
                second,
                mode,
            } => {
                first.validate(1)?;
                second.validate(1)?;
                let se = (first.sd.powi(2) / first.count as f64
                    + second.sd.powi(2) / second.count as f64)
                    .sqrt();
                location(
                    "Two-means Z",
                    first.mean - second.mean,
                    se,
                    Distribution::Normal,
                    mode,
                    vec![],
                )
            }
            Self::MeanT { sample, mode } => {
                sample.validate(2)?;
                let df = (sample.count - 1) as f64;
                location(
                    "One-mean t",
                    sample.mean,
                    sample.sd / (sample.count as f64).sqrt(),
                    Distribution::Student { df },
                    mode,
                    vec![df],
                )
            }
            Self::TwoMeansT {
                first,
                second,
                pooled,
                mode,
            } => {
                first.validate(2)?;
                second.validate(2)?;
                let (n1, n2) = (first.count as f64, second.count as f64);
                let (v1, v2) = (first.sd.powi(2), second.sd.powi(2));
                let (se, df) = if *pooled {
                    let df = n1 + n2 - 2.;
                    let variance = ((n1 - 1.) * v1 + (n2 - 1.) * v2) / df;
                    ((variance * (1. / n1 + 1. / n2)).sqrt(), df)
                } else {
                    let (a, b) = (v1 / n1, v2 / n2);
                    (
                        (a + b).sqrt(),
                        (a + b).powi(2) / (a * a / (n1 - 1.) + b * b / (n2 - 1.)),
                    )
                };
                location(
                    if *pooled {
                        "Two-means pooled t"
                    } else {
                        "Two-means Welch t"
                    },
                    first.mean - second.mean,
                    se,
                    Distribution::Student { df },
                    mode,
                    vec![df],
                )
            }
            Self::OneProportion { sample, mode } => {
                let p = sample.value()?;
                let variance = if let InferenceMode::Test { null_value, .. } = mode {
                    if !null_value.is_finite() || *null_value <= 0. || *null_value >= 1. {
                        return Err("Null proportion must be strictly between 0 and 1".into());
                    }
                    null_value * (1. - null_value)
                } else {
                    p * (1. - p)
                };
                location(
                    "One-proportion Z",
                    p,
                    (variance / sample.total as f64).sqrt(),
                    Distribution::Normal,
                    mode,
                    vec![],
                )
            }
            Self::TwoProportions {
                first,
                second,
                mode,
            } => {
                let (p1, p2) = (first.value()?, second.value()?);
                let (n1, n2) = (first.total as f64, second.total as f64);
                let variance = if let InferenceMode::Test { null_value, .. } = mode {
                    if *null_value != 0. {
                        return Err("Two-proportion Z test uses the documented equal-proportions null (difference 0)".into());
                    }
                    let p = (first.successes as f64 + second.successes as f64) / (n1 + n2);
                    p * (1. - p) * (1. / n1 + 1. / n2)
                } else {
                    p1 * (1. - p1) / n1 + p2 * (1. - p2) / n2
                };
                location(
                    "Two-proportions Z",
                    p1 - p2,
                    variance.sqrt(),
                    Distribution::Normal,
                    mode,
                    vec![],
                )
            }
            Self::VarianceRatio {
                sd1,
                n1,
                sd2,
                n2,
                mode,
            } => {
                positive(*sd1)?;
                positive(*sd2)?;
                count(*n1, 2)?;
                count(*n2, 2)?;
                let df = [(*n1 - 1) as f64, (*n2 - 1) as f64];
                let distribution = Distribution::F {
                    numerator_df: df[0],
                    denominator_df: df[1],
                };
                let ratio = (*sd1 / *sd2).powi(2);
                finite([ratio])?;
                let mut result = InferenceResult {
                    method: "F variance ratio".into(),
                    estimate: Some(ratio),
                    degrees_of_freedom: df.to_vec(),
                    ..Default::default()
                };
                match *mode {
                    InferenceMode::Interval { confidence } => {
                        probability(confidence)?;
                        let tail = (1. - confidence) / 2.;
                        result.interval = Some([
                            ratio / distribution.inverse_upper_tail(tail)?,
                            ratio / distribution.inverse_upper_tail(1. - tail)?,
                        ]);
                    }
                    InferenceMode::Test {
                        null_value,
                        alpha,
                        alternative,
                    } => {
                        positive(null_value)?;
                        probability(alpha)?;
                        let statistic = ratio / null_value;
                        finite([statistic])?;
                        let upper = distribution.upper_tail(statistic)?;
                        let p = selected_p(distribution, statistic, alternative)?;
                        result.statistic = Some(statistic);
                        result.p_value = Some(p);
                        result.upper_tail_probability = Some(upper);
                        result.reject_null = Some(p <= alpha);
                        result.critical_values = match alternative {
                            Alternative::TwoSided => vec![
                                distribution.inverse_upper_tail(1. - alpha / 2.)?,
                                distribution.inverse_upper_tail(alpha / 2.)?,
                            ],
                            Alternative::Less => vec![distribution.inverse_upper_tail(1. - alpha)?],
                            Alternative::Greater => vec![distribution.inverse_upper_tail(alpha)?],
                        };
                        result.critical_estimates = result
                            .critical_values
                            .iter()
                            .map(|v| v * null_value)
                            .collect();
                    }
                }
                Ok(result)
            }
            Self::GoodnessOfFit {
                observed,
                expected,
                estimated_parameters,
                alpha,
            } => {
                if observed.len() != expected.len()
                    || observed.len() < 2
                    || *estimated_parameters >= observed.len() - 1
                {
                    return Err("Goodness-of-fit needs matching categories and positive residual degrees of freedom".into());
                }
                validate_counts(observed)?;
                finite(expected.iter().copied())?;
                if expected.iter().any(|v| *v <= 0.) {
                    return Err("Every expected count must be positive".into());
                }
                let total = observed.iter().sum::<f64>();
                if total == 0. || (total - expected.iter().sum::<f64>()).abs() > 1e-10 * total {
                    return Err("Observed and expected totals must match and be positive".into());
                }
                let df = (observed.len() - 1 - estimated_parameters) as f64;
                let statistic = observed
                    .iter()
                    .zip(expected)
                    .map(|(o, e)| (o - e).powi(2) / e)
                    .sum();
                upper_test(
                    "Chi-square goodness of fit",
                    statistic,
                    Distribution::ChiSquare { df },
                    *alpha,
                    vec![df],
                )
            }
            Self::Contingency { table, alpha } => contingency(table, *alpha, flag),
            Self::Anova { groups, alpha } => anova(groups, *alpha, flag),
            Self::Regression {
                pairs,
                target,
                mode,
            } => regression(pairs, target, mode, flag),
        }?;
        result.validate()?;
        Ok(result)
    }
}
fn validate_counts(values: &[f64]) -> Result<(), String> {
    finite(values.iter().copied())?;
    if values
        .iter()
        .any(|v| *v < 0. || v.fract() != 0. || *v > MAX_COUNT as f64)
    {
        return Err("Observed counts must be non-negative exact integers".into());
    }
    Ok(())
}
fn contingency(
    table: &[Vec<f64>],
    alpha: f64,
    flag: &AtomicBool,
) -> Result<InferenceResult, String> {
    let rows = table.len();
    let cols = table.first().map_or(0, Vec::len);
    if rows < 2 || cols < 2 || table.iter().any(|r| r.len() != cols) {
        return Err(
            "Contingency table must be rectangular with at least two rows and columns".into(),
        );
    }
    let mut row_totals = vec![0.; rows];
    let mut col_totals = vec![0.; cols];
    for (i, row) in table.iter().enumerate() {
        cancel(flag)?;
        validate_counts(row)?;
        for (j, value) in row.iter().enumerate() {
            row_totals[i] += value;
            col_totals[j] += value;
        }
    }
    if row_totals.iter().chain(&col_totals).any(|v| *v <= 0.) {
        return Err("Every contingency row and column needs a positive total".into());
    }
    let n = row_totals.iter().sum::<f64>();
    finite([n])?;
    let mut statistic = 0.;
    for (i, row) in table.iter().enumerate() {
        cancel(flag)?;
        for (j, observed) in row.iter().enumerate() {
            let expected = row_totals[i] / n * col_totals[j];
            statistic += (observed - expected).powi(2) / expected;
        }
    }
    let df = ((rows - 1) * (cols - 1)) as f64;
    let mut result = upper_test(
        "Chi-square contingency",
        statistic,
        Distribution::ChiSquare { df },
        alpha,
        vec![df],
    )?;
    result.details.insert("count".into(), n);
    Ok(result)
}
fn moments(values: &[f64], flag: &AtomicBool) -> Result<(f64, f64), String> {
    if values.is_empty() {
        return Err("Each sample needs at least one observation".into());
    }
    finite(values.iter().copied())?;
    let mut mean = 0.;
    let mut m2 = 0.;
    for (i, value) in values.iter().enumerate() {
        if i % 1024 == 0 {
            cancel(flag)?;
        }
        let delta = value - mean;
        mean += delta / (i + 1) as f64;
        m2 += delta * (value - mean);
    }
    finite([mean, m2])?;
    Ok((mean, m2.max(0.)))
}
fn anova(groups: &[Vec<f64>], alpha: f64, flag: &AtomicBool) -> Result<InferenceResult, String> {
    if groups.len() < 2 {
        return Err("ANOVA needs at least two groups".into());
    }
    let mut stats = Vec::new();
    let mut n = 0usize;
    let mut grand = 0.;
    for group in groups {
        cancel(flag)?;
        let (mean, ss) = moments(group, flag)?;
        n = n
            .checked_add(group.len())
            .ok_or("ANOVA sample size overflow")?;
        grand += mean * group.len() as f64;
        stats.push((group.len(), mean, ss));
    }
    if n <= groups.len() {
        return Err("ANOVA needs positive within-group degrees of freedom".into());
    }
    grand /= n as f64;
    let between = stats
        .iter()
        .map(|(count, mean, _)| *count as f64 * (mean - grand).powi(2))
        .sum::<f64>();
    let within = stats.iter().map(|(_, _, ss)| ss).sum::<f64>();
    let df1 = (groups.len() - 1) as f64;
    let df2 = (n - groups.len()) as f64;
    if within == 0. {
        return Err(
            "ANOVA F statistic is not finite when every group has zero internal variance".into(),
        );
    }
    let statistic = (between / df1) / (within / df2);
    let mut result = upper_test(
        "One-way ANOVA",
        statistic,
        Distribution::F {
            numerator_df: df1,
            denominator_df: df2,
        },
        alpha,
        vec![df1, df2],
    )?;
    result.details = BTreeMap::from([
        ("between_sum_squares".into(), between),
        ("within_sum_squares".into(), within),
        ("total_sum_squares".into(), between + within),
        ("grand_mean".into(), grand),
    ]);
    Ok(result)
}
fn regression(
    pairs: &[[f64; 2]],
    target: &RegressionTarget,
    mode: &InferenceMode,
    flag: &AtomicBool,
) -> Result<InferenceResult, String> {
    if pairs.len() < 3 {
        return Err("Regression inference needs at least three paired observations".into());
    }
    finite(pairs.iter().flatten().copied())?;
    let mut x = Vec::with_capacity(pairs.len());
    let mut y = Vec::with_capacity(pairs.len());
    for (i, p) in pairs.iter().enumerate() {
        if i % 1024 == 0 {
            cancel(flag)?;
        }
        x.push(p[0]);
        y.push(p[1]);
    }
    let (mx, sxx) = moments(&x, flag)?;
    let (my, syy) = moments(&y, flag)?;
    if sxx == 0. {
        return Err("Regression needs distinct x values".into());
    }
    let sxy = pairs.iter().map(|p| (p[0] - mx) * (p[1] - my)).sum::<f64>();
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let sse = pairs
        .iter()
        .map(|p| (p[1] - (intercept + slope * p[0])).powi(2))
        .sum::<f64>();
    let n = pairs.len() as f64;
    let df = n - 2.;
    let variance = sse / df;
    if matches!(mode, InferenceMode::Test { .. }) && !matches!(target, RegressionTarget::Slope) {
        return Err("The documented linear regression test tests the slope; choose an interval for other targets".into());
    }
    let (estimate, se) = match *target {
        RegressionTarget::Slope => (slope, (variance / sxx).sqrt()),
        RegressionTarget::Intercept => (intercept, (variance * (1. / n + mx * mx / sxx)).sqrt()),
        RegressionTarget::MeanResponse { x } | RegressionTarget::Prediction { x } => {
            finite([x])?;
            let prediction = if matches!(target, RegressionTarget::Prediction { .. }) {
                1.
            } else {
                0.
            };
            (
                intercept + slope * x,
                (variance * (prediction + 1. / n + (x - mx).powi(2) / sxx)).sqrt(),
            )
        }
    };
    let mut result = location(
        "Linear regression t",
        estimate,
        se,
        Distribution::Student { df },
        mode,
        vec![df],
    )?;
    result.details = BTreeMap::from([
        ("slope".into(), slope),
        ("intercept".into(), intercept),
        ("residual_sum_squares".into(), sse),
        ("residual_standard_error".into(), variance.sqrt()),
    ]);
    if syy > 0. {
        result
            .details
            .insert("r_squared".into(), (1. - sse / syy).clamp(0., 1.));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: f64, b: f64, tolerance: f64) {
        assert!(
            (a - b).abs() <= tolerance * b.abs().max(1e-100),
            "{a:.17e} != {b:.17e}"
        );
    }
    fn two_sided(null_value: f64) -> InferenceMode {
        InferenceMode::Test {
            null_value,
            alpha: 0.05,
            alternative: Alternative::TwoSided,
        }
    }
    #[test]
    fn distribution_tails_match_independent_50_digit_mpmath_references() {
        for (df, x, p) in [
            (1., 1., 0.25),
            (5., 2., 0.05096973941492918),
            (10., 10., 7.94776587798206e-7),
            (1000., 2., 0.02288517324662582),
            (1e6, 2., 0.022750266925659604),
        ] {
            near(Distribution::Student { df }.upper_tail(x).unwrap(), p, 2e-9);
        }
        for (df, x, p) in [
            (1., 3., 0.0832645166635504),
            (2., 10., 0.006737946999085467),
            (10., 100., 5.449701982920529e-17),
        ] {
            near(
                Distribution::ChiSquare { df }.upper_tail(x).unwrap(),
                p,
                2e-12,
            );
        }
        for (a, b, x, p) in [
            (1., 1., 1e6, 0.0006366195601611179),
            (5., 10., 3., 0.06555756209384411),
            (2., 6., 12., 0.008),
        ] {
            near(
                Distribution::F {
                    numerator_df: a,
                    denominator_df: b,
                }
                .upper_tail(x)
                .unwrap(),
                p,
                2e-12,
            );
        }
        near(
            Distribution::Normal.upper_tail(8.).unwrap(),
            6.220960574271784e-16,
            2e-12,
        );
    }
    #[test]
    fn quantiles_have_known_analytic_and_tabulated_values() {
        near(
            Distribution::Normal.inverse_upper_tail(0.025).unwrap(),
            1.959963984540054,
            2e-12,
        );
        near(
            Distribution::Student { df: 1. }
                .inverse_upper_tail(0.25)
                .unwrap(),
            1.,
            2e-12,
        );
        near(
            Distribution::Student { df: 10. }
                .inverse_upper_tail(0.025)
                .unwrap(),
            2.2281388519649385,
            2e-11,
        );
        near(
            Distribution::ChiSquare { df: 2. }
                .inverse_upper_tail(0.05)
                .unwrap(),
            -2. * 0.05_f64.ln(),
            2e-12,
        );
        for d in [
            Distribution::Normal,
            Distribution::Student { df: 3. },
            Distribution::ChiSquare { df: 10. },
            Distribution::F {
                numerator_df: 5.,
                denominator_df: 7.,
            },
        ] {
            for p in [1e-8, 0.025, 0.5, 0.975] {
                let q = d.inverse_upper_tail(p).unwrap();
                near(d.upper_tail(q).unwrap(), p, 2e-8);
            }
        }
    }
    #[test]
    fn small_positive_quantiles_use_relative_not_absolute_stopping() {
        let d = Distribution::F {
            numerator_df: 2.,
            denominator_df: 2.,
        };
        let p = 1. - 1e-12;
        let expected = (1. - p) / p;
        near(d.inverse_upper_tail(p).unwrap(), expected, 2e-10);
        let p = 1. - 1e-15;
        near(d.inverse_upper_tail(p).unwrap(), (1. - p) / p, 2e-10);
    }
    #[test]
    fn mean_z_alternative_probabilities_and_critical_values() {
        let mut request = InferenceRequest::default();
        let r = request.calculate().unwrap();
        near(r.statistic.unwrap(), 2., 1e-12);
        near(r.p_value.unwrap(), 0.0455002638963584, 1e-12);
        assert_eq!(r.reject_null, Some(true));
        assert_eq!(r.critical_values.len(), 2);
        if let InferenceRequest::MeanZ { mode, .. } = &mut request {
            *mode = InferenceMode::Test {
                null_value: 9.6,
                alpha: 0.05,
                alternative: Alternative::Less,
            };
        }
        let r = request.calculate().unwrap();
        near(r.p_value.unwrap(), 0.9772498680518208, 1e-12);
        assert_eq!(r.reject_null, Some(false));
        assert!(r.critical_values[0] < 0.);
    }
    #[test]
    fn extreme_left_tail_probabilities_do_not_cancel_to_zero() {
        let r = InferenceRequest::MeanZ {
            mean: -12.,
            sigma: 1.,
            count: 1,
            mode: InferenceMode::Test {
                null_value: 0.,
                alpha: 0.05,
                alternative: Alternative::Less,
            },
        }
        .calculate()
        .unwrap();
        near(r.p_value.unwrap(), 1.776482112077679e-33, 2e-12);
        near(
            Distribution::Student { df: 10. }.lower_tail(-10.).unwrap(),
            7.94776587798206e-7,
            2e-12,
        );
        near(
            Distribution::F {
                numerator_df: 2.,
                denominator_df: 2.,
            }
            .lower_tail(1e-20)
            .unwrap(),
            1e-20,
            2e-12,
        );
    }
    #[test]
    fn mean_t_and_pooled_welch_tests_and_intervals() {
        let first = MeanSummary {
            mean: 5.,
            sd: 2.,
            count: 12,
        };
        let second = MeanSummary {
            mean: 4.,
            sd: 1.,
            count: 10,
        };
        let r = InferenceRequest::TwoMeansT {
            first: first.clone(),
            second: second.clone(),
            pooled: false,
            mode: two_sided(0.),
        }
        .calculate()
        .unwrap();
        near(r.degrees_of_freedom[0], 16.74774774774775, 1e-12);
        near(r.p_value.unwrap(), 0.14738714851296224, 1e-12);
        let r = InferenceRequest::TwoMeansT {
            first,
            second,
            pooled: true,
            mode: two_sided(0.),
        }
        .calculate()
        .unwrap();
        assert_eq!(r.degrees_of_freedom, [20.]);
        near(r.p_value.unwrap(), 0.1668302199813224, 1e-12);
        let r = InferenceRequest::MeanT {
            sample: MeanSummary {
                mean: 10.,
                sd: 2.,
                count: 10,
            },
            mode: InferenceMode::Interval { confidence: 0.95 },
        }
        .calculate()
        .unwrap();
        assert!(r.interval.unwrap()[0] < 10. && r.interval.unwrap()[1] > 10.);
        assert_eq!(r.degrees_of_freedom, [9.]);
    }
    #[test]
    fn proportion_tests_use_pooled_null_and_unpooled_interval_errors() {
        let first = ProportionSample {
            successes: 60,
            total: 100,
        };
        let second = ProportionSample {
            successes: 45,
            total: 100,
        };
        let r = InferenceRequest::OneProportion {
            sample: first.clone(),
            mode: two_sided(0.5),
        }
        .calculate()
        .unwrap();
        near(r.statistic.unwrap(), 2., 1e-12);
        let test = InferenceRequest::TwoProportions {
            first: first.clone(),
            second: second.clone(),
            mode: two_sided(0.),
        }
        .calculate()
        .unwrap();
        near(
            test.standard_error.unwrap(),
            (0.525_f64 * 0.475 * 0.02).sqrt(),
            1e-12,
        );
        let interval = InferenceRequest::TwoProportions {
            first,
            second,
            mode: InferenceMode::Interval { confidence: 0.95 },
        }
        .calculate()
        .unwrap();
        near(
            interval.standard_error.unwrap(),
            (0.24_f64 / 100. + 0.2475 / 100.).sqrt(),
            1e-12,
        );
        assert!(interval.interval.unwrap()[0] > 0.);
    }
    #[test]
    fn chi_square_count_tests_and_anova_have_known_results() {
        let r = InferenceRequest::GoodnessOfFit {
            observed: vec![10., 20., 30.],
            expected: vec![20., 20., 20.],
            estimated_parameters: 0,
            alpha: 0.05,
        }
        .calculate()
        .unwrap();
        assert_eq!(r.statistic, Some(10.));
        near(r.p_value.unwrap(), (-5_f64).exp(), 1e-12);
        let r = InferenceRequest::Contingency {
            table: vec![vec![10., 20.], vec![30., 40.]],
            alpha: 0.05,
        }
        .calculate()
        .unwrap();
        near(r.statistic.unwrap(), 0.7936507936507936, 1e-12);
        assert_eq!(r.degrees_of_freedom, [1.]);
        let r = InferenceRequest::Anova {
            groups: vec![vec![1., 2., 3.], vec![3., 4., 5.], vec![5., 6., 7.]],
            alpha: 0.05,
        }
        .calculate()
        .unwrap();
        near(r.statistic.unwrap(), 12., 1e-12);
        near(r.p_value.unwrap(), 0.008, 1e-12);
        assert_eq!(r.degrees_of_freedom, [2., 6.]);
        assert_eq!(r.details["within_sum_squares"], 6.);
    }
    #[test]
    fn regression_slope_test_and_response_prediction_intervals() {
        let points = vec![[1., 2.], [2., 4.], [3., 5.], [4., 4.], [5., 5.]];
        let r = InferenceRequest::Regression {
            pairs: points.clone(),
            target: RegressionTarget::Slope,
            mode: two_sided(0.),
        }
        .calculate()
        .unwrap();
        near(r.estimate.unwrap(), 0.6, 1e-12);
        near(r.p_value.unwrap(), 0.12402706265755462, 1e-12);
        near(r.details["residual_sum_squares"], 2.4, 1e-12);
        let mean = InferenceRequest::Regression {
            pairs: points.clone(),
            target: RegressionTarget::MeanResponse { x: 3. },
            mode: InferenceMode::Interval { confidence: 0.95 },
        }
        .calculate()
        .unwrap();
        let prediction = InferenceRequest::Regression {
            pairs: points,
            target: RegressionTarget::Prediction { x: 3. },
            mode: InferenceMode::Interval { confidence: 0.95 },
        }
        .calculate()
        .unwrap();
        assert_eq!(mean.estimate, Some(4.));
        assert!(prediction.interval.unwrap()[0] < mean.interval.unwrap()[0]);
        assert!(prediction.interval.unwrap()[1] > mean.interval.unwrap()[1]);
    }
    #[test]
    fn variance_ratio_interval_brackets_estimate_and_data_roundtrip() {
        let request = InferenceRequest::VarianceRatio {
            sd1: 2.,
            n1: 10,
            sd2: 1.,
            n2: 12,
            mode: InferenceMode::Interval { confidence: 0.95 },
        };
        let data = serde_json::to_string(&request).unwrap();
        let restored: InferenceRequest = serde_json::from_str(&data).unwrap();
        let r = restored.calculate().unwrap();
        assert_eq!(r.estimate, Some(4.));
        assert!(r.interval.unwrap()[0] < 4. && r.interval.unwrap()[1] > 4.);
        serde_json::to_string(&r).unwrap();
    }
    #[test]
    fn equal_variances_have_unit_two_sided_probability() {
        let r = InferenceRequest::VarianceRatio {
            sd1: 1.,
            n1: 10,
            sd2: 1.,
            n2: 10,
            mode: two_sided(1.),
        }
        .calculate()
        .unwrap();
        near(r.p_value.unwrap(), 1., 2e-12);
        assert_eq!(r.reject_null, Some(false));
    }
    #[test]
    fn invalid_and_degenerate_data_is_not_fabricated_success() {
        assert!(
            InferenceRequest::MeanT {
                sample: MeanSummary {
                    mean: 1.,
                    sd: 0.,
                    count: 2
                },
                mode: two_sided(0.)
            }
            .calculate()
            .is_err()
        );
        assert!(
            InferenceRequest::GoodnessOfFit {
                observed: vec![1., 2.],
                expected: vec![1., 1.],
                estimated_parameters: 0,
                alpha: 0.05
            }
            .calculate()
            .is_err()
        );
        assert!(
            InferenceRequest::Contingency {
                table: vec![vec![0., 0.], vec![1., 2.]],
                alpha: 0.05
            }
            .calculate()
            .is_err()
        );
        assert!(
            InferenceRequest::Anova {
                groups: vec![vec![1., 1.], vec![2., 2.]],
                alpha: 0.05
            }
            .calculate()
            .is_err()
        );
        assert!(
            InferenceRequest::default()
                .calculate_with_cancel(&AtomicBool::new(true))
                .is_err()
        );
        assert!(Distribution::Student { df: 0. }.upper_tail(1.).is_err());
        assert!(Distribution::Normal.inverse_upper_tail(0.).is_err());
    }
}
