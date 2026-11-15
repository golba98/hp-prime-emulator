//! Independent legacy15157 controls: selected denominators, signs, precision
//! and ordinary-number fallback all come from the original executable.
mod common;
use common::Run;
use ppl_vm::Value;
fn projection(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(projection).collect(),
        value => panic!("Unexpected formatting projection {value:?}"),
    }
}
#[test]
fn legacy_fraction_modes_and_independent_holdouts_match_original() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-legacy-fraction-controls.json"
    ))
    .unwrap();
    let mut failures = vec![];
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["native_transport_source"].as_str().unwrap(),
        );
        run.machine.settings.string_format_version = ppl_vm::StringFormatVersion::Legacy15157;
        let result = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        let actual = projection(&result);
        let actual = actual.as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            if actual != expected {
                failures.push(format!("{}: {actual} expected {expected}", case["name"]));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn selected_string_version_restores_and_unknown_versions_are_atomic_errors() {
    use ppl_vm::{HeadlessHost, Limits, SessionContext, StringFormatVersion};
    let mut session = SessionContext::default();
    session.settings.string_format_version = StringFormatVersion::Legacy15157;
    let image = session.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    assert_eq!(
        restored.settings.string_format_version,
        StringFormatVersion::Legacy15157
    );
    assert_eq!(
        restored
            .evaluate_home(
                "STRING(-7/3,15,3)",
                Box::new(HeadlessHost::new()),
                Limits::default()
            )
            .result
            .unwrap()
            .expect_str("result")
            .unwrap()
            .as_ref(),
        "−2−1/3"
    );
    let preserved = restored.memory_json();
    for invalid in [
        serde_json::json!(0),
        serde_json::json!("15157"),
        serde_json::Value::Null,
    ] {
        let mut broken = preserved.clone();
        broken["settings"]["string_format_version"] = invalid;
        assert!(restored.restore_memory(&broken).is_err());
        assert_eq!(restored.memory_json(), preserved);
    }
    let mut old = preserved;
    old["settings"]
        .as_object_mut()
        .unwrap()
        .remove("string_format_version");
    restored.restore_memory(&old).unwrap();
    assert_eq!(
        restored.settings.string_format_version,
        StringFormatVersion::Current15751
    );
}
