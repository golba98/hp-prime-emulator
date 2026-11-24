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
        let values = data
            .iter()
            .map(|&n| {
                ppl_math::Real::from_f64(n)
                    .map(PortableValue::Real)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        overrides.push((Columns::label(index, two), PortableValue::List(values)));
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
pub fn cell_value(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<f64, String> {
    number(&evaluated(source, two, columns, snapshot, cancel)?)
}
pub fn column_values(
    source: &str,
    two: bool,
    columns: &Columns,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<f64>, String> {
    let Value::List(values) = evaluated(source, two, columns, snapshot, cancel)? else {
        return Err("A statistics column requires a list of real numbers".into());
    };
    if values.len() > super::MAX_ROWS {
        return Err("Statistics column exceeds1000 rows".into());
    }
    values.iter().map(number).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_vm::SessionContext;
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
