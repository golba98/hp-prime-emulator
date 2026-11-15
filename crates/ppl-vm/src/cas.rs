//! Explicit symbolic/exact evaluation. Ordinary BCD arithmetic stays native.
use crate::{ErrorKind, Machine, RtResult, RuntimeError, Settings, Value};
use std::{rc::Rc, sync::atomic::AtomicBool};
/// Render a value as algebra data, never as executable source text. Explicit
/// CAS(string) is the separate source boundary in the builtin dispatcher.
pub fn expression(value: &Value) -> RtResult<String> {
    to_cas(value)
        .ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::BadArgType,
                "This calculator value cannot be represented as Unicode algebra data",
            )
        })?
        .try_expression()
        .map_err(|error| RuntimeError::new(ErrorKind::InvalidInput, error.to_string()))
}
/// Typed boundary for calculator objects shared with CAS. GROB handles are
/// deliberately excluded: they are graphics resources, not algebra values.
pub fn to_cas(value: &Value) -> Option<ppl_cas::CasValue> {
    use ppl_cas::CasValue as C;
    Some(match value {
        Value::Symbolic(value) => (**value).clone(),
        Value::Real(value) => match value.origin() {
            ppl_math::RealOrigin::Rational {
                numerator,
                denominator,
            } => {
                let numerator =
                    format!("{}{numerator}", if value.is_negative() { "-" } else { "" });
                if denominator == 1 {
                    C::Integer(numerator)
                } else {
                    C::Rational {
                        numerator,
                        denominator: denominator.to_string(),
                    }
                }
            }
            ppl_math::RealOrigin::PiMultiple {
                numerator,
                denominator,
            } => C::Symbolic(format!(
                "({}{numerator}*pi/{denominator})",
                if value.is_negative() { "-" } else { "" }
            )),
            _ => C::Decimal(value.to_string()),
        },
        Value::Int(value) => C::Integer(value.numeric_value().to_string()),
        Value::Complex(value) => C::Complex {
            real: Box::new(C::Decimal(value.re.to_string())),
            imaginary: Box::new(C::Decimal(value.im.to_string())),
        },
        Value::Str(value) => C::String(value.to_string()),
        Value::List(values) => C::List(values.iter().map(to_cas).collect::<Option<_>>()?),
        Value::Matrix(value) if value.vector => {
            C::Vector(value.data.iter().map(to_cas).collect::<Option<_>>()?)
        }
        Value::Matrix(value) => C::Matrix {
            rows: value.rows,
            cols: value.cols,
            values: value.data.iter().map(to_cas).collect::<Option<_>>()?,
        },
        Value::Grob(_) | Value::RawString(_) => return None,
    })
}

/// Old profiles stored units as generic symbolic text. Recover their root type
/// through Giac's parse-only operation, without executing saved expressions or changing
/// either calculator environment. The underscore only selects candidates; it
/// never decides whether a value is a measurement.
pub fn type_code(machine: &mut Machine, value: &Value) -> RtResult<i64> {
    let Value::Symbolic(value) = value else {
        return Ok(value.type_code());
    };
    let ppl_cas::CasValue::Symbolic(source) = value.as_ref() else {
        return Ok(Value::Symbolic(value.clone()).type_code());
    };
    if !source.contains('_') {
        return Ok(crate::value::type_code::CAS);
    }
    if let Some(code) = machine.legacy_type_cache.get(source) {
        return Ok(*code);
    }
    let cancel = machine.host.interrupt_flag();
    let fallback = AtomicBool::new(false);
    let mut options = ppl_cas::Options::default();
    if let Some(remaining) = machine.remaining_time() {
        options.timeout = options.timeout.min(remaining);
    }
    let parsed =
        ppl_cas::CasSession::parse_value(source, &options, cancel.as_deref().unwrap_or(&fallback))
            .map_err(|error| {
                RuntimeError::new(
                    match error {
                        ppl_cas::CasError::Cancelled => ErrorKind::Interrupted,
                        ppl_cas::CasError::TimedOut => {
                            ErrorKind::ResourceLimit(crate::ResourceKind::Time)
                        }
                        _ => ErrorKind::InvalidInput,
                    },
                    error.to_string(),
                )
            })?;
    let code = if matches!(parsed, ppl_cas::CasValue::Unit(_)) {
        crate::value::type_code::UNIT
    } else {
        crate::value::type_code::CAS
    };
    if source.len() <= 4096 {
        if machine.legacy_type_cache.len() >= 64 {
            machine.legacy_type_cache.pop_first();
        }
        machine.legacy_type_cache.insert(source.clone(), code);
    }
    Ok(code)
}
pub fn evaluate(machine: &mut Machine, source: &str) -> RtResult<Value> {
    evaluate_context(machine, &hp_list_source(source), None, true)
}

/// PPL's CAS() converts numeric array cells to Home numbers, independently of
/// the exact CAS answer. Roll back that answer if the additional projection
/// fails, before applying any Home writes from the original evaluation.
pub(crate) fn evaluate_ppl_cas(machine: &mut Machine, source: &str) -> RtResult<Value> {
    let saved = machine.cas_context.clone();
    let result = evaluate_context_projected(machine, &hp_list_source(source), None, true, true);
    if result.is_err() {
        machine.cas_context = saved;
    }
    result
}

pub(crate) fn evaluate_namespace(machine: &mut Machine, source: &str) -> RtResult<Value> {
    evaluate_context(machine, source, None, false)
}

/// Translate HP list syntax only when the entire input is a valid PPL
/// expression. The renderer preserves quoted strings and removes comments;
/// Giac program blocks and other Giac syntax keep their own parser.
fn hp_list_source(source: &str) -> std::borrow::Cow<'_, str> {
    use ppl_syntax::ast::{Item, StmtKind};
    if !source.contains('{') {
        return source.into();
    }
    let wrapped = format!("EXPORT __CASINPUT() BEGIN {source}; END;");
    let (ast, diagnostics) = ppl_syntax::parse(&wrapped);
    if diagnostics.has_errors() {
        return source.into();
    }
    let [Item::Function(function)] = ast.items.as_slice() else {
        return source.into();
    };
    let [statement] = function.body.as_slice() else {
        return source.into();
    };
    let StmtKind::Expr(expression) = &statement.kind else {
        return source.into();
    };
    ppl_compiler::cas_entry_source(expression).into()
}

/// Measurement expressions typed in EXPR/INPUT also see the live PPL frame.
pub(crate) fn evaluate_expression(machine: &mut Machine, source: &str) -> RtResult<Value> {
    let saved = machine.cas_context.clone();
    machine.cas_context.settings = settings(&machine.settings);
    machine.cas_context.last_value = machine.expression_values().get("Ans").cloned();
    let names = saved.bindings.keys().cloned().collect();
    let result = evaluate_context(machine, source, Some(&names), true);
    if result.is_err() {
        machine.cas_context = saved;
    } else {
        machine.cas_context.settings = saved.settings;
        machine.cas_context.last_value = saved.last_value;
    }
    result
}

fn evaluate_context(
    machine: &mut Machine,
    source: &str,
    expression: Option<&std::collections::BTreeSet<String>>,
    frame: bool,
) -> RtResult<Value> {
    evaluate_context_projected(machine, source, expression, frame, false)
}

fn evaluate_context_projected(
    machine: &mut Machine,
    source: &str,
    expression: Option<&std::collections::BTreeSet<String>>,
    frame: bool,
    project_containers: bool,
) -> RtResult<Value> {
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
    let home = if frame {
        machine.expression_values()
    } else {
        machine.cas_home_values()
    };
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
    let container = matches!(
        result.value,
        ppl_cas::CasValue::List(_)
            | ppl_cas::CasValue::Vector(_)
            | ppl_cas::CasValue::Matrix { .. }
    );
    let value = if project_containers && container && contains_symbolic(&result.value) {
        if let Some(remaining) = machine.remaining_time() {
            options.timeout = options.timeout.min(remaining);
        }
        ppl_cas::CasSession::project_home_container(
            &result.value,
            machine.cas_context.settings.clone(),
            &options,
            cancel.as_deref().unwrap_or(&fallback),
        )
        .map_err(|e| RuntimeError::new(ErrorKind::InvalidInput, e.to_string()))?
    } else {
        result.value
    };
    if let Some(names) = expression
        && machine
            .cas_context
            .bindings
            .keys()
            .any(|name| !names.contains(name))
    {
        return Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            "Create Home variables through HVars before assigning a measurement",
        ));
    }
    for (name, value) in result.home_updates {
        if frame {
            machine.store_expression_value(
                &name,
                from_cas_home_in_model(value, machine.settings.numeric_version),
            )?;
        } else {
            machine.store_home_value(
                &name,
                from_cas_home_in_model(value, machine.settings.numeric_version),
            )?;
        }
    }
    Ok(if project_containers && container {
        from_ppl_container(value)
    } else if expression.is_some() {
        from_cas_home_in_model(value, machine.settings.numeric_version)
    } else {
        from_cas_in_model(value, machine.settings.numeric_version)
    })
}
fn from_ppl_container(value: ppl_cas::CasValue) -> Value {
    use ppl_cas::CasValue as C;
    match value {
        C::List(values) => Value::list(values.into_iter().map(from_ppl_container).collect()),
        C::Vector(values) => vector_value(values.into_iter().map(from_ppl_container).collect()),
        C::Matrix { rows, cols, values } => {
            let data = values
                .into_iter()
                .map(from_ppl_container)
                .collect::<Vec<_>>();
            if let Some(numeric) = numeric_cells(&data) {
                Value::Matrix(Rc::new(crate::value::Matrix {
                    rows,
                    cols,
                    data: numeric,
                    vector: false,
                }))
            } else {
                Value::list(
                    data.chunks(cols.max(1))
                        .map(|row| vector_value(row.to_vec()))
                        .collect(),
                )
            }
        }
        value => from_cas_home(value),
    }
}

fn contains_symbolic(value: &ppl_cas::CasValue) -> bool {
    use ppl_cas::CasValue as C;
    match value {
        C::Symbolic(_) => true,
        C::List(v) | C::Vector(v) | C::Matrix { values: v, .. } => v.iter().any(contains_symbolic),
        C::Complex { real, imaginary } => contains_symbolic(real) || contains_symbolic(imaginary),
        _ => false,
    }
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
    from_cas_in_model(value, ppl_math::NumericVersion::Legacy15157)
}
pub fn from_cas_in_model(value: ppl_cas::CasValue, version: ppl_math::NumericVersion) -> Value {
    // The Home boundary converts integers/decimals and numeric collections to
    // calculator numbers. Exact rationals and other algebra retain CAS identity.
    match value {
        ppl_cas::CasValue::Integer(ref text) => ppl_math::Real::parse_versioned(text, version)
            .map(Value::Real)
            .unwrap_or_else(|| Value::Symbolic(Rc::new(value))),
        ppl_cas::CasValue::Decimal(ref text) => ppl_math::Real::parse(text)
            .map(|value| Value::Real(value.decimal()))
            .unwrap_or_else(|| Value::Symbolic(Rc::new(value))),
        ppl_cas::CasValue::String(text) => Value::str(text),
        ppl_cas::CasValue::List(values) => Value::list(
            values
                .into_iter()
                .map(|value| from_cas_in_model(value, version))
                .collect(),
        ),
        ppl_cas::CasValue::Vector(values) => {
            let data = values
                .into_iter()
                .map(|value| from_cas_in_model(value, version))
                .collect::<Vec<_>>();
            vector_value(data)
        }
        ppl_cas::CasValue::Complex { real, imaginary } => {
            let real = from_cas_in_model(*real, version);
            let imaginary = from_cas_in_model(*imaginary, version);
            match (real.as_real(), imaginary.as_real()) {
                (Some(re), Some(im)) => Value::Complex(ppl_math::Complex::new(re, im)),
                _ => Value::Symbolic(Rc::new(ppl_cas::CasValue::Complex {
                    real: Box::new(to_cas(&real).unwrap()),
                    imaginary: Box::new(to_cas(&imaginary).unwrap()),
                })),
            }
        }
        ppl_cas::CasValue::Matrix { rows, cols, values } => {
            let data = values
                .into_iter()
                .map(|value| from_cas_in_model(value, version))
                .collect::<Vec<_>>();
            match numeric_cells(&data) {
                Some(data) => Value::Matrix(Rc::new(crate::value::Matrix {
                    rows,
                    cols,
                    data,
                    vector: false,
                })),
                None => Value::list(
                    data.chunks(cols.max(1))
                        .map(|row| vector_value(row.to_vec()))
                        .collect(),
                ),
            }
        }
        _ => Value::Symbolic(Rc::new(value)),
    }
}

fn vector_value(values: Vec<Value>) -> Value {
    // Home matrix cells are numeric individually, even when an unresolved
    // symbol forces the whole vector to become a list.
    let values = values
        .into_iter()
        .map(|value| value.as_real().map(Value::Real).unwrap_or(value))
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Value::list(values);
    }
    match numeric_cells(&values) {
        Some(data) => Value::Matrix(Rc::new(crate::value::Matrix {
            rows: 1,
            cols: data.len(),
            data,
            vector: true,
        })),
        None => Value::list(values),
    }
}

fn numeric_cells(values: &[Value]) -> Option<Vec<Value>> {
    values
        .iter()
        .map(|value| match value {
            Value::Complex(value) => Some(Value::Complex(*value)),
            value => value.as_real().map(Value::Real),
        })
        .collect()
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
                expression(a)?,
                expression(b)?,
                expression(b)?
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
        &format!("({}){operator}({})", expression(a)?, expression(b)?),
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
        ppl_cas::CasValue::Infinity(_) => Some(true),
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
    from_cas_home_in_model(value, ppl_math::NumericVersion::Legacy15157)
}
pub fn from_cas_home_in_model(
    value: ppl_cas::CasValue,
    version: ppl_math::NumericVersion,
) -> Value {
    use ppl_cas::CasValue as C;
    match value {
        C::Integer(ref text) => ppl_math::Real::parse_versioned(text, version)
            .map(Value::Real)
            .unwrap_or_else(|| Value::Symbolic(Rc::new(value))),
        C::Decimal(ref text) => ppl_math::Real::parse(text)
            .map(|value| Value::Real(value.decimal()))
            .unwrap_or_else(|| Value::Symbolic(Rc::new(value))),
        C::Rational {
            ref numerator,
            ref denominator,
        } => {
            let numeric = ppl_math::Real::from_fraction_strings(numerator, denominator, version);
            numeric
                .map(Value::Real)
                .unwrap_or_else(|| from_cas_in_model(value, version))
        }
        C::Complex { real, imaginary } => {
            let real = from_cas_home_in_model(*real, version);
            let imaginary = from_cas_home_in_model(*imaginary, version);
            if let (Some(re), Some(im)) = (real.as_real(), imaginary.as_real()) {
                Value::Complex(ppl_math::Complex::new(re, im))
            } else {
                Value::Symbolic(Rc::new(C::Complex {
                    real: Box::new(to_cas(&real).unwrap()),
                    imaginary: Box::new(to_cas(&imaginary).unwrap()),
                }))
            }
        }
        C::List(values) => Value::list(
            values
                .into_iter()
                .map(|value| from_cas_home_in_model(value, version))
                .collect(),
        ),
        C::Vector(values) => from_cas_in_model(C::Vector(values), version),
        C::Matrix { rows, cols, values } => {
            let data = values
                .into_iter()
                .map(|value| from_cas_home_in_model(value, version))
                .collect::<Vec<_>>();
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
                Value::Symbolic(Rc::new(C::Matrix {
                    rows,
                    cols,
                    values: data.iter().map(|value| to_cas(value).unwrap()).collect(),
                }))
            }
        }
        other => from_cas_in_model(other, version),
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

/// Compose compiler-owned quoted syntax with runtime data. Only the template
/// is searched: a returned string containing a marker cannot become code or
/// replace another operand. Typed values supply their own algebra quotation.
pub(crate) fn compose(source: &str, marker: &str, values: &[Value]) -> RtResult<String> {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find(marker) {
        out.push_str(&rest[..at]);
        rest = &rest[at + marker.len()..];
        let end = rest
            .find("__")
            .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
        let index = rest[..end]
            .parse::<usize>()
            .map_err(|_| RuntimeError::kind(ErrorKind::InvalidInput))?;
        let value = values
            .get(index)
            .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
        out.push('(');
        out.push_str(&expression(value)?);
        out.push(')');
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod template_tests {
    use super::*;
    #[test]
    fn inserted_strings_cannot_replace_later_operands_or_become_code() {
        let values = [Value::str("__ARG_1__\";side():=7"), Value::from_i64(2)];
        assert_eq!(
            compose("f(__ARG_0__,__ARG_1__)", "__ARG_", &values).unwrap(),
            "f((\"__ARG_1__\\\";side():=7\"),(2))"
        );
    }
}
