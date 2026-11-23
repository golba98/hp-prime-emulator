//! Spreadsheet regression predictions with calculator argument and coefficient order.

use super::{inverse_trig, nth_root, trig};
use crate::{ErrorKind, RuntimeError, Settings, Value, error::RtResult};
use ppl_compiler::builtins::Builtin;
use ppl_math::{NumericVersion, Real, RealOrigin};

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

fn invalid(name: &str, message: &str) -> RuntimeError {
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
            let polynomial = [
                residual_at_zero,
                coefficients[1],
                coefficients[2],
                coefficients[3],
                coefficients.get(4).copied().unwrap_or(Real::ZERO),
            ];
            if let Some(root) = exact_first_secant(polynomial, residual_at_one) {
                return Ok(root);
            }
            if let Some(root) = unique_initial_root(polynomial, residual_at_one)? {
                return Ok(root);
            }
            Err(RuntimeError::new(
                ErrorKind::Unsupported,
                "PredX root selection for this polynomial is unavailable",
            ))
        }
        _ => unreachable!("prediction models supply at most five coefficients"),
    }
}

fn exact_first_secant(polynomial: [Real; 5], residual_at_one: Real) -> Option<Real> {
    let rational = |value: Real| matches!(value.origin(), RealOrigin::Rational { .. });
    let mut coefficients = [Real::ZERO; 5];
    for (input, coeff) in polynomial.into_iter().zip(&mut coefficients) {
        *coeff = if rational(input) {
            input
        } else {
            if matches!(input.origin(), RealOrigin::PiMultiple { .. }) {
                return None;
            }
            Real::parse_versioned(&input.decimal().to_string(), NumericVersion::Current15751)
                .filter(|value| rational(*value))?
        };
    }
    let [constant, linear, tail @ ..] = coefficients;
    let delta = tail.into_iter().try_fold(linear, |sum, term| {
        sum.add(term).ok().filter(|value| rational(*value))
    })?;
    if constant.add(delta).ok()?.cmp_value(residual_at_one).is_ne() {
        return None;
    }
    let root = constant
        .neg()
        .div(delta)
        .ok()
        .filter(|value| rational(*value))?;
    let residual =
        coefficients
            .iter()
            .rev()
            .skip(1)
            .try_fold(coefficients[4], |term, coefficient| {
                let product = term.mul(root).ok().filter(|value| rational(*value))?;
                product
                    .add(*coefficient)
                    .ok()
                    .filter(|value| rational(*value))
            })?;
    residual.is_zero().then(|| root.decimal())
}

fn unique_initial_root(polynomial: [Real; 5], residual_at_one: Real) -> RtResult<Option<Real>> {
    if polynomial[0].is_negative() == residual_at_one.is_negative() {
        return Ok(None);
    }
    let Some([d, c, b, a, leading]) = normalized_polynomial(polynomial)? else {
        return Ok(None);
    };
    let direction = -d.signum();
    // Same-sign Bernstein coefficients certify the derivative on [0, 1].
    // Degree elevation makes the same test valid for cubic residuals.
    let derivative = [
        c,
        c + (2.0 / 3.0) * b,
        c + (4.0 / 3.0) * b + a,
        c + 2.0 * b + 3.0 * a + 4.0 * leading,
    ];
    let rounding =
        8.0 * f64::EPSILON * (c.abs() + 2.0 * b.abs() + 3.0 * a.abs() + 4.0 * leading.abs());
    let monotone = derivative
        .into_iter()
        .all(|value| direction * value > rounding);
    // Opposite endpoint derivatives certify one stationary point for a cubic.
    // Its two monotone pieces meet at an extremum, so only one can cross zero
    // when the residual has opposite signs at the ends of the interval.
    let [first, _, _, last] = derivative;
    let single_turn = first.abs() > rounding
        && last.abs() > rounding
        && first.is_sign_negative() != last.is_sign_negative();
    if !monotone && (leading != 0.0 || !(single_turn || cubic_has_one_real_root(a, b, c, d))) {
        return Ok(None);
    }
    let residual = |x: f64| {
        direction
            * leading
                .mul_add(x, a)
                .mul_add(x, b)
                .mul_add(x, c)
                .mul_add(x, d)
    };
    if residual(1.0) <= 0.0 {
        return Ok(None);
    }
    Ok(Some(Real::from_f64(bisect_increasing_root(
        0.0, 1.0, residual,
    ))?))
}

fn cubic_has_one_real_root(a: f64, b: f64, c: f64, d: f64) -> bool {
    // A negative discriminant certifies one real root even when the
    // derivative changes sign in the initial bracket. The coefficients
    // have a common scale. Leave near-cancellation and underflow ambiguous.
    let terms = [
        b * b * c * c,
        -4.0 * a * c * c * c,
        -4.0 * b * b * b * d,
        -27.0 * a * a * d * d,
        18.0 * a * b * c * d,
    ];
    let rounding = 32.0 * f64::EPSILON * terms.iter().map(|term| term.abs()).sum::<f64>();
    terms.into_iter().sum::<f64>() < -rounding.max(f64::MIN_POSITIVE)
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
