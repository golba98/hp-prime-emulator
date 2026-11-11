//! Explicit symbolic/exact evaluation. Ordinary BCD arithmetic stays native.
use crate::{ErrorKind, Machine, RtResult, RuntimeError, Settings, Value};
use std::{rc::Rc, sync::atomic::AtomicBool};
pub fn expression(value: &Value) -> String {
    match value {
        Value::Symbolic(value) => value.expression(),
        Value::Str(text) => text.to_string(),
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(expression).collect::<Vec<_>>().join(",")
        ),
        Value::Matrix(m) => format!(
            "[{}]",
            m.data
                .chunks(m.cols.max(1))
                .map(|r| format!(
                    "[{}]",
                    r.iter().map(expression).collect::<Vec<_>>().join(",")
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Real(r) => r.to_string(),
        Value::Int(i) => i.unsigned().to_string(),
        Value::Complex(c) => format!("({})+i*({})", c.re, c.im),
        Value::Grob(n) => format!("G{n}"),
    }
}
/// Typed boundary for calculator objects shared with CAS. GROB handles are
/// deliberately excluded: they are graphics resources, not algebra values.
pub fn to_cas(value: &Value) -> Option<ppl_cas::CasValue> {
    use ppl_cas::CasValue as C;
    Some(match value {
        Value::Symbolic(value) => (**value).clone(),
        Value::Real(value) => C::Decimal(value.to_string()),
        Value::Int(value) => C::Integer(value.unsigned().to_string()),
        Value::Complex(value) => C::Complex {
            real: Box::new(C::Decimal(value.re.to_string())),
            imaginary: Box::new(C::Decimal(value.im.to_string())),
        },
        Value::Str(value) => C::String(value.to_string()),
        Value::List(values) => C::List(values.iter().map(to_cas).collect::<Option<_>>()?),
        Value::Matrix(value) => C::Matrix {
            rows: value.rows,
            cols: value.cols,
            values: value.data.iter().map(to_cas).collect::<Option<_>>()?,
        },
        Value::Grob(_) => return None,
    })
}
pub fn evaluate(machine: &mut Machine, source: &str) -> RtResult<Value> {
    let cancel = machine.host.interrupt_flag();
    let fallback = AtomicBool::new(false);
    let mut options = ppl_cas::Options::default();
    if let Some(remaining) = machine.remaining_time() {
        if remaining.is_zero() {
            return Err(RuntimeError::new(
                ErrorKind::ResourceLimit(crate::ResourceKind::Time),
                "Execution time limit reached",
            ));
        }
        options.timeout = options.timeout.min(remaining);
    }
    let home = machine.cas_home_values();
    let result = machine
        .cas_context
        .evaluate_with_home(
            source,
            &home,
            &options,
            cancel.as_deref().unwrap_or(&fallback),
        )
        .map_err(|e| {
            RuntimeError::new(
                match e {
                    ppl_cas::CasError::Cancelled => ErrorKind::Interrupted,
                    ppl_cas::CasError::TimedOut => {
                        ErrorKind::ResourceLimit(crate::ResourceKind::Time)
                    }
                    _ => ErrorKind::InvalidInput,
                },
                e.to_string(),
            )
        })?;
    for (name, value) in result.home_updates {
        machine.store_home_value(&name, from_cas_home(value))?;
    }
    Ok(from_cas(result.value))
}
pub fn evaluate_math(machine: &mut Machine, source: &str) -> RtResult<Value> {
    let saved = machine.cas_context.settings.clone();
    machine.cas_context.settings = settings(&machine.settings);
    let result = evaluate(machine, source);
    machine.cas_context.settings = saved;
    result
}
pub fn evaluate_exact(machine: &mut Machine, source: &str) -> RtResult<Value> {
    let saved = machine.cas_context.settings.approximate;
    machine.cas_context.settings.approximate = false;
    let result = evaluate(machine, source);
    machine.cas_context.settings.approximate = saved;
    result
}
pub(crate) fn settings(s: &Settings) -> ppl_cas::CasSettings {
    ppl_cas::CasSettings {
        angle: match s.angle {
            crate::AngleMode::Radians => ppl_cas::AngleMode::Radians,
            crate::AngleMode::Degrees => ppl_cas::AngleMode::Degrees,
            crate::AngleMode::Gradians => ppl_cas::AngleMode::Gradians,
        },
        approximate: false,
        digits: 12,
    }
}
pub fn from_cas(value: ppl_cas::CasValue) -> Value {
    // Exact algebra values keep their type and full precision; decimal evaluation
    // explicitly converts through the calculator's twelve-digit numeric model.
    match value {
        ppl_cas::CasValue::Decimal(ref text) => ppl_math::Real::parse(text)
            .map(Value::Real)
            .unwrap_or_else(|| Value::Symbolic(Rc::new(value))),
        ppl_cas::CasValue::String(text) => Value::str(text),
        ppl_cas::CasValue::List(values) => Value::list(values.into_iter().map(from_cas).collect()),
        _ => Value::Symbolic(Rc::new(value)),
    }
}
pub fn binary(
    machine: &mut Machine,
    op: ppl_syntax::ast::BinOp,
    a: &Value,
    b: &Value,
) -> RtResult<Value> {
    if matches!(
        op,
        ppl_syntax::ast::BinOp::And | ppl_syntax::ast::BinOp::Or | ppl_syntax::ast::BinOp::Xor
    ) {
        let left = a.truthy()?;
        let right = b.truthy()?;
        return Ok(Value::from_bool(match op {
            ppl_syntax::ast::BinOp::And => left && right,
            ppl_syntax::ast::BinOp::Or => left || right,
            _ => left != right,
        }));
    }
    if op == ppl_syntax::ast::BinOp::Polar {
        return evaluate_math(
            machine,
            &format!(
                "({})*(cos({})+i*sin({}))",
                expression(a),
                expression(b),
                expression(b)
            ),
        );
    }
    let operator = match op {
        ppl_syntax::ast::BinOp::Ne => "!=",
        ppl_syntax::ast::BinOp::Le => "<=",
        ppl_syntax::ast::BinOp::Ge => ">=",
        _ => op.symbol(),
    };
    evaluate_math(
        machine,
        &format!("({}){operator}({})", expression(a), expression(b)),
    )
}

pub fn numeric_truth(value: &ppl_cas::CasValue) -> Option<bool> {
    fn nonzero(text: &str) -> bool {
        text.split(['e', 'E'])
            .next()
            .unwrap_or(text)
            .chars()
            .any(|c| matches!(c, '1'..='9'))
    }
    match value {
        ppl_cas::CasValue::Integer(text) | ppl_cas::CasValue::Decimal(text) => Some(nonzero(text)),
        ppl_cas::CasValue::Rational { numerator, .. } => Some(nonzero(numerator)),
        ppl_cas::CasValue::Complex { real, imaginary } => {
            Some(numeric_truth(real)? || numeric_truth(imaginary)?)
        }
        _ => None,
    }
}

/// Home converts CAS numbers to its decimal model; symbolic/exact objects retain
/// their explicit type rather than being stringified or replaced with zero.
pub fn from_cas_home(value: ppl_cas::CasValue) -> Value {
    use ppl_cas::CasValue as C;
    match value {
        C::Integer(ref text) | C::Decimal(ref text) => ppl_math::Real::parse(text)
            .map(Value::Real)
            .unwrap_or_else(|| Value::Symbolic(Rc::new(value))),
        C::Rational {
            ref numerator,
            ref denominator,
        } => {
            let numeric = ppl_math::Real::parse(numerator)
                .zip(ppl_math::Real::parse(denominator))
                .and_then(|(n, d)| n.div(d).ok());
            numeric.map(Value::Real).unwrap_or_else(|| from_cas(value))
        }
        C::Complex { real, imaginary } => {
            let real = from_cas_home(*real);
            let imaginary = from_cas_home(*imaginary);
            if let (Some(re), Some(im)) = (real.as_real(), imaginary.as_real()) {
                Value::Complex(ppl_math::Complex { re, im })
            } else {
                Value::Symbolic(Rc::new(C::Complex {
                    real: Box::new(C::Symbolic(expression(&real))),
                    imaginary: Box::new(C::Symbolic(expression(&imaginary))),
                }))
            }
        }
        C::List(values) => Value::list(values.into_iter().map(from_cas_home).collect()),
        C::Matrix { rows, cols, values } => {
            let data = values.into_iter().map(from_cas_home).collect::<Vec<_>>();
            if data
                .iter()
                .all(|v| v.as_real().is_some() || matches!(v, Value::Complex(_)))
            {
                let data = data
                    .into_iter()
                    .map(|v| v.as_real().map(Value::Real).unwrap_or(v))
                    .collect();
                Value::Matrix(Rc::new(crate::value::Matrix {
                    rows,
                    cols,
                    vector: false,
                    data,
                }))
            } else {
                Value::Symbolic(Rc::new(C::Symbolic(format!(
                    "[{}]",
                    data.iter().map(expression).collect::<Vec<_>>().join(",")
                ))))
            }
        }
        other => from_cas(other),
    }
}

/// Parameter names are part of the stored algebra object, so they survive
/// copying, catalog edits and restart without a disconnected metadata cache.
pub fn function_parameters(value: &Value) -> Option<Vec<String>> {
    let Value::Symbolic(value) = value else {
        return None;
    };
    let source = value.expression();
    let (expression, diagnostics) = ppl_syntax::parse_expression(&source);
    if !diagnostics.has_errors()
        && let ppl_syntax::ast::ExprKind::Call(function, args) = expression.kind
        && let ppl_syntax::ast::ExprKind::Ident(name) = function.kind
        && name == "unapply"
    {
        return args
            .iter()
            .skip(1)
            .map(|arg| {
                if let ppl_syntax::ast::ExprKind::Ident(name) = &arg.kind {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
    }
    // Giac prints evaluated functions as (x,y)->body.
    let (parameters, _) = source.split_once("->")?;
    let parameters = parameters
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let names = parameters
        .split(',')
        .map(|name| name.trim().to_string())
        .collect::<Vec<_>>();
    names
        .iter()
        .all(|name| valid_parameter(name))
        .then_some(names)
}
fn valid_parameter(name: &str) -> bool {
    !name.is_empty()
        && name.chars().enumerate().all(|(index, ch)| {
            ch == '_' || ch.is_alphabetic() || (index > 0 && ch.is_ascii_digit())
        })
}
pub fn with_parameters(value: &Value, parameters: &[Value]) -> RtResult<Value> {
    let names = parameters
        .iter()
        .map(|value| match value {
            Value::Str(name) if valid_parameter(name) => Ok(name.to_string()),
            _ => Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "Function parameters must be valid names",
            )),
        })
        .collect::<RtResult<Vec<_>>>()?;
    if names.is_empty()
        || names.len() > 64
        || names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != names.len()
    {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "Function parameters must be nonempty and unique",
        ));
    }
    let body = to_cas(value)
        .ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::BadArgType,
                "A graphics handle cannot be a function",
            )
        })?
        .try_expression()
        .map_err(|e| RuntimeError::new(ErrorKind::BadArgValue, e.to_string()))?;
    Ok(Value::Symbolic(Rc::new(ppl_cas::CasValue::Symbolic(
        format!("unapply(({body}),{})", names.join(",")),
    ))))
}
