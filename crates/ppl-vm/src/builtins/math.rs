//! Numeric commands and integer/bit operations.

use ppl_compiler::builtins::Builtin;
use ppl_math::{Complex, HpInt, IntBase, MathError, Real, RealPresentation};
use ppl_syntax::ast::BinOp;

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::ops::{self, map_numeric, norm_complex};
use crate::settings::{AngleMode, Settings};
use crate::value::{Matrix, Value};

mod polynomial_roots;
mod prediction;

/// SUM and AVERAGE flatten list arguments, while matrices are single operands.
pub fn sheet_aggregate(
    args: &[Value],
    settings: &Settings,
    average: bool,
    mut check: impl FnMut() -> RtResult<()>,
) -> RtResult<Value> {
    let mut pending = vec![args.iter()];
    let mut sum = None;
    let mut count = 0i64;
    let mut visits = 0usize;
    while let Some(items) = pending.last_mut() {
        visits += 1;
        if visits % 128 == 0 {
            check()?;
        }
        match items.next() {
            None => {
                pending.pop();
            }
            Some(Value::List(nested)) => pending.push(nested.iter()),
            Some(value) => {
                sum = Some(match sum {
                    None => value.clone(),
                    Some(previous) => ops::binary(BinOp::Add, &previous, value, settings)?,
                });
                count += 1;
            }
        }
    }
    let sum = sum.unwrap_or_else(Value::zero);
    if !average || count == 1 {
        return Ok(sum);
    }
    ops::binary(BinOp::Div, &sum, &Value::from_i64(count), settings)
}

fn real_map(v: &Value, f: impl Fn(Real) -> RtResult<Real>) -> RtResult<Value> {
    map_numeric(v, &mut |x| Ok(Value::Real(f(x.expect_real("argument")?)?)))
}

fn analytic_map(
    value: &Value,
    real: impl Fn(Real) -> RtResult<Real>,
    complex: impl Fn(Complex) -> Result<Complex, MathError>,
) -> RtResult<Value> {
    map_numeric(value, &mut |value| match value {
        Value::Complex(value) => Ok(Value::Complex(complex(*value)?)),
        value => Ok(Value::Real(real(value.expect_real("argument")?)?)),
    })
}

fn sexagesimal_map(value: &Value, presentation: RealPresentation) -> RtResult<Value> {
    match value {
        Value::List(items) => items
            .iter()
            .map(|value| sexagesimal_map(value, presentation))
            .collect::<RtResult<Vec<_>>>()
            .map(Value::list),
        Value::Matrix(_) => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            "Hexagesimal conversion accepts real values or lists, not matrices",
        )),
        value => Ok(Value::Real(
            value
                .expect_real("hexagesimal conversion")?
                .with_presentation(presentation),
        )),
    }
}

fn f64_map(v: &Value, f: impl Fn(f64) -> f64) -> RtResult<Value> {
    real_map(v, |x| Ok(Real::from_f64(f(x.to_f64()))?))
}

pub fn sqrt(m: &Machine, v: &Value) -> RtResult<Value> {
    let complex = m.arithmetic_settings().complex_results;
    map_numeric(v, &mut |x| match x {
        Value::Complex(c) => {
            let root = c.sqrt()?;
            Ok(Value::Complex(root))
        }
        other => {
            let r = other.expect_real("√")?;
            match r.sqrt() {
                Ok(s) => Ok(Value::Real(s.with_presentation(r.presentation()))),
                Err(MathError::NonReal) if complex => {
                    Ok(Value::Complex(Complex::new(Real::ZERO, r.neg().sqrt()?)))
                }
                Err(e) => Err(e.into()),
            }
        }
    })
}

/// Reduces an angle in the current mode and returns exact results for
/// multiples of a quarter turn, so `SIN(180)` is 0 in degree mode.
fn trig(s: &Settings, x: Real, which: u8) -> RtResult<Real> {
    let mode = if x.presentation() == RealPresentation::Sexagesimal {
        AngleMode::Degrees
    } else {
        s.angle
    };
    let turn = match mode {
        AngleMode::Degrees => Some(Real::from_i64(360)),
        AngleMode::Gradians => Some(Real::from_i64(400)),
        AngleMode::Radians => None,
    };
    if let Some(turn) = turn {
        let r = ops::real_mod(x, turn)?;
        let quarter = turn.div(Real::from_i64(4))?;
        let q = r.div(quarter)?;
        if q.is_integer() {
            let k = q.to_i64().unwrap_or(0) % 4;
            let (sn, cs): (i64, i64) = [(0, 1), (1, 0), (0, -1), (-1, 0)][k as usize];
            return match which {
                0 => Ok(Real::from_i64(sn)),
                1 => Ok(Real::from_i64(cs)),
                _ => {
                    if cs == 0 {
                        Err(RuntimeError::new(
                            ErrorKind::Undefined,
                            "TAN of an odd multiple of 90°",
                        ))
                    } else {
                        Ok(Real::from_i64(sn * cs))
                    }
                }
            };
        }
    }
    let rad = if mode == AngleMode::Degrees {
        x.to_f64().to_radians()
    } else {
        s.to_radians(x)
    };
    let v = match which {
        0 => rad.sin(),
        1 => rad.cos(),
        _ => rad.tan(),
    };
    Ok(Real::from_f64(v)?)
}

/// Native Home polar-complex entry. Magnitude multiplication remains decimal
/// across the full BCD exponent range. Explicit complex entry is allowed even
/// when HComplex disables implicit complex results from real-domain functions.
pub(crate) fn polar(magnitude: Real, angle: Real, settings: &Settings) -> RtResult<Complex> {
    if magnitude.is_zero() {
        return Ok(Complex::new(Real::ZERO, Real::ZERO)
            .with_presentation(ppl_math::ComplexPresentation::Polar));
    }
    let real = magnitude.mul(trig(settings, angle, 1)?)?;
    let imaginary = magnitude.mul(trig(settings, angle, 0)?)?;
    Ok(Complex::new(real, imaginary).with_presentation(ppl_math::ComplexPresentation::Polar))
}

fn inverse_trig(s: &Settings, x: Real, f: fn(f64) -> f64, domain_check: bool) -> RtResult<Real> {
    let v = x.to_f64();
    if domain_check && !(-1.0..=1.0).contains(&v) {
        return Err(RuntimeError::kind(ErrorKind::NonReal));
    }
    Ok(Real::from_f64(s.from_radians(f(v)))?)
}

fn nth_root(n: Real, x: Real) -> RtResult<Real> {
    if x.is_negative() {
        let k = n.to_i64().ok_or(RuntimeError::kind(ErrorKind::NonReal))?;
        if k % 2 == 0 {
            return Err(RuntimeError::kind(ErrorKind::NonReal));
        }
        return Ok(x.neg().pow(Real::ONE.div(n)?)?.neg());
    }
    Ok(x.pow(Real::ONE.div(n)?)?)
}

fn real_integer(value: &Value, what: &str) -> RtResult<i128> {
    if let Value::Int(word) = value {
        return Ok(word.numeric_value());
    }
    let value = value.expect_real(what)?;
    if !value.is_integer() {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            format!("{what} requires an integral operand"),
        ));
    }
    value.to_i128().ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::BadArgValue,
            format!("{what}: integer is outside the conversion range"),
        )
    })
}
fn word_argument(value: &Value, what: &str) -> RtResult<i64> {
    i64::try_from(real_integer(value, what)?).map_err(|_| {
        RuntimeError::new(
            ErrorKind::BadArgValue,
            format!("{what} is outside the signed64-bit range"),
        )
    })
}
fn real_bits(value: &Value) -> RtResult<u64> {
    if matches!(value, Value::Int(_)) {
        return Err(RuntimeError::new(
            ErrorKind::BadArgType,
            "BITAND/BITOR/BITXOR/BITNOT require real operands; use word AND/OR/XOR/NOT for integers",
        ));
    }
    let value = real_integer(value, "bit operation")?;
    u64::try_from(value).map_err(|_| {
        RuntimeError::new(
            ErrorKind::BadArgValue,
            "Real bit operands must be nonnegative and fit64 bits",
        )
    })
}
fn word(value: i128, width: i64, base: IntBase) -> RtResult<HpInt> {
    HpInt::new_with_width(value, width, base).ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::BadArgValue,
            "Word width must be1..64 unsigned or -1..-63 signed",
        )
    })
}

fn base_code(b: IntBase) -> i64 {
    match b {
        IntBase::Bin => 1,
        IntBase::Oct => 2,
        IntBase::Dec => 3,
        IntBase::Hex => 4,
    }
}

fn reduce_list(args: &[Value]) -> RtResult<Vec<Real>> {
    let items: Vec<Value> = match args {
        [Value::List(l)] => l.iter().cloned().collect(),
        _ => args.to_vec(),
    };
    items.iter().map(|v| v.expect_real("MAX/MIN")).collect()
}

fn stats_values(args: &[Value], name: &str) -> RtResult<Vec<Real>> {
    let data = arg(args, 0, name)?.expect_list(name)?;
    let freq = match args.get(1) {
        Some(f) => Some(f.expect_list(name)?),
        None => None,
    };
    let mut out = Vec::new();
    for (i, v) in data.iter().enumerate() {
        let r = v.expect_real(name)?;
        let n = match &freq {
            Some(f) => f
                .get(i)
                .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?
                .expect_i64(name)?,
            None => 1,
        };
        for _ in 0..n.max(0) {
            out.push(r);
        }
    }
    if out.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::InvalidDimension,
            format!("{name} of an empty list"),
        ));
    }
    Ok(out)
}

fn numeric_series(value: &Value, name: &str) -> RtResult<Vec<Real>> {
    match value {
        Value::List(values) => values.iter().map(|v| v.expect_real(name)).collect(),
        Value::Matrix(matrix) if matrix.vector => {
            matrix.data.iter().map(|v| v.expect_real(name)).collect()
        }
        _ => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("{name} requires a list or vector"),
        )),
    }
}
fn nonnegative_stddev_weights() -> RuntimeError {
    RuntimeError::new(ErrorKind::BadArgValue, "STDDEV weights must be nonnegative")
}

fn stddev_matrix_shape_error() -> RuntimeError {
    RuntimeError::new(
        ErrorKind::BadArgValue,
        "A STDDEV weight matrix must match the data shape",
    )
}

/// Population deviation without expanding weights or squaring huge or tiny
/// unscaled values. Signed weights can cancel or give negative variance.
fn weighted_deviation(
    values: &[Real],
    weights: Option<&[Real]>,
    allow_signed: bool,
) -> RtResult<Option<Value>> {
    if weights.is_some_and(|weights| weights.len() != values.len()) {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    let weights = weights.map_or_else(|| vec![Real::ONE; values.len()], <[Real]>::to_vec);
    let signed = weights.iter().any(|v| v.is_negative());
    if signed && !allow_signed {
        return Err(nonnegative_stddev_weights());
    }
    let largest_weight = weights
        .iter()
        .copied()
        .map(Real::abs)
        .max_by(|a, b| a.cmp_value(*b))
        .unwrap_or(Real::ZERO);
    if largest_weight.is_zero() {
        return Ok(None);
    }
    let largest_value = values
        .iter()
        .map(|v| v.abs())
        .max_by(|a, b| a.cmp_value(*b))
        .unwrap();
    if largest_value.is_zero() && !signed {
        return Ok(Some(Value::zero()));
    }
    // Powers of ten preserve finite decimal inputs exactly during scaling.
    let weight_scale = Real::from_parts(false, 1, largest_weight.exp10())?;
    let scale = Real::from_parts(false, 1, largest_value.exp10())?;
    let mut total = Real::ZERO;
    let mut sum = Real::ZERO;
    for (value, weight) in values.iter().zip(&weights) {
        let weight = weight.div(weight_scale)?;
        total = total.add(weight)?;
        sum = sum.add(value.div(scale)?.mul(weight)?)?;
    }
    if total.is_zero() {
        // The original fractional signed-list path reports this as a string.
        // Integral cancellation has the same undefined value as zero weights.
        return Ok(weights
            .iter()
            .chain(values)
            .any(|value| !value.is_integer())
            .then(|| Value::str("Undefined")));
    }
    let mean = sum.div(total)?;
    let mut variance = Real::ZERO;
    for (value, weight) in values.iter().zip(weights) {
        let delta = value.div(scale)?.sub(mean)?;
        variance = variance.add(delta.mul(delta)?.mul(weight.div(weight_scale)?)?)?;
    }
    let variance = variance.div(total)?;
    let magnitude = variance.abs().sqrt()?.mul(scale)?;
    Ok(Some(if variance.is_negative() {
        // The signed-list function returns a complex value in either
        // calculator complex-result mode.
        Value::Complex(Complex::new(Real::ZERO, magnitude))
    } else {
        Value::Real(magnitude)
    }))
}

pub(super) fn evaluated_matrix_deviation(
    machine: &mut Machine,
    args: &[Value],
) -> RtResult<Option<Value>> {
    let [Value::Matrix(data), Value::Matrix(weights)] = args else {
        return Ok(None);
    };
    if data.vector
        || weights.vector
        || data.rows != weights.rows
        || data.rows == 0
        || data.cols.min(weights.cols) == 0
        || (data.cols == weights.cols
            && !weights
                .data
                .iter()
                .any(|value| value.as_real().is_some_and(Real::is_negative)))
    {
        return Ok(None);
    }
    matrix_deviation(machine, data, Some(weights), "STDDEV", true).map(Some)
}

fn matrix_deviation(
    machine: &mut Machine,
    matrix: &Matrix,
    weights: Option<&Matrix>,
    name: &str,
    allow_signed: bool,
) -> RtResult<Value> {
    let differing_widths = weights.is_some_and(|weights| weights.cols != matrix.cols);
    let column_count = weights.map_or(matrix.cols, |weights| matrix.cols.min(weights.cols));
    let mut columns = Vec::with_capacity(column_count);
    for col in 0..column_count {
        machine.check_limits()?;
        let column = |matrix: &Matrix| {
            (0..matrix.rows)
                .map(|row| matrix.get(row, col).expect_real(name))
                .collect::<RtResult<Vec<_>>>()
        };
        let values = column(matrix)?;
        let weights = weights.map(column).transpose()?;
        match weighted_deviation(&values, weights.as_deref(), allow_signed)? {
            // A fractional cancellation needs a numeric NaN cell, including
            // the all-zero weights that take the None branch.
            Some(Value::Str(_)) if differing_widths => return Err(stddev_matrix_shape_error()),
            Some(Value::Str(_)) => return Err(nonnegative_stddev_weights()),
            Some(deviation) => columns.push(deviation),
            None if differing_widths && values.iter().any(|value| !value.is_integer()) => {
                return Err(stddev_matrix_shape_error());
            }
            None => {
                return Ok(Value::list(vec![crate::cas::from_cas(
                    ppl_cas::CasValue::Undefined,
                )]));
            }
        }
    }
    if columns
        .iter()
        .any(|value| matches!(value, Value::Complex(_)))
    {
        // A complex vector stores every cell as complex, including real columns.
        for value in &mut columns {
            if let Value::Real(real) = value {
                *value = Value::Complex(Complex::new(*real, Real::ZERO));
            }
        }
    }
    Ok(Value::Matrix(std::rc::Rc::new(Matrix {
        rows: 1,
        cols: columns.len(),
        vector: true,
        data: columns,
    })))
}

/// The calculator takes the principal square root of algebraic variance
/// for complex lists. Conjugating the centered values would return a
/// different, real-valued measure.
pub(super) fn complex_list_deviation(
    m: &mut Machine,
    values: &[Value],
    weight_arg: Option<&Value>,
    name: &str,
    encoded_vector: bool,
) -> RtResult<Value> {
    let values: Option<Vec<_>> = values
        .iter()
        .map(|value| match value {
            Value::Complex(value) => Some(*value),
            value => value.as_real().map(|re| Complex::new(re, Real::ZERO)),
        })
        .collect();
    let Some(values) = values else {
        return Ok(Value::str("Error: Bad Argument Value"));
    };
    if let Some(Value::List(weights)) = weight_arg
        && weights
            .iter()
            .any(|value| matches!(value, Value::Str(_) | Value::RawString(_)))
    {
        return Ok(Value::str("Error: Bad Argument Value"));
    }
    let weights = weight_arg
        .map(|arg| numeric_series(arg, name))
        .transpose()?
        .unwrap_or_else(|| vec![Real::ONE; values.len()]);
    if weights.len() != values.len() {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    if !matches!(weight_arg, Some(Value::List(_))) && weights.iter().any(|v| v.is_negative()) {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "STDDEV weights must be nonnegative",
        ));
    }
    let undefined = || crate::cas::from_cas(ppl_cas::CasValue::Undefined);
    let Some(weight_exp) = weights
        .iter()
        .filter(|value| !value.is_zero())
        .map(|value| value.exp10())
        .max()
    else {
        return Ok(undefined());
    };
    let value_exp = values
        .iter()
        .flat_map(|value| [value.re, value.im])
        .filter(|value| !value.is_zero())
        .map(Real::exp10)
        .max()
        .unwrap_or(0);

    let scaled: RtResult<Vec<_>> = values
        .iter()
        .zip(&weights)
        .map(|(value, weight)| {
            Ok((
                Complex::new(value.re.scale10(-value_exp)?, value.im.scale10(-value_exp)?),
                weight.scale10(-weight_exp)?,
            ))
        })
        .collect();
    let scaled = scaled?;
    let mut total = Real::ZERO;
    let mut re = Real::ZERO;
    let mut im = Real::ZERO;
    for (value, weight) in &scaled {
        m.check_limits()?;
        total = total.add(*weight)?;
        re = re.add(value.re.mul(*weight)?)?;
        im = im.add(value.im.mul(*weight)?)?;
    }
    if total.is_zero() {
        // A nonreal cell in an encoded vector only changes the cancellation
        // type if its frequency contributes to the result.
        let fractional = weights.iter().any(|weight| !weight.is_integer())
            || values.iter().zip(&weights).any(|(value, weight)| {
                let integral = value.re.is_integer() && value.im.is_integer();
                let ignored_pair = encoded_vector && weight.is_zero() && !value.im.is_zero();
                !(integral || ignored_pair)
            });
        return Ok(if fractional {
            Value::str("Undefined")
        } else {
            undefined()
        });
    }
    let mean = Complex::new(re.div(total)?, im.div(total)?);
    let mut re = Real::ZERO;
    let mut im = Real::ZERO;
    for (value, weight) in scaled {
        m.check_limits()?;
        let delta = value.sub(mean)?;
        let square = delta.mul(delta)?;
        re = re.add(square.re.mul(weight)?)?;
        im = im.add(square.im.mul(weight)?)?;
    }
    let variance = Complex::new(re.div(total)?, im.div(total)?);
    let mut root = variance.sqrt()?;
    if !variance.im.is_zero() {
        // Recover the smaller component with 2 Re(root) Im(root) = Im(variance).
        // Subtracting nearly equal terms in the complex square-root formula
        // can erase a genuine imaginary part near either coordinate axis.
        let two = Real::from_i64(2);
        if variance.re.is_negative() {
            root.re = variance.im.div(root.im.mul(two)?)?;
        } else {
            root.im = variance.im.div(root.re.mul(two)?)?;
        }
    }
    let re = root.re.scale10(value_exp)?;
    let im = root.im.scale10(value_exp)?;
    if im.is_zero() {
        Ok(Value::Real(re))
    } else {
        Ok(Value::Complex(Complex::new(re, im)))
    }
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let s = m.arithmetic_settings();
    let a0 = || arg(args, 0, name);
    match b {
        B::PredX | B::PredY => prediction::call(b, name, args, &s),
        B::Abs => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Real(c.abs()?)),
            Value::Int(i) => Ok(Value::Int(*i)),
            other => Ok(Value::Real(other.expect_real("ABS")?.abs())),
        }),
        B::Sqrt => sqrt(m, a0()?),
        B::Sq => map_numeric(a0()?, &mut |x| ops::binary(BinOp::Mul, x, x, &s)),
        B::Exp => analytic_map(a0()?, |x| Ok(x.exp_e()?), Complex::exp),
        B::Ln => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Complex(c.ln()?)),
            value => {
                let value = value.expect_real(name)?;
                if value.is_negative() && s.complex_results {
                    Ok(Value::Complex(Complex::new(value, Real::ZERO).ln()?))
                } else {
                    Ok(Value::Real(value.ln()?))
                }
            }
        }),
        B::Log => match args.get(1) {
            None => analytic_map(
                a0()?,
                |x| Ok(x.log10()?),
                |x| {
                    let value = x.ln()?;
                    let base = Real::from_i64(10).ln()?;
                    Ok(Complex::new(value.re.div(base)?, value.im.div(base)?))
                },
            ),
            Some(base) => {
                let b = base.expect_real("LOG")?;
                real_map(a0()?, |x| Ok(x.ln()?.div(b.ln()?)?))
            }
        },
        B::Alog => real_map(a0()?, |x| Ok(Real::from_i64(10).pow(x)?)),
        B::Expm1 => f64_map(a0()?, f64::exp_m1),
        B::Lnp1 => real_map(a0()?, |x| {
            if x.to_f64() <= -1.0 {
                return Err(RuntimeError::kind(ErrorKind::NonReal));
            }
            Ok(Real::from_f64(x.to_f64().ln_1p())?)
        }),
        B::Sin => analytic_map(a0()?, |x| trig(&s, x, 0), Complex::sin),
        B::Cos => analytic_map(a0()?, |x| trig(&s, x, 1), Complex::cos),
        B::Tan => analytic_map(a0()?, |x| trig(&s, x, 2), Complex::tan),
        B::Csc => real_map(a0()?, |x| Ok(Real::ONE.div(trig(&s, x, 0)?)?)),
        B::Sec => real_map(a0()?, |x| Ok(Real::ONE.div(trig(&s, x, 1)?)?)),
        B::Cot => real_map(a0()?, |x| Ok(Real::ONE.div(trig(&s, x, 2)?)?)),
        B::Asin => real_map(a0()?, |x| inverse_trig(&s, x, f64::asin, true)),
        B::Acos => real_map(a0()?, |x| inverse_trig(&s, x, f64::acos, true)),
        B::Atan => real_map(a0()?, |x| inverse_trig(&s, x, f64::atan, false)),
        B::Sinh => f64_map(a0()?, f64::sinh),
        B::Cosh => f64_map(a0()?, f64::cosh),
        B::Tanh => f64_map(a0()?, f64::tanh),
        B::Asinh => f64_map(a0()?, f64::asinh),
        B::Acosh => real_map(a0()?, |x| {
            if x.to_f64() < 1.0 {
                return Err(RuntimeError::kind(ErrorKind::NonReal));
            }
            Ok(Real::from_f64(x.to_f64().acosh())?)
        }),
        B::Atanh => real_map(a0()?, |x| {
            let v = x.to_f64();
            if v.abs() >= 1.0 {
                return Err(RuntimeError::kind(if v.abs() == 1.0 {
                    ErrorKind::Undefined
                } else {
                    ErrorKind::NonReal
                }));
            }
            Ok(Real::from_f64(v.atanh())?)
        }),
        B::Floor => real_map(a0()?, |x| Ok(x.floor().with_presentation(x.presentation()))),
        B::Ceiling => real_map(a0()?, |x| {
            Ok(x.ceil().with_presentation(RealPresentation::Decimal))
        }),
        B::Ip => real_map(a0()?, |x| Ok(x.trunc().with_presentation(x.presentation()))),
        B::Fp => real_map(a0()?, |x| {
            Ok(x.fract().with_presentation(RealPresentation::Decimal))
        }),
        B::Round | B::Truncate => {
            let places = match args.get(1) {
                Some(p) if m.settings.numeric_version == ppl_math::NumericVersion::Current15751 => {
                    let precision = p
                        .expect_real(name)?
                        .to_i64()
                        .filter(|p| (-12..=12).contains(p))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                ErrorKind::BadArgValue,
                                "Precision must be an integer from -12 through 12",
                            )
                        })?;
                    precision as i32
                }
                // Clamp rather than wrap: ROUND(x, 2^32) must not act like
                // ROUND(x, 0).
                Some(p) => p.expect_i64(name)?.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                None => 0,
            };
            let round = b == B::Round;
            real_map(a0()?, |x| {
                Ok((if round {
                    x.round_places(places)
                } else {
                    x.truncate_places(places)
                })
                .with_presentation(x.presentation()))
            })
        }
        B::Sign => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => {
                let r = c.abs()?;
                Ok(norm_complex(Complex::new(c.re.div(r)?, c.im.div(r)?)))
            }
            other => {
                let r = other.expect_real("SIGN")?;
                Ok(Value::from_i64(if r.is_zero() {
                    0
                } else if r.is_negative() {
                    -1
                } else {
                    1
                }))
            }
        }),
        B::Max | B::Min => {
            let vals = reduce_list(args)?;
            let first = *vals
                .first()
                .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?;
            let pick = vals.into_iter().fold(first, |acc, v| {
                let better = if b == B::Max { v > acc } else { v < acc };
                if better { v } else { acc }
            });
            Ok(Value::Real(pick))
        }
        B::Comb | B::Perm => {
            let n = a0()?.expect_i64(name)?;
            let k = arg(args, 1, name)?.expect_i64(name)?;
            if n < 0 || k < 0 || k > n {
                return Err(RuntimeError::kind(ErrorKind::BadArgValue));
            }
            let mut acc = Real::ONE;
            for i in 0..k {
                acc = acc.mul(Real::from_i64(n - i))?;
                if b == B::Comb {
                    acc = acc.div(Real::from_i64(i + 1))?;
                }
            }
            Ok(Value::Real(acc.round_places(0)))
        }
        B::Random => {
            if args.len() >= 3 {
                m.check_len(args[0].expect_i64(name)?.max(0) as usize)?;
            }
            let mut draw = |lo: f64, hi: f64| -> RtResult<Value> {
                let u = m.rng.next_f64();
                Ok(Value::Real(Real::from_f64(lo + (hi - lo) * u)?))
            };
            match args.len() {
                0 => draw(0.0, 1.0),
                1 => draw(0.0, args[0].expect_real(name)?.to_f64()),
                2 => draw(
                    args[0].expect_real(name)?.to_f64(),
                    args[1].expect_real(name)?.to_f64(),
                ),
                _ => {
                    let n = args[0].expect_i64(name)?.max(0) as usize;
                    let (lo, hi) = (
                        args[1].expect_real(name)?.to_f64(),
                        args[2].expect_real(name)?.to_f64(),
                    );
                    Ok(Value::list(
                        (0..n).map(|_| draw(lo, hi)).collect::<RtResult<_>>()?,
                    ))
                }
            }
        }
        B::Randint => {
            if args.len() == 3 {
                let n = args[0].expect_i64(name)?.max(0) as usize;
                m.check_len(n)?;
                let (lo, hi) = (args[1].expect_i64(name)?, args[2].expect_i64(name)?);
                return Ok(Value::list(
                    (0..n)
                        .map(|_| Value::from_i64(m.rng.range(lo, hi)))
                        .collect(),
                ));
            }
            // User Guide 3rd ed.: no argument gives 0 or 1, one argument a
            // gives an integer between 0 and a.
            let (lo, hi) = match args.len() {
                0 => (0, 1),
                1 => (0, a0()?.expect_i64(name)?),
                _ => (
                    a0()?.expect_i64(name)?,
                    arg(args, 1, name)?.expect_i64(name)?,
                ),
            };
            Ok(Value::from_i64(m.rng.range(lo, hi)))
        }
        B::Randnorm => {
            let mu = args
                .first()
                .map(|v| v.expect_real(name))
                .transpose()?
                .map(|r| r.to_f64())
                .unwrap_or(0.0);
            let sigma = args
                .get(1)
                .map(|v| v.expect_real(name))
                .transpose()?
                .map(|r| r.to_f64())
                .unwrap_or(1.0);
            Ok(Value::Real(Real::from_f64(mu + sigma * m.rng.normal())?))
        }
        B::Randseed => {
            let seed = match args.first() {
                Some(v) => v.expect_real(name)?.to_f64().to_bits(),
                None => 0x5EED,
            };
            m.set_seed(seed);
            Ok(Value::zero())
        }
        B::Mant => real_map(a0()?, |x| {
            if x.is_zero() {
                return Ok(Real::ZERO);
            }
            let e = x.exp10();
            Ok(x.abs().div(Real::from_i64(10).powi(e as i64)?)?)
        }),
        B::Xpon => real_map(a0()?, |x| Ok(Real::from_i64(x.exp10() as i64))),
        B::Re => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Real(c.re)),
            o => Ok(Value::Real(o.expect_real("RE")?)),
        }),
        B::Im => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Real(c.im)),
            o => {
                o.expect_real("IM")?;
                Ok(Value::zero())
            }
        }),
        B::Conj => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Complex(c.conj())),
            o => Ok(Value::Real(o.expect_real("CONJ")?)),
        }),
        B::Arg => map_numeric(a0()?, &mut |x| {
            let c = match x {
                Value::Complex(c) => *c,
                o => Complex::new(o.expect_real("ARG")?, Real::ZERO),
            };
            Ok(Value::Real(Real::from_f64(
                s.from_radians(c.arg_radians()?.to_f64()),
            )?))
        }),
        B::Approx => match a0()? {
            Value::Complex(value) if value.im.is_zero() => Ok(Value::Real(value.re.decimal())),
            Value::Real(value)
                if m.settings.numeric_version == ppl_math::NumericVersion::Current15751 =>
            {
                Ok(Value::Real(value.decimal()))
            }
            Value::List(items) => items
                .iter()
                .map(|value| super::call(m, B::Approx, vec![value.clone()]))
                .collect::<RtResult<Vec<_>>>()
                .map(Value::list),
            Value::Matrix(matrix) => ops::map_matrix(matrix, |value| {
                super::call(m, B::Approx, vec![value.clone()])
            }),
            value => Ok(value.clone()),
        },
        B::Hms => sexagesimal_map(a0()?, RealPresentation::Sexagesimal),
        B::HmsFrom => sexagesimal_map(a0()?, RealPresentation::Decimal),
        B::Nthroot => {
            let n = a0()?.expect_real(name)?;
            real_map(arg(args, 1, name)?, |x| nth_root(n, x))
        }
        B::Factorial => real_map(a0()?, ops::factorial),
        B::Mean => {
            let v = stats_values(args, name)?;
            let mut sum = Real::ZERO;
            for x in &v {
                sum = sum.add(*x)?;
            }
            Ok(Value::Real(sum.div(Real::from_i64(v.len() as i64))?))
        }
        B::Median => {
            let mut v = stats_values(args, name)?;
            v.sort_by(|a, b| a.cmp_value(*b));
            let n = v.len();
            if n % 2 == 1 {
                Ok(Value::Real(v[n / 2]))
            } else {
                Ok(Value::Real(
                    v[n / 2 - 1].add(v[n / 2])?.div(Real::from_i64(2))?,
                ))
            }
        }
        B::Stddev => {
            let data = arg(args, 0, name)?;
            let undefined = || crate::cas::from_cas(ppl_cas::CasValue::Undefined);
            if args.len() == 1 && (data.as_real().is_some() || matches!(data, Value::Complex(_))) {
                return Ok(Value::zero());
            }
            if let Value::Matrix(matrix) = data
                && !matrix.vector
            {
                let weights = match args.get(1) {
                    None => None,
                    Some(Value::Matrix(weights))
                        if !weights.vector
                            && (matrix.rows, matrix.cols) == (weights.rows, weights.cols) =>
                    {
                        Some(weights.as_ref())
                    }
                    Some(_) => return Err(stddev_matrix_shape_error()),
                };
                return matrix_deviation(m, matrix, weights, name, false);
            }
            if let Value::List(values) = data
                && values
                    .iter()
                    .any(|value| matches!(value, Value::Complex(value) if !value.im.is_zero()))
            {
                return complex_list_deviation(m, values, args.get(1), name, false);
            }
            let values = numeric_series(data, name)?;
            let weights = args.get(1).map(|v| numeric_series(v, name)).transpose()?;
            let allow_signed =
                matches!((data, args.get(1)), (Value::List(_), Some(Value::List(_))));
            m.check_limits()?;
            Ok(
                weighted_deviation(&values, weights.as_deref(), allow_signed)?
                    .unwrap_or_else(undefined),
            )
        }
        B::Ifte => {
            let c = a0()?.truthy()?;
            Ok(arg(args, if c { 1 } else { 2 }, name)?.clone())
        }
        B::Bitand | B::Bitor | B::Bitxor => {
            let mut result = real_bits(a0()?)?;
            for value in &args[1..] {
                let value = real_bits(value)?;
                result = match b {
                    B::Bitand => result & value,
                    B::Bitor => result | value,
                    _ => result ^ value,
                };
            }
            Ok(Value::Real(Real::from_u64(result)))
        }
        B::Bitnot => Ok(Value::Real(Real::from_u64(
            !real_bits(a0()?)? & ((1u64 << 39) - 1),
        ))),
        B::Bitsl | B::Bitsr => {
            let count = args
                .get(1)
                .map(|value| word_argument(value, name))
                .transpose()?
                .unwrap_or(1);
            let left = (b == B::Bitsl) == (count >= 0);
            let count = count.unsigned_abs().min(u32::MAX as u64) as u32;
            match a0()? {
                Value::Int(value) => Ok(Value::Int(if left {
                    value.shift_left(count)
                } else {
                    value.shift_right(count)
                })),
                value => {
                    let value = real_integer(value, name)? as u64;
                    let shifted = if count >= 64 {
                        0
                    } else if left {
                        value << count
                    } else {
                        value >> count
                    };
                    Ok(Value::Real(Real::from_u64(shifted)))
                }
            }
        }
        B::RtoB => {
            let width = args
                .get(1)
                .map(|value| word_argument(value, name))
                .transpose()?
                .unwrap_or(s.integer_width_code());
            let template = word(0, width, s.int_base)?;
            let upper = (1i128 << template.bits()) - 1;
            let lower = if template.is_signed() {
                -(1i128 << template.bits())
            } else {
                0
            };
            let value = match a0()? {
                Value::Int(value) => value.numeric_value(),
                value => {
                    let value = value.expect_real(name)?.round_places(0);
                    value
                        .to_i128()
                        .unwrap_or(if value.is_negative() { lower } else { upper })
                }
            }
            .clamp(lower, upper);
            Ok(Value::Int(
                template.with_value(value).with_system_base(true),
            ))
        }
        B::BtoR => Ok(Value::Real(a0()?.expect_real(name)?)),
        B::Setbase => {
            let value = match a0()? {
                Value::Int(value) => *value,
                value => word(
                    real_integer(value, name)?,
                    s.integer_width_code(),
                    s.int_base,
                )?
                .with_system_base(true),
            };
            let code = args
                .get(1)
                .map(|value| word_argument(value, name))
                .transpose()?
                .unwrap_or(0);
            let value = match code {
                0 => value.with_system_base(true),
                1 => value.with_base(IntBase::Bin),
                2 => value.with_base(IntBase::Oct),
                3 => value.with_base(IntBase::Dec),
                4 => value.with_base(IntBase::Hex),
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "Base must be0 (system) through4 (hex)",
                    ));
                }
            };
            Ok(Value::Int(value))
        }
        B::Getbase => match a0()? {
            Value::Int(value) => Ok(Value::Int(
                word(
                    if value.is_system_base() {
                        0
                    } else {
                        base_code(value.base) as i128
                    },
                    s.integer_width_code(),
                    s.int_base,
                )?
                .with_system_base(true),
            )),
            _ => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "GETBASE needs an integer (#…)",
            )),
        },
        B::Setbits => {
            let width = args
                .get(1)
                .map(|value| word_argument(value, name))
                .transpose()?
                .unwrap_or(s.integer_width_code());
            let value = match a0()? {
                Value::Int(value) => value.with_word_size(width).ok_or_else(|| {
                    RuntimeError::new(ErrorKind::BadArgValue, "Invalid signed word width")
                })?,
                value => {
                    word(real_integer(value, name)?, width, s.int_base)?.with_system_base(true)
                }
            };
            Ok(Value::Int(value))
        }
        B::Getbits => match a0()? {
            Value::Int(value) => Ok(Value::from_i64(value.width_code())),
            _ => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "GETBITS needs an integer (#…)",
            )),
        },
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a math command"),
        )),
    }
}
