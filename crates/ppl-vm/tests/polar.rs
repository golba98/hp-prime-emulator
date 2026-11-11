//! Native calculator input tests. These exercise documented Home polar entry;
//! acceptance in PPL source is a native extension, not hardware parity evidence.
use ppl_compiler::{CompileOptions, compile_source};
use ppl_math::{Complex, Real};
use ppl_syntax::SourceFile;
use ppl_vm::{
    AngleMode, ErrorKind, HeadlessHost, Limits, Machine, SessionContext, Settings, Value,
};
use std::rc::Rc;

fn expression(text: &str, angle: AngleMode) -> Value {
    let mut session = SessionContext::new(
        Settings {
            angle,
            ..Settings::default()
        },
        0,
    );
    session
        .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap()
}
fn cartesian(value: Value) -> Complex {
    match value {
        Value::Complex(value) => value,
        other => Complex::new(other.expect_real("result").unwrap(), Real::ZERO),
    }
}
#[test]
fn polar_entry_obeys_angle_mode_and_both_documented_input_forms() {
    for (text, mode) in [
        ("2∠90", AngleMode::Degrees),
        ("(2,∡100)", AngleMode::Gradians),
    ] {
        let result = cartesian(expression(text, mode));
        assert_eq!((result.re, result.im), (Real::ZERO, Real::from_i64(2)));
    }
    let value = cartesian(expression("2∠(π/2)", AngleMode::Radians));
    assert!(value.re.to_f64().abs() < 2e-11, "{value:?}");
    assert!((value.im.to_f64() - 2.).abs() < 5e-12);
}
#[test]
fn polar_entry_preserves_large_small_zero_and_negative_magnitudes() {
    for (text, real, imaginary) in [
        ("-2∠90", "0", "-2"),
        ("2∠-90", "0", "-2"),
        ("1E400∠90", "0", "1E400"),
        ("1E-400∠90", "0", "1E-400"),
        ("0∠1E499", "0", "0"),
    ] {
        let value = cartesian(expression(text, AngleMode::Degrees));
        assert_eq!(
            (value.re, value.im),
            (Real::parse(real).unwrap(), Real::parse(imaginary).unwrap()),
            "{text}"
        );
    }
}
#[test]
fn compiled_polar_operands_can_be_variables_and_join_complex_arithmetic() {
    let source = SourceFile::new(
        "native-polar.hpprgm",
        "EXPORT NATIVEPOLAR() BEGIN LOCAL radius:=2,theta:=90; RETURN (radius∠theta)+(3∠0); END;",
    );
    let (program, diagnostics) = compile_source(&source, &CompileOptions::default());
    assert!(!diagnostics.has_errors(), "{:?}", diagnostics.items);
    let mut machine = Machine::new(
        Rc::new(program.unwrap()),
        Box::new(HeadlessHost::new()),
        Settings {
            angle: AngleMode::Degrees,
            ..Settings::default()
        },
        Limits::default(),
    );
    let value = cartesian(machine.call_function("NATIVEPOLAR", vec![]).unwrap());
    assert_eq!((value.re, value.im), (Real::from_i64(3), Real::from_i64(2)));
}
#[test]
fn polar_entry_rejects_nonreal_operands_without_fabricated_success() {
    for text in ["(1,2)∠90", "2∠(1,2)"] {
        let mut session = SessionContext::default();
        let outcome = session.evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default());
        assert_eq!(
            outcome.result.unwrap_err().kind,
            ErrorKind::BadArgType,
            "{text}"
        );
    }
}
