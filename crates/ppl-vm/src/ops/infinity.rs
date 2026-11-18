//! Native scalar operations for signed infinities carried by calculator values.
use ppl_cas::{CasValue, InfinitySign};
use ppl_compiler::builtins::Builtin;
use ppl_math::Real;
use ppl_syntax::ast::BinOp;
use std::cmp::Ordering;

use crate::{ErrorKind, RtResult, RuntimeError, Value};

fn sign(value: &Value) -> Option<InfinitySign> {
    match value {
        Value::Symbolic(value) => match value.as_ref() {
            CasValue::Infinity(sign @ (InfinitySign::Positive | InfinitySign::Negative)) => {
                Some(*sign)
            }
            _ => None,
        },
        _ => None,
    }
}

fn value(sign: InfinitySign) -> Value {
    crate::cas::from_cas(CasValue::Infinity(sign))
}

pub(crate) fn negate(operand: &Value) -> Option<Value> {
    Some(value(match sign(operand)? {
        InfinitySign::Positive => InfinitySign::Negative,
        InfinitySign::Negative => InfinitySign::Positive,
        InfinitySign::Unsigned => unreachable!("signed infinity"),
    }))
}

fn real(operand: &Value) -> Option<Real> {
    match operand {
        Value::Real(_) | Value::Int(_) => operand.as_real(),
        _ => None,
    }
}

/// Map finite numbers to their sign so values outside the f64 magnitude
/// range retain their distinction from true infinities and zero.
fn magnitude(operand: &Value) -> Option<f64> {
    match sign(operand) {
        Some(InfinitySign::Positive) => Some(f64::INFINITY),
        Some(InfinitySign::Negative) => Some(f64::NEG_INFINITY),
        _ => real(operand).map(|v| {
            if v.is_zero() {
                0.
            } else if v.is_negative() {
                -1.
            } else {
                1.
            }
        }),
    }
}

fn undefined() -> RuntimeError {
    RuntimeError::kind(ErrorKind::Undefined)
}

pub(crate) fn binary(op: BinOp, left: &Value, right: &Value) -> Option<RtResult<Value>> {
    if sign(left).is_none() && sign(right).is_none() {
        return None;
    }
    let a = magnitude(left)?;
    let b = magnitude(right)?;
    let result = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div if b == 0. => f64::INFINITY,
        BinOp::Div => a / b,
        BinOp::Eq => return Some(Ok(Value::from_bool(a == b))),
        BinOp::Ne => return Some(Ok(Value::from_bool(a != b))),
        BinOp::Lt => return Some(Ok(Value::from_bool(a < b))),
        BinOp::Le => return Some(Ok(Value::from_bool(a <= b))),
        BinOp::Gt => return Some(Ok(Value::from_bool(a > b))),
        BinOp::Ge => return Some(Ok(Value::from_bool(a >= b))),
        BinOp::Pow => return Some(power(sign(left)?, real(right)?)),
        _ => return None,
    };
    Some(if result.is_nan() {
        Err(undefined())
    } else if result == 0. {
        Ok(Value::zero())
    } else {
        Ok(value(if result.is_sign_positive() {
            InfinitySign::Positive
        } else {
            InfinitySign::Negative
        }))
    })
}

fn power(sign: InfinitySign, exponent: Real) -> RtResult<Value> {
    if exponent.is_zero() || (sign == InfinitySign::Negative && !exponent.is_integer()) {
        return Err(undefined());
    }
    if exponent.is_negative() {
        return Ok(Value::zero());
    }
    let odd = exponent.to_i128().is_some_and(|n| n.rem_euclid(2) != 0);
    Ok(value(if sign == InfinitySign::Negative && odd {
        InfinitySign::Negative
    } else {
        InfinitySign::Positive
    }))
}

pub(crate) fn sqrt(operand: &Value) -> Option<RtResult<Value>> {
    Some(if sign(operand)? == InfinitySign::Positive {
        Ok(operand.clone())
    } else {
        Err(undefined())
    })
}

pub(crate) fn builtin(builtin: Builtin, args: &[Value]) -> Option<RtResult<Value>> {
    match (builtin, args) {
        (Builtin::Abs, [arg]) if sign(arg).is_some() => Some(Ok(value(InfinitySign::Positive))),
        (Builtin::Sign, [arg]) => Some(Ok(Value::from_i64(
            if sign(arg)? == InfinitySign::Positive {
                1
            } else {
                -1
            },
        ))),
        (Builtin::Sqrt, [arg]) => sqrt(arg),
        (Builtin::Min | Builtin::Max, _) if args.iter().any(|arg| sign(arg).is_some()) => {
            let mut best = args.first()?;
            for candidate in &args[1..] {
                let comparison = match (real(candidate), real(best)) {
                    (Some(candidate), Some(best)) => candidate.cmp_value(best),
                    _ => magnitude(candidate)?.total_cmp(&magnitude(best)?),
                };
                if (builtin == Builtin::Max && comparison == Ordering::Greater)
                    || (builtin == Builtin::Min && comparison == Ordering::Less)
                {
                    best = candidate;
                }
            }
            Some(Ok(real(best)
                .map(Value::Real)
                .unwrap_or_else(|| best.clone())))
        }
        _ => None,
    }
}
