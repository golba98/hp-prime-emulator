//! Matrix and vector commands.

use std::rc::Rc;

use ppl_compiler::builtins::Builtin;
use ppl_math::Real;
use ppl_syntax::ast::BinOp;

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::ops;
use crate::value::{Matrix, Value};

fn matrix<'a>(v: &'a Value, name: &str) -> RtResult<&'a Rc<Matrix>> {
    match v {
        Value::Matrix(m) => Ok(m),
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("{name}: expected a matrix, got {}", other.type_name()),
        )),
    }
}

fn wrap(m: Matrix) -> Value {
    Value::Matrix(Rc::new(m))
}

fn size(v: &Value, name: &str) -> RtResult<usize> {
    let n = v.expect_i64(name)?;
    if !(1..=10_000).contains(&n) {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            format!("{name}: dimension {n} out of range"),
        ));
    }
    Ok(n as usize)
}

fn vector_values(v: &Value, name: &str) -> RtResult<Vec<Value>> {
    match v {
        Value::Matrix(m) => Ok(m.data.clone()),
        Value::List(l) => Ok(l.iter().cloned().collect()),
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("{name}: expected a vector, got {}", other.type_name()),
        )),
    }
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let a0 = arg(args, 0, name)?;
    let s = m.settings.clone();
    match b {
        B::Makemat => {
            // Only constant fill values are supported; MAKEMAT(expr in I,J)
            // is evaluated once (approximation). MAKEMAT(expr, n) makes a
            // vector of n elements.
            if args.len() == 2 {
                let n = size(arg(args, 1, name)?, name)?;
                m.check_len(n)?;
                let mut mx = Matrix::zeros(1, n);
                mx.vector = true;
                mx.data.fill(a0.clone());
                return Ok(wrap(mx));
            }
            let (r, c) = (
                size(arg(args, 1, name)?, name)?,
                size(arg(args, 2, name)?, name)?,
            );
            m.check_len(r * c)?;
            let mut mx = Matrix::zeros(r, c);
            mx.data.fill(a0.clone());
            Ok(wrap(mx))
        }
        B::Det => Ok(Value::Real(ops::determinant(matrix(a0, name)?)?)),
        B::Trn => {
            let mx = matrix(a0, name)?;
            if mx.vector {
                return Ok(Value::Matrix(mx.clone()));
            }
            let mut out = Matrix::zeros(mx.cols, mx.rows);
            for r in 0..mx.rows {
                for c in 0..mx.cols {
                    out.set(c, r, mx.get(r, c).clone());
                }
            }
            Ok(wrap(out))
        }
        B::Idenmat => Ok(wrap(ops::identity(size(a0, name)?))),
        B::Inverse => Ok(wrap(ops::inverse(matrix(a0, name)?, &s)?)),
        B::Rank => {
            let mx = matrix(a0, name)?;
            let mut a: Vec<f64> = mx
                .data
                .iter()
                .map(|v| v.expect_real(name).map(|r| r.to_f64()))
                .collect::<RtResult<_>>()?;
            let (rows, cols) = (mx.rows, mx.cols);
            let mut rank = 0;
            for col in 0..cols {
                if rank == rows {
                    break;
                }
                let pivot = (rank..rows)
                    .max_by(|&i, &j| a[i * cols + col].abs().total_cmp(&a[j * cols + col].abs()))
                    .unwrap();
                if a[pivot * cols + col].abs() < 1e-11 {
                    continue;
                }
                for k in 0..cols {
                    a.swap(rank * cols + k, pivot * cols + k);
                }
                for r in 0..rows {
                    if r != rank {
                        let f = a[r * cols + col] / a[rank * cols + col];
                        for k in 0..cols {
                            a[r * cols + k] -= f * a[rank * cols + k];
                        }
                    }
                }
                rank += 1;
            }
            Ok(Value::from_i64(rank as i64))
        }
        B::Colnorm | B::Rownorm => {
            let mx = matrix(a0, name)?;
            let (outer, inner) = if b == B::Colnorm {
                (mx.cols, mx.rows)
            } else {
                (mx.rows, mx.cols)
            };
            let mut best = Real::ZERO;
            for o in 0..outer {
                let mut sum = Real::ZERO;
                for i in 0..inner {
                    let v = if b == B::Colnorm {
                        mx.get(i, o)
                    } else {
                        mx.get(o, i)
                    };
                    sum = sum.add(match v {
                        Value::Complex(c) => c.abs()?,
                        other => other.expect_real(name)?.abs(),
                    })?;
                }
                if sum > best {
                    best = sum;
                }
            }
            Ok(Value::Real(best))
        }
        B::Addrow | B::Addcol => {
            let mx = matrix(a0, name)?;
            let vals = vector_values(arg(args, 1, name)?, name)?;
            let at = arg(args, 2, name)?.expect_i64(name)?;
            let row = b == B::Addrow;
            let (len, count) = if row {
                (mx.cols, mx.rows)
            } else {
                (mx.rows, mx.cols)
            };
            if vals.len() != len || at < 1 || at as usize > count + 1 {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            let at = at as usize - 1;
            let (nr, nc) = if row {
                (mx.rows + 1, mx.cols)
            } else {
                (mx.rows, mx.cols + 1)
            };
            let mut out = Matrix::zeros(nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    let v = if row {
                        if r == at {
                            vals[c].clone()
                        } else {
                            mx.get(if r > at { r - 1 } else { r }, c).clone()
                        }
                    } else if c == at {
                        vals[r].clone()
                    } else {
                        mx.get(r, if c > at { c - 1 } else { c }).clone()
                    };
                    out.set(r, c, v);
                }
            }
            Ok(wrap(out))
        }
        B::Delrow | B::Delcol => {
            let mx = matrix(a0, name)?;
            let at = arg(args, 1, name)?.expect_i64(name)?;
            let row = b == B::Delrow;
            let count = if row { mx.rows } else { mx.cols };
            if at < 1 || at as usize > count || count == 1 {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            let at = at as usize - 1;
            let (nr, nc) = if row {
                (mx.rows - 1, mx.cols)
            } else {
                (mx.rows, mx.cols - 1)
            };
            let mut out = Matrix::zeros(nr, nc);
            for r in 0..nr {
                for c in 0..nc {
                    let (sr, sc) = if row {
                        (if r >= at { r + 1 } else { r }, c)
                    } else {
                        (r, if c >= at { c + 1 } else { c })
                    };
                    out.set(r, c, mx.get(sr, sc).clone());
                }
            }
            Ok(wrap(out))
        }
        B::Redim => {
            let mx = matrix(a0, name)?;
            let dims = arg(args, 1, name)?.expect_list(name)?;
            let r = size(
                dims.first()
                    .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?,
                name,
            )?;
            let c = match dims.get(1) {
                Some(v) => size(v, name)?,
                None => 1,
            };
            let mut out = Matrix::zeros(r, c);
            for i in 0..r.min(mx.rows) {
                for j in 0..c.min(mx.cols) {
                    out.set(i, j, mx.get(i, j).clone());
                }
            }
            Ok(wrap(out))
        }
        B::Dot => {
            let (x, y) = (
                vector_values(a0, name)?,
                vector_values(arg(args, 1, name)?, name)?,
            );
            if x.len() != y.len() {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            let mut acc = Value::zero();
            for (p, q) in x.iter().zip(&y) {
                acc = ops::binary(BinOp::Add, &acc, &ops::binary(BinOp::Mul, p, q, &s)?, &s)?;
            }
            Ok(acc)
        }
        B::Cross => {
            let (x, y) = (
                vector_values(a0, name)?,
                vector_values(arg(args, 1, name)?, name)?,
            );
            if x.len() != 3 || y.len() != 3 {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            let mul = |a: &Value, b: &Value| ops::binary(BinOp::Mul, a, b, &s);
            let sub = |a: Value, b: Value| ops::binary(BinOp::Sub, &a, &b, &s);
            let data = vec![
                sub(mul(&x[1], &y[2])?, mul(&x[2], &y[1])?)?,
                sub(mul(&x[2], &y[0])?, mul(&x[0], &y[2])?)?,
                sub(mul(&x[0], &y[1])?, mul(&x[1], &y[0])?)?,
            ];
            Ok(wrap(Matrix {
                rows: 1,
                cols: 3,
                data,
                vector: true,
            }))
        }
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a matrix command"),
        )),
    }
}
