#![cfg(feature = "giac-integration")]
//! Executable G1 controls, compared as typed scalars/lists rather than strings.
mod common;
use common::Run;
use ppl_vm::Value;
fn json_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Str(value) => serde_json::Value::String(value.to_string()),
        Value::List(values) => serde_json::Value::Array(values.iter().map(json_value).collect()),
        value => serde_json::json!(value.expect_i64("control value").unwrap()),
    }
}
#[test]
fn genuine_g1_assignment_arguments_functions_and_special_value_controls_match() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-cas-special-controls.json"
    ))
    .unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(json_value(&value), case["expected"], "{}", case["name"]);
    }
}
