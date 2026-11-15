//! Independent original-G1 controls for the versioned real representation.
use ppl_math::{NumericVersion, Real, RealOrigin};

fn current(text: &str) -> Real {
    Real::parse_versioned(text, NumericVersion::Current15751).unwrap()
}
fn rational(value: Real, numerator: u64, denominator: u64) {
    assert_eq!(
        value.origin(),
        RealOrigin::Rational {
            numerator,
            denominator
        }
    );
}
#[test]
fn original_literal_precision_and_u64_representation_limits() {
    let a = current("1234567890123456789");
    assert_eq!(a.to_i128(), Some(1_234_567_890_123_456_789));
    assert_eq!(
        a.add(current("1")).unwrap().to_i128(),
        Some(1_234_567_890_123_456_790)
    );
    assert_eq!(
        current("-1234567890123456789").to_i128(),
        Some(-1_234_567_890_123_456_789)
    );
    rational(
        current("0.1234567890123456789"),
        1_234_567_890_123_456_789,
        10_000_000_000_000_000_000,
    );
    rational(current("1E19"), 10_000_000_000_000_000_000, 1);
    rational(current("1E-19"), 1, 10_000_000_000_000_000_000);
    assert_eq!(current("1E20").origin(), RealOrigin::Decimal);
    assert_eq!(current("1E-20").origin(), RealOrigin::Decimal);
    rational(current("18446744073709551615"), u64::MAX, 1);
    assert_eq!(
        current("18446744073709551616").origin(),
        RealOrigin::Decimal
    );
}
#[test]
fn original_rational_arithmetic_keeps_provenance_until_a_real_limit_is_exceeded() {
    let third = current("1").div(current("3")).unwrap();
    rational(third, 1, 3);
    rational(third.mul(current("3")).unwrap(), 1, 1);
    rational(
        current("0.333333333333").mul(current("3")).unwrap(),
        999_999_999_999,
        1_000_000_000_000,
    );
    rational(third.add(current("0.1")).unwrap(), 13, 30);
    let huge = current("1E19").div(current("3")).unwrap();
    rational(
        huge.mul(current("3")).unwrap(),
        10_000_000_000_000_000_000,
        1,
    );
    assert_eq!(
        current("1E-19").div(current("3")).unwrap().origin(),
        RealOrigin::Decimal
    );
    let decimal = current("1E20").div(current("1E10")).unwrap();
    assert_eq!(decimal.origin(), RealOrigin::Decimal);
    let decimal_one = decimal
        .sub(current("1E10"))
        .unwrap()
        .add(current("1"))
        .unwrap();
    assert_eq!(decimal_one.origin(), RealOrigin::Decimal);
    assert_eq!(decimal_one.to_string(), "1");
}
#[test]
fn original_overflow_rounds_the_full_exact_operands_once() {
    let a = current("1234567890123456789")
        .div(current("1000000000000000000"))
        .unwrap();
    for result in [a.mul(a).unwrap(), a.powi(2).unwrap()] {
        assert_eq!(result.origin(), RealOrigin::Decimal);
        assert_eq!(result.to_string(), "1.52415787532");
    }
}
#[test]
fn original_square_roots_powers_integer_parts_and_rounding_keep_their_actual_types() {
    let third = current("1").div(current("3")).unwrap();
    rational(third.powi(2).unwrap(), 1, 9);
    rational(third.powi(-2).unwrap(), 9, 1);
    rational(third.round_places(2), 33, 100);
    rational(third.trunc(), 0, 1);
    rational(current("0.25").sqrt().unwrap(), 1, 2);
    rational(
        current("4").div(current("9")).unwrap().sqrt().unwrap(),
        2,
        3,
    );
    let root = current("2").sqrt().unwrap();
    assert_eq!(root.origin(), RealOrigin::Decimal);
    assert_eq!(root.trunc().origin(), RealOrigin::Decimal);
    assert_eq!(root.round_places(2).origin(), RealOrigin::Decimal);
}
#[test]
fn original_pi_coefficients_and_cancellation_are_exact() {
    let pi = Real::exact_pi_multiple(false, 1, 1).unwrap();
    assert_eq!(pi.to_string(), "3.14159265359");
    let half = pi.div(current("2")).unwrap();
    assert_eq!(
        half.origin(),
        RealOrigin::PiMultiple {
            numerator: 1,
            denominator: 2
        }
    );
    assert_eq!(half.to_string(), "1.57079632679");
    assert_eq!(
        pi.add(pi).unwrap().origin(),
        RealOrigin::PiMultiple {
            numerator: 2,
            denominator: 1
        }
    );
    rational(pi.div(pi).unwrap(), 1, 1);
    rational(pi.sub(pi).unwrap(), 0, 1);
    rational(pi.mul(current("0")).unwrap(), 0, 1);
    assert_eq!(pi.mul(pi).unwrap().origin(), RealOrigin::Decimal);
    assert_eq!(pi.add(current("1")).unwrap().origin(), RealOrigin::Decimal);
    rational(pi.trunc(), 3, 1);
    assert_eq!(pi.round_places(2).origin(), RealOrigin::Decimal);
    assert_eq!(pi.round_places(2).to_string(), "3.14");
}
#[test]
fn exact_comparisons_do_not_collapse_distinct_values_to_the_same_bcd_or_f64() {
    let a = current("1234567890123456789");
    let b = current("1234567890123456790");
    assert_eq!(a.to_string(), b.to_string());
    assert!(a < b);
    assert!(a.neg() > b.neg());
    let pi = Real::exact_pi_multiple(false, 1, 1).unwrap();
    let below = current("3141592653589793238")
        .div(current("1000000000000000000"))
        .unwrap();
    let above = current("3141592653589793239")
        .div(current("1000000000000000000"))
        .unwrap();
    assert_eq!(below.to_f64(), above.to_f64());
    assert!(below < pi && pi < above);
    assert!(above.neg() < pi.neg() && pi.neg() < below.neg());
}
#[test]
fn legacy_arithmetic_remains_bcd_and_is_selected_independently() {
    let legacy = |text| Real::parse_versioned(text, NumericVersion::Legacy15157).unwrap();
    assert_eq!(
        legacy("1")
            .div(legacy("3"))
            .unwrap()
            .mul(legacy("3"))
            .unwrap()
            .to_string(),
        "0.999999999999"
    );
    assert_eq!(
        legacy("1234567890123456789").to_i128(),
        Some(1_234_567_890_120_000_000)
    );
}
