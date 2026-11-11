#![cfg(feature = "giac-integration")]
use ppl_cas::{AngleMode, CasError, CasSession, CasValue, Options};
use std::sync::atomic::AtomicBool;
fn evaluate(session: &mut CasSession, expression: &str) -> CasValue {
    session
        .evaluate(expression, &Options::default(), &AtomicBool::new(false))
        .unwrap()
}
#[test]
fn exact_arithmetic_and_arbitrary_precision_remain_exact() {
    let mut s = CasSession::default();
    assert_eq!(
        evaluate(&mut s, "1/3"),
        CasValue::Rational {
            numerator: "1".into(),
            denominator: "3".into()
        }
    );
    assert_eq!(
        evaluate(&mut s, "2^100"),
        CasValue::Integer("1267650600228229401496703205376".into())
    );
}
#[test]
fn symbolic_algebra_calculus_and_solving_are_real_giac_operations() {
    let mut s = CasSession::default();
    assert_eq!(evaluate(&mut s, "factor(x^2-1)").to_string(), "(x-1)*(x+1)");
    assert_eq!(evaluate(&mut s, "diff(sin(x),x)").to_string(), "cos(x)");
    assert_eq!(evaluate(&mut s, "int(x,x)").to_string(), "x^2/2");
    assert_eq!(
        evaluate(&mut s, "solve(x^2=4,x)"),
        CasValue::List(vec![
            CasValue::Integer("-2".into()),
            CasValue::Integer("2".into())
        ])
    );
}
#[test]
fn complex_lists_matrices_and_strings_are_typed() {
    let mut s = CasSession::default();
    assert_eq!(
        evaluate(&mut s, "sqrt(-1)"),
        CasValue::Complex {
            real: Box::new(CasValue::Integer("0".into())),
            imaginary: Box::new(CasValue::Integer("1".into()))
        }
    );
    assert!(matches!(
        evaluate(&mut s, "[[1,2],[3,4]]"),
        CasValue::Matrix {
            rows: 2,
            cols: 2,
            ..
        }
    ));
    assert_eq!(
        evaluate(&mut s, "\"hello\""),
        CasValue::String("hello".into())
    );
}
#[test]
fn bindings_survive_processes_and_serialization_but_errors_do_not_commit() {
    let mut s = CasSession::default();
    evaluate(&mut s, "a:=7");
    let encoded = serde_json::to_string(&s).unwrap();
    let mut s: CasSession = serde_json::from_str(&encoded).unwrap();
    assert_eq!(evaluate(&mut s, "a+5"), CasValue::Integer("12".into()));
    assert!(matches!(
        s.evaluate("1+", &Options::default(), &AtomicBool::new(false)),
        Err(CasError::Evaluation(_))
    ));
    assert_eq!(s.bindings["a"], "7");
    evaluate(&mut s, "purge(a)");
    assert!(!s.bindings.contains_key("a"));
}
#[test]
fn independent_contexts_and_angle_settings_do_not_leak() {
    let mut a = CasSession::default();
    let mut b = CasSession::default();
    evaluate(&mut a, "k:=5");
    assert_eq!(evaluate(&mut b, "k"), CasValue::Symbolic("k".into()));
    b.settings.angle = AngleMode::Degrees;
    assert_eq!(
        evaluate(&mut b, "sin(30)"),
        CasValue::Rational {
            numerator: "1".into(),
            denominator: "2".into()
        }
    );
    assert_ne!(
        evaluate(&mut a, "sin(30)"),
        CasValue::Rational {
            numerator: "1".into(),
            denominator: "2".into()
        }
    );
}
#[test]
fn approximate_settings_and_syntax_errors_are_not_fabricated_successes() {
    let mut s = CasSession::default();
    s.settings.approximate = true;
    assert!(matches!(evaluate(&mut s, "1/3"), CasValue::Decimal(_)));
    assert!(matches!(
        s.evaluate("1+", &Options::default(), &AtomicBool::new(false)),
        Err(CasError::Evaluation(_))
    ));
}

#[test]
fn chained_bindings_follow_giac_default_evaluation_depth() {
    let mut session = CasSession::default();
    for expression in ["a:=b", "b:=c", "c:=2"] {
        evaluate(&mut session, expression);
    }
    assert_eq!(evaluate(&mut session, "a+1"), CasValue::Integer("3".into()));
}

#[test]
fn restoration_preserves_symbolic_references_independent_of_binding_sort_order() {
    let mut session = CasSession::default();
    evaluate(&mut session, "z:=a+1");
    evaluate(&mut session, "a:=2");
    assert_eq!(evaluate(&mut session, "z"), CasValue::Integer("3".into()));
    let image = serde_json::to_string(&session).unwrap();
    let mut session: CasSession = serde_json::from_str(&image).unwrap();
    evaluate(&mut session, "a:=4");
    assert_eq!(evaluate(&mut session, "z"), CasValue::Integer("5".into()));
    assert_eq!(session.bindings["z"], "a+1");
}
