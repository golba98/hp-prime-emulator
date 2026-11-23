//! PPL Graph 3D requests share the definitions edited in Symbolic view.
use super::{AccessResult, RuntimeError, checkbox, definition::Definition, invalid};
use crate::ide::{
    native_apps::Evaluator,
    native_graph3d::{Definitions, FUNCTIONS, plot_context},
};
use ppl_math::Real;
use ppl_vm::{ErrorKind, PortableValue, SessionSnapshot, Value, host::AppVariableRequest};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) fn access(
    mut definitions: Definitions,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<AccessResult, RuntimeError> {
    let finish = |value, definitions| {
        if cancel.load(Ordering::Acquire) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        Ok(AccessResult {
            value,
            graph3d: definitions,
            sheet: None,
            function: None,
            statistics1: None,
            statistics2: None,
            geometry: None,
        })
    };
    if matches!(request.name.as_str(), "CHECK" | "UNCHECK" | "ISCHECK") {
        let ([position], None) = (request.indices.as_slice(), request.value.as_ref()) else {
            return Err(invalid("A Graph 3D checkbox command requires one index"));
        };
        let number = position
            .to_value()
            .expect_real("Surface definition index")?;
        let digit = number
            .is_integer()
            .then(|| number.to_i64())
            .flatten()
            .filter(|digit| (0..=9).contains(digit))
            .ok_or_else(|| invalid("Surface definition index must be 0 through 9"))?;
        let row = &mut definitions[if digit == 0 { 9 } else { digit as usize - 1 }];
        let defined = !row.source.trim().is_empty();
        let (value, changed) = checkbox(&request.name, &mut row.enabled, defined)?;
        return finish(value, changed.then_some(definitions));
    }
    let index = FUNCTIONS
        .iter()
        .position(|name| *name == request.name)
        .ok_or_else(|| invalid("Unknown Graph 3D field"))?;
    let definition = &mut definitions[index];

    if let Some(value) = &request.value {
        if !request.indices.is_empty() {
            return Err(invalid("Assign a surface definition without coordinates"));
        }
        let (source, value) = match value {
            PortableValue::String(text) if text.is_empty() => (String::new(), value.clone()),
            PortableValue::Real(_) | PortableValue::Integer(_) => (
                value
                    .to_value()
                    .string_form(ppl_vm::value::default_format()),
                value.clone(),
            ),
            PortableValue::String(text)
            | PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(text)) => {
                let parsed = Definition::parse(text, &snapshot.settings)?;
                (parsed.source, parsed.value)
            }
            _ => {
                return Err(invalid(
                    "A surface definition requires a real, word or expression",
                ));
            }
        };
        definition.literal = matches!(value, PortableValue::Real(_) | PortableValue::Integer(_))
            .then(|| (source.clone(), ppl_vm::session::encode_value(&value)));
        definition.source = source;
        if definition.source.is_empty() {
            definition.enabled = false;
        }
        return finish(value, Some(definitions));
    }

    if definition.source.trim().is_empty() {
        return Err(invalid("No definition in Symbolic view"));
    }
    let literal = definition
        .literal
        .as_ref()
        .map(|(_, value)| ppl_vm::session::decode_value(value).map_err(invalid))
        .transpose()?;
    if request.indices.is_empty() {
        return finish(
            literal.map_or_else(
                || {
                    Definition::parse(&definition.source, &snapshot.settings)
                        .map(|parsed| parsed.value)
                },
                Ok,
            )?,
            None,
        );
    }
    let [x, y] = request.indices.as_slice() else {
        return Err(invalid("A surface call requires X and Y"));
    };
    let coordinate = |value: &PortableValue| -> Result<Real, RuntimeError> {
        value.to_value().expect_real("Surface coordinate")
    };
    let (x, y) = (coordinate(x)?, coordinate(y)?);
    if let Some(value) = literal {
        return finish(value, None);
    }
    let context = plot_context(snapshot);
    let context = crate::ide::native_graph_context::resolve(
        &context,
        &[&definition.source],
        &["X", "Y", "x", "y"],
        &cancel,
    )
    .map_err(invalid)?;
    let mut evaluator =
        Evaluator::implicit(&definition.source, &context, cancel.clone()).map_err(invalid)?;
    let result = evaluator.at_coordinates(Value::Real(x), Value::Real(y))?;
    finish(PortableValue::from(&result), None)
}
