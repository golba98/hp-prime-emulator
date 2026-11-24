//! Independently returned original-firmware types and complete bitmap pixels.
mod common;
use common::Run;
use ppl_vm::Value;
fn encoded(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::json!({"type":"real","value":value.to_string()}),
        Value::Int(value) => {
            serde_json::json!({"type":"integer","bits":value.bits(),"unsigned":value.unsigned().to_string()})
        }
        Value::Str(value) => serde_json::json!({"type":"string","value":value.to_string()}),
        Value::List(values) => {
            serde_json::json!({"type":"list","value":values.iter().map(encoded).collect::<Vec<_>>()})
        }
        value => panic!("Unexpected graphics control value {value:?}"),
    }
}
#[test]
fn original_g1_graphics_types_returns_resampling_clipping_and_color_controls() {
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-graphics-overload-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(encoded(&value), case["expected"], "{}", case["name"]);
    }
}
