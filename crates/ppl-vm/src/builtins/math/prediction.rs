//! Spreadsheet regression predictions with calculator argument and coefficient order.

use super::{inverse_trig, nth_root, trig};
use crate::{ErrorKind, RuntimeError, Settings, Value, error::RtResult};
use ppl_compiler::builtins::Builtin;
use ppl_math::Real;

pub(super) fn call(
    builtin: Builtin,
    name: &str,
    args: &[Value],
    settings: &Settings,
) -> RtResult<Value> {
    let [mode, input, coefficients @ ..] = args else {
        return Err(invalid(name, "requires a model, value and coefficients"));
    };
    let mode = mode
        .expect_real(name)?
        .to_i64()
        .ok_or_else(|| invalid(name, "requires an integral regression model"))?;
    let count = match mode {
        1..=6 => 2,
        7 | 9 => 3,
        8 | 10 => 4,
        11 => 5,
        _ => return Err(invalid(name, "unknown regression model")),
    };
    let coefficients = match coefficients {
        [Value::List(values)] => values,
        values => values,
    };
    if coefficients.len() != count {
        return Err(invalid(name, "wrong number of coefficients for this model"));
    }
    let coefficients = coefficients
        .iter()
        .map(|value| value.expect_real(name))
        .collect::<RtResult<Vec<_>>>()?;
    let input = input.expect_real(name)?;
    let value = if builtin == Builtin::PredY {
        predict_y(mode, input, &coefficients, settings)?
    } else {
        predict_x(mode, input, &coefficients, settings)?
    };
    Ok(Value::Real(value))
}

pub(super) fn invalid(name: &str, message: &str) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, format!("{name}: {message}"))
}

fn logistic_y(limit: Real, scale: Real, rate: Real, x: Real) -> RtResult<Real> {
    // Keep extra digits through exp, addition and division before converting
    // to a display real. Use calculator arithmetic outside the binary range.
    let exponential = (-rate.to_f64() * x.to_f64()).exp();
    let denominator = scale.to_f64().mul_add(exponential, 1.0);
    let prediction = limit.to_f64() / denominator;
    if prediction.is_normal() && exponential.is_normal() && denominator.is_normal() {
        return Ok(Real::from_f64(prediction)?);
    }
    let exponential = rate.mul(x)?.neg().exp_e()?;
    Ok(limit.div(Real::ONE.add(scale.mul(exponential)?)?)?)
}

fn predict_y(mode: i64, x: Real, coefficients: &[Real], settings: &Settings) -> RtResult<Real> {
    let a = coefficients[0];
    let b = coefficients[1];
    Ok(match mode {
        1 => a.mul(x)?.add(b)?,
        2 => a.mul(x.ln()?)?.add(b)?,
        3 => b.mul(a.mul(x)?.exp_e()?)?,
        4 => b.mul(x.pow(a)?)?,
        5 => b.mul(a.pow(x)?)?,
        6 => a.div(x)?.add(b)?,
        7 => logistic_y(a, b, coefficients[2], x)?,
        8 => a
            .mul(trig(settings, b.mul(x)?.add(coefficients[2])?, 0)?)?
            .add(coefficients[3])?,
        9..=11 => coefficients
            .iter()
            .rev()
            .try_fold(Real::ZERO, |value, &c| value.mul(x)?.add(c))?,
        _ => unreachable!("regression model is checked before evaluation"),
    })
}

fn predict_x(mode: i64, y: Real, coefficients: &[Real], settings: &Settings) -> RtResult<Real> {
    let a = coefficients[0];
    let b = coefficients[1];
    Ok(match mode {
        1 => y.sub(b)?.div(a)?,
        2 => y.sub(b)?.div(a)?.exp_e()?,
        3 => y.div(b)?.ln()?.div(a)?,
        4 => nth_root(a, y.div(b)?)?,
        5 => y.div(b)?.ln()?.div(a.ln()?)?,
        6 => a.div(y.sub(b)?)?,
        7 => a
            .div(y)?
            .sub(Real::ONE)?
            .div(b)?
            .ln()?
            .neg()
            .div(coefficients[2])?,
        8 => inverse_trig(settings, y.sub(coefficients[3])?.div(a)?, f64::asin, true)?
            .sub(coefficients[2])?
            .div(b)?,
        9..=11 => polynomial_x(y, coefficients)?,
        _ => unreachable!("regression model is checked before evaluation"),
    })
}

fn polynomial_x(y: Real, coefficients: &[Real]) -> RtResult<Real> {
    let residual_at_zero = coefficients[0].sub(y)?;
    if residual_at_zero.is_zero() {
        return Ok(Real::ZERO);
    }
    let residual_at_one = coefficients[1..]
        .iter()
        .try_fold(residual_at_zero, |sum, &coefficient| sum.add(coefficient))?;
    if residual_at_one.is_zero() {
        return Ok(Real::ONE);
    }
    match coefficients
        .iter()
        .rposition(|coefficient| !coefficient.is_zero())
    {
        None | Some(0) => Err(RuntimeError::new(ErrorKind::Undefined, "Constant function")),
        Some(1) => Ok(residual_at_zero.neg().div(coefficients[1])?),
        Some(2) => quadratic_x(coefficients[2], coefficients[1], residual_at_zero),
        Some(degree @ (3 | 4)) => {
            if let [_, linear, square, cubic, quartic] = coefficients
                && linear.is_zero()
                && cubic.is_zero()
                && residual_at_zero.is_negative() == quartic.is_negative()
                && (square.is_zero() || square.is_negative() == quartic.is_negative())
            {
                // Every even term has the sign of the nonzero constant.
                return Err(RuntimeError::new(ErrorKind::Undefined, "Extremum found"));
            }
            if degree == 3
                && let Some(root) = monotone_cubic_x(
                    coefficients[3],
                    coefficients[2],
                    coefficients[1],
                    residual_at_zero,
                )?
            {
                return Ok(root);
            }
            super::polynomial_roots::inverse(
                &coefficients[..=degree],
                y,
                residual_at_zero,
                residual_at_one,
            )
        }
        _ => unreachable!("prediction models supply at most five coefficients"),
    }
}

fn normalized_polynomial<const N: usize>(coefficients: [Real; N]) -> RtResult<Option<[f64; N]>> {
    let exponent = coefficients
        .iter()
        .filter(|coefficient| !coefficient.is_zero())
        .map(|coefficient| coefficient.exp10())
        .max()
        .expect("a polynomial inverse has a nonzero leading coefficient");
    let mut normalized = [0.0; N];
    for (coefficient, result) in coefficients.into_iter().zip(&mut normalized) {
        *result = coefficient.scale10(-exponent)?.to_f64();
        if !coefficient.is_zero() && !result.is_normal() {
            return Ok(None);
        }
    }
    Ok(Some(normalized))
}

fn monotone_cubic_x(a: Real, b: Real, c: Real, d: Real) -> RtResult<Option<Real>> {
    if b.is_zero() && c.is_zero() {
        let exponent = d.exp10() - a.exp10();
        let mantissa = -d.scale10(-d.exp10())?.to_f64() / a.scale10(-a.exp10())?.to_f64();
        let scaled_root = (mantissa * 10.0_f64.powi(exponent.rem_euclid(3))).cbrt();
        return Ok(Some(
            Real::from_f64(scaled_root)?.scale10(exponent.div_euclid(3))?,
        ));
    }
    let range = || {
        RuntimeError::new(
            ErrorKind::Unsupported,
            "PredX cubic coefficients exceed the supported numerical range",
        )
    };
    let normalized = normalized_polynomial([a, b, c, d])?.ok_or_else(range)?;
    let [a, b, c, d] = normalized.map(|coefficient| coefficient * normalized[0].signum());
    // A derivative with negative discriminant guarantees a unique crossing.
    // Leave other cubic shapes to the separate root-selection behavior.
    if c <= 0.0 || b.abs() >= a.sqrt() * (3.0 * c).sqrt() {
        return Ok(None);
    }
    let direction = -d.signum();
    let (b, d) = (b * direction, d * direction);
    let residual = |magnitude: f64| {
        if magnitude <= 1.0 {
            a.mul_add(magnitude, b)
                .mul_add(magnitude, c)
                .mul_add(magnitude, d)
        } else {
            // For large x use the sign of f(x)/x³ to avoid overflow.
            let reciprocal = magnitude.recip();
            d.mul_add(reciprocal, c)
                .mul_add(reciprocal, b)
                .mul_add(reciprocal, a)
        }
    };
    let (mut low, mut high) = (0.0, 1.0);
    while residual(high) < 0.0 {
        if high == f64::MAX {
            return Err(range());
        }
        low = high;
        high = (2.0 * high).min(f64::MAX);
    }
    Ok(Some(Real::from_f64(
        direction * bisect_increasing_root(low, high, residual),
    )?))
}

fn bisect_increasing_root(mut low: f64, mut high: f64, residual: impl Fn(f64) -> f64) -> f64 {
    loop {
        let mid = low + (high - low) / 2.0;
        let value = residual(mid);
        if mid == low || mid == high || value == 0.0 {
            return mid;
        }
        if value < 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
}

fn quadratic_x(a: Real, b: Real, c: Real) -> RtResult<Real> {
    let sum_over_one = if a.is_negative() {
        b.cmp_value(a.neg()).is_gt()
    } else {
        b.cmp_value(a.neg()).is_lt()
    };
    // Scale the discriminant terms, rather than the coefficients. This
    // keeps a*c and b² comparable even when a and c differ by hundreds
    // of decimal orders of magnitude.
    let a_exp = a.exp10();
    let c_exp = c.exp10();
    let cross_exp = a_exp + c_exp;
    let mut exponent = cross_exp.div_euclid(2);
    if !b.is_zero() {
        exponent = exponent.max(b.exp10());
    }
    let qa = a.scale10(-a_exp)?.to_f64();
    let qb = b.scale10(-exponent)?.to_f64();
    let qc = c.scale10(-c_exp)?.to_f64();
    let cross = 4.0 * qa * qc * 10.0_f64.powi(cross_exp - 2 * exponent);
    let discriminant = qb.mul_add(qb, -cross);
    let rounding = (qb * qb + cross.abs()) * f64::EPSILON;
    if discriminant <= rounding {
        let vertex = (-0.5 * qb) / qa;
        if discriminant.abs() <= rounding
            && vertex > 0.0
            && vertex * 10.0_f64.powi(exponent - a_exp) < 1.0
            && let Some(root) = sampled_quadratic_vertex(a, b, c)?
        {
            return Ok(root);
        }
        return Err(RuntimeError::new(ErrorKind::Undefined, "Extremum found"));
    }

    // The stable formula produces the root farther from zero first. Keep
    // each root's exponent separate so only the selected root is rounded.
    let b_sign = if b.is_negative() { -1.0 } else { 1.0 };
    let q = -0.5 * (qb + discriminant.sqrt().copysign(b_sign));
    let first = (q / qa, exponent - a_exp);
    let second = (qc / q, c_exp - exponent);
    let (left, right) = if first.0 < 0.0 {
        (first, second)
    } else {
        (second, first)
    };
    // The search starts at 0 and 1. The sum of the roots identifies the
    // closer endpoint when both roots are inside or straddle that interval.
    let right_over_one = right.0 > 0.0 && right.0 * 10.0_f64.powi(right.1) > 1.0;
    let (mantissa, power) = match (left.0 < 0.0, right_over_one) {
        (true, true) if sum_over_one => left,
        (true, _) => right,
        (false, false) if sum_over_one => right,
        (false, _) => left,
    };
    Ok(Real::from_f64(mantissa)?.scale10(power)?)
}

// Sample on the original 0,1 secant, then interpolate the vertex using divided
// differences at calculator precision. A flat residual alone is ambiguous at
// a repeated zero. Require the same stationary point from the coefficients.
fn sampled_quadratic_vertex(a: Real, b: Real, c: Real) -> RtResult<Option<Real>> {
    let f = |x: Real| a.mul(x)?.add(b)?.mul(x)?.add(c);
    let first_slope = a.add(b)?;
    let half = Real::ONE.div(Real::from_i64(2))?;
    let z = if first_slope.is_zero() {
        half
    } else {
        c.neg().div(first_slope)?
    };
    if z == Real::ONE || z.is_zero() {
        return Ok(None);
    }
    let exact = b.neg().div(a)?.mul(half)?;
    if z == exact && f(z)?.is_zero() {
        return Ok(Some(z));
    }
    let second_slope = f(z)?.sub(f(Real::ONE)?)?.div(z.sub(Real::ONE)?)?;
    let curvature = second_slope.sub(first_slope)?.div(z)?;
    if curvature.is_zero() {
        return Ok(None);
    }
    let vertex = Real::ONE.sub(first_slope.div(curvature)?)?.mul(half)?;
    Ok((vertex == exact && f(vertex)?.is_zero()).then_some(vertex))
}
