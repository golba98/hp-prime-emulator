//! Documented Home hexagesimal literals, HP Prime User Guide p.15.
//! No physical-device evidence is implied by these native regression tests.
use ppl_math::Real;
use ppl_vm::{HeadlessHost, Limits, SessionContext};
fn evaluate(text: &str) -> Real {
    SessionContext::default()
        .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap()
        .expect_real("result")
        .unwrap()
}
#[test]
fn degrees_minutes_seconds_are_one_numeric_operand() {
    for (text, expected) in [
        ("1°22′30″", "1.375"),
        ("-1°22′30″", "-1.375"),
        ("1°30′", "1.5"),
        ("36°40′20″", "36.6722222222"),
        ("2°30′^2", "6.25"),
        ("1°30′+2°45′", "4.25"),
        ("1°0′1.5″", "1.00041666667"),
    ] {
        assert_eq!(evaluate(text), Real::parse(expected).unwrap(), "{text}");
    }
}
#[test]
fn invalid_fractional_degrees_minutes_or_negative_components_fail() {
    for text in ["1.5°", "1°2.5′", "1°-2′", "1°2′-3″", "1°2′3"] {
        let (_, diagnostics) = ppl_syntax::parse_expression(text);
        assert!(diagnostics.has_errors(), "{text}");
    }
    // Integer-valued exponential spellings retain exact numeric interpretation.
    assert_eq!(evaluate("2E1°3E1′"), Real::parse("20.5").unwrap());
}
