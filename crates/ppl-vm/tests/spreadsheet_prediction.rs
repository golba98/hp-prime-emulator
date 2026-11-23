//! Prediction controls from the original virtual G1 Spreadsheet app.

use ppl_math::{AngleMode, NumericVersion, Real};
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
        ("PredX(10,0,0,0,-2,1)", "0"),
        ("PredX(10,0,0.6,-3.8,4.2,-1)", "1"),
        ("PredX(11,0,0,-6,11,-6,1)", "0"),
        ("PredX(11,0,2.4,-15.8,20.6,-8.2,1)", "1"),
    ] {
        expect_real(&mut context, source, expected);
    }
}

#[test]
fn polynomial_inverse_matches_original_monotone_cubics() {
    let mut context = spreadsheet();
    for (source, expected) in [
        ("PredX(10,10,0,1,0,1)", "2"),
        ("PredX(10,-10,0,1,0,1)", "-2"),
        ("PredX(10,8,0,0,0,1)", "2"),
        ("PredX(10,-8,0,0,0,1)", "-2"),
        ("PredX(10,17,1,2,1,1)", "2"),
        ("PredX(10,-5,5,7,-2,1)", "-1"),
        ("PredX(10,-17,-1,-2,-1,-1)", "2"),
        ("PredX(10,-23,3,2,2,1)", "-3.52503661549"),
        ("PredX(10,5,2,3,1,3)", "0.625251455106"),
        ("PredX(10,7,0,0,0,1)", "1.91293118277"),
        ("PredX(10,-7,0,0,0,1)", "-1.91293118277"),
        ("PredX(10,10e50,0,1e50,0,1e50)", "2"),
        ("PredX(10,-10e-50,0,1e-50,0,1e-50)", "-2"),
        ("PredX(11,23,{3,2,2,1,0})", "2"),
        ("PredX(11,10,0,1,0,1,0)", "2"),
        ("PredX(10,10,0,1,0,1e-15)", "10"),
        ("PredX(10,8e30,0,0,0,1)", "20000000000"),
    ] {
        expect_real(&mut context, source, expected);
    }
    let result = evaluate(&mut context, "PredX(10,0,1,3.0001,3,1)").unwrap();
    let Value::Real(result) = result else {
        panic!("expected a real result, got {result:?}");
    };
    // The measured roots differ by about 1e-12 near the flat region.
    let expected = Real::parse("-0.95430219837").unwrap();
    assert!((result.to_f64() - expected.to_f64()).abs() <= 2e-12);
}

#[test]
fn polynomial_inverse_selects_the_unique_root_between_initial_guesses() {
    let mut context = spreadsheet();
    for (source, expected) in [
        ("PredX(11,0,-3,20.75,24.875,8.875,1)", "0.125"),
        ("PredX(11,0,-21,1.25,18.125,8.125,1)", "0.875"),
        ("PredX(11,0,42,-122.5,-166.25,-61.25,-7)", "0.25"),
        ("PredX(11,0,1056,-1609.6,388.4,-33.8,1)", "0.8"),
        ("PredX(11,0.2,0,1,0,0,1)", "0.198449055888"),
        ("PredX(11,1.9,0,1,0,0,1)", "0.979502686242"),
        ("PredX(10,0,2.4,-4,-0.6,1)", "0.6"),
        ("PredX(10,0,-0.070625,0.7525,-1.625,1)", "0.125"),
        ("PredX(10,0,-0.16232,1.1716,-2,1)", "0.2"),
        ("PredX(10,0,-0.125,0.85,-1.65,1)", "0.25"),
        ("PredX(10,0,-0.287484375,1.422875,-2.125,1)", "0.375"),
        ("PredX(10,0,-0.01953125,0.1875,-0.875,1)", "0.625"),
        ("PredX(10,7,7.067265625,-0.733125,3.375,-3)", "0.875"),
        ("PredX(10,0,-0.03328,0.3616,-1.2,1)", "0.8"),
        ("PredX(10,0,-0.0075,0.1325,-0.8,1)", "0.6"),
        ("PredX(10,0,-0.032708,0.3337,-1.1,1)", "0.68"),
        ("PredX(10,0,-0.038175,0.450125,-1.22,1)", "0.12"),
        ("PredX(10,7,12.771484375,-15.796875,12.375,-3)", "0.625"),
        ("PredX(10,0,0.625,-2.6875,0.5,1)", "0.25"),
        ("PredX(10,0,0.24,0.46,-1.75,1)", "0.8"),
        ("PredX(10,0,0.1,0.05,-2.05,1)", "0.25"),
        ("PredX(10,0,-0.24,-0.46,1.75,-1)", "0.8"),
        ("PredX(11,0,{0.625,-2.6875,0.5,1,0})", "0.25"),
    ] {
        expect_real(&mut context, source, expected);
    }
    let result = evaluate(&mut context, "PredX(10,0.2,0,1,10,1)").unwrap();
    let Value::Real(result) = result else {
        panic!("expected a real result, got {result:?}");
    };
    // The original solver rounds this root one unit higher in its last digit.
    let original = Real::parse("0.0996695959134").unwrap();
    assert!((result.to_f64() - original.to_f64()).abs() <= 1.1e-13);

    let source = "PredX(10,7,7.1494140625,-1.32421875,3.75,-3)";
    let Value::Real(result) = evaluate(&mut context, source).unwrap() else {
        panic!("expected a real result from {source}");
    };
    let original = Real::parse("0.749999999997").unwrap();
    assert!((result.to_f64() - original.to_f64()).abs() <= 3.1e-12);
}

#[test]
fn polynomial_inverse_returns_an_exact_first_secant_root_on_either_side() {
    let mut context = spreadsheet();
    let cases = [
        ("PredX(10,0,-18,-9,2,1)", "-3"),
        ("PredX(10,0,24,-2,-5,1)", "4"),
        ("PredX(11,0,-0.9,1.95,4.85,-4.8,1)", "0.3"),
        ("PredX(11,0,9,13.5,-10,-1.5,1)", "-3"),
        ("PredX(11,0,-12,-11,21.5,-8.5,1)", "4"),
    ];
    for version in [NumericVersion::Legacy15157, NumericVersion::Current15751] {
        context.settings.numeric_version = version;
        for (source, expected) in cases {
            expect_real(&mut context, source, expected);
        }
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
        (
            "PredX(11,2,1,0,-3,0,-2)",
            ErrorKind::Undefined,
            "Extremum found",
        ),
        (
            "PredX(10,0,26,-1.5,-5.25,1)",
            ErrorKind::Unsupported,
            "PredX root selection for this polynomial is unavailable",
        ),
        (
            "PredX(10,0,-0.03515625,0.421875,-1.25,1)",
            ErrorKind::Unsupported,
            "PredX root selection for this polynomial is unavailable",
        ),
        (
            "PredX(10,0,0.24,0.5,-1.75,1)",
            ErrorKind::Unsupported,
            "PredX root selection for this polynomial is unavailable",
        ),
        (
            "PredX(11,0,72,18,-17,-2,1)",
            ErrorKind::Unsupported,
            "PredX root selection for this polynomial is unavailable",
        ),
    ] {
        let error = evaluate(&mut context, source).unwrap_err();
        assert_eq!(error.kind, kind, "{source}");
        assert_eq!(error.detail, detail, "{source}");
    }
}
