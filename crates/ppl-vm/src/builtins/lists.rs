//! List commands. All return new lists; arguments are never modified.

use ppl_compiler::builtins::Builtin;
use ppl_math::Real;
use ppl_syntax::ast::BinOp;

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::ops;
use crate::value::Value;

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let a0 = arg(args, 0, name)?;
    let s = m.settings.clone();
    match b {
        B::Concat => {
            if args.iter().all(|v| matches!(v, Value::Str(_))) {
                let mut out = String::new();
                for v in args {
                    out.push_str(&v.expect_str(name)?);
                }
                return Ok(Value::str(out));
            }
            let mut out = Vec::new();
            for v in args {
                match v {
                    Value::List(l) => out.extend(l.iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            m.check_len(out.len())?;
            Ok(Value::list(out))
        }
        B::Makelist => Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            "MAKELIST(expression, variable, start, end[, step])",
        )),
        B::Size => Ok(match a0 {
            Value::List(l) => Value::from_i64(l.len() as i64),
            Value::Str(st) => Value::from_i64(st.chars().count() as i64),
            Value::Matrix(mx) if mx.vector => Value::list(vec![Value::from_i64(mx.cols as i64)]),
            Value::Matrix(mx) => Value::list(vec![
                Value::from_i64(mx.rows as i64),
                Value::from_i64(mx.cols as i64),
            ]),
            _ => Value::from_i64(1),
        }),
        B::Sort => {
            let mut items: Vec<Value> = a0.expect_list(name)?.iter().cloned().collect();
            let mut err = None;
            items.sort_by(|x, y| {
                ops::compare(x, y).unwrap_or_else(|e| {
                    err.get_or_insert(e);
                    std::cmp::Ordering::Equal
                })
            });
            if let Some(e) = err {
                return Err(e);
            }
            if let Some(dir) = args.get(1) {
                if dir.expect_i64(name)? == 2 {
                    items.reverse();
                }
            }
            Ok(Value::list(items))
        }
        B::Reverse => match a0 {
            Value::Str(st) => Ok(Value::str(st.chars().rev().collect::<String>())),
            other => {
                let mut items: Vec<Value> = other.expect_list(name)?.iter().cloned().collect();
                items.reverse();
                Ok(Value::list(items))
            }
        },
        B::Pos => {
            let needle = arg(args, 1, name)?;
            match a0 {
                Value::Str(st) => {
                    let n = needle.expect_str(name)?;
                    Ok(Value::from_i64(
                        st.find(&*n)
                            .map(|i| st[..i].chars().count() as i64 + 1)
                            .unwrap_or(0),
                    ))
                }
                other => {
                    let l = other.expect_list(name)?;
                    Ok(Value::from_i64(
                        l.iter()
                            .position(|v| v.equals(needle))
                            .map(|i| i as i64 + 1)
                            .unwrap_or(0),
                    ))
                }
            }
        }
        B::SumList | B::ProdList => {
            let l = a0.expect_list(name)?;
            let mut acc = Value::Real(if b == B::SumList {
                Real::ZERO
            } else {
                Real::ONE
            });
            let op = if b == B::SumList {
                BinOp::Add
            } else {
                BinOp::Mul
            };
            for v in l.iter() {
                acc = ops::binary(op, &acc, v, &s)?;
            }
            Ok(acc)
        }
        B::DeltaList => {
            let l = a0.expect_list(name)?;
            let out = l
                .windows(2)
                .map(|w| ops::binary(BinOp::Sub, &w[1], &w[0], &s))
                .collect::<RtResult<Vec<_>>>()?;
            Ok(Value::list(out))
        }
        B::Equal => Ok(Value::from_bool(a0.equals(arg(args, 1, name)?))),
        B::Head => match a0 {
            Value::Str(st) => Ok(Value::str(
                st.chars().next().map(String::from).unwrap_or_default(),
            )),
            other => other.expect_list(name)?.first().cloned().ok_or_else(|| {
                RuntimeError::new(ErrorKind::InvalidDimension, "HEAD of an empty list")
            }),
        },
        B::Tail => match a0 {
            Value::Str(st) => Ok(Value::str(st.chars().skip(1).collect::<String>())),
            other => {
                let l = other.expect_list(name)?;
                if l.is_empty() {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidDimension,
                        "TAIL of an empty list",
                    ));
                }
                Ok(Value::list(l[1..].to_vec()))
            }
        },
        B::Append => {
            let mut items: Vec<Value> = a0.expect_list(name)?.iter().cloned().collect();
            items.push(arg(args, 1, name)?.clone());
            m.check_len(items.len())?;
            Ok(Value::list(items))
        }
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a list command"),
        )),
    }
}
