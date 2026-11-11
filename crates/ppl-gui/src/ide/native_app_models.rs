//! Persistable mathematical models for the G1's non-graphing applications.
//! No UI or global session access: calculations can run on an operation worker.
//! Contracts: HP Prime User Guide 813269-002, Statistics, Linear/Triangle Solver,
//! and Finance chapters. Binary floating-point results are numerical, not CAS exact values.
use serde::{Deserialize, Serialize};

fn finite(values: impl IntoIterator<Item = f64>) -> Result<(), String> {
    if values.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err("Values must be finite real numbers".into())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Statistics1 {
    pub samples: Vec<f64>,
    pub frequencies: Vec<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary1 {
    pub count: u64,
    pub sum: f64,
    pub mean: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub median: f64,
    pub population_variance: f64,
    pub sample_variance: Option<f64>,
}
impl Statistics1 {
    pub fn calculate(&self) -> Result<Summary1, String> {
        finite(self.samples.iter().copied())?;
        if self.samples.is_empty() {
            return Err("Enter at least one observation".into());
        }
        if !self.frequencies.is_empty() && self.frequencies.len() != self.samples.len() {
            return Err("Each observation needs one frequency".into());
        }
        let mut weighted = Vec::new();
        let mut n = 0u64;
        let mut mean = 0.;
        let mut m2 = 0.;
        let mut sum = 0.;
        for (i, &x) in self.samples.iter().enumerate() {
            let frequency = self.frequencies.get(i).copied().unwrap_or(1);
            if frequency == 0 {
                continue;
            }
            let next = n.checked_add(frequency).ok_or("Frequency total overflow")?;
            // Integer frequencies above 2^53 cannot be represented exactly by f64.
            if next > 9_007_199_254_740_992 {
                return Err("Frequency total exceeds numerical precision".into());
            }
            let delta = x - mean;
            mean += delta * frequency as f64 / next as f64;
            m2 += delta * (x - mean) * frequency as f64;
            sum += x * frequency as f64;
            n = next;
            weighted.push((x, frequency));
        }
        if n == 0 {
            return Err("At least one frequency must be positive".into());
        }
        weighted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let kth = |rank: u64| {
            let mut total = 0;
            for &(value, f) in &weighted {
                total += f;
                if rank < total {
                    return value;
                }
            }
            weighted.last().unwrap().0
        };
        let median = if n % 2 == 0 {
            kth(n / 2 - 1) / 2. + kth(n / 2) / 2.
        } else {
            kth(n / 2)
        };
        finite([mean, m2, sum, median])?;
        Ok(Summary1 {
            count: n,
            sum,
            mean,
            minimum: weighted[0].0,
            maximum: weighted.last().unwrap().0,
            median,
            population_variance: (m2 / n as f64).max(0.),
            sample_variance: (n > 1).then(|| (m2 / (n - 1) as f64).max(0.)),
        })
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Statistics2 {
    pub pairs: Vec<[f64; 2]>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearRegression {
    pub count: usize,
    pub intercept: f64,
    pub slope: f64,
    pub correlation: Option<f64>,
    pub residual_sum_squares: f64,
}
impl Statistics2 {
    pub fn linear_fit(&self) -> Result<LinearRegression, String> {
        if self.pairs.len() < 2 {
            return Err("Enter at least two paired observations".into());
        }
        finite(self.pairs.iter().flatten().copied())?;
        let n = self.pairs.len() as f64;
        let mx = self.pairs.iter().map(|p| p[0] / n).sum::<f64>();
        let my = self.pairs.iter().map(|p| p[1] / n).sum::<f64>();
        let xx = self.pairs.iter().map(|p| (p[0] - mx).powi(2)).sum::<f64>();
        let yy = self.pairs.iter().map(|p| (p[1] - my).powi(2)).sum::<f64>();
        let xy = self
            .pairs
            .iter()
            .map(|p| (p[0] - mx) * (p[1] - my))
            .sum::<f64>();
        if xx == 0. {
            return Err("Linear fit requires distinct x coordinates".into());
        }
        let slope = xy / xx;
        let intercept = my - slope * mx;
        let residual_sum_squares = self
            .pairs
            .iter()
            .map(|p| (p[1] - (intercept + slope * p[0])).powi(2))
            .sum::<f64>();
        finite([slope, intercept, residual_sum_squares])?;
        Ok(LinearRegression {
            count: self.pairs.len(),
            intercept,
            slope,
            correlation: (yy > 0.).then(|| ((xy / xx.sqrt()) / yy.sqrt()).clamp(-1., 1.)),
            residual_sum_squares,
        })
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinearSystem {
    pub coefficients: Vec<Vec<f64>>,
    pub constants: Vec<f64>,
}
impl LinearSystem {
    pub fn solve(&self) -> Result<Vec<f64>, String> {
        let n = self.constants.len();
        if !(2..=3).contains(&n)
            || self.coefficients.len() != n
            || self.coefficients.iter().any(|r| r.len() != n)
        {
            return Err(
                "Linear Solver requires two or three equations of matching dimensions".into(),
            );
        }
        finite(
            self.coefficients
                .iter()
                .flatten()
                .chain(&self.constants)
                .copied(),
        )?;
        let mut a = self.coefficients.clone();
        let mut b = self.constants.clone();
        for (row, rhs) in a.iter_mut().zip(&mut b) {
            let scale = row.iter().map(|v| v.abs()).fold(0., f64::max);
            if scale > 0. {
                for v in row {
                    *v /= scale;
                }
                *rhs /= scale;
            }
        }
        let rhs_scale = b.iter().map(|v| v.abs()).fold(0., f64::max);
        let mut rank = 0;
        let mut pivots = Vec::new();
        for col in 0..n {
            let row = (rank..n)
                .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
                .unwrap();
            if a[row][col].abs() <= 32. * f64::EPSILON {
                continue;
            }
            a.swap(rank, row);
            b.swap(rank, row);
            let pivot = a[rank][col];
            for value in &mut a[rank][col..] {
                *value /= pivot;
            }
            b[rank] /= pivot;
            let pivot_row = a[rank].clone();
            for r in 0..n {
                if r == rank {
                    continue;
                }
                let factor = a[r][col];
                for (value, pivot) in a[r][col..].iter_mut().zip(&pivot_row[col..]) {
                    *value -= factor * pivot;
                }
                b[r] -= factor * b[rank];
            }
            pivots.push(col);
            rank += 1;
            if rank == n {
                break;
            }
        }
        if (rank..n).any(|r| {
            a[r].iter().all(|v| v.abs() <= 32. * f64::EPSILON)
                && b[r].abs() > 32. * f64::EPSILON * rhs_scale
        }) {
            return Err("No solution: inconsistent equations".into());
        }
        if rank < n {
            return Err("Infinitely many solutions: equations are dependent".into());
        }
        let mut result = vec![0.; n];
        for (row, &col) in pivots.iter().enumerate() {
            result[col] = b[row];
        }
        finite(result.iter().copied())?;
        Ok(result)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FinanceUnknown {
    Periods,
    AnnualRate,
    PresentValue,
    Payment,
    FutureValue,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finance {
    pub periods: f64,
    pub annual_rate: f64,
    pub present_value: f64,
    pub payment: f64,
    pub future_value: f64,
    pub payments_per_year: f64,
    pub compounds_per_year: f64,
    pub begin: bool,
}
impl Default for Finance {
    fn default() -> Self {
        Self {
            periods: 60.,
            annual_rate: 5.5,
            present_value: 16500.,
            payment: 0.,
            future_value: 0.,
            payments_per_year: 12.,
            compounds_per_year: 12.,
            begin: false,
        }
    }
}
impl Finance {
    fn rate(&self) -> Result<f64, String> {
        let base = 1. + self.annual_rate / (100. * self.compounds_per_year);
        if base <= 0. {
            return Err("Compounding interest base must be positive".into());
        }
        let rate = (base.ln() * self.compounds_per_year / self.payments_per_year).exp_m1();
        finite([rate])?;
        Ok(rate)
    }
    fn factors(&self, i: f64) -> Result<(f64, f64), String> {
        let discount = (-self.periods * i.ln_1p()).exp();
        let annuity = if i.abs() < 1e-14 {
            self.periods
        } else {
            -(-self.periods * i.ln_1p()).exp_m1() / i
        };
        let annuity = annuity * if self.begin { 1. + i } else { 1. };
        finite([discount, annuity])?;
        Ok((discount, annuity))
    }
    pub fn solve(&self, unknown: FinanceUnknown) -> Result<f64, String> {
        finite([
            self.periods,
            self.annual_rate,
            self.present_value,
            self.payment,
            self.future_value,
            self.payments_per_year,
            self.compounds_per_year,
        ])?;
        if self.payments_per_year <= 0. || self.compounds_per_year <= 0. {
            return Err("Payment and compounding frequencies must be positive".into());
        }
        if self.periods <= 0. && unknown != FinanceUnknown::Periods {
            return Err("Number of payments must be positive".into());
        }
        let answer = if unknown == FinanceUnknown::AnnualRate {
            let flows = [self.present_value, self.payment, self.future_value];
            if !flows.iter().any(|v| *v > 0.) || !flows.iter().any(|v| *v < 0.) {
                return Err("Interest needs both incoming and outgoing cash flows".into());
            }
            // Solve the per-payment rate, then convert back to nominal annual percent.
            let residual = |i: f64| -> Option<f64> {
                let (d, a) = self.factors(i).ok()?;
                let r = self.present_value + self.payment * a + self.future_value * d;
                r.is_finite().then_some(r)
            };
            let mut bracket = None;
            let mut previous = None;
            // Log spacing in 1+i covers negative rates and large positive rates.
            for k in 0..=600 {
                let i = (-12. + k as f64 * 0.04).exp() - 1.;
                if let Some(y) = residual(i) {
                    if y == 0. {
                        bracket = Some((i, i));
                        break;
                    }
                    if let Some((last, last_y)) = previous {
                        if (y > 0.) != (last_y > 0.) {
                            bracket = Some((last, i));
                            break;
                        }
                    }
                    previous = Some((i, y));
                }
            }
            let (mut lo, mut hi) =
                bracket.ok_or("No interest-rate solution in the supported numerical range")?;
            for _ in 0..160 {
                if lo == hi {
                    break;
                }
                let mid = (lo + hi) / 2.;
                let f = residual(mid).ok_or("Interest computation overflow")?;
                if f == 0. || (hi - lo).abs() < 1e-13 * (1. + mid.abs()) {
                    lo = mid;
                    hi = mid;
                    break;
                }
                if (f > 0.) == (residual(lo).unwrap() > 0.) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            self.compounds_per_year
                * ((((lo + hi) / 2.).ln_1p() * self.payments_per_year / self.compounds_per_year)
                    .exp_m1())
                * 100.
        } else {
            let i = self.rate()?;
            let (d, a) = self.factors(i)?;
            match unknown {
                FinanceUnknown::PresentValue => -self.payment * a - self.future_value * d,
                FinanceUnknown::FutureValue => {
                    if d == 0. {
                        return Err("Future value exceeds numerical range".into());
                    }
                    -(self.present_value + self.payment * a) / d
                }
                FinanceUnknown::Payment => {
                    if a == 0. {
                        return Err("Payment is indeterminate".into());
                    }
                    -(self.present_value + self.future_value * d) / a
                }
                FinanceUnknown::Periods => {
                    if i.abs() < 1e-14 {
                        if self.payment == 0. {
                            return Err("Number of payments is indeterminate".into());
                        }
                        -(self.present_value + self.future_value) / self.payment
                    } else {
                        let p = self.payment * if self.begin { 1. + i } else { 1. } / i;
                        let d = -(self.present_value + p) / (self.future_value - p);
                        if d <= 0. {
                            return Err("No positive real payment-count solution".into());
                        }
                        -d.ln() / i.ln_1p()
                    }
                }
                FinanceUnknown::AnnualRate => unreachable!(),
            }
        };
        finite([answer])?;
        if unknown == FinanceUnknown::Periods && answer <= 0. {
            return Err("Payment-count solution is not positive".into());
        }
        Ok(answer)
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Triangle {
    pub sides: [Option<f64>; 3],
    pub angles_degrees: [Option<f64>; 3],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriangleSolution {
    pub sides: [f64; 3],
    pub angles_degrees: [f64; 3],
    pub area: f64,
}
impl Triangle {
    pub fn solve(&self) -> Result<Vec<TriangleSolution>, String> {
        finite(
            self.sides
                .iter()
                .chain(&self.angles_degrees)
                .flatten()
                .copied(),
        )?;
        if self.sides.iter().flatten().any(|v| *v <= 0.)
            || self
                .angles_degrees
                .iter()
                .flatten()
                .any(|v| *v <= 0. || *v >= 180.)
        {
            return Err("Sides must be positive and angles between 0 and 180 degrees".into());
        }
        let mut candidates = Vec::new();
        if self.sides.iter().all(Option::is_some) {
            candidates.push(self.sides.map(Option::unwrap));
        } else if self.angles_degrees.iter().filter(|a| a.is_some()).count() >= 2 {
            let mut angles = self.angles_degrees;
            let known = angles.iter().flatten().sum::<f64>();
            if let Some(missing) = angles.iter().position(Option::is_none) {
                if known >= 180. {
                    return Err("Triangle angles must total 180 degrees".into());
                }
                angles[missing] = Some(180. - known);
            } else if (known - 180.).abs() > 1e-8 {
                return Err("Triangle angles must total 180 degrees".into());
            }
            let side = self
                .sides
                .iter()
                .position(Option::is_some)
                .ok_or("At least one side is required")?;
            let scale = self.sides[side].unwrap() / angles[side].unwrap().to_radians().sin();
            candidates.push(angles.map(|a| scale * a.unwrap().to_radians().sin()));
        } else {
            for angle in 0..3 {
                let Some(degrees) = self.angles_degrees[angle] else {
                    continue;
                };
                let a = degrees.to_radians();
                let others: Vec<_> = (0..3).filter(|i| *i != angle).collect();
                if let (Some(b), Some(c)) = (self.sides[others[0]], self.sides[others[1]]) {
                    let mut sides = [0.; 3];
                    sides[others[0]] = b;
                    sides[others[1]] = c;
                    sides[angle] = (b * b + c * c - 2. * b * c * a.cos()).sqrt();
                    candidates.push(sides);
                } else if let Some(opposite) = self.sides[angle] {
                    if let Some(other) = others.iter().copied().find(|i| self.sides[*i].is_some()) {
                        let ratio = self.sides[other].unwrap() * a.sin() / opposite;
                        if ratio <= 1. + 1e-12 {
                            let first = ratio.clamp(-1., 1.).asin().to_degrees();
                            for b in [first, 180. - first] {
                                let c = 180. - degrees - b;
                                if c > 1e-10 {
                                    let mut sides = [0.; 3];
                                    let third = 3 - angle - other;
                                    sides[angle] = opposite;
                                    sides[other] = self.sides[other].unwrap();
                                    sides[third] = opposite * c.to_radians().sin() / a.sin();
                                    candidates.push(sides);
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut result = Vec::new();
        for sides in candidates {
            if sides.iter().any(|s| !s.is_finite() || *s <= 0.)
                || (0..3).any(|i| sides[i] >= sides[(i + 1) % 3] + sides[(i + 2) % 3])
            {
                continue;
            }
            let angles = std::array::from_fn(|i| {
                let b = sides[(i + 1) % 3];
                let c = sides[(i + 2) % 3];
                ((b * b + c * c - sides[i] * sides[i]) / (2. * b * c))
                    .clamp(-1., 1.)
                    .acos()
                    .to_degrees()
            });
            if (0..3).any(|i| {
                self.sides[i].is_some_and(|x| (x - sides[i]).abs() > 1e-8 * x.max(1.))
                    || self.angles_degrees[i].is_some_and(|x| (x - angles[i]).abs() > 1e-7)
            }) {
                continue;
            }
            if result.iter().any(|old: &TriangleSolution| {
                (0..3).all(|i| (old.angles_degrees[i] - angles[i]).abs() < 1e-7)
            }) {
                continue;
            }
            let area = 0.5 * sides[1] * sides[2] * angles[0].to_radians().sin();
            finite([area])?;
            result.push(TriangleSolution {
                sides,
                angles_degrees: angles,
                area,
            });
        }
        if result.is_empty() {
            return Err("No triangle satisfies the supplied measurements, or more measurements are required".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-7 * b.abs().max(1.), "{a} != {b}");
    }
    #[test]
    fn weighted_statistics_match_expanded_data() {
        let a = Statistics1 {
            samples: vec![1., 3., 5.],
            frequencies: vec![1, 2, 1],
        }
        .calculate()
        .unwrap();
        assert_eq!(a.count, 4);
        near(a.mean, 3.);
        near(a.median, 3.);
        near(a.population_variance, 2.);
        near(a.sample_variance.unwrap(), 8. / 3.);
        assert!(
            Statistics1 {
                samples: vec![1.],
                frequencies: vec![0]
            }
            .calculate()
            .is_err()
        );
    }
    #[test]
    fn singleton_statistics_and_large_offset_stability() {
        let a = Statistics1 {
            samples: vec![1e12 + 1., 1e12 + 2., 1e12 + 3.],
            frequencies: vec![],
        }
        .calculate()
        .unwrap();
        near(a.sample_variance.unwrap(), 1.);
        assert!(
            Statistics1 {
                samples: vec![1.],
                frequencies: vec![]
            }
            .calculate()
            .unwrap()
            .sample_variance
            .is_none()
        );
    }
    #[test]
    fn regression_and_degenerate_inputs() {
        let a = Statistics2 {
            pairs: vec![[0., 1.], [1., 3.], [2., 5.]],
        }
        .linear_fit()
        .unwrap();
        near(a.slope, 2.);
        near(a.intercept, 1.);
        near(a.correlation.unwrap(), 1.);
        near(a.residual_sum_squares, 0.);
        assert!(
            Statistics2 {
                pairs: vec![[1., 2.], [1., 3.]]
            }
            .linear_fit()
            .is_err()
        );
    }
    #[test]
    fn linear_solver_pivots_and_classifies_singular_systems() {
        let a = LinearSystem {
            coefficients: vec![vec![0., 2.], vec![1., 1.]],
            constants: vec![4., 3.],
        }
        .solve()
        .unwrap();
        near(a[0], 1.);
        near(a[1], 2.);
        let mut a = LinearSystem {
            coefficients: vec![vec![1., 1.], vec![2., 2.]],
            constants: vec![1., 2.],
        };
        assert!(a.solve().unwrap_err().contains("Infinitely"));
        a.constants[1] = 3.;
        assert!(a.solve().unwrap_err().contains("No solution"));
    }
    #[test]
    fn linear_solver_three_equation_documented_example() {
        let a = LinearSystem {
            coefficients: vec![vec![6., 9., 6.], vec![7., 10., 8.], vec![6., 4., 0.]],
            constants: vec![5., 10., 6.],
        };
        let x = a.solve().unwrap();
        for (r, b) in a.coefficients.iter().zip(a.constants) {
            near(r.iter().zip(&x).map(|(a, b)| a * b).sum(), b);
        }
    }
    #[test]
    fn finance_hp_car_loan_and_inverse_solutions() {
        let mut f = Finance::default();
        let payment = f.solve(FinanceUnknown::Payment).unwrap();
        assert_eq!((payment * 100.).round() / 100., -315.17);
        f.payment = payment;
        near(f.solve(FinanceUnknown::Periods).unwrap(), 60.);
        near(f.solve(FinanceUnknown::PresentValue).unwrap(), 16500.);
        near(f.solve(FinanceUnknown::FutureValue).unwrap(), 0.);
        near(f.solve(FinanceUnknown::AnnualRate).unwrap(), 5.5);
    }
    #[test]
    fn finance_zero_interest_and_begin_end() {
        let mut f = Finance {
            periods: 10.,
            annual_rate: 0.,
            present_value: 100.,
            ..Default::default()
        };
        near(f.solve(FinanceUnknown::Payment).unwrap(), -10.);
        f.payment = -10.;
        near(f.solve(FinanceUnknown::Periods).unwrap(), 10.);
        f.annual_rate = 12.;
        let end = f.solve(FinanceUnknown::Payment).unwrap();
        f.begin = true;
        near(f.solve(FinanceUnknown::Payment).unwrap(), end / 1.01);
    }
    #[test]
    fn triangles_sss_sas_aas_and_ambiguous_ssa() {
        let s = Triangle {
            sides: [Some(3.), Some(4.), Some(5.)],
            ..Default::default()
        }
        .solve()
        .unwrap();
        near(s[0].area, 6.);
        near(s[0].angles_degrees[2], 90.);
        let s = Triangle {
            sides: [None, Some(3.), Some(4.)],
            angles_degrees: [Some(90.), None, None],
        }
        .solve()
        .unwrap();
        near(s[0].sides[0], 5.);
        let s = Triangle {
            sides: [Some(2.), None, None],
            angles_degrees: [Some(30.), Some(60.), None],
        }
        .solve()
        .unwrap();
        near(s[0].sides[2], 4.);
        let s = Triangle {
            sides: [Some(5.), Some(8.), None],
            angles_degrees: [Some(30.), None, None],
        }
        .solve()
        .unwrap();
        assert_eq!(s.len(), 2);
        assert!(
            Triangle {
                sides: [Some(1.), Some(2.), Some(5.)],
                ..Default::default()
            }
            .solve()
            .is_err()
        );
    }
    #[test]
    fn app_models_roundtrip() {
        let f = Finance::default();
        let text = serde_json::to_string(&f).unwrap();
        let restored: Finance = serde_json::from_str(&text).unwrap();
        near(
            restored.solve(FinanceUnknown::Payment).unwrap(),
            f.solve(FinanceUnknown::Payment).unwrap(),
        );
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}
impl Point2 {
    pub fn distance(self, other: Self) -> Result<f64, String> {
        finite([self.x, self.y, other.x, other.y])?;
        let d = (self.x - other.x).hypot(self.y - other.y);
        finite([d])?;
        Ok(d)
    }
    pub fn rotate(self, center: Self, degrees: f64) -> Result<Self, String> {
        finite([self.x, self.y, center.x, center.y, degrees])?;
        let (s, c) = degrees.to_radians().sin_cos();
        let x = self.x - center.x;
        let y = self.y - center.y;
        let p = Self {
            x: center.x + x * c - y * s,
            y: center.y + x * s + y * c,
        };
        finite([p.x, p.y])?;
        Ok(p)
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Geometry {
    pub points: std::collections::BTreeMap<String, Point2>,
    pub segments: Vec<[String; 2]>,
}
impl Geometry {
    pub fn polygon_area(points: &[Point2]) -> Result<f64, String> {
        if points.len() < 3 {
            return Err("A polygon needs at least three vertices".into());
        }
        finite(points.iter().flat_map(|p| [p.x, p.y]))?;
        // Translate to the first point before summation to reduce cancellation.
        let origin = points[0];
        let mut area = 0.;
        for i in 1..points.len() - 1 {
            area += (points[i].x - origin.x) * (points[i + 1].y - origin.y)
                - (points[i].y - origin.y) * (points[i + 1].x - origin.x);
        }
        finite([area])?;
        Ok(area.abs() / 2.)
    }
    pub fn line_intersection(a: Point2, b: Point2, c: Point2, d: Point2) -> Result<Point2, String> {
        finite([a.x, a.y, b.x, b.y, c.x, c.y, d.x, d.y])?;
        let u = Point2 {
            x: b.x - a.x,
            y: b.y - a.y,
        };
        let v = Point2 {
            x: d.x - c.x,
            y: d.y - c.y,
        };
        let det = u.x * v.y - u.y * v.x;
        if u.x == 0. && u.y == 0. || v.x == 0. && v.y == 0. {
            return Err("A line needs two distinct points".into());
        }
        if det.abs() <= 16. * f64::EPSILON * u.x.hypot(u.y) * v.x.hypot(v.y) {
            return Err("Lines are parallel or coincident".into());
        }
        let t = ((c.x - a.x) * v.y - (c.y - a.y) * v.x) / det;
        let p = Point2 {
            x: a.x + t * u.x,
            y: a.y + t * u.y,
        };
        finite([p.x, p.y])?;
        Ok(p)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Alternative {
    TwoSided,
    Less,
    Greater,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZInference {
    pub statistic: f64,
    pub p_value: f64,
    pub interval: [f64; 2],
}
// Normal CDF, Abramowitz-Stegun 26.2.17 (absolute approximation error < 7.5e-8).
fn normal_cdf(z: f64) -> f64 {
    if z == 0. {
        return 0.5;
    }
    let x = z.abs();
    let t = 1. / (1. + 0.2316419 * x);
    let tail = (-x * x / 2.).exp() / std::f64::consts::TAU.sqrt()
        * t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));
    if z >= 0. { 1. - tail } else { tail }
}
fn z_critical(confidence: f64) -> Result<f64, String> {
    if !confidence.is_finite() || confidence <= 0. || confidence >= 1. {
        return Err("Confidence must be strictly between 0 and 1".into());
    }
    let target = (1. + confidence) / 2.;
    let (mut lo, mut hi) = (0., 9.);
    for _ in 0..80 {
        let mid = (lo + hi) / 2.;
        if normal_cdf(mid) < target {
            lo = mid
        } else {
            hi = mid
        }
    }
    Ok((lo + hi) / 2.)
}
fn p_value(z: f64, alternative: Alternative) -> f64 {
    match alternative {
        Alternative::TwoSided => 2. * normal_cdf(-z.abs()),
        Alternative::Less => normal_cdf(z),
        Alternative::Greater => normal_cdf(-z),
    }
}
impl ZInference {
    /// One-sample normal mean test/interval with known population sigma.
    pub fn mean(
        sample_mean: f64,
        sigma: f64,
        count: u64,
        null_mean: f64,
        confidence: f64,
        alternative: Alternative,
    ) -> Result<Self, String> {
        finite([sample_mean, sigma, null_mean])?;
        if sigma <= 0. || count == 0 {
            return Err("Known sigma and sample count must be positive".into());
        }
        let se = sigma / (count as f64).sqrt();
        let statistic = (sample_mean - null_mean) / se;
        let margin = z_critical(confidence)? * se;
        let result = Self {
            statistic,
            p_value: p_value(statistic, alternative),
            interval: [sample_mean - margin, sample_mean + margin],
        };
        finite([statistic, result.interval[0], result.interval[1]])?;
        Ok(result)
    }
    /// One-proportion Z test and two-sided Wald confidence interval.
    pub fn proportion(
        successes: u64,
        total: u64,
        null_proportion: f64,
        confidence: f64,
        alternative: Alternative,
    ) -> Result<Self, String> {
        if total == 0
            || successes > total
            || !null_proportion.is_finite()
            || null_proportion <= 0.
            || null_proportion >= 1.
        {
            return Err("Enter valid counts and a null proportion strictly between 0 and 1".into());
        }
        let p = successes as f64 / total as f64;
        let se = (null_proportion * (1. - null_proportion) / total as f64).sqrt();
        let statistic = (p - null_proportion) / se;
        let margin = z_critical(confidence)? * (p * (1. - p) / total as f64).sqrt();
        Ok(Self {
            statistic,
            p_value: p_value(statistic, alternative),
            interval: [p - margin, p + margin],
        })
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spreadsheet {
    pub cells: std::collections::BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CellValue {
    Number(f64),
    Text(String),
}
fn cell_coordinates(name: &str) -> Option<(usize, usize)> {
    let split = name.bytes().position(|c| c.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let mut col = 0usize;
    for ch in name[..split].bytes() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        col = col
            .checked_mul(26)?
            .checked_add((ch.to_ascii_uppercase() - b'A' + 1) as usize)?;
    }
    let row = name[split..].parse::<usize>().ok()?;
    if row == 0 || col > 16384 || row > 1_048_576 {
        return None;
    }
    Some((col, row))
}
fn cell_name(mut col: usize, row: usize) -> String {
    let mut letters = Vec::new();
    while col > 0 {
        letters.push((b'A' + ((col - 1) % 26) as u8) as char);
        col = (col - 1) / 26;
    }
    letters.reverse();
    format!("{}{row}", letters.into_iter().collect::<String>())
}
impl Spreadsheet {
    pub fn set(&mut self, name: &str, source: &str) -> Result<(), String> {
        let (col, row) = cell_coordinates(name).ok_or("Invalid cell address")?;
        let name = cell_name(col, row);
        if source.is_empty() {
            self.cells.remove(&name);
        } else {
            self.cells.insert(name, source.into());
        }
        Ok(())
    }
    pub fn evaluate(&self, name: &str, settings: &ppl_vm::Settings) -> Result<CellValue, String> {
        let (col, row) = cell_coordinates(name).ok_or("Invalid cell address")?;
        self.cell(
            &cell_name(col, row),
            settings,
            &mut std::collections::BTreeSet::new(),
        )
    }
    fn cell(
        &self,
        name: &str,
        settings: &ppl_vm::Settings,
        visiting: &mut std::collections::BTreeSet<String>,
    ) -> Result<CellValue, String> {
        if visiting.len() > 256 {
            return Err("Spreadsheet dependency depth exceeded".into());
        }
        if !visiting.insert(name.into()) {
            return Err(format!("Circular spreadsheet reference at {name}"));
        }
        let result = (|| {
            let source = self
                .cells
                .get(name)
                .ok_or_else(|| format!("Cell {name} is empty"))?;
            let formula = source.trim();
            if let Some(formula) = formula.strip_prefix('=') {
                let mut expression = self.expand_sums(formula, settings, visiting)?;
                let (ast, diags) = ppl_syntax::parse_expression(&expression);
                if diags.has_errors() {
                    return Err(diags.errors().next().unwrap().message.clone());
                }
                let mut refs = Vec::new();
                collect_cells(&ast, &mut refs);
                refs.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
                for (span, reference) in refs {
                    let value = self.cell(&reference.to_ascii_uppercase(), settings, visiting)?;
                    let CellValue::Number(value) = value else {
                        return Err(format!("Cell {reference} is text, not a numeric value"));
                    };
                    expression.replace_range(span, &format!("({value})"));
                }
                let value = ppl_vm::eval_text_with(&expression, settings)
                    .map_err(|e| e.to_string())?
                    .expect_real("Spreadsheet formula")
                    .map_err(|e| e.to_string())?
                    .to_f64();
                finite([value])?;
                Ok(CellValue::Number(value))
            } else if let Ok(number) = formula.parse::<f64>() {
                finite([number])?;
                Ok(CellValue::Number(number))
            } else {
                Ok(CellValue::Text(
                    source.strip_prefix('\'').unwrap_or(source).to_string(),
                ))
            }
        })();
        visiting.remove(name);
        result
    }
    fn expand_sums(
        &self,
        formula: &str,
        settings: &ppl_vm::Settings,
        visiting: &mut std::collections::BTreeSet<String>,
    ) -> Result<String, String> {
        let mut expression = formula.to_string();
        let mut search = 0;
        while let Some(start) = expression[search..].find("SUM(").map(|p| p + search) {
            let Some(close) = expression[start + 4..].find(')').map(|p| p + start + 4) else {
                break;
            };
            let body = &expression[start + 4..close];
            let Some((first, last)) = body.split_once(':') else {
                search = close + 1;
                continue;
            };
            let (c1, r1) = cell_coordinates(first.trim()).ok_or("Invalid SUM range start")?;
            let (c2, r2) = cell_coordinates(last.trim()).ok_or("Invalid SUM range end")?;
            if c1 > c2 || r1 > r2 || (c2 - c1 + 1).saturating_mul(r2 - r1 + 1) > 4096 {
                return Err("SUM range must be increasing and contain at most 4096 cells".into());
            }
            let mut sum = 0.;
            for r in r1..=r2 {
                for c in c1..=c2 {
                    match self.cell(&cell_name(c, r), settings, visiting)? {
                        CellValue::Number(v) => sum += v,
                        CellValue::Text(_) => return Err("SUM range contains text".into()),
                    }
                }
            }
            finite([sum])?;
            expression.replace_range(start..close + 1, &format!("({sum})"));
            search = start;
        }
        Ok(expression)
    }
}
fn collect_cells(expr: &ppl_syntax::ast::Expr, refs: &mut Vec<(std::ops::Range<usize>, String)>) {
    use ppl_syntax::ast::ExprKind::*;
    match &expr.kind {
        Ident(name) if cell_coordinates(name).is_some() => refs.push((
            expr.span.start as usize..expr.span.end as usize,
            name.clone(),
        )),
        Unary(_, e) | Postfix(_, e) | Quoted(e) => collect_cells(e, refs),
        Binary(_, a, b) | Assign(a, b) | Store(a, b) | ComplexPair(a, b) => {
            collect_cells(a, refs);
            collect_cells(b, refs);
        }
        Index(c, args) => {
            collect_cells(c, refs);
            for e in args {
                collect_cells(e, refs);
            }
        }
        Call(c, args) => {
            if !matches!(c.kind, Ident(_)) {
                collect_cells(c, refs);
            }
            for e in args {
                collect_cells(e, refs);
            }
        }
        List(values) | Bracket(values) => {
            for e in values {
                collect_cells(e, refs);
            }
        }
        _ => {}
    }
}
#[cfg(test)]
mod additional_tests {
    use super::*;
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-5, "{a}!={b}");
    }
    #[test]
    fn geometry_measure_transform_and_intersect() {
        let a = Point2 { x: 0., y: 0. };
        let b = Point2 { x: 3., y: 4. };
        near(a.distance(b).unwrap(), 5.);
        let p = b.rotate(a, 90.).unwrap();
        near(p.x, -4.);
        near(p.y, 3.);
        near(
            Geometry::polygon_area(&[
                a,
                Point2 { x: 4., y: 0. },
                Point2 { x: 4., y: 3. },
                Point2 { x: 0., y: 3. },
            ])
            .unwrap(),
            12.,
        );
        let p = Geometry::line_intersection(
            a,
            Point2 { x: 2., y: 2. },
            Point2 { x: 0., y: 2. },
            Point2 { x: 2., y: 0. },
        )
        .unwrap();
        near(p.x, 1.);
        near(p.y, 1.);
    }
    #[test]
    fn normal_mean_and_proportion_inference() {
        let z = ZInference::mean(10., 2., 100, 9.6, 0.95, Alternative::TwoSided).unwrap();
        near(z.statistic, 2.);
        near(z.p_value, 0.04550026);
        near(z.interval[0], 9.6080072);
        near(z.interval[1], 10.3919928);
        let z = ZInference::proportion(60, 100, 0.5, 0.95, Alternative::Greater).unwrap();
        near(z.statistic, 2.);
        near(z.p_value, 0.02275013);
        assert!(ZInference::proportion(101, 100, 0.5, 0.95, Alternative::Less).is_err());
    }
    #[test]
    fn spreadsheet_dependencies_ranges_recalculation_and_cycle() {
        let mut sheet = Spreadsheet::default();
        sheet.set("a1", "2").unwrap();
        sheet.set("A2", "=A1^2").unwrap();
        sheet.set("B1", "=SUM(A1:A2)+1").unwrap();
        assert_eq!(
            sheet.evaluate("B1", &Default::default()).unwrap(),
            CellValue::Number(7.)
        );
        sheet.set("A1", "3").unwrap();
        assert_eq!(
            sheet.evaluate("B1", &Default::default()).unwrap(),
            CellValue::Number(13.)
        );
        sheet.set("A1", "=B1").unwrap();
        assert!(
            sheet
                .evaluate("B1", &Default::default())
                .unwrap_err()
                .contains("Circular")
        );
    }
    #[test]
    fn invalid_and_missing_cells_do_not_become_successful_zeroes() {
        let mut s = Spreadsheet::default();
        assert!(s.set("A0", "2").is_err());
        assert!(s.evaluate("A1", &Default::default()).is_err());
        s.set("A1", "hello").unwrap();
        s.set("B1", "=A1+1").unwrap();
        assert!(s.evaluate("B1", &Default::default()).is_err());
        assert_eq!(
            s.evaluate("A1", &Default::default()).unwrap(),
            CellValue::Text("hello".into())
        );
    }
}
