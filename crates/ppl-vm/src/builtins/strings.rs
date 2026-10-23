//! String commands. Positions are 1-based and count characters.

use ppl_compiler::builtins::Builtin;
use ppl_math::NumFormat;

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::value::Value;

fn chars(v: &Value, name: &str) -> RtResult<Vec<char>> {
    Ok(v.expect_str(name)?.chars().collect())
}

fn s(v: Vec<char>) -> Value {
    Value::str(v.into_iter().collect::<String>())
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let a0 = arg(args, 0, name)?;
    match b {
        // ASC returns a list of codes (User Guide); unverified for 1 char.
        B::Asc => Ok(Value::list(
            chars(a0, name)?
                .into_iter()
                .map(|c| Value::from_i64(c as i64))
                .collect(),
        )),
        B::Char => {
            let codes: Vec<Value> = match a0 {
                Value::List(l) => l.iter().cloned().collect(),
                other => vec![other.clone()],
            };
            let mut out = String::new();
            for c in codes {
                let n = c.expect_i64(name)?;
                out.push(char::from_u32(n as u32).ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::BadArgValue,
                        format!("{n} is not a character code"),
                    )
                })?);
            }
            Ok(Value::str(out))
        }
        B::Dim => match a0 {
            Value::Str(st) => Ok(Value::from_i64(st.chars().count() as i64)),
            Value::List(l) => Ok(Value::from_i64(l.len() as i64)),
            Value::Matrix(mx) => Ok(Value::list(vec![
                Value::from_i64(mx.rows as i64),
                Value::from_i64(mx.cols as i64),
            ])),
            other => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("DIM of a {}", other.type_name()),
            )),
        },
        B::Instring => {
            let hay = a0.expect_str(name)?;
            let needle = arg(args, 1, name)?.expect_str(name)?;
            Ok(Value::from_i64(match hay.find(&*needle) {
                Some(byte) if !needle.is_empty() => hay[..byte].chars().count() as i64 + 1,
                _ => 0,
            }))
        }
        B::Left | B::Right => {
            let c = chars(a0, name)?;
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
            let c = chars(a0, name)?;
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
            let to = (from + len).min(c.len());
            Ok(s(c[from..to].to_vec()))
        }
        B::Rotate => {
            let mut c = chars(a0, name)?;
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
        B::StringFn => {
            if args.len() > 3 {
                return Err(RuntimeError::new(
                    ErrorKind::Unsupported,
                    "STRING: the separator and size-limit arguments are not emulated",
                ));
            }
            let mut opts = m.settings.format();
            if let Some(mode) = args.get(1) {
                let digits = args
                    .get(2)
                    .map(|v| v.expect_i64(name))
                    .transpose()?
                    .unwrap_or(m.settings.hdigits)
                    .clamp(0, 11) as u8;
                opts.format = match mode.expect_i64(name)? {
                    2 => NumFormat::Fixed(digits),
                    3 => NumFormat::Scientific(digits),
                    4 => NumFormat::Engineering(digits),
                    _ => NumFormat::Standard,
                };
            }
            Ok(Value::str(a0.string_form(opts)))
        }
        B::Upper => Ok(Value::str(a0.expect_str(name)?.to_uppercase())),
        B::Lower => Ok(Value::str(a0.expect_str(name)?.to_lowercase())),
        B::Replace => {
            let src = a0.expect_str(name)?;
            let with = arg(args, 2, name)?.expect_str(name)?;
            match arg(args, 1, name)? {
                Value::Str(old) => {
                    if old.is_empty() {
                        return Ok(a0.clone());
                    }
                    Ok(Value::str(src.replace(&**old, &with)))
                }
                pos => {
                    let start = pos.expect_i64(name)?;
                    let mut c: Vec<char> = src.chars().collect();
                    if start < 1 || start as usize > c.len() + 1 {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgValue,
                            "REPLACE position out of range",
                        ));
                    }
                    let from = start as usize - 1;
                    let new: Vec<char> = with.chars().collect();
                    let end = (from + new.len()).min(c.len());
                    c.splice(from..end, new);
                    Ok(s(c))
                }
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
