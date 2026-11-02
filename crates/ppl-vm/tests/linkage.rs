use ppl_vm::{
    ErrorKind, HeadlessHost, Limits, ModuleSource, SessionContext, SourceIdentity, Value,
    link_sources,
};
use std::collections::BTreeMap;
fn modules(items: &[(&str, &str)]) -> BTreeMap<String, ModuleSource> {
    items
        .iter()
        .map(|(name, source)| {
            (
                (*name).into(),
                ModuleSource {
                    identity: SourceIdentity::new(*name, *source),
                    source: (*source).into(),
                },
            )
        })
        .collect()
}
fn call(
    s: &mut SessionContext,
    sources: &BTreeMap<String, ModuleSource>,
    root: &str,
    entry: &str,
    args: Vec<Value>,
) -> ppl_vm::SessionRun {
    s.run_linked(
        link_sources(sources, root).unwrap(),
        Box::new(HeadlessHost::new()),
        Limits::default(),
        false,
        |m| m.call_function(entry, args),
    )
}
fn initialize(s: &mut SessionContext, sources: &BTreeMap<String, ModuleSource>, root: &str) {
    s.run_linked(
        link_sources(sources, root).unwrap(),
        Box::new(HeadlessHost::new()),
        Limits::default(),
        false,
        |m| {
            m.run_init()?;
            Ok(Value::zero())
        },
    )
    .result
    .unwrap();
}
fn n(v: Value) -> i64 {
    v.expect_i64("test").unwrap()
}
#[test]
fn three_program_calls_share_globals_and_isolate_private_values() {
    let sources = modules(&[
        (
            "A",
            "LOCAL count:=20; EXPORT TOP(x) BEGIN count:=count+1; RETURN MIDDLE(x)+count; END;",
        ),
        (
            "B",
            "LOCAL count:=10; EXPORT LEAF(x) BEGIN count:=count+1; A:=A+x; RETURN count; END;",
        ),
        ("C", "EXPORT MIDDLE(x) BEGIN RETURN LEAF(x); END;"),
    ]);
    let mut s = SessionContext::default();
    assert_eq!(
        n(call(&mut s, &sources, "A", "TOP", vec![Value::from_i64(3)])
            .result
            .unwrap()),
        32
    );
    assert_eq!(
        n(call(&mut s, &sources, "A", "TOP", vec![Value::from_i64(5)])
            .result
            .unwrap()),
        34
    );
    assert_eq!(
        s.snapshot().globals["A"]
            .to_value()
            .expect_i64("A")
            .unwrap(),
        8
    );
}
#[test]
fn cross_module_recursion_uses_one_depth_budget() {
    let sources = modules(&[
        (
            "A",
            "EXPORT EVEN(n) BEGIN IF n==0 THEN RETURN 1; END; RETURN ODD(n-1); END;",
        ),
        (
            "B",
            "EXPORT ODD(n) BEGIN IF n==0 THEN RETURN 0; END; RETURN EVEN(n-1); END;",
        ),
    ]);
    let mut s = SessionContext::default();
    assert_eq!(
        n(
            call(&mut s, &sources, "A", "EVEN", vec![Value::from_i64(8)])
                .result
                .unwrap()
        ),
        1
    );
    let failed = s.run_linked(
        link_sources(&sources, "A").unwrap(),
        Box::new(HeadlessHost::new()),
        Limits {
            max_depth: 5,
            ..Default::default()
        },
        false,
        |m| m.call_function("EVEN", vec![Value::from_i64(20)]),
    );
    assert!(matches!(
        failed.result.unwrap_err().kind,
        ErrorKind::ResourceLimit(_)
    ));
}
#[test]
fn edited_dependency_resets_only_its_own_private_state_and_relinks_arity() {
    let mut sources = modules(&[
        (
            "A",
            "LOCAL count:=100; EXPORT TOP() BEGIN count:=count+1; RETURN count+LEAF(); END;",
        ),
        (
            "B",
            "LOCAL count:=10; EXPORT LEAF() BEGIN count:=count+1; RETURN count; END;",
        ),
    ]);
    let mut s = SessionContext::default();
    assert_eq!(
        n(call(&mut s, &sources, "A", "TOP", vec![]).result.unwrap()),
        112
    );
    sources.insert(
        "B".into(),
        modules(&[(
            "B",
            "LOCAL count:=40; EXPORT LEAF() BEGIN count:=count+1; RETURN count; END;",
        )])
        .remove("B")
        .unwrap(),
    );
    initialize(&mut s, &sources, "B");
    assert_eq!(
        n(call(&mut s, &sources, "A", "TOP", vec![]).result.unwrap()),
        143
    );
    sources.insert(
        "B".into(),
        modules(&[("B", "EXPORT LEAF(x) BEGIN RETURN x; END;")])
            .remove("B")
            .unwrap(),
    );
    assert!(
        link_sources(&sources, "A")
            .err()
            .unwrap()
            .to_string()
            .contains("expects 1")
    );
}
#[test]
fn firmware_observed_install_initialization_is_not_repeated_on_calls() {
    let sources = modules(&[(
        "B",
        "MAKE() BEGIN A:=A+1; RETURN 10; END; LOCAL count:=MAKE(); EXPORT NEXT() BEGIN count:=count+1; RETURN count; END;",
    )]);
    let mut s = SessionContext::default();
    initialize(&mut s, &sources, "B");
    assert_eq!(
        s.snapshot().globals["A"]
            .to_value()
            .expect_i64("A")
            .unwrap(),
        1
    );
    assert_eq!(
        n(call(&mut s, &sources, "B", "NEXT", vec![]).result.unwrap()),
        11
    );
    assert_eq!(
        n(call(&mut s, &sources, "B", "NEXT", vec![]).result.unwrap()),
        12
    );
    assert_eq!(
        s.snapshot().globals["A"]
            .to_value()
            .expect_i64("A")
            .unwrap(),
        1
    );
}
#[test]
fn missing_private_and_ambiguous_exports_fail_precisely() {
    for items in [
        vec![("A", "EXPORT TOP() BEGIN RETURN MISSING(); END;")],
        vec![
            ("A", "EXPORT TOP() BEGIN RETURN PRIVATE(); END;"),
            ("B", "PRIVATE() BEGIN RETURN 1; END;"),
        ],
        vec![
            ("C", "EXPORT TOP() BEGIN RETURN SAME(); END;"),
            ("A", "EXPORT SAME() BEGIN RETURN 1; END;"),
            ("B", "EXPORT SAME() BEGIN RETURN 2; END;"),
        ],
    ] {
        let root = if items.iter().any(|(name, _)| *name == "C") {
            "C"
        } else {
            "A"
        };
        assert!(link_sources(&modules(&items), root).is_err());
    }
}
#[test]
fn cross_program_error_traces_retain_real_module_and_caller_catches() {
    let sources = modules(&[
        (
            "A",
            "EXPORT TOP() BEGIN RETURN FAIL(); END; EXPORT CATCH() BEGIN IFERR FAIL(); THEN RETURN A+1; END; RETURN 0; END;",
        ),
        ("B", "EXPORT FAIL() BEGIN A:=41; RETURN 1/0; END;"),
    ]);
    let mut s = SessionContext::default();
    let e = call(&mut s, &sources, "A", "TOP", vec![])
        .result
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::DivisionByZero);
    assert_eq!(e.trace[0].module.as_ref().unwrap().name, "B");
    assert_eq!(e.trace[1].module.as_ref().unwrap().name, "A");
    assert_eq!(
        n(call(&mut s, &sources, "A", "CATCH", vec![]).result.unwrap()),
        42
    );
}
#[test]
fn source_mutation_of_an_executable_module_is_explicitly_unsupported() {
    let sources = modules(&[(
        "A",
        "EXPORT TOP() BEGIN Programs(\"A\"):=\"replacement\"; RETURN 1; END;",
    )]);
    let mut s = SessionContext::default();
    assert_eq!(
        call(&mut s, &sources, "A", "TOP", vec![])
            .result
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn unrelated_broken_modules_do_not_block_valid_closure() {
    let sources = modules(&[
        ("A", "EXPORT TOP() BEGIN RETURN ABS(-42); END;"),
        ("RESERVED", "EXPORT ABS(x) BEGIN RETURN x; END;"),
        ("BROKEN", "EXPORT BAD() BEGIN RETURN 1+; END;"),
        ("OTHER", "EXPORT SIDE() BEGIN RETURN UNKNOWN(); END;"),
    ]);
    let mut s = SessionContext::default();
    assert_eq!(
        n(call(&mut s, &sources, "A", "TOP", vec![]).result.unwrap()),
        42
    );
}

#[test]
fn retained_session_remains_send_without_transferring_rc_execution_images() {
    fn send<T: Send>() {}
    send::<SessionContext>();
    send::<ModuleSource>();
}

#[test]
fn ambiguous_external_is_reported_before_candidate_argument_counts() {
    let sources = modules(&[
        ("A", "EXPORT TOP() BEGIN RETURN SAME(1); END;"),
        ("B", "EXPORT SAME() BEGIN RETURN 2; END;"),
        ("C", "EXPORT SAME(x,y) BEGIN RETURN x+y; END;"),
    ]);
    let error = link_sources(&sources, "A").err().unwrap();
    assert!(error.detail.contains("ambiguous external"), "{error:?}");
}
