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

#[test]
fn linked_debug_frames_and_globals_keep_original_source_revisions() {
    use std::{cell::RefCell, rc::Rc};
    struct Inspect(Rc<RefCell<usize>>);
    impl ppl_vm::Debugger for Inspect {
        fn on_statement(
            &mut self,
            m: &ppl_vm::Machine,
            function: &str,
            _: ppl_syntax::Span,
            _: usize,
        ) -> ppl_vm::DebugAction {
            if function == "B::LEAF" {
                let frames = m.call_stack_frames();
                assert_eq!(frames[0].module.as_ref().unwrap().name, "B");
                assert!(
                    frames[0]
                        .module
                        .as_ref()
                        .unwrap()
                        .revision
                        .contains("count:=10")
                );
                assert_eq!(frames[1].module.as_ref().unwrap().name, "A");
                let value = |depth| {
                    m.globals_for_frame(depth)
                        .into_iter()
                        .find(|(name, _)| name == "count")
                        .unwrap()
                        .1
                        .expect_i64("count")
                        .unwrap()
                };
                assert_eq!(value(0), 10);
                assert_eq!(value(1), 100);
                assert_eq!(m.locals(0)[0].0, "x");
                *self.0.borrow_mut() += 1;
            }
            ppl_vm::DebugAction::Continue
        }
    }
    let sources = modules(&[
        (
            "A",
            "LOCAL count:=100; EXPORT TOP() BEGIN RETURN LEAF(7)+count; END;",
        ),
        (
            "B",
            "LOCAL count:=10; EXPORT LEAF(x) BEGIN RETURN x+count; END;",
        ),
    ]);
    let calls = Rc::new(RefCell::new(0));
    let mut s = SessionContext::default();
    let result = s.run_linked(
        link_sources(&sources, "A").unwrap(),
        Box::new(HeadlessHost::new()),
        Limits::default(),
        false,
        |m| {
            m.set_debugger(Some(Box::new(Inspect(calls.clone()))));
            m.call_function("TOP", vec![])
        },
    );
    assert_eq!(n(result.result.unwrap()), 117);
    assert!(*calls.borrow() > 0);
}

#[test]
fn exported_variables_relocate_reads_writes_and_indexed_writes_to_owner() {
    let mut sources = modules(&[
        (
            "A",
            "EXPORT m18shared:=10,m18list:={1,2},m18matrix:=[[1,2],[3,4]]; LOCAL private:=99; EXPORT READBACK() BEGIN RETURN {m18shared,m18list,m18matrix,private}; END;",
        ),
        (
            "B",
            "EXPORT GETSHARED() BEGIN RETURN m18shared; END; EXPORT SETSHARED() BEGIN m18shared:=m18shared+1; m18list(2):=9; m18matrix(2,1):=8; RETURN m18shared; END;",
        ),
        (
            "C",
            "LOCAL m18shared:=200; EXPORT SHADOW() BEGIN m18shared:=m18shared+1; RETURN m18shared; END;",
        ),
    ]);
    let mut s = SessionContext::default();
    initialize(&mut s, &sources, "A");
    assert_eq!(
        n(call(&mut s, &sources, "B", "GETSHARED", vec![])
            .result
            .unwrap()),
        10
    );
    assert_eq!(
        n(call(&mut s, &sources, "B", "SETSHARED", vec![])
            .result
            .unwrap()),
        11
    );
    let Value::List(read) = call(&mut s, &sources, "A", "READBACK", vec![])
        .result
        .unwrap()
    else {
        panic!("list result");
    };
    assert_eq!(n(read[0].clone()), 11);
    let Value::List(list) = &read[1] else {
        panic!("exported list");
    };
    assert_eq!(n(list[1].clone()), 9);
    let Value::Matrix(matrix) = &read[2] else {
        panic!("exported matrix");
    };
    assert_eq!((matrix.rows, matrix.cols), (2, 2));
    assert_eq!(n(matrix.data[2].clone()), 8);
    assert_eq!(n(read[3].clone()), 99);
    assert_eq!(
        n(call(&mut s, &sources, "C", "SHADOW", vec![])
            .result
            .unwrap()),
        201
    );
    assert_eq!(
        n(call(&mut s, &sources, "B", "GETSHARED", vec![])
            .result
            .unwrap()),
        11
    );
    let replacement = sources["A"]
        .source
        .replace("m18shared:=10", "m18shared:=40");
    sources.insert(
        "A".into(),
        ModuleSource {
            identity: SourceIdentity::new("A", &replacement),
            source: replacement,
        },
    );
    initialize(&mut s, &sources, "A");
    assert_eq!(
        n(call(&mut s, &sources, "B", "GETSHARED", vec![])
            .result
            .unwrap()),
        40
    );
}

#[test]
fn external_variables_reject_private_ambiguous_and_unlinked_references() {
    let sources = modules(&[
        (
            "A",
            "LOCAL secret:=9; EXPORT READPRIVATE() BEGIN RETURN secret; END;",
        ),
        ("B", "EXPORT ROOT() BEGIN RETURN secret; END;"),
    ]);
    assert!(link_sources(&sources, "B").is_err());
    let sources = modules(&[
        ("A", "EXPORT shared:=1;"),
        ("B", "EXPORT shared:=2;"),
        ("C", "EXPORT ROOT() BEGIN RETURN shared; END;"),
    ]);
    assert!(
        link_sources(&sources, "C")
            .err()
            .unwrap()
            .to_string()
            .contains("ambiguous external variable")
    );
    for body in [
        "RETURN shared;",
        "shared:=2; RETURN 0;",
        "shared(1):=2; RETURN 0;",
    ] {
        let file = ppl_syntax::SourceFile::new("raw", format!("EXPORT ROOT() BEGIN {body} END;"));
        let (prog, diagnostics) = ppl_compiler::compile_source(
            &file,
            &ppl_compiler::CompileOptions {
                external_variables: std::collections::BTreeSet::from(["shared".into()]),
                ..Default::default()
            },
        );
        assert!(!diagnostics.has_errors());
        let mut machine = ppl_vm::Machine::new(
            std::rc::Rc::new(prog.unwrap()),
            Box::new(HeadlessHost::new()),
            Default::default(),
            Default::default(),
        );
        let error = machine.call_function("ROOT", vec![]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Undefined);
        assert!(
            error
                .to_string()
                .contains("requires a linked module catalog")
        );
    }
}

#[test]
fn ascii_imaginary_unit_is_a_fallback_after_ordinary_names() {
    for (source, expected) in [
        ("EXPORT ROOT() BEGIN RETURN i; END;", "i"),
        ("EXPORT ROOT() BEGIN LOCAL i:=7; RETURN i; END;", "7"),
        (
            "i() BEGIN RETURN 6; END; EXPORT ROOT() BEGIN RETURN i; END;",
            "6",
        ),
        (
            "i() BEGIN RETURN 6; END; EXPORT ROOT() BEGIN RETURN i(); END;",
            "6",
        ),
        ("EXPORT ROOT() BEGIN RETURN I; END;", "0"),
    ] {
        let sources = modules(&[("A", source)]);
        let mut s = SessionContext::default();
        assert_eq!(
            call(&mut s, &sources, "A", "ROOT", vec![])
                .result
                .unwrap()
                .display(s.settings.format()),
            expected
        );
    }
}

#[test]
fn fresh_sdk_external_variable_reference_initializes_owner_instead_of_returning_zero() {
    let sources = modules(&[
        ("A", "EXPORT shared:=10;"),
        ("B", "EXPORT ROOT() BEGIN RETURN shared; END;"),
    ]);
    let mut machine = ppl_vm::Machine::new_linked(
        link_sources(&sources, "B").unwrap(),
        Box::new(HeadlessHost::new()),
        Default::default(),
        Default::default(),
    );
    assert_eq!(n(machine.call_function("ROOT", vec![]).unwrap()), 10);
}

#[test]
fn cross_module_export_variable_initializer_cycles_are_explicitly_unsupported() {
    let sources = modules(&[
        ("A", "EXPORT first:=second+1;"),
        (
            "B",
            "EXPORT second:=first+1; EXPORT ROOT() BEGIN RETURN second; END;",
        ),
    ]);
    let mut machine = ppl_vm::Machine::new_linked(
        link_sources(&sources, "B").unwrap(),
        Box::new(HeadlessHost::new()),
        Default::default(),
        Default::default(),
    );
    let error = machine.call_function("ROOT", vec![]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(error.to_string().contains("initializer cycle"));
}
