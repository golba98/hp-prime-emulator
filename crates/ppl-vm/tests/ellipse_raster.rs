//! Full independent guest-pixel controls for the actual drawing commands.
mod common;
use common::Run;
use ppl_vm::Value;
fn projection(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(projection).collect(),
        value => panic!("Unexpected graphics projection {value:?}"),
    }
}
#[test]
fn original_ellipse_sector_line_and_transparent_frames_match() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-ellipse-line-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        let actual = projection(&value);
        let actual = actual.as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "{}", case["name"]);
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(actual, expected, "{} frame{index}", case["name"]);
        }
    }
}
#[test]
fn command_lines_clip_their_phase_before_work_even_at_i64_extremes() {
    use ppl_graphics::{BLACK, Grob, WHITE};
    let mut frame = Grob::new(3, 3, WHITE);
    frame.calculator_line((i64::MIN, i64::MIN), (i64::MAX, i64::MAX), BLACK);
    assert_eq!(
        frame.pixels(),
        &[
            BLACK, WHITE, WHITE, WHITE, BLACK, WHITE, WHITE, WHITE, BLACK
        ]
    );
    frame.calculator_line((i64::MAX, 0), (i64::MIN, 0), BLACK);
    assert_eq!(&frame.pixels()[0..3], &[BLACK, BLACK, BLACK]);
    frame
        .ellipse_arc(
            (i64::MIN, i64::MAX),
            (400, 300),
            (0., std::f64::consts::TAU),
            BLACK,
            None,
        )
        .unwrap();
    assert_eq!(&frame.pixels()[0..3], &[BLACK, BLACK, BLACK]);
    // Reversal must preserve the opaque line, including both clipping stages,
    // very uneven slopes and coordinates that overflow signed products.
    let coordinates = [i64::MIN, -1000, -1, 0, 1, 3, 1000, i64::MAX];
    for x1 in coordinates {
        for y1 in coordinates {
            for x2 in coordinates {
                for y2 in coordinates {
                    let mut forward = Grob::new(3, 3, WHITE);
                    let mut reverse = forward.clone();
                    forward.calculator_line((x1, y1), (x2, y2), BLACK);
                    reverse.calculator_line((x2, y2), (x1, y1), BLACK);
                    assert_eq!(forward, reverse, "({x1},{y1})->({x2},{y2})");
                }
            }
        }
    }
    assert!(
        frame
            .ellipse_arc(
                (0, 0),
                (500, 500),
                (0., 20. * std::f64::consts::TAU),
                BLACK,
                None,
            )
            .is_err()
    );
}
