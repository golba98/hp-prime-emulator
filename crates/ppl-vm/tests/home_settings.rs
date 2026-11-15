//! Original G1 codes, number formats and Complex setting semantics.
mod common;
use common::Run;
use ppl_vm::Value;
fn json(value: &Value) -> serde_json::Value {
    match value {
        Value::List(values) => serde_json::Value::Array(values.iter().map(json).collect()),
        Value::Str(value) => serde_json::json!(value.to_string()),
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        value => panic!("Unexpected control value: {value:?}"),
    }
}
#[test]
fn original_g1_setting_and_string_controls_match_without_glyph_normalization() {
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-settings-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(json(&value), case["expected"], "{}", case["name"]);
    }
}

#[test]
fn display_settings_survive_memory_restore_and_reject_invalid_codes_atomically() {
    use ppl_vm::{HeadlessHost, Limits, PortableValue, SessionContext};
    let mut original = SessionContext::default();
    for (name, code) in [
        ("HSeparator", 10),
        ("HComplex", 11),
        ("HFormat", 4),
        ("HDigits", 3),
    ] {
        original
            .set_variable(name, PortableValue::Real(ppl_math::Real::from_i64(code)))
            .unwrap();
    }
    let image = original.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    assert_eq!(restored.memory_json(), image);
    assert_eq!(
        restored
            .settings
            .home_format()
            .digit_grouping
            .unwrap()
            .code(),
        10
    );
    assert_eq!(restored.settings.complex_code(), 11);
    let value = restored
        .evaluate_home(
            "{HSeparator,HComplex,HFormat,HDigits,STRING((3,4))}",
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap();
    assert_eq!(json(&value), serde_json::json!([10, 11, 4, 3, "(3,4)"]));
    let saved = restored.memory_json();
    for (name, code) in [
        ("HSeparator", 11),
        ("HComplex", 2),
        ("HFormat", 6),
        ("HDigits", 12),
    ] {
        assert!(
            restored
                .set_variable(name, PortableValue::Real(ppl_math::Real::from_i64(code)))
                .is_err()
        );
        assert_eq!(restored.memory_json(), saved);
    }
    for (key, code) in [("separator", 11), ("complex_format", 2)] {
        let mut broken = saved.clone();
        broken["settings"][key] = code.into();
        assert!(restored.restore_memory(&broken).is_err());
        assert_eq!(restored.memory_json(), saved);
    }
    let mut legacy = image;
    for key in ["separator", "complex_format"] {
        legacy["settings"].as_object_mut().unwrap().remove(key);
    }
    restored.restore_memory(&legacy).unwrap();
    assert_eq!(restored.settings.digit_grouping.code(), 0);
    assert_eq!(restored.settings.complex_code(), 1);
}
