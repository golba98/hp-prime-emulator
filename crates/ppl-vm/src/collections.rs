//! 1-based indexing of lists, matrices and strings.

use std::rc::Rc;

use crate::error::{ErrorKind, ResourceKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::value::{Matrix, Value};

fn index_arg(v: &Value) -> RtResult<i64> {
    let r = v.expect_real("index")?;
    if !r.is_integer() {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            format!("index {r} is not an integer"),
        ));
    }
    r.to_i64()
        .ok_or_else(|| RuntimeError::new(ErrorKind::InvalidDimension, "index out of range"))
}

fn out_of_range(i: i64, len: usize) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::InvalidDimension,
        format!("index {i} is outside 1..{len}"),
    )
}

pub fn index_value(base: &Value, idx: &[Value], _m: &Machine) -> RtResult<Value> {
    if idx.is_empty() {
        return Ok(base.clone());
    }
    match base {
        Value::List(items) => {
            let i = index_arg(&idx[0])?;
            if i < 1 || i as usize > items.len() {
                return Err(out_of_range(i, items.len()));
            }
            index_value(&items[i as usize - 1], &idx[1..], _m)
        }
        Value::Matrix(m) => {
            let i = index_arg(&idx[0])?;
            if m.vector {
                if i < 1 || i as usize > m.cols {
                    return Err(out_of_range(i, m.cols));
                }
                let v = m.data[i as usize - 1].clone();
                return index_value(&v, &idx[1..], _m);
            }
            if i < 1 || i as usize > m.rows {
                return Err(out_of_range(i, m.rows));
            }
            if idx.len() == 1 {
                // M(r) is row r as a vector.
                let row = m.row(i as usize - 1).to_vec();
                return Ok(Value::Matrix(Rc::new(Matrix {
                    rows: 1,
                    cols: m.cols,
                    data: row,
                    vector: true,
                })));
            }
            let j = index_arg(&idx[1])?;
            if j < 1 || j as usize > m.cols {
                return Err(out_of_range(j, m.cols));
            }
            index_value(m.get(i as usize - 1, j as usize - 1), &idx[2..], _m)
        }
        // s(i) returns the character code (unverified on the G1).
        Value::Str(s) => {
            let i = index_arg(&idx[0])?;
            let chars: Vec<char> = s.chars().collect();
            if i < 1 || i as usize > chars.len() {
                return Err(out_of_range(i, chars.len()));
            }
            Ok(Value::from_i64(chars[i as usize - 1] as i64))
        }
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("a {} cannot be indexed", other.type_name()),
        )),
    }
}

/// Writes `v` at `idx` inside `target` (copy-on-write). Writing one past
/// the end of a list, or at index 0, appends.
pub fn store_index(target: &mut Value, idx: &[Value], v: Value, max_len: usize) -> RtResult<()> {
    if idx.is_empty() {
        *target = v;
        return Ok(());
    }
    // An uninitialised LOCAL (real 0) becomes a list on first element write
    // (unverified convenience inherited from the Python emulator).
    if matches!(target, Value::Real(r) if r.is_zero()) {
        *target = Value::list(Vec::new());
    }
    match target {
        Value::List(items) => {
            let i = index_arg(&idx[0])?;
            let len = items.len();
            let items = Rc::make_mut(items);
            if i >= 1 && (i as usize) <= len {
                return store_index(&mut items[i as usize - 1], &idx[1..], v, max_len);
            }
            if (i == 0 || i as usize == len + 1) && idx.len() == 1 {
                if len >= max_len {
                    return Err(RuntimeError::new(
                        ErrorKind::ResourceLimit(ResourceKind::Memory),
                        "list too long",
                    ));
                }
                items.push(v);
                return Ok(());
            }
            Err(out_of_range(i, len))
        }
        Value::Matrix(m) => {
            let m = Rc::make_mut(m);
            let i = index_arg(&idx[0])?;
            match &v {
                Value::Real(_) | Value::Int(_) | Value::Complex(_) => {}
                other => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        format!("matrix elements must be numbers, got {}", other.type_name()),
                    ));
                }
            }
            if m.vector {
                if i >= 1 && (i as usize) <= m.cols {
                    m.data[i as usize - 1] = v;
                    return Ok(());
                }
                if i as usize == m.cols + 1 {
                    m.data.push(v);
                    m.cols += 1;
                    return Ok(());
                }
                return Err(out_of_range(i, m.cols));
            }
            if idx.len() != 2 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidDimension,
                    "a matrix element needs (row, column)",
                ));
            }
            let j = index_arg(&idx[1])?;
            if i < 1 || j < 1 || i as usize > m.rows + 1 || j as usize > m.cols + 1 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidDimension,
                    format!("({i},{j}) is outside {}×{}", m.rows, m.cols),
                ));
            }
            let (i, j) = (i as usize, j as usize);
            if j > m.cols {
                let mut data = Vec::with_capacity(m.rows * j);
                for r in 0..m.rows {
                    data.extend_from_slice(m.row(r));
                    data.push(Value::zero());
                }
                m.data = data;
                m.cols = j;
            }
            if i > m.rows {
                m.data.extend(std::iter::repeat_n(Value::zero(), m.cols));
                m.rows = i;
            }
            m.set(i - 1, j - 1, v);
            Ok(())
        }
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("cannot assign an element of a {}", other.type_name()),
        )),
    }
}
