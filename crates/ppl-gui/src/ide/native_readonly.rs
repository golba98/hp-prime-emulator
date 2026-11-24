//! Isolated app expressions with cancellation and a fence against memory writes.
use ppl_vm::{PortableValue, SessionContext, SessionSnapshot, Value};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Calculation interrupted".into())
    } else {
        Ok(())
    }
}
pub fn evaluate(
    source: &str,
    app: &str,
    snapshot: &SessionSnapshot,
    overrides: impl IntoIterator<Item = (String, PortableValue)>,
    cancel: Arc<AtomicBool>,
) -> Result<Value, String> {
    check(&cancel)?;
    if source.len() > 65536 {
        return Err("Expression exceeds64KiB".into());
    }
    let mut context = SessionContext::new(snapshot.settings.clone(), 0x5354415453);
    context.active_app = app.into();
    context.app_settings = snapshot.app_settings.clone();
    context.app_numeric_settings = snapshot.app_numeric_settings.clone();
    context.app_files = snapshot.app_files.clone();
    context.cas = snapshot.cas.clone();
    for (name, value) in &snapshot.globals {
        if name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit() {
            continue;
        }
        context
            .set_variable(name, value.clone())
            .map_err(|e| e.to_string())?;
    }
    for (name, value) in overrides {
        context
            .set_variable(&name, value)
            .map_err(|e| e.to_string())?;
    }
    context.settings = snapshot.settings.clone();
    context.app_settings = snapshot.app_settings.clone();
    context.app_numeric_settings = snapshot.app_numeric_settings.clone();
    for (name, source) in &snapshot.programs {
        context
            .store_program(name, source.clone())
            .map_err(|e| e.to_string())?;
    }
    let before = context.snapshot();
    let result = context
        .evaluate_home(
            source,
            Box::new(ppl_vm::HeadlessHost {
                interrupt: Some(cancel.clone()),
                ..Default::default()
            }),
            ppl_vm::Limits {
                max_steps: Some(1_000_000),
                max_time: Some(std::time::Duration::from_secs(2)),
                ..Default::default()
            },
        )
        .result;
    check(&cancel)?;
    let after = context.snapshot();
    let writes = changed(&before, &after);
    if writes {
        return Err("Expression cannot change calculator memory or settings".into());
    }
    result.map_err(|e| e.to_string())
}

pub(super) fn changed(before: &SessionSnapshot, after: &SessionSnapshot) -> bool {
    after
        .globals
        .iter()
        .any(|(name, value)| name != "Ans" && before.globals.get(name) != Some(value))
        || before
            .globals
            .keys()
            .any(|name| !after.globals.contains_key(name))
        || format!("{:?}", before.settings) != format!("{:?}", after.settings)
        || before.active_app != after.active_app
        || before.app_settings != after.app_settings
        || before.app_numeric_settings != after.app_numeric_settings
        || before.app_files != after.app_files
        || before.notes != after.notes
        || before.programs != after.programs
        || before.cas.bindings != after.cas.bindings
}

#[cfg(test)]
mod numeric_tests {
    use super::*;
    #[test]
    fn called_programs_cannot_write_numeric_settings_or_custom_rows() {
        for assignment in [
            "NumStart:=7",
            "NumStep:=2",
            "NumZoom:=3",
            "NumType:=1",
            "NumIndep:={2,4,6}",
        ] {
            let mut context = SessionContext::default();
            context
                .store_program(
                    "MUTATE",
                    format!("EXPORT MUTATE() BEGIN {assignment}; RETURN 7; END;"),
                )
                .unwrap();
            let before = context.memory_json();
            let result = evaluate(
                "MUTATE()",
                "Function",
                &context.snapshot(),
                [],
                Arc::new(AtomicBool::new(false)),
            );
            assert!(
                result
                    .unwrap_err()
                    .contains("cannot change calculator memory"),
                "{assignment}"
            );
            assert_eq!(context.memory_json(), before);
        }
    }
}
