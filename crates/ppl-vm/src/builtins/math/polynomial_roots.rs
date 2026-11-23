//! Real roots of the low-degree polynomials used by Spreadsheet PredX.
//! Consecutive derivative roots partition the domain into monotone intervals.
use super::prediction::invalid;
use crate::{ErrorKind, RuntimeError, error::RtResult};
use ppl_math::Real;

fn evaluate(c: &[f64], x: f64) -> f64 {
    c.iter().rev().fold(0.0, |sum, &a| sum.mul_add(x, a))
}

// Outside the unit interval divide by |x|^degree to preserve the sign without
// overflowing while testing the outer bounds.
fn signed_value(c: &[f64], x: f64) -> f64 {
    if x.abs() <= 1.0 {
        return evaluate(c, x);
    }
    let scaled = c.iter().fold(0.0_f64, |sum, &a| sum.mul_add(x.recip(), a));
    if x.is_sign_negative() && c.len() % 2 == 0 {
        -scaled
    } else {
        scaled
    }
}

fn real_roots(c: &[f64]) -> Vec<f64> {
    if let [a, b] = c {
        return vec![-a / b];
    }
    let degree = c.len() - 1;
    let lead = c[degree].abs().log2();
    let bound = c[..degree]
        .iter()
        .enumerate()
        .map(|(i, a)| ((a.abs().log2() - lead) / (degree - i) as f64 + 1.).exp2())
        .fold(1.0, f64::max)
        .min(f64::MAX);
    let derivative: Vec<_> = c
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, a)| i as f64 * a)
        .collect();
    let mut cuts = vec![-bound];
    cuts.extend(
        real_roots(&derivative)
            .into_iter()
            .filter(|x| x.abs() < bound),
    );
    cuts.push(bound);
    let mut roots = Vec::new();
    for interval in cuts.windows(2) {
        let [mut lo, mut hi] = [interval[0], interval[1]];
        let left = signed_value(c, lo);
        let right = signed_value(c, hi);
        if left == 0. {
            roots.push(lo);
        }
        if left == 0. || right == 0. || left.is_sign_negative() == right.is_sign_negative() {
            continue;
        }
        // A binary64 bracket has at most 2098 exponent and precision bits.
        for _ in 0..2100 {
            let mid = lo / 2. + hi / 2.;
            if mid == lo || mid == hi {
                break;
            }
            let val = signed_value(c, mid);
            if val == 0. {
                lo = mid;
                hi = mid;
                break;
            }
            if val.is_sign_negative() == left.is_sign_negative() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        roots.push(lo / 2. + hi / 2.);
    }
    if signed_value(c, bound) == 0. {
        roots.push(bound);
    }
    roots
}

pub(super) fn inverse(coefficients: &[Real], target: Real, f0: Real, f1: Real) -> RtResult<Real> {
    // The original checks the secant through (0,f0) and (1,f1) before searching
    // for an intervening root. An exact intercept can skip a nearer solution.
    let step = f0.sub(f1)?;
    let guess = if step.is_zero() {
        None
    } else {
        Some(f0.div(step)?)
    };
    if let Some(x) = guess
        && coefficients
            .iter()
            .rev()
            .try_fold(Real::ZERO, |v, &a| v.mul(x)?.add(a))
            .is_ok_and(|v| v == target)
    {
        return Ok(x);
    }
    let exponent = coefficients
        .iter()
        .skip(1)
        .copied()
        .chain([f0])
        .filter(|c| !c.is_zero())
        .map(Real::exp10)
        .max()
        .unwrap();
    let c = [f0]
        .into_iter()
        .chain(coefficients[1..].iter().copied())
        .map(|value| {
            let x = value.scale10(-exponent)?.to_f64();
            if x == 0. && !value.is_zero() {
                return Err(invalid(
                    "PredX",
                    "coefficient range exceeds the root solver precision",
                ));
            }
            Ok(x)
        })
        .collect::<RtResult<Vec<_>>>()?;
    let roots = real_roots(&c);
    let direction = guess.map_or(0.5, Real::to_f64);
    let origin = direction.clamp(0., 1.);
    let distance = |x: f64| {
        (
            (direction < 0. && x > 0.) || (direction > 1. && x < 1.),
            (x - origin).abs(),
        )
    };
    let x = roots
        .into_iter()
        .filter(|x| x.is_finite())
        .min_by(|a, b| {
            let (outside_a, da) = distance(*a);
            let (outside_b, db) = distance(*b);
            outside_a.cmp(&outside_b).then_with(|| da.total_cmp(&db))
        })
        .ok_or_else(|| RuntimeError::new(ErrorKind::Undefined, "Extremum found"))?;
    Ok(Real::from_f64(x)?)
}
