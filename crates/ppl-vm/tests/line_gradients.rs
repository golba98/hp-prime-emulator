//! Independently returned original-firmware colored-line pixel data.
mod common;
use common::Run;
use ppl_vm::Value;
fn json(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(json).collect(),
        value => panic!("Unexpected pixel projection {value:?}"),
    }
}
#[test]
fn original_line_pixels_match_for_full_frames_clipping_directions_and_channel_boundaries() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-line-gradient-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(json(&value), case["expected"], "{}", case["name"]);
    }
}
