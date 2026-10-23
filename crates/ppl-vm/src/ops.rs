//! Operators on PPL values.
//!
//! Mixed-type rules (all **unverified** on the G1 unless the compatibility
//! matrix says otherwise):
//! * string `+` anything concatenates the display forms;
//! * lists combine element by element, and with scalars by broadcasting;
//! * integer ⊕ integral real stays an integer in the integer's base;
//! * complex operands promote the other side to complex.

use std::cmp::Ordering;
use std::rc::Rc;

use ppl_math::{Complex, HpInt, Real};
use ppl_syntax::ast::BinOp;

use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::settings::Settings;
use crate::value::{Matrix, Value};

fn bad_type(op: &str, a: &Value, b: &Value) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::BadArgType,
        format!(
            "cannot apply `{op}` to {} and {}",
            a.type_name(),
            b.type_name()
        ),
    )
}

pub fn norm_complex(c: Complex) -> Value {
    if c.im.is_zero() {
        Value::Real(c.re)
    } else {
        Value::Complex(c)
    }
}

pub fn binary(op: BinOp, a: &Value, b: &Value, s: &Settings) -> RtResult<Value> {
    match op {
        BinOp::Eq => Ok(Value::from_bool(a.equals(b))),
        BinOp::Ne => Ok(Value::from_bool(!a.equals(b))),
        BinOp::Lt => compare(a, b).map(|o| Value::from_bool(o == Ordering::Less)),
        BinOp::Le => compare(a, b).map(|o| Value::from_bool(o != Ordering::Greater)),
        BinOp::Gt => compare(a, b).map(|o| Value::from_bool(o == Ordering::Greater)),
        BinOp::Ge => compare(a, b).map(|o| Value::from_bool(o != Ordering::Less)),
        BinOp::And => Ok(Value::from_bool(a.truthy()? && b.truthy()?)),
        BinOp::Or => Ok(Value::from_bool(a.truthy()? || b.truthy()?)),
        BinOp::Xor => Ok(Value::from_bool(a.truthy()? != b.truthy()?)),
        BinOp::Add if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) => {
            if matches!(a, Value::List(_)) || matches!(b, Value::List(_)) {
                return arith(op, a, b, s);
            }
            let fmt = s.format();
            let mut out = a.display(fmt);
            out.push_str(&b.display(fmt));
            Ok(Value::str(out))
        }
        _ => arith(op, a, b, s),
    }
}

pub fn compare(a: &Value, b: &Value) -> RtResult<Ordering> {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        _ => match (a.as_real(), b.as_real()) {
            (Some(x), Some(y)) => Ok(x.cmp_value(y)),
            _ => Err(bad_type("<", a, b)),
        },
    }
}

fn arith(op: BinOp, a: &Value, b: &Value, s: &Settings) -> RtResult<Value> {
    match (a, b) {
        (Value::List(x), Value::List(y)) => {
            if x.len() != y.len() {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidDimension,
                    format!("lists of sizes {} and {}", x.len(), y.len()),
                ));
            }
            let v: RtResult<Vec<Value>> = x
                .iter()
                .zip(y.iter())
                .map(|(p, q)| binary(op, p, q, s))
                .collect();
            Ok(Value::list(v?))
        }
        (Value::List(x), y) => Ok(Value::list(
            x.iter()
                .map(|p| binary(op, p, y, s))
                .collect::<RtResult<_>>()?,
        )),
        (x, Value::List(y)) => Ok(Value::list(
            y.iter()
                .map(|q| binary(op, x, q, s))
                .collect::<RtResult<_>>()?,
        )),
        (Value::Matrix(x), Value::Matrix(y)) => matrix_op(op, x, y, s),
        (Value::Matrix(m), k) if matches!(op, BinOp::Mul | BinOp::Div) => {
            map_matrix(m, |e| binary(op, e, k, s))
        }
        (k, Value::Matrix(m)) if op == BinOp::Mul => map_matrix(m, |e| binary(op, k, e, s)),
        (Value::Matrix(m), k) if op == BinOp::Pow => {
            let n = k.expect_i64("matrix power")?;
            matrix_pow(m, n, s)
        }
        _ => scalar(op, a, b, s),
    }
}

fn int_of(v: &Value) -> Option<HpInt> {
    match v {
        Value::Int(i) => Some(*i),
        _ => None,
    }
}

fn scalar(op: BinOp, a: &Value, b: &Value, s: &Settings) -> RtResult<Value> {
    // Integer arithmetic when one side is an HP integer and the other is
    // an integer-valued number.
    if let (Some(x), Some(y)) = (int_of(a), int_of(b)) {
        return int_op(op, x, y);
    }
    if let Some(x) = int_of(a) {
        if let Some(r) = b.as_real().filter(|r| r.is_integer()) {
            if let Some(v) = r.to_i64() {
                return int_op(op, x, HpInt::new(v as i128, x.bits(), x.base));
            }
        }
    }
    if let Some(y) = int_of(b) {
        if let Some(r) = a.as_real().filter(|r| r.is_integer()) {
            if let Some(v) = r.to_i64() {
                return int_op(op, HpInt::new(v as i128, y.bits(), y.base), y);
            }
        }
    }
    if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
        let to_c = |v: &Value| match v {
            Value::Complex(c) => Some(*c),
            other => other.as_real().map(|r| Complex::new(r, Real::ZERO)),
        };
        let (Some(x), Some(y)) = (to_c(a), to_c(b)) else {
            return Err(bad_type(op.symbol(), a, b));
        };
        return complex_op(op, x, y).map(norm_complex);
    }
    let (Some(x), Some(y)) = (a.as_real(), b.as_real()) else {
        return Err(bad_type(op.symbol(), a, b));
    };
    let r = match op {
        BinOp::Add => x.add(y)?,
        BinOp::Sub => x.sub(y)?,
        BinOp::Mul => x.mul(y)?,
        BinOp::Div => x.div(y)?,
        BinOp::Pow => match x.pow(y) {
            Ok(r) => r,
            Err(ppl_math::MathError::NonReal) if s.complex_results => {
                return complex_pow(Complex::new(x, Real::ZERO), Complex::new(y, Real::ZERO))
                    .map(norm_complex);
            }
            Err(e) => return Err(e.into()),
        },
        BinOp::Mod => real_mod(x, y)?,
        BinOp::IntDiv => {
            if y.is_zero() {
                return Err(RuntimeError::kind(ErrorKind::DivisionByZero));
            }
            x.div(y)?.floor()
        }
        _ => return Err(bad_type(op.symbol(), a, b)),
    };
    Ok(Value::Real(r))
}

/// `a MOD b` with the sign of `b` (floored), so `-7 MOD 3 = 2`.
pub fn real_mod(x: Real, y: Real) -> RtResult<Real> {
    if y.is_zero() {
        return Err(RuntimeError::kind(ErrorKind::DivisionByZero));
    }
    let q = x.div(y)?.floor();
    Ok(x.sub(q.mul(y)?)?)
}

fn int_op(op: BinOp, x: HpInt, y: HpInt) -> RtResult<Value> {
    Ok(Value::Int(match op {
        BinOp::Add => x.add(y),
        BinOp::Sub => x.sub(y),
        BinOp::Mul => x.mul(y),
        BinOp::Div | BinOp::IntDiv => x.div(y)?,
        BinOp::Mod => x.rem(y)?,
        BinOp::Pow => {
            let mut acc = HpInt::new(1, x.bits(), x.base);
            for _ in 0..y.unsigned().min(64) {
                acc = acc.mul(x);
            }
            acc
        }
        _ => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("`{}` on integers", op.symbol()),
            ));
        }
    }))
}

fn complex_op(op: BinOp, x: Complex, y: Complex) -> RtResult<Complex> {
    Ok(match op {
        BinOp::Add => x.add(y)?,
        BinOp::Sub => x.sub(y)?,
        BinOp::Mul => x.mul(y)?,
        BinOp::Div => x.div(y)?,
        BinOp::Pow => complex_pow(x, y)?,
        _ => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("`{}` on complex numbers", op.symbol()),
            ));
        }
    })
}

pub fn complex_pow(x: Complex, y: Complex) -> RtResult<Complex> {
    let (a, b) = (x.re.to_f64(), x.im.to_f64());
    let (c, d) = (y.re.to_f64(), y.im.to_f64());
    if a == 0.0 && b == 0.0 {
        return Ok(Complex::new(Real::ZERO, Real::ZERO));
    }
    let r = a.hypot(b);
    let t = b.atan2(a);
    let ln_r = r.ln();
    let mag = (c * ln_r - d * t).exp();
    let ang = d * ln_r + c * t;
    Ok(Complex::new(
        Real::from_f64(mag * ang.cos())?,
        Real::from_f64(mag * ang.sin())?,
    ))
}

pub fn map_matrix(m: &Matrix, mut f: impl FnMut(&Value) -> RtResult<Value>) -> RtResult<Value> {
    let data = m.data.iter().map(&mut f).collect::<RtResult<Vec<_>>>()?;
    Ok(Value::Matrix(Rc::new(Matrix { data, ..m.clone() })))
}

fn matrix_op(op: BinOp, x: &Matrix, y: &Matrix, s: &Settings) -> RtResult<Value> {
    match op {
        BinOp::Add | BinOp::Sub => {
            if x.rows != y.rows || x.cols != y.cols {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            let data = x
                .data
                .iter()
                .zip(&y.data)
                .map(|(p, q)| binary(op, p, q, s))
                .collect::<RtResult<Vec<_>>>()?;
            Ok(Value::Matrix(Rc::new(Matrix { data, ..x.clone() })))
        }
        BinOp::Mul => {
            // vector·vector is the dot product
            if x.vector && y.vector {
                if x.cols != y.cols {
                    return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
                }
                let mut acc = Value::zero();
                for (p, q) in x.data.iter().zip(&y.data) {
                    acc = binary(BinOp::Add, &acc, &binary(BinOp::Mul, p, q, s)?, s)?;
                }
                return Ok(acc);
            }
            // A vector on the right acts as a column.
            let (yr, yc) = if y.vector {
                (y.cols, 1)
            } else {
                (y.rows, y.cols)
            };
            if x.cols != yr {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidDimension,
                    format!("{}×{} times {}×{}", x.rows, x.cols, yr, yc),
                ));
            }
            let mut out = Matrix::zeros(x.rows, yc);
            for i in 0..x.rows {
                for j in 0..yc {
                    let mut acc = Value::zero();
                    for k in 0..x.cols {
                        let q = &y.data[k * yc + j];
                        acc = binary(BinOp::Add, &acc, &binary(BinOp::Mul, x.get(i, k), q, s)?, s)?;
                    }
                    out.set(i, j, acc);
                }
            }
            if y.vector {
                out = Matrix {
                    rows: 1,
                    cols: x.rows,
                    data: out.data,
                    vector: true,
                };
            }
            Ok(Value::Matrix(Rc::new(out)))
        }
        BinOp::Div => {
            let inv = inverse(y, s)?;
            matrix_op(BinOp::Mul, x, &inv, s)
        }
        _ => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("`{}` on matrices", op.symbol()),
        )),
    }
}

fn matrix_pow(m: &Matrix, n: i64, s: &Settings) -> RtResult<Value> {
    if m.rows != m.cols {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    let base = if n < 0 { inverse(m, s)? } else { m.clone() };
    let mut acc = identity(m.rows);
    for _ in 0..n.unsigned_abs() {
        let Value::Matrix(p) = matrix_op(BinOp::Mul, &acc, &base, s)? else {
            unreachable!()
        };
        acc = (*p).clone();
    }
    Ok(Value::Matrix(Rc::new(acc)))
}

pub fn identity(n: usize) -> Matrix {
    let mut m = Matrix::zeros(n, n);
    for i in 0..n {
        m.set(i, i, Value::Real(Real::ONE));
    }
    m
}

fn real_entries(m: &Matrix) -> RtResult<Vec<Real>> {
    m.data
        .iter()
        .map(|v| {
            v.as_real().ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::BadArgType,
                    "complex matrices are not supported here",
                )
            })
        })
        .collect()
}

/// Gauss–Jordan inverse with partial pivoting in 12-digit arithmetic.
pub fn inverse(m: &Matrix, _s: &Settings) -> RtResult<Matrix> {
    if m.rows != m.cols {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    let n = m.rows;
    let mut a = real_entries(m)?;
    let mut inv: Vec<Real> = (0..n * n)
        .map(|i| {
            if i / n == i % n {
                Real::ONE
            } else {
                Real::ZERO
            }
        })
        .collect();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| a[i * n + col].abs().cmp_value(a[j * n + col].abs()))
            .unwrap();
        if a[pivot * n + col].is_zero() {
            return Err(RuntimeError::new(ErrorKind::Undefined, "singular matrix"));
        }
        for k in 0..n {
            a.swap(col * n + k, pivot * n + k);
            inv.swap(col * n + k, pivot * n + k);
        }
        let p = a[col * n + col];
        for k in 0..n {
            a[col * n + k] = a[col * n + k].div(p)?;
            inv[col * n + k] = inv[col * n + k].div(p)?;
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r * n + col];
            if f.is_zero() {
                continue;
            }
            for k in 0..n {
                a[r * n + k] = a[r * n + k].sub(f.mul(a[col * n + k])?)?;
                inv[r * n + k] = inv[r * n + k].sub(f.mul(inv[col * n + k])?)?;
            }
        }
    }
    Ok(Matrix {
        rows: n,
        cols: n,
        data: inv.into_iter().map(Value::Real).collect(),
        vector: false,
    })
}

/// Determinant by elimination with partial pivoting.
pub fn determinant(m: &Matrix) -> RtResult<Real> {
    if m.rows != m.cols {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    let n = m.rows;
    let mut a = real_entries(m)?;
    if n == 0 {
        return Ok(Real::ONE);
    }
    if n == 1 {
        return Ok(a[0]);
    }
    if n == 2 {
        return Ok(a[0].mul(a[3])?.sub(a[1].mul(a[2])?)?);
    }
    let mut det = Real::ONE;
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| a[i * n + col].abs().cmp_value(a[j * n + col].abs()))
            .unwrap();
        if a[pivot * n + col].is_zero() {
            return Ok(Real::ZERO);
        }
        if pivot != col {
            for k in 0..n {
                a.swap(col * n + k, pivot * n + k);
            }
            det = det.neg();
        }
        let p = a[col * n + col];
        det = det.mul(p)?;
        for r in col + 1..n {
            let f = a[r * n + col].div(p)?;
            for k in col..n {
                a[r * n + k] = a[r * n + k].sub(f.mul(a[col * n + k])?)?;
            }
        }
    }
    Ok(det)
}

pub fn negate(v: &Value) -> RtResult<Value> {
    Ok(match v {
        Value::Real(r) => Value::Real(r.neg()),
        Value::Int(i) => Value::Int(i.neg()),
        Value::Complex(c) => Value::Complex(c.neg()),
        Value::List(l) => Value::list(l.iter().map(negate).collect::<RtResult<_>>()?),
        Value::Matrix(m) => map_matrix(m, negate)?,
        other => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("cannot negate a {}", other.type_name()),
            ));
        }
    })
}

/// Applies a real function element-wise over lists and matrices.
pub fn map_numeric(v: &Value, f: &mut dyn FnMut(&Value) -> RtResult<Value>) -> RtResult<Value> {
    match v {
        Value::List(l) => Ok(Value::list(
            l.iter()
                .map(|x| map_numeric(x, f))
                .collect::<RtResult<_>>()?,
        )),
        Value::Matrix(m) => map_matrix(m, |x| map_numeric(x, f)),
        other => f(other),
    }
}

/// n! for non-negative integers, Γ(n+1) otherwise.
pub fn factorial(x: Real) -> RtResult<Real> {
    if x.is_integer() {
        let n = x.to_i64().ok_or(RuntimeError::kind(ErrorKind::Overflow))?;
        if n < 0 {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "factorial of a negative integer",
            ));
        }
        if n > 300 {
            return Err(RuntimeError::kind(ErrorKind::Overflow));
        }
        let mut acc = Real::ONE;
        for i in 2..=n {
            acc = acc.mul(Real::from_i64(i))?;
        }
        return Ok(acc);
    }
    Ok(Real::from_f64(gamma(x.to_f64() + 1.0))?)
}

/// Lanczos approximation (g = 7), adequate for 12 digits.
pub fn gamma(x: f64) -> f64 {
    const G: [f64; 9] = [
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
    if x < 0.5 {
        return std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x));
    }
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    (2.0 * std::f64::consts::PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: i64) -> Value {
        Value::from_i64(v)
    }

    #[test]
    fn arithmetic_and_concat() {
        let s = Settings::default();
        assert!(binary(BinOp::Add, &n(2), &n(3), &s).unwrap().equals(&n(5)));
        assert_eq!(
            binary(BinOp::Add, &Value::str("a"), &n(1), &s)
                .unwrap()
                .display(s.format()),
            "a1"
        );
        assert!(binary(BinOp::Mod, &n(-7), &n(3), &s).unwrap().equals(&n(2)));
        assert!(
            binary(BinOp::IntDiv, &n(10), &n(3), &s)
                .unwrap()
                .equals(&n(3))
        );
        let l = binary(BinOp::Mul, &Value::list(vec![n(1), n(2)]), &n(3), &s).unwrap();
        assert_eq!(l.display(s.format()), "{3,6}");
        assert!(binary(BinOp::Div, &n(1), &n(0), &s).is_err());
    }

    #[test]
    fn matrices() {
        let s = Settings::default();
        let m = |d: &[i64]| {
            Value::Matrix(Rc::new(Matrix {
                rows: 2,
                cols: 2,
                data: d.iter().map(|&x| n(x)).collect(),
                vector: false,
            }))
        };
        let p = binary(BinOp::Mul, &m(&[1, 2, 3, 4]), &m(&[2, 0, 1, 2]), &s).unwrap();
        assert_eq!(p.display(s.format()), "[[4,4],[10,8]]");
        let Value::Matrix(mm) = m(&[1, 2, 3, 4]) else {
            unreachable!()
        };
        assert_eq!(determinant(&mm).unwrap().to_string(), "-2");
    }
}
