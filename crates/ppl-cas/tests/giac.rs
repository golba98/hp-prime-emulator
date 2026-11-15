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
fn measurements_have_a_structural_type_and_aliases_are_rewritten() {
    use std::collections::BTreeMap;
    let mut session = CasSession::default();
    assert_eq!(evaluate(&mut session, "1_m"), CasValue::Unit("1_m".into()));
    assert!(matches!(
        evaluate(&mut session, "sqrt(2)"),
        CasValue::Symbolic(_)
    ));
    let result = session
        .evaluate_with_home(
            "A*1_m",
            &BTreeMap::from([("A".into(), CasValue::Integer("3".into()))]),
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(result.value, CasValue::Unit("3_m".into()));
    let encoded = serde_json::to_string(&result.value).unwrap();
    let restored: CasValue = serde_json::from_str(&encoded).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, result.value);
    assert!(!encoded.contains("pplHomeBridge"));
}

#[test]
fn actual_helper_preserves_legacy_reply_shapes_and_negotiates_typed_metadata() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for (version, source, tag) in [
        (1, "1_m", "symbolic"),
        (2, "1_m", "symbolic"),
        (3, "1_m", "unit"),
        (1, "[1,2]", "list"),
        (2, "[1,2]", "list"),
        (3, "[1,2]", "vector"),
    ] {
        let mut request = format!("PPLCAS0{version}").into_bytes();
        request.extend(42u64.to_le_bytes());
        request.extend([0, 0, 12]);
        if version >= 3 {
            request.push(0);
        }
        request.extend((source.len() as u32).to_le_bytes());
        request.extend(source.as_bytes());
        request.extend(0u32.to_le_bytes());
        if version >= 2 {
            request.extend(0u32.to_le_bytes());
        }
        let mut child = Command::new(Options::default().helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&request).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["version"], version);
        assert_eq!(reply["request"], 42);
        assert_eq!(reply["error"], serde_json::Value::Null);
        assert_eq!(reply["result"]["type"], tag);
        assert_eq!(reply.get("home_updates").is_some(), version >= 2);
    }
}

#[test]
fn parse_only_inspects_root_types_without_evaluating_saved_source() {
    let options = Options::default();
    let cancel = AtomicBool::new(false);
    for source in ["2_m", "-2_m", "(1+i)_m", "2_(m^2)"] {
        assert!(
            matches!(
                CasSession::parse_value(source, &options, &cancel).unwrap(),
                CasValue::Unit(_)
            ),
            "{source}"
        );
    }
    assert_eq!(
        CasSession::parse_value("quantity_m", &options, &cancel).unwrap(),
        CasValue::Symbolic("quantity_m".into())
    );
    assert!(matches!(
        CasSession::parse_value("sin(0)", &options, &cancel).unwrap(),
        CasValue::Symbolic(_)
    ));
    assert!(matches!(
        CasSession::parse_value("side_m:=7", &options, &cancel).unwrap(),
        CasValue::Symbolic(_)
    ));
    assert!(CasSession::parse_value("2_m); side_m:=7; (", &options, &cancel).is_err());
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

#[test]
fn undefined_and_infinity_are_typed_values_while_syntax_errors_do_not_commit() {
    use ppl_cas::InfinitySign;
    let mut session = CasSession::default();
    assert_eq!(evaluate(&mut session, "0/0"), CasValue::Undefined);
    assert_eq!(
        evaluate(&mut session, "1/0"),
        CasValue::Infinity(InfinitySign::Unsigned)
    );
    assert_eq!(
        evaluate(&mut session, "+infinity"),
        CasValue::Infinity(InfinitySign::Positive)
    );
    assert_eq!(
        evaluate(&mut session, "-infinity"),
        CasValue::Infinity(InfinitySign::Negative)
    );
    assert_eq!(
        evaluate(&mut session, "special:=undef"),
        CasValue::Undefined
    );
    let image = serde_json::to_string(&session).unwrap();
    let mut restored: CasSession = serde_json::from_str(&image).unwrap();
    assert_eq!(evaluate(&mut restored, "special"), CasValue::Undefined);
    let before = serde_json::to_value(&restored).unwrap();
    assert!(matches!(
        restored.evaluate(
            "special:=7;1+",
            &Options::default(),
            &AtomicBool::new(false)
        ),
        Err(CasError::Evaluation(_))
    ));
    assert_eq!(serde_json::to_value(&restored).unwrap(), before);
    assert_eq!(
        evaluate(&mut restored, "[special,-infinity]"),
        CasValue::Vector(vec![
            CasValue::Undefined,
            CasValue::Infinity(InfinitySign::Negative)
        ])
    );
    for value in [
        CasValue::Undefined,
        CasValue::Infinity(InfinitySign::Unsigned),
    ] {
        value.validate().unwrap();
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<CasValue>(&encoded).unwrap(), value);
    }
}

#[test]
fn home_projection_is_separate_from_exact_state_and_parse_only_migration() {
    let cancel = AtomicBool::new(false);
    let options = Options::default();
    let mut session = CasSession::default();
    let exact = evaluate(&mut session, "[1/3,sqrt(2)]");
    let saved = serde_json::to_value(&session).unwrap();
    let projected =
        CasSession::project_home_container(&exact, session.settings.clone(), &options, &cancel)
            .unwrap();
    let CasValue::Vector(values) = projected else {
        panic!("vector type lost")
    };
    assert!(matches!(&values[0], CasValue::Decimal(s) if s == "0.333333333333"));
    assert!(matches!(&values[1], CasValue::Decimal(s) if s == "1.41421356237"));
    assert_eq!(serde_json::to_value(&session).unwrap(), saved);
    assert!(matches!(
        CasSession::parse_value("sin(0)", &options, &cancel).unwrap(),
        CasValue::Symbolic(_)
    ));
    let data = CasValue::Vector(vec![
        CasValue::Symbolic("unbound_projection/3".into()),
        CasValue::String("sqrt(2)".into()),
        CasValue::Symbolic("projection_side_m:=7".into()),
    ]);
    let projected =
        CasSession::project_home_container(&data, session.settings.clone(), &options, &cancel)
            .unwrap();
    let CasValue::Vector(values) = projected else {
        panic!("vector type lost")
    };
    assert!(matches!(&values[0], CasValue::Symbolic(s) if s.contains("unbound_projection")));
    assert_eq!(values[1], CasValue::String("sqrt(2)".into()));
    assert!(matches!(&values[2], CasValue::Symbolic(s) if s.contains(":=")));
    assert_eq!(serde_json::to_value(&session).unwrap(), saved);
    assert!(
        CasSession::project_home_container(
            &CasValue::Integer("2".into()),
            session.settings.clone(),
            &options,
            &cancel
        )
        .is_err()
    );
    let cancelled = AtomicBool::new(true);
    assert!(matches!(
        CasSession::project_home_container(&exact, session.settings.clone(), &options, &cancelled),
        Err(CasError::Cancelled)
    ));
}
