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
        Some(degree) => super::polynomial_roots::inverse(
            &coefficients[..=degree],
            y,
            residual_at_zero,
            residual_at_one,
        ),
    }
}

fn quadratic_x(a: Real, b: Real, c: Real) -> RtResult<Real> {
    let sum_over_one = if a.is_negative() {
        b.cmp_value(a.neg()).is_gt()
    } else {
        b.cmp_value(a.neg()).is_lt()
    };
    // A common decimal scale keeps the discriminant in range even when
    // the coefficients are too large or small to square directly.
    let exponent = [a, b, c]
        .into_iter()
        .filter(|coefficient| !coefficient.is_zero())
        .map(Real::exp10)
        .max()
        .expect("a quadratic has a nonzero leading coefficient");
    let [a, b, c] = [a, b, c].map(|coefficient| {
        let value = coefficient.scale10(-exponent)?.to_f64();
        if !coefficient.is_zero() && value == 0.0 {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                "PredX quadratic coefficients exceed the supported numerical range",
            ));
        }
        Ok(value)
    });
    let (a, b, c) = (a?, b?, c?);
    let cross = 4.0 * a * c;
    let discriminant = b.mul_add(b, -cross);
    let rounding = (b * b + cross.abs()) * f64::EPSILON;
    if discriminant <= rounding {
        let root = (-0.5 * b) / a;
        if discriminant.abs() <= rounding && (0.0..1.0).contains(&root) {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                "PredX repeated root search inside the initial interval is unavailable",
            ));
        }
        return Err(RuntimeError::new(ErrorKind::Undefined, "Extremum found"));
    }

    // Compute the farther root without subtracting nearly equal terms and
    // derive the other from the product of the two roots.
    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
    let (first, second) = (q / a, c / q);
    let (left, right) = (first.min(second), first.max(second));
    // The search starts at 0 and 1. The sum of the roots identifies the
    // closer endpoint when both roots are inside or straddle that interval.
    let root = match (left < 0.0, right > 1.0) {
        (true, true) if sum_over_one => left,
        (true, _) => right,
        (false, false) if sum_over_one => right,
        (false, _) => left,
    };
    Ok(Real::from_f64(root)?)
}
