//! PPL values.
//!
//! Collections are reference counted and copied on write (`Rc::make_mut`),
//! which gives PPL's by-value semantics for assignment and argument
//! passing without copying on every read.

use std::fmt::Write as _;
use std::rc::Rc;

use ppl_math::{Complex, FormatOptions, HpInt, NumFormat, Real};

use crate::error::{ErrorKind, RuntimeError};

#[derive(Clone, Debug)]
pub enum Value {
    Real(Real),
    Int(HpInt),
    Complex(Complex),
    Str(Rc<str>),
    List(Rc<Vec<Value>>),
    Matrix(Rc<Matrix>),
    /// Reference to graphics variable G0–G9.
    Grob(u8),
}

/// A matrix or vector of reals/complex numbers, row-major.
#[derive(Clone, Debug)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Value>,
    /// A one-dimensional vector `[a,b,c]` (rows = 1).
    pub vector: bool,
}

impl Matrix {
    pub fn zeros(rows: usize, cols: usize) -> Matrix {
        Matrix {
            rows,
            cols,
            data: vec![Value::zero(); rows * cols],
            vector: false,
        }
    }

    pub fn get(&self, r: usize, c: usize) -> &Value {
        &self.data[r * self.cols + c]
    }

    pub fn set(&mut self, r: usize, c: usize, v: Value) {
        self.data[r * self.cols + c] = v;
    }

    pub fn row(&self, r: usize) -> &[Value] {
        &self.data[r * self.cols..(r + 1) * self.cols]
    }
}

/// `TYPE()` codes (forum-reported for Home/PPL; **unverified** on the G1).
pub mod type_code {
    pub const REAL: i64 = 0;
    pub const INTEGER: i64 = 1;
    pub const STRING: i64 = 2;
    pub const COMPLEX: i64 = 3;
    pub const MATRIX: i64 = 4;
    pub const LIST: i64 = 6;
    pub const GRAPHIC: i64 = 8;
}

impl Value {
    pub fn zero() -> Value {
        Value::Real(Real::ZERO)
    }

    pub fn real(r: Real) -> Value {
        Value::Real(r)
    }

    pub fn from_i64(v: i64) -> Value {
        Value::Real(Real::from_i64(v))
    }

    pub fn from_bool(b: bool) -> Value {
        Value::Real(if b { Real::ONE } else { Real::ZERO })
    }

    pub fn str(s: impl Into<Rc<str>>) -> Value {
        Value::Str(s.into())
    }

    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(v))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Real(_) => "real",
            Value::Int(_) => "integer",
            Value::Complex(_) => "complex",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Matrix(_) => "matrix",
            Value::Grob(_) => "graphic",
        }
    }

    pub fn type_code(&self) -> i64 {
        match self {
            Value::Real(_) => type_code::REAL,
            Value::Int(_) => type_code::INTEGER,
            Value::Str(_) => type_code::STRING,
            Value::Complex(_) => type_code::COMPLEX,
            Value::Matrix(_) => type_code::MATRIX,
            Value::List(_) => type_code::LIST,
            Value::Grob(_) => type_code::GRAPHIC,
        }
    }

    /// The value as a real. An HP integer converts to its unsigned word,
    /// the same number its `#…` display shows (`#FFFFFFFFh` is 4294967295).
    /// UNVERIFIED on G1: with the `Signed` integer setting on, the
    /// calculator may convert words with the top bit set as negative
    /// two's-complement values in implicit conversions too (only `B→R`
    /// honours `Signed` here).
    pub fn as_real(&self) -> Option<Real> {
        match self {
            Value::Real(r) => Some(*r),
            // `from_u64`, not `as i64`: a 64-bit word with the top bit set
            // must not turn negative.
            Value::Int(i) => Some(Real::from_u64(i.unsigned())),
            Value::Complex(c) if c.im.is_zero() => Some(c.re),
            _ => None,
        }
    }

    /// A real argument, or "Bad argument type".
    pub fn expect_real(&self, what: &str) -> Result<Real, RuntimeError> {
        self.as_real().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::BadArgType,
                format!("{what}: expected a number, got {}", self.type_name()),
            )
        })
    }

    /// An integer-valued argument (reals are truncated toward zero).
    pub fn expect_i64(&self, what: &str) -> Result<i64, RuntimeError> {
        let r = self.expect_real(what)?;
        r.trunc().to_i64().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::BadArgValue,
                format!("{what}: {r} is out of range"),
            )
        })
    }

    pub fn expect_str(&self, what: &str) -> Result<Rc<str>, RuntimeError> {
        match self {
            Value::Str(s) => Ok(s.clone()),
            other => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("{what}: expected a string, got {}", other.type_name()),
            )),
        }
    }

    pub fn expect_list(&self, what: &str) -> Result<Rc<Vec<Value>>, RuntimeError> {
        match self {
            Value::List(l) => Ok(l.clone()),
            other => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("{what}: expected a list, got {}", other.type_name()),
            )),
        }
    }

    /// PPL truth: non-zero numbers are true.
    pub fn truthy(&self) -> Result<bool, RuntimeError> {
        match self {
            Value::Real(r) => Ok(!r.is_zero()),
            Value::Int(i) => Ok(i.unsigned() != 0),
            Value::Complex(c) => Ok(!(c.re.is_zero() && c.im.is_zero())),
            other => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("a condition must be a number, got {}", other.type_name()),
            )),
        }
    }

    /// Approximate memory footprint, used by host resource limits.
    pub fn approx_bytes(&self) -> usize {
        match self {
            Value::Str(s) => 16 + s.len(),
            Value::List(l) => 16 + l.iter().map(Value::approx_bytes).sum::<usize>(),
            Value::Matrix(m) => 32 + m.data.len() * 24,
            _ => 24,
        }
    }

    /// How the value appears in the terminal and in `STRING()`.
    pub fn display(&self, opts: FormatOptions) -> String {
        let mut s = String::new();
        self.write_display(&mut s, opts, false);
        s
    }

    /// The `STRING()` form of this value: like [`Value::display`], but a
    /// top-level string is wrapped in double quotes, matching the
    /// hardware (`STRING("a")` on a physical G1 yields `"a"` — verified
    /// 2026-10-23 by note-channel differential evidence; the emulator
    /// used to return the bare text).
    pub fn string_form(&self, opts: FormatOptions) -> String {
        let mut s = String::new();
        self.write_display(&mut s, opts, true);
        s
    }

    fn write_display(&self, out: &mut String, opts: FormatOptions, nested: bool) {
        match self {
            Value::Real(r) => out.push_str(&r.format(opts)),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Complex(c) => out.push_str(&c.format(opts)),
            Value::Str(s) => {
                if nested {
                    let _ = write!(out, "\"{s}\"");
                } else {
                    out.push_str(s);
                }
            }
            Value::List(items) => {
                out.push('{');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_display(out, opts, true);
                }
                out.push('}');
            }
            Value::Matrix(m) => {
                let row = |out: &mut String, r: usize| {
                    out.push('[');
                    for (i, v) in m.row(r).iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        v.write_display(out, opts, true);
                    }
                    out.push(']');
                };
                if m.vector {
                    row(out, 0);
                } else {
                    out.push('[');
                    for r in 0..m.rows {
                        if r > 0 {
                            out.push(',');
                        }
                        row(out, r);
                    }
                    out.push(']');
                }
            }
            Value::Grob(n) => {
                let _ = write!(out, "G{n}");
            }
        }
    }

    /// Structural equality used by `==` and `=` (numbers compare by value
    /// across real/integer/complex).
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.equals(y))
            }
            (Value::Matrix(a), Value::Matrix(b)) => {
                a.rows == b.rows
                    && a.cols == b.cols
                    && a.data.iter().zip(&b.data).all(|(x, y)| x.equals(y))
            }
            (Value::Grob(a), Value::Grob(b)) => a == b,
            (Value::Complex(a), b) | (b, Value::Complex(a)) => match b {
                Value::Complex(c) => a == c,
                other => other.as_real().is_some_and(|r| a.im.is_zero() && a.re == r),
            },
            (a, b) => match (a.as_real(), b.as_real()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }
}

pub fn default_format() -> FormatOptions {
    FormatOptions {
        format: NumFormat::Standard,
        exp_char: 'E',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_like_the_terminal() {
        let o = default_format();
        let l = Value::list(vec![
            Value::from_i64(1),
            Value::str("a"),
            Value::list(vec![]),
        ]);
        assert_eq!(l.display(o), "{1,\"a\",{}}");
        let m = Value::Matrix(Rc::new(Matrix {
            rows: 2,
            cols: 2,
            data: (1..=4).map(Value::from_i64).collect(),
            vector: false,
        }));
        assert_eq!(m.display(o), "[[1,2],[3,4]]");
        assert_eq!(Value::str("hi").display(o), "hi");
    }

    #[test]
    fn integers_convert_to_their_displayed_unsigned_word() {
        use ppl_math::IntBase;
        let o = default_format();
        let max64 = Value::Int(HpInt::new(-1, 64, IntBase::Hex));
        assert_eq!(max64.display(o), "#FFFFFFFFFFFFFFFFh");
        // Was -1: the word was reinterpreted as i64.
        assert_eq!(max64.as_real().unwrap().to_string(), "1.84467440737E19");
        let top = Value::Int(HpInt::new(1i128 << 63, 64, IntBase::Dec));
        assert_eq!(top.display(o), "#9223372036854775808d");
        assert!(!top.as_real().unwrap().is_negative());
        // UNVERIFIED on G1: -5 in a 32-bit word converts as 4294967291
        // (unsigned, like its display), not -5, even with `Signed` on.
        let neg5 = Value::Int(HpInt::new(-5, 32, IntBase::Dec));
        assert_eq!(neg5.display(o), "#4294967291d");
        assert_eq!(neg5.as_real().unwrap().to_string(), "4294967291");
        let small = Value::Int(HpInt::new(255, 8, IntBase::Hex));
        assert_eq!(small.as_real().unwrap().to_string(), "255");
        assert!(max64.equals(&Value::Real(Real::from_u64(u64::MAX))));
        assert!(!max64.equals(&Value::from_i64(-1)));
    }
}
