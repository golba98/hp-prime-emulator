//! List commands and collection append. Collection operands are never modified.

use std::{collections::BTreeMap, rc::Rc};

use ppl_compiler::builtins::Builtin;
use ppl_math::Real;
use ppl_syntax::ast::BinOp;

use super::arg;
use crate::cas;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::{Machine, numeric_entry};
use crate::ops;
use crate::value::{Matrix, Value};

fn append_list(machine: &Machine, entries: &[Value], item: &Value) -> RtResult<Value> {
    machine.check_len(entries.len() + 1)?;
    let mut entries = entries.to_vec();
    entries.push(item.clone());
    Ok(Value::list(entries))
}

fn append_vector(machine: &Machine, vector: &Matrix, item: Value) -> RtResult<Value> {
    machine.check_len(vector.data.len() + 1)?;
    let mut vector = vector.clone();
    vector.data.push(item);
    vector.cols = vector.data.len();
    Ok(Value::Matrix(Rc::new(vector)))
}

struct TextAssignment<'a> {
    target: Option<&'a str>,
    payload: Value,
}

pub(crate) enum AppendPayload {
    Literal,
    Evaluated,
    Callback,
}

pub(crate) fn append_collection(
    machine: &mut Machine,
    args: &[Value],
    payload: AppendPayload,
    encoded_base: bool,
) -> RtResult<Value> {
    let Ok(args) = <&[Value; 2]>::try_from(args) else {
        return call(machine, Builtin::Append, "APPEND", args);
    };
    let [base, item] = args;
    let literal_payload = matches!(payload, AppendPayload::Literal);
    if (!literal_payload && matches!(item, Value::Str(_)))
        || (matches!(payload, AppendPayload::Callback) && item.is_positive_infinity())
    {
        if encoded_base && let Value::Matrix(array) = base {
            machine.check_len(array.data.len().saturating_add(4))?;
            let entries = array
                .encoding_prefix()
                .into_iter()
                .chain(array.data.iter().cloned())
                .chain(std::iter::once(item.clone()));
            return Ok(Value::list(entries.collect()));
        }
        return append_item(machine, args, Some(item));
    }
    let mut assignment = None;
    let text_item = if literal_payload
        && let Value::Str(text) = item
        && matches!(base, Value::List(_) | Value::Matrix(_))
    {
        if let Value::Matrix(vector) = base
            && vector.vector
            && text.contains(';')
            && let Some(result) = append_assignment_sequence(machine, vector, text)?
        {
            return Ok(result);
        }
        if let Some(result) = append_text_assignment(machine, text)? {
            assignment = result.target.map(|target| (target, result.payload.clone()));
            Some(result.payload)
        } else {
            append_text_payload(machine, text, &BTreeMap::new())?
        }
    } else {
        None
    };
    let result = append_item(machine, args, text_item.as_ref())?;
    if let Some((target, value)) = assignment {
        if target == "HAngle" {
            machine
                .settings
                .set_angle_code(value.expect_i64("setting")?)?;
        } else {
            machine.store_home_value(target, value)?;
        }
    }
    Ok(result)
}

pub(crate) fn append_nonfinite_quotient(
    machine: &mut Machine,
    base: Value,
    undefined: bool,
) -> RtResult<Value> {
    let value = if undefined {
        ppl_cas::CasValue::Undefined
    } else {
        ppl_cas::CasValue::Infinity(ppl_cas::InfinitySign::Positive)
    };
    let args = [base, cas::from_cas(value)];
    append_item(machine, &args, Some(&args[1]))
}

pub(crate) fn append_vector_infinity(machine: &Machine, vector: &Matrix) -> RtResult<Value> {
    let infinity = cas::from_cas(ppl_cas::CasValue::Infinity(ppl_cas::InfinitySign::Positive));
    append_vector(machine, vector, infinity)
}

pub(crate) fn append_callback_division_error(
    machine: &mut Machine,
    base: Value,
    encoded_base: bool,
) -> RtResult<Value> {
    let encoded_matrix = encoded_base && matches!(&base, Value::Matrix(matrix) if !matrix.vector);
    append_collection(
        machine,
        &[base, Value::str("Error: X/0")],
        AppendPayload::Evaluated,
        encoded_matrix,
    )
}

fn append_item(
    machine: &mut Machine,
    args: &[Value; 2],
    text_item: Option<&Value>,
) -> RtResult<Value> {
    let [base, item] = args;
    let item = text_item.unwrap_or(item);
    let array = match base {
        Value::List(entries) => return append_list(machine, entries, item),
        Value::Matrix(array) => array,
        _ => return call(machine, Builtin::Append, "APPEND", args),
    };
    let numeric = matches!(item, Value::Real(_) | Value::Int(_) | Value::Complex(_));
    let vector_or_list =
        matches!(item, Value::List(_)) || matches!(item, Value::Matrix(payload) if payload.vector);
    if array.vector {
        if numeric {
            return append_vector(machine, array, numeric_entry(item.clone())?);
        }
        if vector_or_list || text_item.is_some() {
            return append_list(machine, &array.data, item);
        }
    } else if let Value::Matrix(row) = item
        && row.vector
        && row.cols == array.cols
    {
        machine.check_len(array.data.len() + row.data.len())?;
        let mut matrix = (**array).clone();
        for cell in &row.data {
            matrix.data.push(numeric_entry(cell.clone())?);
        }
        matrix.rows += 1;
        return Ok(Value::Matrix(Rc::new(matrix)));
    } else if text_item.is_some()
        || (array.cols == 1
            && (numeric
                || matches!(item, Value::Matrix(column) if !column.vector && column.cols == 1)))
    {
        machine.check_len(array.rows + 1)?;
        let mut rows = Vec::with_capacity(array.rows + 1);
        for row in 0..array.rows {
            rows.push(Value::Matrix(Rc::new(Matrix {
                rows: 1,
                cols: array.cols,
                data: array.row(row).to_vec(),
                vector: true,
            })));
        }
        rows.push(item.clone());
        return Ok(Value::list(rows));
    }
    call(machine, Builtin::Append, "APPEND", args)
}

fn append_assignment_sequence(
    machine: &mut Machine,
    vector: &Matrix,
    text: &str,
) -> RtResult<Option<Value>> {
    let mut result = vector.clone();
    let mut pending = BTreeMap::new();
    let mut assignments = Vec::new();
    for statement in text.split(';') {
        let Some((target, expression)) = statement.split_once(":=") else {
            return Ok(None);
        };
        let target = target.trim();
        if !matches!(target.as_bytes(), [b'A'..=b'Z']) {
            return Ok(None);
        }
        let Some(Value::Real(value)) = append_text_payload(machine, expression, &pending)? else {
            return Ok(None);
        };
        machine.check_len(result.data.len() + 1)?;
        result.data.push(numeric_entry(Value::Real(value))?);
        pending.insert(target, value);
        assignments.push((target, value));
    }

    // Reads see earlier assignments through pending. Commit once the entire
    // vector has been built and its resource limits have been checked.
    result.cols = result.data.len();
    for (target, value) in assignments {
        machine.store_home_value(target, Value::Real(value))?;
    }
    Ok(Some(Value::Matrix(Rc::new(result))))
}

fn append_text_assignment<'a>(
    machine: &Machine,
    text: &'a str,
) -> RtResult<Option<TextAssignment<'a>>> {
    let Some((target, expression)) = text.split_once(":=") else {
        return Ok(None);
    };
    let target = target.trim();
    if target != "HAngle" && !matches!(target.as_bytes(), [b'A'..=b'Z']) {
        return Ok(None);
    }
    let pending = BTreeMap::new();
    let payload = if let Some((numerator, denominator)) = expression.split_once('/') {
        let (Some(Value::Real(numerator)), Some(Value::Real(denominator))) = (
            append_text_atom(machine, numerator.trim(), &pending),
            append_text_atom(machine, denominator.trim(), &pending),
        ) else {
            return Ok(None);
        };
        if denominator.is_zero() {
            (!numerator.is_zero()).then(|| {
                cas::from_cas(ppl_cas::CasValue::Infinity(ppl_cas::InfinitySign::Positive))
            })
        } else {
            numerator.quotient_if_integer(denominator)?.map(Value::Real)
        }
    } else {
        match append_text_payload(machine, expression, &pending)? {
            Some(payload @ Value::Real(_)) => Some(payload),
            Some(Value::Symbolic(value))
                if target != "HAngle"
                    && matches!(value.as_ref(), ppl_cas::CasValue::Symbolic(_)) =>
            {
                None
            }
            _ => return Ok(None),
        }
    };
    Ok(Some(match payload {
        Some(payload) => TextAssignment {
            target: Some(target),
            payload,
        },
        None => TextAssignment {
            target: None,
            payload: Value::str(RuntimeError::kind(ErrorKind::BadArgType).to_string()),
        },
    }))
}

fn append_text_atom(
    machine: &Machine,
    text: &str,
    pending: &BTreeMap<&str, Real>,
) -> Option<Value> {
    if let Some(number) = Real::parse_versioned(text, machine.settings.numeric_version) {
        return Some(Value::Real(number));
    }
    if !cas::is_bare_identifier(text) {
        return None;
    }
    Some(
        pending
            .get(text)
            .copied()
            .or_else(|| machine.calculator_global(text).and_then(Value::as_real))
            .map_or_else(
                || cas::from_cas(ppl_cas::CasValue::Symbolic(text.into())),
                Value::Real,
            ),
    )
}

fn append_text_payload(
    machine: &Machine,
    text: &str,
    pending: &BTreeMap<&str, Real>,
) -> RtResult<Option<Value>> {
    let content = text.trim();
    if content.is_empty() {
        return Ok(Some(cas::from_cas(ppl_cas::CasValue::Undefined)));
    }
    if content.len() >= 2 && content.starts_with('"') && content.ends_with('"') {
        return Ok(Some(Value::str(text)));
    }
    if let Some(value) = append_text_atom(machine, content, pending) {
        return Ok(Some(value));
    }
    // Text conversion supports one addition of two atoms.
    let Some((left, right)) = content.split_once('+') else {
        return Ok(None);
    };
    let (left, right) = (left.trim(), right.trim());
    let (Some(a), Some(b)) = (
        append_text_atom(machine, left, pending),
        append_text_atom(machine, right, pending),
    ) else {
        return Ok(None);
    };
    if let (Value::Real(a), Value::Real(b)) = (a, b) {
        return Ok(Some(Value::Real(a.add(b)?)));
    }
    if !content.contains('_') {
        return Ok(Some(cas::from_cas(ppl_cas::CasValue::Symbolic(format!(
            "{left}+{right}"
        )))));
    }
    Ok(None)
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let a0 = arg(args, 0, name)?;
    let s = m.settings.clone();
    match b {
        B::Concat => {
            if args.iter().all(Value::is_string) {
                let mut out = Vec::new();
                for v in args {
                    out.extend_from_slice(&v.expect_string_units(name)?);
                }
                return Ok(Value::from_utf16(out));
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
            value if value.is_string() => {
                Value::from_i64(value.expect_string_units(name)?.len() as i64)
            }
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
            value if value.is_string() => {
                let mut units = value.expect_string_units(name)?.into_owned();
                units.reverse();
                Ok(Value::from_utf16(units))
            }
            other => {
                let mut items: Vec<Value> = other.expect_list(name)?.iter().cloned().collect();
                items.reverse();
                Ok(Value::list(items))
            }
        },
        B::Pos => {
            let needle = arg(args, 1, name)?;
            match a0 {
                value if value.is_string() => {
                    let hay = value.expect_string_units(name)?;
                    let needle = needle.expect_string_units(name)?;
                    Ok(Value::from_i64(
                        super::strings::position(&hay, &needle) as i64
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
            value if value.is_string() => Ok(Value::from_utf16(
                value
                    .expect_string_units(name)?
                    .iter()
                    .take(1)
                    .copied()
                    .collect::<Vec<_>>(),
            )),
            other => other.expect_list(name)?.first().cloned().ok_or_else(|| {
                RuntimeError::new(ErrorKind::InvalidDimension, "HEAD of an empty list")
            }),
        },
        B::Tail => match a0 {
            value if value.is_string() => Ok(Value::from_utf16(
                value
                    .expect_string_units(name)?
                    .iter()
                    .skip(1)
                    .copied()
                    .collect::<Vec<_>>(),
            )),
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
            let item = arg(args, 1, name)?;
            match a0 {
                Value::Matrix(vector) if vector.vector => {
                    if !matches!(item, Value::Real(_) | Value::Int(_) | Value::Complex(_)) {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgType,
                            "APPEND to a vector requires a numeric element",
                        ));
                    }
                    append_vector(m, vector, item.clone())
                }
                _ => append_list(m, &a0.expect_list(name)?, item),
            }
        }
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a list command"),
        )),
    }
}
