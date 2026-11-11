#![cfg(feature = "giac-integration")]
use ppl_cas::{CasError, CasSession, CasValue, Options};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn run(
    session: &mut CasSession,
    expression: &str,
    home: &BTreeMap<String, CasValue>,
) -> ppl_cas::CasEvaluation {
    session
        .evaluate_with_home(
            expression,
            home,
            &Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap()
}
fn integer(n: i64) -> CasValue {
    CasValue::Integer(n.to_string())
}
fn clean(session: &CasSession) {
    assert!(
        session
            .bindings
            .keys()
            .all(|name| !name.starts_with("pplHomeBridge") && name != "Q")
    );
    assert!(
        session
            .bindings
            .values()
            .all(|value| !value.contains("pplHomeBridge"))
    );
}

#[test]
fn official_guide_q_and_uppercase_q_are_distinct_environments() {
    let mut session = CasSession::default();
    let home = BTreeMap::from([("Q".into(), integer(100))]);
    assert_eq!(run(&mut session, "q:=1000", &home).value, integer(1000));
    assert_eq!(run(&mut session, "5*q", &home).value, integer(5000));
    let read = run(&mut session, "5*Q", &home);
    assert_eq!(read.value, integer(500));
    assert!(read.home_updates.is_empty());
    assert_eq!(session.bindings["q"], "1000");
    clean(&session);
}

#[test]
fn home_writes_return_typed_changes_without_polluting_cas_bindings() {
    let mut session = CasSession::default();
    let mut home = BTreeMap::from([("Q".into(), integer(100))]);
    run(&mut session, "q:=1000", &home);
    let changed = run(&mut session, "Q:=Q+3", &home);
    assert_eq!(changed.value, integer(103));
    assert_eq!(
        changed.home_updates,
        BTreeMap::from([("Q".into(), integer(103))])
    );
    home.extend(changed.home_updates);
    assert_eq!(run(&mut session, "Q+q", &home).value, integer(1103));
    assert!(run(&mut session, "Q", &home).home_updates.is_empty());
    clean(&session);
}

#[test]
fn presets_and_reserved_giac_identifiers_use_actual_home_values() {
    let mut session = CasSession::default();
    let home = BTreeMap::from([
        ("I".into(), integer(9)),
        ("D".into(), integer(7)),
        ("θ".into(), integer(3)),
    ]);
    assert_eq!(run(&mut session, "I+D+θ+Q", &home).value, integer(19));
    assert_eq!(
        run(&mut session, "M0", &home).value,
        CasValue::Matrix {
            rows: 1,
            cols: 1,
            values: vec![integer(0)]
        }
    );
    assert_eq!(run(&mut session, "L0", &home).value, CasValue::List(vec![]));
    clean(&session);
}

#[test]
fn strings_identifier_boundaries_and_cas_answer_are_protected() {
    let mut session = CasSession::default();
    let home = BTreeMap::from([("Q".into(), integer(100)), ("Ans".into(), integer(999))]);
    assert_eq!(
        run(&mut session, "\"Q q Ans I /*Q*/\"", &home).value,
        CasValue::String("Q q Ans I /*Q*/".into())
    );
    run(&mut session, "QQ:=7", &home);
    assert_eq!(session.bindings["QQ"], "7");
    assert_eq!(run(&mut session, "2+2", &home).value, integer(4));
    assert_eq!(run(&mut session, "Ans*3", &home).value, integer(12));
    assert_eq!(
        run(&mut session, "'Q'", &home).value,
        CasValue::Symbolic("Q".into())
    );
    clean(&session);
}

#[test]
fn symbolic_home_references_survive_binding_serialization_without_private_names() {
    let mut session = CasSession::default();
    let mut home = BTreeMap::from([("Q".into(), integer(100))]);
    run(&mut session, "q:=1000", &home);
    run(&mut session, "held:='Q+q'", &home);
    clean(&session);
    let image = serde_json::to_string(&session).unwrap();
    assert!(!image.contains("pplHomeBridge"));
    let mut restored: CasSession = serde_json::from_str(&image).unwrap();
    home.insert("Q".into(), integer(200));
    assert_eq!(run(&mut restored, "held", &home).value, integer(1200));
    clean(&restored);
}

#[test]
fn failures_and_cancellation_do_not_commit_either_environment() {
    let mut session = CasSession::default();
    let home = BTreeMap::from([("Q".into(), integer(100))]);
    run(&mut session, "q:=7", &home);
    let before = serde_json::to_value(&session).unwrap();
    assert!(matches!(
        session.evaluate_with_home(
            "Q:=1;error(\"deliberate\")",
            &home,
            &Options::default(),
            &AtomicBool::new(false)
        ),
        Err(CasError::Evaluation(_))
    ));
    assert_eq!(serde_json::to_value(&session).unwrap(), before);
    assert_eq!(
        session.evaluate_with_home("Q:=2", &home, &Options::default(), &AtomicBool::new(true)),
        Err(CasError::Cancelled)
    );
    assert_eq!(serde_json::to_value(&session).unwrap(), before);
}

#[test]
fn matrices_and_fraction_writes_keep_typed_updates() {
    let mut session = CasSession::default();
    let home = BTreeMap::new();
    let fraction = run(&mut session, "Q:=1/3", &home);
    assert_eq!(
        fraction.home_updates["Q"],
        CasValue::Rational {
            numerator: "1".into(),
            denominator: "3".into()
        }
    );
    let matrix = run(&mut session, "M0:=[[1,2],[3,4]]", &home);
    assert_eq!(
        matrix.home_updates["M0"],
        CasValue::Matrix {
            rows: 2,
            cols: 2,
            values: vec![integer(1), integer(2), integer(3), integer(4)]
        }
    );
    clean(&session);
}

#[test]
fn function_parameter_shadows_home_name_without_mutating_it() {
    let mut session = CasSession::default();
    let home = BTreeMap::from([("Q".into(), integer(100))]);
    assert!(
        run(&mut session, "f(Q):=Q+1", &home)
            .home_updates
            .is_empty()
    );
    assert_eq!(run(&mut session, "f(2)", &home).value, integer(3));
    assert!(run(&mut session, "Q", &home).home_updates.is_empty());
    clean(&session);
}

#[test]
fn scientific_exponent_markers_are_not_home_variable_identifiers() {
    let mut session = CasSession::default();
    let home = BTreeMap::from([
        ("E".into(), integer(7)),
        ("Q".into(), CasValue::Decimal("1E-3".into())),
    ]);
    for (expression, number) in [
        ("1E+3", 1000.),
        ("1E-3", 0.001),
        ("2.5E-2", 0.025),
        ("Q", 0.001),
    ] {
        let value = run(&mut session, expression, &home).value;
        let CasValue::Decimal(text) = value else {
            panic!("scientific literal was not numeric: {value:?}");
        };
        assert_eq!(text.parse::<f64>().unwrap(), number);
    }
    assert_eq!(run(&mut session, "E+1", &home).value, integer(8));
    clean(&session);
}
