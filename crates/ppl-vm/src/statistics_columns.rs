//! Statistics data columns are shared calculator lists, including when another
//! app is active. The original firmware admits real numeric data only and
//! converts word integers before storing. Validation precedes any mutation.
use crate::{ErrorKind, RtResult, RuntimeError, Value};

pub(crate) fn is_column(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 2 && matches!(bytes[0], b'C' | b'D') && bytes[1].is_ascii_digit()
}

pub(crate) fn coerce(name: &str, value: Value) -> RtResult<Value> {
    if !is_column(name) {
        return Ok(value);
    }
    let Value::List(values) = value else {
        return Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("{name} requires a list of real statistics data"),
        ));
    };
    values
        .iter()
        .map(|value| value.expect_real("statistics data column").map(Value::Real))
        .collect::<RtResult<Vec<_>>>()
        .map(Value::list)
}
