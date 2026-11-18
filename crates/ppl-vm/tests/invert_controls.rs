//! INVERT_P admission and pixels captured from the original virtual G1.
mod common;
use common::Run;
use ppl_vm::Value;
use sha2::{Digest, Sha256};

fn project(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::json!(value.to_i64().unwrap()),
        Value::List(values) => values.iter().map(project).collect(),
        value => panic!("Unexpected INVERT_P result {value:?}"),
    }
}

#[test]
fn original_invert_admission_and_pixels_match() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-invert-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let name = case["name"].as_str().unwrap();
        let mut run = Run::new(name, source);
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(project(&value), case["expected"], "{name}");
    }
}
