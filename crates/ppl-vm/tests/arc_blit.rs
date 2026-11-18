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
fn original_arc_blit_and_subgrob_results_match() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-arc-blit-controls.json"
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
fn arc_work_is_bounded_by_visible_pixels_and_handles_extreme_coordinates() {
    use ppl_graphics::{BLACK, Grob, WHITE};
    let mut frame = Grob::new(3, 3, WHITE);
    // Only the rightmost column of this enormous, mostly clipped circle is
    // visible. This must not iterate the radius or a huge angular sweep.
    frame.arc(-i64::MAX, 1, i64::MAX, 0., f64::MAX, BLACK);
    assert_eq!(
        frame.pixels(),
        &[
            BLACK, WHITE, WHITE, BLACK, WHITE, WHITE, BLACK, WHITE, WHITE
        ]
    );
    frame.arc(i64::MAX, i64::MIN, i64::MAX, -f64::MAX, f64::MAX, BLACK);
    assert_eq!(frame.get(0, 1), Some(BLACK));
}
