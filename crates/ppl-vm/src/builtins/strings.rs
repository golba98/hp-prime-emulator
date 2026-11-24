//! String commands. Positions are 1-based and count HP UTF-16 code units.

use ppl_compiler::builtins::Builtin;
mod formatting;
mod fractions;

pub use formatting::default_string_units;

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::value::Value;

pub(crate) fn menu_label(machine: &Machine, value: &Value) -> RtResult<Value> {
    formatting::menu_label(machine, value)
}

fn units(v: &Value, name: &str) -> RtResult<Vec<u16>> {
    Ok(v.expect_string_units(name)?.into_owned())
}

fn s(v: Vec<u16>) -> Value {
    Value::from_utf16(v)
}

pub(super) fn position(hay: &[u16], needle: &[u16]) -> usize {
    if needle.is_empty() {
        return 0;
    }
    hay.windows(needle.len())
        .position(|window| window == needle)
        .map_or(0, |i| i + 1)
}

fn change_case(value: &Value, name: &str, upper: bool) -> RtResult<Value> {
    let mut out = Vec::new();
    for decoded in char::decode_utf16(units(value, name)?) {
        match decoded {
            Ok(ch) => {
                let mapped = if upper {
                    ch.to_uppercase().collect::<String>()
                } else {
                    ch.to_lowercase().collect::<String>()
                };
                out.extend(mapped.encode_utf16());
            }
            Err(unit) => out.push(unit.unpaired_surrogate()),
        }
    }
    Ok(s(out))
}

// DIM maps over nested lists; SIZE is the list-length operation. Matrix
// dimensions describe the complete array rather than mapping over its cells.
fn dimension(value: &Value, name: &str) -> RtResult<Value> {
    match value {
        value if value.is_string() => Ok(Value::from_i64(
            value.expect_string_units(name)?.len() as i64
        )),
        Value::List(values) => Ok(Value::list(
            values
                .iter()
                .map(|value| dimension(value, name))
                .collect::<RtResult<_>>()?,
        )),
        Value::Matrix(matrix) => Ok(Value::list(if matrix.vector {
            vec![Value::from_i64(matrix.cols as i64)]
        } else {
            vec![
                Value::from_i64(matrix.rows as i64),
                Value::from_i64(matrix.cols as i64),
            ]
        })),
        Value::Real(_) | Value::Int(_) | Value::Complex(_) | Value::Symbolic(_) => {
            Ok(Value::from_i64(1))
        }
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("DIM of a {}", other.type_name()),
        )),
    }
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let a0 = arg(args, 0, name)?;
    match b {
        // ASC returns a list of codes (User Guide); unverified for 1 char.
        B::Asc => Ok(Value::list(
            units(a0, name)?
                .into_iter()
                .map(|unit| Value::from_i64(i64::from(unit)))
                .collect(),
        )),
        B::Char => {
            let codes: Vec<Value> = match a0 {
                Value::List(l) => l.iter().cloned().collect(),
                other => vec![other.clone()],
            };
            let mut out = Vec::new();
            for c in codes {
                // G1 truncates the numeric code to its low 16 bits, including
                // negative values, rather than treating it as a Unicode scalar.
                out.push(c.expect_i64(name)? as u16);
            }
            Ok(s(out))
        }

        B::Dim => dimension(a0, name),
        B::Instring => {
            let hay = a0.expect_string_units(name)?;
            let needle = arg(args, 1, name)?.expect_string_units(name)?;
            Ok(Value::from_i64(position(&hay, &needle) as i64))
        }

        B::Left | B::Right => {
            let c = units(a0, name)?;
            let n = arg(args, 1, name)?
                .expect_i64(name)?
                .clamp(0, c.len() as i64) as usize;
            Ok(if b == B::Left {
                s(c[..n].to_vec())
            } else {
                s(c[c.len() - n..].to_vec())
            })
        }
        B::Mid => {
            let c = units(a0, name)?;
            let start = arg(args, 1, name)?.expect_i64(name)?;
            if start < 1 {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "MID start must be ≥ 1",
                ));
            }
            let from = (start as usize - 1).min(c.len());
            let len = match args.get(2) {
                Some(n) => n.expect_i64(name)?.max(0) as usize,
                None => c.len(),
            };
            let to = from.saturating_add(len).min(c.len());
            Ok(s(c[from..to].to_vec()))
        }
        B::Rotate => {
            let mut c = units(a0, name)?;
            if c.is_empty() {
                return Ok(Value::str(""));
            }
            let n = args
                .get(1)
                .map(|v| v.expect_i64(name))
                .transpose()?
                .unwrap_or(-1);
            let k = n.rem_euclid(c.len() as i64) as usize;
            c.rotate_left(k);
            Ok(s(c))
        }
        B::StringFn => formatting::string(m, args),
        B::Upper => change_case(a0, name, true),
        B::Lower => change_case(a0, name, false),
        B::Replace => {
            let src = a0.expect_string_units(name)?;
            let with = arg(args, 2, name)?.expect_string_units(name)?;
            let selector = arg(args, 1, name)?;
            if selector.is_string() {
                let old = selector.expect_string_units(name)?;
                if old.is_empty() {
                    return Ok(a0.clone());
                }
                let mut result = Vec::new();
                let mut at = 0;
                while at < src.len() {
                    if src[at..].starts_with(&old) {
                        result.extend_from_slice(&with);
                        at += old.len();
                    } else {
                        result.push(src[at]);
                        at += 1;
                    }
                }
                Ok(s(result))
            } else {
                let start = selector.expect_i64(name)?;
                if start < 1 || start as usize > src.len() + 1 {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "REPLACE position out of range",
                    ));
                }
                let from = start as usize - 1;
                let end = from.saturating_add(with.len()).min(src.len());
                let mut result = src.into_owned();
                result.splice(from..end, with.iter().copied());
                Ok(s(result))
            }
        }

        B::Expr => {
            let text = a0.expect_str(name)?;
            m.eval_text(&text)
        }
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a string command"),
        )),
    }
}
