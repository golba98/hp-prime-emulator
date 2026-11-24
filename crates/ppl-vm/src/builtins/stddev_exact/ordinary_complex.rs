//! Exact collinear radicals for directly weighted ordinary complex lists.

use std::rc::Rc;

use ppl_cas::{CasValue, NativeProjection};
use ppl_math::Real;

use crate::{Machine, RtResult, Value};

use super::{Moments, coefficient, gcd, integral, render_variance, series, square_factors};

type Sample = ([i128; 2], i128);

pub(crate) fn ordinary_complex_deviation(
    machine: &mut Machine,
    args: &[Value],
) -> RtResult<Option<Value>> {
    let [Value::List(values), weight_arg] = args else {
        return Ok(None);
    };
    if !values.iter().any(nonreal) {
        return Ok(None);
    }
    let Some(weights) = series(weight_arg).filter(|weights| weights.len() == values.len()) else {
        return Ok(None);
    };
    let signed = matches!(weight_arg, Value::List(_));
    let mut samples = Vec::with_capacity(values.len());
    let mut count = 0i128;
    let mut ignored_fractional_pair = false;
    for (value, weight) in values.iter().zip(weights) {
        machine.check_limits()?;
        let Some(weight) = integral(weight).filter(|weight| signed || *weight >= 0) else {
            return Ok(None);
        };
        let Some(point) = integral_point(value) else {
            if weight == 0 && nonreal(value) {
                ignored_fractional_pair = true;
                continue;
            }
            return Ok(None);
        };
        if weight != 0 {
            let Some(total) = count.checked_add(weight) else {
                return Ok(None);
            };
            count = total;
            samples.push((point, weight));
        }
    }
    if count == 0 {
        // The ignored nonreal observation does not change integral cancellation.
        return Ok(Some(super::inline_undefined()));
    }
    if ignored_fractional_pair {
        return Ok(None);
    }
    let Some((direction, numerator, denominator)) = collinear_variance(&samples) else {
        return Ok(None);
    };
    if numerator == 0 {
        return Ok(Some(Value::zero()));
    }
    if direction.contains(&0) {
        return render_variance(machine, direction[0] == 0, numerator, denominator);
    }
    gaussian_radical(machine, direction, numerator, denominator)
}

fn nonreal(value: &Value) -> bool {
    matches!(value, Value::Complex(value) if !value.im.is_zero())
}

fn integral_point(value: &Value) -> Option<[i128; 2]> {
    match value {
        Value::Complex(value) => Some([value.re.to_i128()?, value.im.to_i128()?]),
        value => Some([integral(value)?, 0]),
    }
}

fn collinear_variance(samples: &[Sample]) -> Option<([i128; 2], u128, u128)> {
    let origin = samples.first()?.0;
    let Some((second, _)) = samples.iter().find(|(point, _)| *point != origin) else {
        return Some(([1, 0], 0, 1));
    };
    let dx = second[0].checked_sub(origin[0])?;
    let dy = second[1].checked_sub(origin[1])?;
    let divisor = i128::try_from(gcd(dx.unsigned_abs(), dy.unsigned_abs())).ok()?;
    let [x, y] = [dx / divisor, dy / divisor];
    let mut moments = Moments::default();
    for ([re, im], weight) in samples {
        let re = re.checked_sub(origin[0])?;
        let im = im.checked_sub(origin[1])?;
        let multiple = if x == 0 {
            im.checked_div(y)?
        } else {
            re.checked_div(x)?
        };
        if multiple.checked_mul(x)? != re || multiple.checked_mul(y)? != im {
            return None;
        }
        moments.add(multiple, *weight)?;
    }
    let (negative, numerator, denominator) = moments.variance()?;
    let [x, y] = if negative {
        [y.checked_neg()?, x]
    } else {
        [x, y]
    };
    let direction = if x < 0 || (x == 0 && y < 0) {
        [x.checked_neg()?, y.checked_neg()?]
    } else {
        [x, y]
    };
    Some((direction, numerator, denominator))
}

fn gaussian_radical(
    machine: &mut Machine,
    [real, imaginary]: [i128; 2],
    numerator: u128,
    denominator: u128,
) -> RtResult<Option<Value>> {
    let Some((outside_numerator, inside_numerator)) = square_factors(machine, numerator)? else {
        return Ok(None);
    };
    let Some((outside_denominator, inside_denominator)) = square_factors(machine, denominator)?
    else {
        return Ok(None);
    };
    let Some(radicand) = inside_numerator
        .checked_mul(inside_denominator)
        .and_then(|value| u64::try_from(value).ok())
    else {
        return Ok(None);
    };
    let outside = outside_numerator * outside_denominator;
    let divisor = gcd(outside, denominator);
    let denominator = denominator / divisor;
    let outside = outside / divisor;
    let (Ok(denominator), Ok(scale)) = (u64::try_from(denominator), i128::try_from(outside)) else {
        return Ok(None);
    };
    let scaled = |part: i128| {
        part.checked_mul(scale)
            .and_then(|value| i64::try_from(value).ok())
    };
    let (Some(real_numerator), Some(imaginary_numerator)) = (scaled(real), scaled(imaginary))
    else {
        return Ok(None);
    };
    let (source, display) = if radicand == 1 {
        let Some(expression) = rational_gaussian_expression(
            [real, imaginary],
            outside,
            denominator,
            [real_numerator, imaginary_numerator],
        ) else {
            return Ok(None);
        };
        expression
    } else {
        let coefficient = coefficient(outside);
        let divisor = if denominator == 1 {
            String::new()
        } else {
            format!("/{denominator}")
        };
        let direction = rectangular(real, imaginary);
        let printed_radicand = Real::from_u64(radicand).to_string().replace('E', "ᴇ");
        (
            format!("{coefficient}sqrt({radicand}){divisor}*({direction})"),
            format!(
                "{coefficient}√{printed_radicand}{divisor}*({})",
                direction.replace('i', "\u{e003}")
            ),
        )
    };
    Ok(Some(Value::Symbolic(Rc::new(CasValue::NativeExpression {
        source,
        display,
        approximation: NativeProjection::GaussianRadical {
            real: real_numerator,
            imaginary: imaginary_numerator,
            radicand,
            denominator,
        },
        imaginary: false,
        pair_display: None,
    }))))
}

fn rectangular(real: i128, imaginary: i128) -> String {
    let sign = if imaginary < 0 { '-' } else { '+' };
    format!("{real}{sign}{}i", coefficient(imaginary.unsigned_abs()))
}

fn rational_gaussian_expression(
    [real, imaginary]: [i128; 2],
    scale: u128,
    denominator: u64,
    [real_numerator, imaginary_numerator]: [i64; 2],
) -> Option<(String, String)> {
    // Whole Gaussian values retain the numeric complex result.
    if denominator == 1 {
        return None;
    }
    let norm = real.unsigned_abs().pow(2) + imaginary.unsigned_abs().pow(2);
    let source = if norm == u128::from(denominator) {
        // Division by the conjugate recovers the Gaussian direction. A
        // quarter turn keeps the imaginary term in the divisor negative.
        if imaginary < 0 {
            format!(
                "-{}i/({})",
                coefficient(scale),
                rectangular(-imaginary, -real)
            )
        } else {
            format!("{scale}/({})", rectangular(real, -imaginary))
        }
    } else if gcd(norm, u128::from(denominator)) == 1 {
        format!(
            "({})/{denominator}",
            rectangular(real_numerator.into(), imaginary_numerator.into())
        )
    } else {
        let ([re, im], [divisor_re, divisor_im]) = reduced_gaussian_quotient(
            [real_numerator, imaginary_numerator],
            i64::try_from(denominator).ok()?,
        );
        let numerator = match (re, im) {
            (re, 0) => re.to_string(),
            (0, im) => {
                let sign = if im < 0 { "-" } else { "" };
                format!("{sign}{}i", coefficient(im.unsigned_abs()))
            }
            (re, im) => format!("({})", rectangular(re, im)),
        };
        format!("{numerator}/({})", rectangular(divisor_re, divisor_im))
    };
    let mut display = source.replace('i', "\u{e003}");
    if source.starts_with('-') && scale > 1 {
        display.replace_range(..1, "−");
    } else if source.starts_with("(-") {
        display.replace_range(1..2, "−");
    }
    Some((source, display))
}

/// Cancel the Gaussian gcd, then choose the associate with a positive real
/// part and nonpositive imaginary part in the divisor.
fn reduced_gaussian_quotient(numerator: [i64; 2], denominator: i64) -> ([i128; 2], [i128; 2]) {
    let numerator = numerator.map(i128::from);
    let denominator = [i128::from(denominator), 0];
    let (mut dividend, mut divisor) = (numerator, denominator);
    // Starting with a real i64 divisor bounds the squared norms of all
    // remainders by its square, so the arithmetic fits in i128.
    while divisor != [0, 0] {
        let [q_re, q_im] = gaussian_quotient(dividend, divisor);
        let [d_re, d_im] = divisor;
        let remainder = [
            dividend[0] - (q_re * d_re - q_im * d_im),
            dividend[1] - (q_re * d_im + q_im * d_re),
        ];
        (dividend, divisor) = (divisor, remainder);
    }
    let mut numerator = gaussian_quotient(numerator, dividend);
    let mut denominator = gaussian_quotient(denominator, dividend);
    while denominator[0] <= 0 || denominator[1] > 0 {
        numerator = [-numerator[1], numerator[0]];
        denominator = [-denominator[1], denominator[0]];
    }
    (numerator, denominator)
}

fn gaussian_quotient([a, b]: [i128; 2], [c, d]: [i128; 2]) -> [i128; 2] {
    let norm = c * c + d * d;
    // Round each component to the nearest integer. Euclidean residues make
    // the same error bound apply on both sides of zero.
    let nearest = |part: i128| part.div_euclid(norm) + i128::from(part.rem_euclid(norm) > norm / 2);
    [nearest(a * c + b * d), nearest(b * c - a * d)]
}
