//! A cell expression runs against an isolated app/column snapshot. The worker
//! commits only its resulting real cell; Home/CAS answers and memory stay intact.
use super::Columns;
use ppl_vm::{PortableValue, SessionSnapshot, Value};
use std::sync::{Arc, atomic::AtomicBool};
fn evaluated(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Value, String> {
    columns.validate()?;
    let mut overrides = Vec::new();
    for (index, data) in columns.0.iter().enumerate() {
        let name = Columns::label(index, two);
        // The VM snapshot owns typed observations. The f64 columns are only a
        // legacy migration fallback, never an override of existing calculator data.
        if snapshot.globals.contains_key(&name) {
            continue;
        }
        let values = data
            .iter()
            .map(|&n| {
                ppl_math::Real::from_f64(n)
                    .map(PortableValue::Real)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        overrides.push((name, PortableValue::List(values)));
    }
    super::super::native_readonly::evaluate(
        source,
        if two {
            "Statistics 2Var"
        } else {
            "Statistics 1Var"
        },
        snapshot,
        overrides,
        cancel,
    )
}
fn number(value: &Value) -> Result<f64, String> {
    let n = value
        .expect_real("statistics data")
        .map_err(|e| e.to_string())?;
    let number = n.to_f64();
    if !number.is_finite() || (!n.is_zero() && number == 0.) {
        return Err("Statistics value exceeds numerical range".into());
    }
    Ok(number)
}
pub fn typed_cell(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<PortableValue, String> {
    let value = evaluated(source, two, columns, snapshot, cancel)?;
    value
        .expect_real("statistics data")
        .map(PortableValue::Real)
        .map_err(|error| error.to_string())
}
pub fn typed_column(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<PortableValue, String> {
    let Value::List(values) = evaluated(source, two, columns, snapshot, cancel)? else {
        return Err("A statistics column requires a list of real numbers".into());
    };
    if values.len() > super::MAX_ROWS {
        return Err("Statistics column exceeds1000 rows".into());
    }
    Ok(PortableValue::List(
        values
            .iter()
            .map(|value| {
                value
                    .expect_real("statistics data")
                    .map(PortableValue::Real)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
    ))
}
pub fn cell_value(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<f64, String> {
    number(&typed_cell(source, two, columns, snapshot, cancel)?.to_value())
}
pub fn column_values(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<f64>, String> {
    let PortableValue::List(values) = typed_column(source, two, columns, snapshot, cancel)? else {
        unreachable!("typed_column always returns a list")
    };
    values
        .iter()
        .map(|value| number(&value.to_value()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_vm::SessionContext;
    #[test]
    fn typed_evaluation_uses_exact_vm_columns_instead_of_the_lossy_read_cache() {
        let mut context = SessionContext::default();
        context.settings.numeric_version = ppl_math::NumericVersion::Current15751;
        let exact = ppl_math::Real::parse_versioned(
            "1234567890123456789",
            context.settings.numeric_version,
        )
        .unwrap();
        context
            .set_variable("D1", PortableValue::List(vec![PortableValue::Real(exact)]))
            .unwrap();
        let snapshot = context.snapshot();
        let mut columns = Columns::default();
        columns.0[0] = vec![1.];
        let cancel = Arc::new(AtomicBool::new(false));
        let value = typed_cell("D1(1)+1", false, &columns, &snapshot, cancel.clone()).unwrap();
        let expected = exact.add(ppl_math::Real::ONE).unwrap();
        assert_eq!(
            ppl_vm::session::encode_value(&value),
            ppl_vm::session::encode_value(&PortableValue::Real(expected))
        );
        let value = typed_column(
            "{D1(1),1/3,1E400,1E-400}",
            false,
            &columns,
            &snapshot,
            cancel,
        )
        .unwrap();
        let PortableValue::List(values) = value else {
            panic!()
        };
        assert_eq!(
            ppl_vm::session::encode_value(&values[0]),
            ppl_vm::session::encode_value(&PortableValue::Real(exact))
        );
        assert_eq!(
            values[1]
                .to_value()
                .expect_real("fraction")
                .unwrap()
                .ppl_source(),
            "(1/3)"
        );
        assert!(!values[2].to_value().expect_real("large").unwrap().is_zero());
        assert!(!values[3].to_value().expect_real("small").unwrap().is_zero());
        assert_eq!(context.snapshot().globals, snapshot.globals);
        assert!(context.snapshot().last_value.is_none());
    }
    #[test]
    fn cells_use_current_columns_home_values_installed_functions_and_app_angle() {
        let mut context = SessionContext::default();
        context
            .set_variable("A", PortableValue::Real(ppl_math::Real::from_i64(4)))
            .unwrap();
        context
            .store_program("STFN", "EXPORT STFN(X) BEGIN RETURN X+A; END;".into())
            .unwrap();
        let mut snapshot = context.snapshot();
        snapshot.app_settings.insert(
            "Statistics 2Var".into(),
            ppl_vm::AppSettings {
                angle: 2,
                ..Default::default()
            },
        );
        let mut columns = Columns::default();
        columns.0[0] = vec![1., 2., 3.];
        let cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(
            cell_value(
                "STFN(ΣLIST(C1))+SIN(30)",
                true,
                &columns,
                &snapshot,
                cancel.clone()
            )
            .unwrap(),
            10.5
        );
        assert_eq!(
            cell_value("#FF:8h", true, &columns, &snapshot, cancel).unwrap(),
            255.
        );
        assert_eq!(context.snapshot().globals, snapshot.globals);
        assert!(context.snapshot().last_value.is_none());
    }
    #[test]
    fn cells_refuse_memory_writes_objects_and_cancelled_computation() {
        let context = SessionContext::default();
        let snapshot = context.snapshot();
        let columns = Columns::default();
        for expression in ["A:=9", "HAngle:=1", "{1,2}", "\"word\""] {
            assert!(
                cell_value(
                    expression,
                    true,
                    &columns,
                    &snapshot,
                    Arc::new(AtomicBool::new(false))
                )
                .is_err(),
                "{expression}"
            );
        }
        assert!(
            cell_value(
                "1+1",
                true,
                &columns,
                &snapshot,
                Arc::new(AtomicBool::new(true))
            )
            .is_err()
        );
        assert_eq!(context.snapshot().globals, snapshot.globals);
    }
}
