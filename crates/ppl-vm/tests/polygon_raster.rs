//! FILLPOLY_P admission and test-region pixels from the original virtual G1.
mod common;
use common::Run;
use ppl_vm::Value;
use sha2::{Digest, Sha256};

fn project(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::json!(value.to_i64().unwrap()),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(project).collect(),
        other => panic!("Unexpected polygon result {other:?}"),
    }
}

#[test]
fn original_polygon_admission_and_pixels_match() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-polygon-controls.json"
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

#[test]
fn polygons_leave_print_output_in_the_terminal() {
    let mut run = Run::new(
        "polygon terminal isolation",
        "EXPORT POLYTERM() BEGIN PRINT(); PRINT(\"before\"); +         FILLPOLY_P({{0,0},{5,0},{0,5}},#FF0000h); +         PRINT(\"after\"); RETURN B→R(GETPIX_P(1,1)); END;",
    );
    let pixels = run.call("POLYTERM", &[]).unwrap();
    assert_eq!(project(&pixels), 0xff_00_00);
    assert_eq!(run.machine.terminal, ["before", "after"]);
}
