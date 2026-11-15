use ppl_math::{MathError, OverflowPolicy, Real, with_overflow_policy};
fn huge() -> Real {
    Real::parse("1E400").unwrap()
}
#[test]
fn arithmetic_saturation_keeps_sign_and_does_not_change_strict_literal_validation() {
    assert_eq!(huge().mul(huge()), Err(MathError::Overflow));
    with_overflow_policy(OverflowPolicy::Saturate, || {
        assert_eq!(huge().mul(huge()).unwrap(), Real::max_finite(false));
        assert_eq!(huge().neg().mul(huge()).unwrap(), Real::max_finite(true));
        assert_eq!(huge().neg().powi(3).unwrap(), Real::max_finite(true));
        assert_eq!(
            Real::from_i64(1200).exp_e().unwrap(),
            Real::max_finite(false)
        );
        assert!(Real::parse("1E500").is_none());
        assert_eq!(Real::ZERO.div(Real::ZERO), Err(MathError::DivisionByZero));
    });
    assert_eq!(huge().mul(huge()), Err(MathError::Overflow));
}
#[test]
fn nested_threads_and_unwinding_restore_the_numeric_context() {
    with_overflow_policy(OverflowPolicy::Saturate, || {
        with_overflow_policy(OverflowPolicy::Error, || {
            assert_eq!(huge().mul(huge()), Err(MathError::Overflow))
        });
        assert!(huge().mul(huge()).is_ok());
        std::thread::spawn(|| assert_eq!(huge().mul(huge()), Err(MathError::Overflow)))
            .join()
            .unwrap();
    });
    let panic = std::panic::catch_unwind(|| {
        with_overflow_policy(OverflowPolicy::Saturate, || panic!("unwind numeric scope"))
    });
    assert!(panic.is_err());
    assert_eq!(huge().mul(huge()), Err(MathError::Overflow));
}
