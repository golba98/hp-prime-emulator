//! Isolated app expressions with cancellation and a fence against memory writes.
use super::native_application_access::geometry::{ReadSnapshot, read_only_host};
use ppl_vm::{PortableValue, SessionContext, SessionSnapshot, Value};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
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
    evaluate_with_geometry(source, app, snapshot, overrides, cancel, None)
}

pub(crate) fn evaluate_with_geometry(
    source: &str,
    app: &str,
    snapshot: &SessionSnapshot,
    overrides: impl IntoIterator<Item = (String, PortableValue)>,
    cancel: Arc<AtomicBool>,
    geometry: ReadSnapshot,
) -> Result<Value, String> {
    check(&cancel)?;
    if let Some(book) = &geometry {
        book.validate()?;
    }
    if source.len() > 65536 {
        return Err("Expression exceeds64KiB".into());
    }
    let mut context = SessionContext::new(snapshot.settings.clone(), 0x5354415453);
    context.active_app = app.into();
    context.app_settings = snapshot.app_settings.clone();
    context.app_numeric_settings = snapshot.app_numeric_settings.clone();
    context.app_plot_settings = snapshot.app_plot_settings.clone();
    context.geometry_plot_settings = snapshot.geometry_plot_settings.clone();
    context.app_files = snapshot.app_files.clone();
    context.cas = snapshot.cas.clone();
    for (name, value) in &snapshot.globals {
        if (name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit())
            || (context.active_app == "Geometry" && ppl_vm::GeometryPlotSettings::recognizes(name))
        {
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
    context.app_plot_settings = snapshot.app_plot_settings.clone();
    context.geometry_plot_settings = snapshot.geometry_plot_settings.clone();
    for (name, source) in &snapshot.programs {
        context
            .store_program(name, source.clone())
            .map_err(|e| e.to_string())?;
    }
    let before = context.snapshot();
    let timeout = Duration::from_secs(2);
    let result = context
        .evaluate_home(
            source,
            read_only_host(
                geometry,
                cancel.clone(),
                Instant::now() + timeout,
                "Calculation timed out",
            ),
            ppl_vm::Limits {
                max_steps: Some(1_000_000),
                max_time: Some(timeout),
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
        || before.app_plot_settings != after.app_plot_settings
        || before.geometry_plot_settings != after.geometry_plot_settings
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
            "Xmin:=7",
            "Function.Xmax:=23",
            "Statistics_1Var.Hwidth:=3",
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
    #[test]
    fn scoped_plot_reads_use_the_requested_app_and_do_not_mutate_the_snapshot() {
        let mut context = SessionContext::default();
        context
            .set_variable("Xmin", PortableValue::Real(ppl_math::Real::from_i64(7)))
            .unwrap();
        let before = context.memory_json();
        for (app, source, expected) in [
            ("Function", "Xmin", "7"),
            ("Statistics 1Var", "Function.Xmin", "7"),
            ("Spreadsheet", "6*7", "42"),
            ("Geometry", "6*7", "42"),
        ] {
            let result = evaluate(
                source,
                app,
                &context.snapshot(),
                [],
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert_eq!(
                result.string_form(ppl_vm::value::default_format()),
                expected
            );
            assert_eq!(context.memory_json(), before);
        }
    }
}
