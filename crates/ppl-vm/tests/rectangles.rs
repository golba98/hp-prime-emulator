//! Full reference frames from independent original-firmware execution.
mod common;
use common::Run;
use ppl_vm::Value;

fn projection(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(projection).collect(),
        value => panic!("Unexpected pixel projection {value:?}"),
    }
}

#[test]
fn rectangles_match_original_transparency_degenerate_and_clipped_frames() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-rect-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(projection(&value), case["expected"], "{}", case["name"]);
    }
}

#[test]
fn rectangle_work_is_visible_bounded_and_extreme_clipping_does_not_create_borders() {
    use ppl_graphics::{Grob, WHITE};
    let mut target = Grob::new(3, 3, WHITE);
    target.calculator_rect(
        (i64::MIN, i64::MIN),
        (i64::MAX, i64::MAX),
        0xFF0000,
        0x0000FF,
    );
    assert_eq!(target.pixels(), &[0x0000FF; 9]);
    target.calculator_rect((0, i64::MIN), (0, i64::MAX), 0x8000FF00, 0xFF0000);
    assert_eq!(
        target.pixels(),
        &[
            0x007F7F, 0x0000FF, 0x0000FF, 0x007F7F, 0x0000FF, 0x0000FF, 0x007F7F, 0x0000FF,
            0x0000FF
        ]
    );
    let before = target.pixels().to_vec();
    target.calculator_rect((i64::MIN, i64::MIN), (-1, i64::MAX), 0, 0);
    assert_eq!(target.pixels(), before);
    // A single pixel is one edge contribution, with no fill underneath.
    target.calculator_rect((1, 1), (1, 1), 0x80FF0000, 0x00FF00);
    assert_eq!(target.get(1, 1), Some(0x7F007F));
}
