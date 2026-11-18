//! Prediction controls from the original virtual G1 Spreadsheet app.

use ppl_math::{AngleMode, Real};
use ppl_vm::{ErrorKind, HeadlessHost, Limits, RuntimeError, SessionContext, Value};

fn evaluate(context: &mut SessionContext, text: &str) -> Result<Value, RuntimeError> {
    context
        .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
        .result
}

fn spreadsheet() -> SessionContext {
    let mut context = SessionContext::default();
    context.active_app = "Spreadsheet".into();
    context
}

fn expect_real(context: &mut SessionContext, source: &str, expected: &str) {
    let result = evaluate(context, source).unwrap_or_else(|error| panic!("{source}: {error}"));
    let Value::Real(result) = result else {
        panic!("{source}: expected a real result, got {result:?}");
    };
    assert_eq!(
        result.to_string(),
        Real::parse(expected).unwrap().to_string(),
        "{source}"
    );
}

#[test]
fn forward_and_inverse_controls_match_original_coefficient_order() {
    let mut context = spreadsheet();
    for (source, expected) in [
        ("PredY(1,3,{2,5})", "11"),
        ("PredY(#1h,#3h,#2h,#5h)", "11"),
        ("PredY(2,2,3,4)", "6.07944154168"),
        ("PredY(3,2,0.1,10)", "12.2140275816"),
        ("PredY(4,3,2,4)", "36"),
        ("PredY(5,4,2,3)", "48"),
        ("PredY(6,3,1,2)", "2.33333333333"),
        ("PredY(7,2,9,2,1)", "7.08287437945"),
        ("PredY(8,1,2,1,1,5)", "6.81859485365"),
        ("PredY(9,2,1,2,3)", "17"),
        ("PredY(10,2,1,2,3,4)", "49"),
        ("PredY(11,2,{1,2,3,4,5})", "129"),
        ("PredY(1,1e400,2,3)", "2e400"),
        ("PredX(1,11,2,5)", "3"),
        ("PredX(2,5,2,5)", "1"),
        ("PredX(3,5,2,5)", "0"),
        ("PredX(4,32,2,5)", "2.52982212813"),
        ("PredX(4,-40,3,5)", "-2"),
        ("PredX(5,5,2,5)", "0"),
        ("PredX(6,6,2,5)", "2"),
        ("PredX(7,6,9,2,1)", "1.38629436112"),
        ("PredX(8,6,2,1,0,5)", "0.523598775598"),
    ] {
        expect_real(&mut context, source, expected);
    }
}

#[test]
fn trigonometric_predictions_respect_angle_mode_and_validate_inputs() {
    let mut context = spreadsheet();
    context.settings.angle = AngleMode::Degrees;
    expect_real(&mut context, "PredY(8,30,2,1,0,5)", "6");
    expect_real(&mut context, "PredX(8,6,2,1,120,5)", "-90");
    context.settings.angle = AngleMode::Gradians;
    expect_real(&mut context, "PredX(8,6,2,1,0,5)", "33.3333333333");

    for source in [
        "PredY(1.4,3,2,5)",
        "PredY(12,3,2,5)",
        "PredY(1,3,[2,5])",
        "PredY(1,3,{2,5,99})",
    ] {
        assert_eq!(
            evaluate(&mut context, source).unwrap_err().kind,
            ErrorKind::InvalidInput,
            "{source}"
        );
    }
    context.active_app = "Function".into();
    assert_eq!(
        evaluate(&mut context, "PredY(1,3,2,5)").unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn polynomial_inverse_matches_the_original_quadratic_selection() {
    let mut context = spreadsheet();
    for (source, expected) in [
        ("PredX(9,0,-6,-1,1)", "3"),
        ("PredX(9,40,34,-1,1)", "3"),
        ("PredX(9,0,-6.5,-1.25,1)", "-2"),
        ("PredX(9,0,-6.000000002,-1.000000001,1)", "-2"),
        ("PredX(9,0,6,1,-1)", "3"),
        ("PredX(9,0,6,-5,1)", "2"),
        ("PredX(9,0,6,5,1)", "-2"),
        ("PredX(9,0,0.16,-1,1)", "0.2"),
        ("PredX(9,0,0.27,-1.2,1)", "0.9"),
        ("PredX(9,0,0.2,-1.2,1)", "1"),
        ("PredX(9,0,0,-1,1)", "0"),
        ("PredX(9,0,1,-2,1)", "1"),
        ("PredX(9,0,-6e310,-1e310,1e310)", "3"),
        ("PredX(9,0,-6e-330,-1e-330,1e-330)", "3"),
        ("PredX(10,0,-6,-1,1,0)", "3"),
        ("PredX(11,0,{-6,-1,1,0,0})", "3"),
        ("PredX(11,6,0,2,0,0,0)", "3"),
        ("PredX(11,6,6,0,0,0,0)", "0"),
        ("PredX(10,0,-24,26,-9,1)", "2"),
        ("PredX(10,0,24,26,9,1)", "-2"),
        ("PredX(10,0,24,-2,-5,1)", "4"),
        ("PredX(10,6,24,-2,-5,1)", "2.20506763921"),
        ("PredX(10,0,26,-1.5,-5.25,1)", "3.25"),
        ("PredX(10,0,-18,-9,2,1)", "-3"),
        ("PredX(10,0,-19.5,-10.25,1.75,1)", "-2"),
        ("PredX(10,0,0.08,-0.34,-0.5,1)", "-0.5"),
        ("PredX(11,0,120,-154,71,-14,1)", "2"),
        ("PredX(11,0,120,154,71,14,1)", "-2"),
        ("PredX(11,0,-120,34,23,-10,1)", "3"),
        ("PredX(11,0,-130,33.5,24.75,-10.25,1)", "3.25"),
        ("PredX(11,0,72,18,-17,-2,1)", "-2"),
        ("PredX(11,0,78,21.5,-17.25,-2.25,1)", "-2"),
        ("PredX(10,8,0,0,0,1)", "2"),
        ("PredX(10,-8,0,0,0,1)", "-2"),
        ("PredX(10,0,0,0,-2,1)", "0"),
        ("PredX(10,0,0.6,-3.8,4.2,-1)", "1"),
        ("PredX(11,0,0,-6,11,-6,1)", "0"),
        ("PredX(11,0,2.4,-15.8,20.6,-8.2,1)", "1"),
    ] {
        expect_real(&mut context, source, expected);
    }
}

#[test]
fn polynomial_inverse_preserves_distinct_failure_cases() {
    let mut context = spreadsheet();
    for (source, kind, detail) in [
        ("PredX(9,0,4,-4,1)", ErrorKind::Undefined, "Extremum found"),
        ("PredX(9,0,1,0,1)", ErrorKind::Undefined, "Extremum found"),
        (
            "PredX(9,0,0.64,-1.6,1)",
            ErrorKind::Unsupported,
            "PredX repeated root search inside the initial interval is unavailable",
        ),
        (
            "PredX(11,7,6,0,0,0,0)",
            ErrorKind::Undefined,
            "Constant function",
        ),
        (
            "PredX(11,0,1,0,1,0,1)",
            ErrorKind::Undefined,
            "Extremum found",
        ),
    ] {
        let error = evaluate(&mut context, source).unwrap_err();
        assert_eq!(error.kind, kind, "{source}");
        assert_eq!(error.detail, detail, "{source}");
    }
}

#[test]
fn spreadsheet_sum_and_average_preserve_typed_operands() {
    let mut context = spreadsheet();
    for (source, expected) in [
        ("SUM()", "0"),
        ("SUM({1,2},3)", "6"),
        ("AVERAGE({{1,3},{8}})", "4"),
        ("AVERAGE({#2h,#4h})", "3"),
        ("SUM({{1,2},{3,4}})", "10"),
        ("AVERAGE({1,2,3})", "2"),
    ] {
        expect_real(&mut context, source, expected);
    }
    for (source, expected) in [
        ("SUM({#1h,#3h})", "#4h"),
        ("AVERAGE(\"xx\")", "\"xx\""),
        ("SUM({1,\"a\"})", "\"1a\""),
        ("AVERAGE([[1,2],[3,4]])", "[[1,2],[3,4]]"),
    ] {
        assert!(
            evaluate(&mut context, source)
                .unwrap()
                .equals(&evaluate(&mut context, expected).unwrap())
        );
    }
    assert!(evaluate(&mut context, "AVERAGE({})").is_err());
    assert!(evaluate(&mut context, "AVERAGE({1,\"a\"})").is_err());
}
