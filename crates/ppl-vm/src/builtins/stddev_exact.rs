//! Exact real population deviation for direct lowercase stddev calls.

use std::rc::Rc;

use ppl_cas::{CasValue, InfinitySign, NativeProjection};
use ppl_compiler::builtins::Builtin;
use ppl_math::{Complex, Real};
use ppl_syntax::ast::BinOp;

use crate::value::Matrix;
use crate::{ErrorKind, Machine, RtResult, RuntimeError, Value};

mod ordinary_complex;
pub(crate) use ordinary_complex::ordinary_complex_deviation;

fn inline_undefined() -> Value {
    crate::cas::from_cas(CasValue::Undefined)
}

pub(crate) fn is_inline_undefined(value: &Value) -> bool {
    matches!(value, Value::Symbolic(cas) if matches!(cas.as_ref(), CasValue::Undefined))
}

fn is_inline_infinite(value: &Value) -> bool {
    matches!(value, Value::Symbolic(cas) if matches!(cas.as_ref(), CasValue::Infinity(_)))
}

pub(crate) fn inline_arithmetic(
    op: BinOp,
    left: &Value,
    right: &Value,
    literal_zero_quotient: bool,
) -> Option<RtResult<Value>> {
    if literal_zero_quotient || is_inline_undefined(left) || is_inline_undefined(right) {
        return Some(Ok(inline_undefined()));
    }
    if op == BinOp::Div
        && right.as_real().is_some_and(Real::is_zero)
        && let Some(numerator) = left.as_real()
    {
        return Some(Ok(if numerator.is_zero() {
            inline_undefined()
        } else {
            let sign = if numerator.is_negative() {
                InfinitySign::Negative
            } else {
                InfinitySign::Positive
            };
            crate::cas::from_cas(CasValue::Infinity(sign))
        }));
    }
    crate::ops::infinity::binary(op, left, right).map(|result| {
        result.or_else(|error| {
            if error.kind == ErrorKind::Undefined {
                Ok(inline_undefined())
            } else {
                Err(error)
            }
        })
    })
}

pub(crate) fn inline_pair(real: &Value, imaginary: &Value) -> Option<Value> {
    if is_inline_undefined(real) || is_inline_undefined(imaginary) {
        return Some(inline_undefined());
    }
    // Encoded observations keep the infinity through matrix materialization.
    // Its component is immaterial to signed population deviation.
    if is_inline_infinite(real) && (imaginary.as_real().is_some() || is_inline_infinite(imaginary))
    {
        Some(real.clone())
    } else if is_inline_infinite(imaginary) && real.as_real().is_some() {
        Some(imaginary.clone())
    } else {
        None
    }
}

pub(crate) fn inline_collection(
    machine: &mut Machine,
    entries: [&[Value]; 2],
    shapes: [[u32; 3]; 2],
    diagnostic: Value,
) -> RtResult<Value> {
    // The compiler has already evaluated every source cell before a
    // length mismatch is reported.
    if entries[0].len() != entries[1].len() {
        return Ok(diagnostic);
    }
    let mut infinite = [Vec::new(), Vec::new()];
    let mut evaluated = entries.map(|cells| Vec::with_capacity(cells.len()));
    for ((entries, cells), infinite) in entries.into_iter().zip(&mut evaluated).zip(&mut infinite) {
        for (index, value) in entries.iter().enumerate() {
            machine.check_limits()?;
            if is_inline_infinite(value) {
                infinite.push(index);
                cells.push(Value::zero());
            } else if let Ok(value) = crate::machine::numeric_entry(value.clone()) {
                cells.push(value);
            } else {
                return Ok(diagnostic);
            }
        }
    }
    if infinite.iter().all(Vec::is_empty) {
        if shapes[0][0] == 0 {
            return encoded_vector_deviation(machine, evaluated, shapes);
        }
        let matrices = shapes
            .into_iter()
            .zip(evaluated)
            .map(|([_, rows, cols], data)| {
                Value::Matrix(Rc::new(Matrix {
                    rows: rows as usize,
                    cols: cols as usize,
                    vector: false,
                    data,
                }))
            })
            .collect();
        return encoded_complex_matrix(machine, matrices);
    }
    if let ([index], []) = (infinite[0].as_slice(), infinite[1].as_slice()) {
        let Some(weights) = entries[1]
            .iter()
            .map(Value::as_real)
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(diagnostic);
        };
        // The shape fields take part in the calculation as ordinary weights.
        // A negative infinite observation weight yields +Inf while the
        // total carrier weight stays positive.
        let shape_weight = shapes[1].iter().map(|&field| u64::from(field)).sum::<u64>();
        let total = weights
            .iter()
            .try_fold(Real::from_u64(shape_weight), |sum, &weight| sum.add(weight))?;
        if weights[*index].is_negative() && !total.is_zero() && !total.is_negative() {
            return Ok(crate::cas::from_cas(CasValue::Infinity(
                InfinitySign::Positive,
            )));
        }
    }
    Ok(inline_undefined())
}

fn encoded_vector_deviation(
    machine: &mut Machine,
    mut entries: [Vec<Value>; 2],
    shapes: [[u32; 3]; 2],
) -> RtResult<Value> {
    let [observations, frequencies] = &mut entries;
    machine.check_len(observations.len().saturating_add(3))?;
    // Integral pairs with a zero frequency keep the vector eligible for exact arithmetic.
    for (value, frequency) in observations.iter_mut().zip(frequencies) {
        if matches!(value, Value::Complex(pair) if pair.re.is_integer() && pair.im.is_integer())
            && frequency.as_real().is_some_and(Real::is_zero)
        {
            *value = Value::zero();
        }
    }
    let encoded: Vec<_> = shapes
        .into_iter()
        .zip(entries)
        .map(|(fields, cells)| {
            let fields = fields.map(|field| Value::Real(Real::from_u64(u64::from(field))));
            Value::list(fields.into_iter().chain(cells).collect())
        })
        .collect();
    if let [Value::List(values), weights] = encoded.as_slice()
        && values
            .iter()
            .any(|value| matches!(value, Value::Complex(_)))
    {
        return super::math::complex_list_deviation(
            machine,
            values,
            Some(weights),
            super::spec(Builtin::Stddev).name,
            true,
        );
    }
    encoded_series_deviation(machine, encoded)
}

pub(crate) fn encoded_complex_matrix(machine: &mut Machine, args: Vec<Value>) -> RtResult<Value> {
    if let [Value::Matrix(data), Value::Matrix(weights)] = args.as_slice()
        && !data.vector
        && !weights.vector
        && data.data.len() == weights.data.len()
        && data.data.iter().zip(&weights.data).all(|(value, weight)| {
            weight
                .as_real()
                .is_some_and(|weight| weight.is_zero() || !matches!(value, Value::Complex(_)))
        })
    {
        machine.check_len(data.data.len().saturating_add(3))?;
        // Both matrices are evaluated. Their zero-imaginary cells are real.
        // Ignored integral pairs can be replaced by zero for exact arithmetic;
        // fractional pairs still affect the numeric and cancellation types.
        let observations = data.data.iter().map(|value| match value {
            Value::Complex(pair) if pair.re.is_integer() && pair.im.is_integer() => Value::zero(),
            _ => value.clone(),
        });
        let encoded = vec![
            Value::list(
                data.encoding_prefix()
                    .into_iter()
                    .chain(observations)
                    .collect(),
            ),
            Value::list(
                weights
                    .encoding_prefix()
                    .into_iter()
                    .chain(weights.data.iter().cloned())
                    .collect(),
            ),
        ];
        return encoded_series_deviation(machine, encoded);
    }
    super::call(machine, Builtin::Stddev, args)
}

fn encoded_series_deviation(machine: &mut Machine, encoded: Vec<Value>) -> RtResult<Value> {
    match deviation(machine, &encoded, false, false)? {
        Some(value) => Ok(value),
        None => super::call(machine, Builtin::Stddev, encoded),
    }
}

fn series(value: &Value) -> Option<&[Value]> {
    match value {
        Value::List(values) => Some(values),
        Value::Matrix(value) if value.vector => Some(&value.data),
        _ => None,
    }
}

fn integral(value: &Value) -> Option<i128> {
    match value {
        Value::Real(value) => value.to_i128(),
        Value::Int(value) => Some(value.numeric_value()),
        _ => None,
    }
}

#[derive(Default)]
struct Moments {
    count: i128,
    sum: i128,
    squares: i128,
}

impl Moments {
    fn add(&mut self, delta: i128, weight: i128) -> Option<()> {
        let term = weight.checked_mul(delta)?;
        self.count = self.count.checked_add(weight)?;
        self.sum = self.sum.checked_add(term)?;
        self.squares = self.squares.checked_add(term.checked_mul(delta)?)?;
        Some(())
    }

    fn variance(self) -> Option<(bool, u128, u128)> {
        let numerator = self
            .count
            .checked_mul(self.squares)?
            .checked_sub(self.sum.checked_mul(self.sum)?)?;
        let magnitude = numerator.unsigned_abs();
        let denominator = self.count.unsigned_abs().checked_pow(2)?;
        if denominator == 0 {
            return None;
        }
        let divisor = gcd(magnitude, denominator);
        Some((numerator < 0, magnitude / divisor, denominator / divisor))
    }
}

fn variance(
    values: &[Value],
    weights: Option<&[Value]>,
    encoded_vector: bool,
) -> Option<(bool, u128, u128)> {
    let origin = integral(values.first()?)?;
    let mut moments = Moments::default();
    if encoded_vector {
        // The original's direct vector-expression carrier includes these
        // values before the vector cells. Stored vectors have no such prefix.
        for value in [0, 1, values.len() as i128] {
            moments.add(value.checked_sub(origin)?, 1)?;
        }
    }
    for (index, value) in values.iter().enumerate() {
        let weight = match weights {
            Some(weights) => integral(&weights[index])?,
            None => 1,
        };
        moments.add(integral(value)?.checked_sub(origin)?, weight)?;
    }
    moments.variance()
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Returns the coefficient and square-free part of a nonnegative integer.
/// Once p³ exceeds the residual, at most two prime factors can remain. They
/// either form a perfect square or both belong under the radical.
fn square_factors(machine: &mut Machine, mut value: u128) -> RtResult<Option<(u128, u128)>> {
    let (mut coefficient, mut radicand) = (1, 1);
    let mut factor: u128 = 2;
    while factor.pow(3) <= value {
        if factor > 2_000_000 {
            return Ok(None);
        }
        if factor == 2 || factor % 4096 == 1 {
            machine.check_limits()?;
            let root = value.isqrt();
            if root * root == value {
                return Ok(Some((coefficient * root, radicand)));
            }
        }
        while value % (factor * factor) == 0 {
            coefficient *= factor;
            value /= factor * factor;
        }
        if value % factor == 0 {
            radicand *= factor;
            value /= factor;
        }
        factor = if factor == 2 { 3 } else { factor + 2 };
    }
    let root = value.isqrt();
    Ok(Some(if root * root == value {
        (coefficient * root, radicand)
    } else {
        (coefficient, radicand * value)
    }))
}

pub(crate) fn deviation(
    machine: &mut Machine,
    args: &[Value],
    encoded_vector: bool,
    evaluated_matrices: bool,
) -> RtResult<Option<Value>> {
    let Some(values) = args.first().and_then(series) else {
        return if evaluated_matrices {
            super::math::evaluated_matrix_deviation(machine, args)
        } else {
            Ok(None)
        };
    };
    let signed_lists = matches!(args, [Value::List(_), Value::List(_)]);
    let weights = if let Some(arg) = args.get(1) {
        let Some(weights) = series(arg).filter(|weights| weights.len() == values.len()) else {
            return Ok(None);
        };
        if !signed_lists
            && weights
                .iter()
                .any(|weight| weight.as_real().is_some_and(Real::is_negative))
        {
            return Ok(None);
        }
        Some(weights)
    } else {
        None
    };
    machine.check_limits()?;
    let Some((imaginary, numerator, denominator)) = variance(values, weights, encoded_vector)
    else {
        return Ok(None);
    };
    if imaginary && !signed_lists {
        return Ok(None);
    }
    render_variance(machine, imaginary, numerator, denominator)
}

fn render_variance(
    machine: &mut Machine,
    imaginary: bool,
    numerator: u128,
    denominator: u128,
) -> RtResult<Option<Value>> {
    let (numerator_root, denominator_root) = (numerator.isqrt(), denominator.isqrt());
    if numerator_root * numerator_root == numerator
        && denominator_root * denominator_root == denominator
    {
        if denominator_root == 1 {
            let magnitude = Real::from_parts(false, numerator_root, 0)?;
            return Ok(Some(numeric_projection(magnitude, imaginary)));
        }
        let approx = Real::from_f64(numerator_root as f64 / denominator_root as f64)?;
        if imaginary {
            let coefficient = coefficient(numerator_root);
            return Ok(Some(symbolic(
                format!("{coefficient}i/{denominator_root}"),
                format!("{coefficient}\u{e003}/{denominator_root}"),
                Some(format!("(0,{numerator_root})/{denominator_root}")),
                approx,
                true,
            )));
        }
        let fraction = format!("{numerator_root}/{denominator_root}");
        return Ok(Some(symbolic(
            fraction.clone(),
            fraction,
            None,
            approx,
            false,
        )));
    }
    // Preserve the firmware's rationalization order. It reduces the variance
    // first, then extracts squares from sqrt(n*d)/d without cancelling the
    // coefficient again. The reduced numerator and denominator are coprime.
    let Some((outside_numerator, inside_numerator)) = square_factors(machine, numerator)? else {
        return Ok(None);
    };
    let Some((outside_denominator, inside_denominator)) = square_factors(machine, denominator)?
    else {
        return Ok(None);
    };
    let Some(radicand) = inside_numerator.checked_mul(inside_denominator) else {
        return Ok(None);
    };
    let outside = outside_numerator * outside_denominator;
    let coefficient = coefficient(outside);
    let divisor = if denominator == 1 {
        String::new()
    } else {
        format!("/{denominator}")
    };
    let display_radicand = Real::from_parts(false, radicand, 0)?
        .to_string()
        .replace('E', "ᴇ");
    let approximation = Real::from_f64((numerator as f64 / denominator as f64).sqrt())?;
    let (source_i, display_i) = if imaginary {
        ("i*", "\u{e003}*")
    } else {
        ("", "")
    };
    Ok(Some(symbolic(
        format!("{coefficient}{source_i}sqrt({radicand}){divisor}"),
        format!("{coefficient}{display_i}√{display_radicand}{divisor}"),
        imaginary.then(|| format!("(0,{outside})*√{display_radicand}{divisor}")),
        approximation,
        imaginary,
    )))
}

fn coefficient(value: u128) -> String {
    if value == 1 {
        String::new()
    } else {
        format!("{value}*")
    }
}

fn numeric_projection(magnitude: Real, imaginary: bool) -> Value {
    if imaginary {
        Value::Complex(Complex::new(Real::ZERO, magnitude))
    } else {
        Value::Real(magnitude)
    }
}

fn symbolic(
    source: String,
    display: String,
    pair_display: Option<String>,
    approximation: Real,
    imaginary: bool,
) -> Value {
    Value::Symbolic(Rc::new(CasValue::NativeExpression {
        source,
        display,
        approximation: NativeProjection::Scalar(approximation.to_string()),
        imaginary,
        pair_display,
    }))
}

pub(crate) fn symbolic_math(builtin: Builtin, args: &[Value]) -> Option<RtResult<Value>> {
    let Value::Symbolic(value) = args.first()? else {
        return None;
    };
    let CasValue::NativeExpression {
        source,
        display,
        approximation,
        imaginary,
        pair_display,
    } = value.as_ref()
    else {
        return None;
    };
    let approximation = match approximation {
        NativeProjection::Scalar(text) => Real::parse(text)?,
        NativeProjection::GaussianRadical {
            real,
            imaginary,
            radicand,
            denominator,
        } => {
            return (builtin == Builtin::Approx).then(|| {
                let scale = Real::from_u64(*radicand)
                    .sqrt()?
                    .div(Real::from_u64(*denominator))?;
                Ok(Value::Complex(Complex::new(
                    Real::from_i64(*real).mul(scale)?,
                    Real::from_i64(*imaginary).mul(scale)?,
                )))
            });
        }
    };
    let wrap = |name: &str, suffix: &str, magnitude: Real, imaginary| {
        let source_name = name.to_ascii_lowercase();
        let display_suffix = suffix.replace('-', "−");
        symbolic(
            format!("{source_name}({source}{suffix})"),
            format!("{name}({display}{display_suffix})"),
            pair_display
                .as_ref()
                .map(|text| format!("{name}({text}{display_suffix})")),
            magnitude,
            imaginary,
        )
    };
    match builtin {
        Builtin::Approx => Some(Ok(numeric_projection(approximation.decimal(), *imaginary))),
        Builtin::Abs => Some(Ok(wrap("ABS", "", approximation.abs(), false))),
        Builtin::Re if pair_display.is_some() => Some(Ok(wrap(
            "RE",
            "",
            if *imaginary {
                Real::ZERO
            } else {
                approximation
            },
            false,
        ))),
        Builtin::Im if pair_display.is_some() => Some(Ok(wrap(
            "IM",
            "",
            if *imaginary {
                approximation
            } else {
                Real::ZERO
            },
            false,
        ))),
        Builtin::Round => {
            let (places, suffix) = if let Some(arg) = args.get(1) {
                let Some(places) = arg
                    .as_real()
                    .and_then(Real::to_i64)
                    .filter(|places| (-12..=12).contains(places))
                else {
                    return Some(Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "Precision must be an integer from -12 through 12",
                    )));
                };
                (places as i32, format!(",{places}"))
            } else {
                (0, String::new())
            };
            let rounded = approximation.round_places(places);
            Some(Ok(wrap(
                "ROUND",
                &suffix,
                rounded,
                *imaginary && !rounded.is_zero(),
            )))
        }
        _ => None,
    }
}
