use ppl_cas::{CasSession, CasValue};

#[test]
fn malformed_profile_values_are_rejected_and_safe_to_display() {
    for value in [
        CasValue::Matrix {
            rows: 2,
            cols: 2,
            values: vec![],
        },
        CasValue::Matrix {
            rows: usize::MAX,
            cols: 2,
            values: vec![],
        },
        CasValue::Integer("x+1".into()),
        CasValue::Integer("-".into()),
        CasValue::Decimal("NaN".into()),
        CasValue::Decimal("1e+".into()),
        CasValue::Rational {
            numerator: "1".into(),
            denominator: "-000".into(),
        },
        CasValue::Rational {
            numerator: "x".into(),
            denominator: "2".into(),
        },
        CasValue::Symbolic("".into()),
    ] {
        let restored: CasValue =
            serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
        assert!(restored.validate().is_err());
        assert!(restored.try_expression().is_err());
        assert_eq!(restored.to_string(), "<invalid CAS value>");
        assert_eq!(restored.expression(), "<invalid CAS value>");
    }
}

#[test]
fn validated_exact_and_decimal_values_keep_their_expression() {
    for value in [
        CasValue::Integer("1267650600228229401496703205376".into()),
        CasValue::Decimal("-.1234567890123456789e-499".into()),
        CasValue::Rational {
            numerator: "-11".into(),
            denominator: "3".into(),
        },
        CasValue::Matrix {
            rows: 1,
            cols: 2,
            values: vec![CasValue::Integer("1".into()), CasValue::Integer("2".into())],
        },
    ] {
        value.validate().unwrap();
        assert_ne!(value.expression(), "<invalid CAS value>");
        assert_eq!(value.try_expression().unwrap(), value.expression());
    }
}

#[test]
fn profile_session_validation_covers_settings_bindings_and_last_result() {
    let session = CasSession::default();
    session.validate().unwrap();
    let mut invalid = session.clone();
    invalid.settings.digits = 0;
    assert!(invalid.validate().is_err());
    let mut invalid = session.clone();
    invalid.settings.digits = 101;
    assert!(invalid.validate().is_err());
    let mut invalid = session.clone();
    invalid.bindings.insert("a;quit()".into(), "1".into());
    assert!(invalid.validate().is_err());
    let mut invalid = session.clone();
    invalid.bindings.insert("a".into(), "\0".into());
    assert!(invalid.validate().is_err());
    let mut invalid = session.clone();
    invalid.last_value = Some(CasValue::Matrix {
        rows: 2,
        cols: 2,
        values: vec![],
    });
    assert!(invalid.validate().is_err());
    let mut invalid = session;
    invalid.helper_version = Some("unrelated".into());
    assert!(invalid.validate().is_err());
}

#[test]
fn validation_bounds_nesting_and_payload_size_before_rendering() {
    let mut nested = CasValue::Integer("1".into());
    for _ in 0..66 {
        nested = CasValue::List(vec![nested]);
    }
    assert!(nested.validate().is_err());
    assert_eq!(nested.to_string(), "<invalid CAS value>");
    let oversized = CasValue::String("a".repeat(4 * 1024 * 1024 + 1));
    assert!(oversized.validate().is_err());
    let mut session = CasSession::default();
    session
        .bindings
        .insert("a".into(), "a".repeat(4 * 1024 * 1024));
    assert!(session.validate().is_err());
}
