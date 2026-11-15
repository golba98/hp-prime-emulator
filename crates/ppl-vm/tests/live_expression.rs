//! Expressions execute in the suspended PPL frame, rather than a second VM.
mod common;
use common::{Run, text};
use ppl_vm::{ErrorKind, ResourceKind};

fn run(body: &str) -> Run {
    Run::new("live.hpprgm", &format!("EXPORT MAIN() BEGIN {body} END;"))
}

#[test]
fn firmware_reference_global_local_and_assignment_controls() {
    for (body, expected) in [
        ("A:=10; RETURN {EXPR(\"A+2\"),A};", "{12,10}"),
        ("LOCAL n:=7; RETURN STRING(EXPR(\"n+1\"));", "8"),
        ("A:=10; LOCAL v:=EXPR(\"A:=5\"); RETURN {v,A};", "{5,5}"),
        ("A:=10; RETURN TYPE(EXPR(\"A\"));", "0"),
    ] {
        assert_eq!(
            text(&run(body).call("MAIN", &[]).unwrap()),
            expected,
            "{body}"
        );
    }
}

#[test]
fn local_assignments_indexed_writeback_and_nested_evaluation_share_storage() {
    let mut runner = run(r#"
        A:=11;
        LOCAL A:=7, items:={1,2}, copy:=items;
        LOCAL first:=EXPR("A:=A+3");
        LOCAL second:=EXPR("items(2):=A");
        LOCAL nested:=EXPR("EXPR(\"A:=A+2\")");
        RETURN {first,second,nested,A,items,copy};
    "#);
    assert_eq!(
        text(&runner.call("MAIN", &[]).unwrap()),
        "{10,10,12,12,{1,10},{1,2}}"
    );
    assert_eq!(text(runner.machine.calculator_global("A").unwrap()), "11");
}

#[test]
fn functions_called_from_expression_keep_the_host_and_file_state() {
    let source = r#"
        LOCAL counter:=3;
        bump(x) BEGIN counter:=counter+x; PRINT(counter); RETURN counter; END;
        EXPORT MAIN() BEGIN LOCAL n:=4; RETURN {EXPR("bump(n)"),counter}; END;
    "#;
    let mut runner = Run::new("live.hpprgm", source);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{7,7}");
    assert_eq!(runner.output(), ["7"]);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{11,11}");
}

#[test]
fn expression_errors_unwind_only_the_nested_frame() {
    let mut runner = run(r#"
        LOCAL n:=17;
        IFERR EXPR("n:=1/0"); THEN n:=n+2; END;
        IFERR EXPR("not a valid expression"); THEN n:=n+3; END;
        RETURN {n,EXPR("n*2")};
    "#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{22,44}");
}

#[test]
fn input_field_expressions_see_the_actual_waiting_frame() {
    let mut runner = run(r#"
        A:=10; LOCAL n:=7, answer:=0, items:={2,4};
        LOCAL ok:=INPUT(answer);
        RETURN {ok,answer,n,A};
    "#);
    assert_eq!(
        text(&runner.call("MAIN", &["A+n+items(2)"]).unwrap()),
        "{1,21,7,10}"
    );
}

#[test]
fn evaluation_obeys_the_parent_budget_and_can_be_interrupted() {
    let source = r#"
        forever() BEGIN WHILE 1 DO END; RETURN 0; END;
        EXPORT MAIN() BEGIN RETURN EXPR("forever()"); END;
    "#;
    let mut runner = Run::new("live.hpprgm", source);
    runner.machine.limits.max_steps = Some(2000);
    assert_eq!(
        runner.call("MAIN", &[]).unwrap_err().kind,
        ErrorKind::ResourceLimit(ResourceKind::Steps)
    );
    assert!(runner.machine.call_stack().is_empty());
    // Standalone evaluation uses the same spent budget, rather than resetting it.
    assert!(runner.machine.eval_text("2+2").is_err());
}

#[test]
fn input_editors_do_not_swallow_expression_execution_limits() {
    for editor in ["INPUT(value)", "EDITLIST(value)", "EDITMAT(value)"] {
        let initial = if editor.starts_with("INPUT") {
            "0"
        } else if editor.starts_with("EDITLIST") {
            "{0}"
        } else {
            "[[0]]"
        };
        let source = format!(
            "forever() BEGIN WHILE 1 DO END; END; EXPORT MAIN() BEGIN LOCAL value:={initial}; {editor}; RETURN value; END;"
        );
        let mut runner = Run::new("live.hpprgm", &source);
        runner.machine.limits.max_steps = Some(2000);
        let error = runner.call("MAIN", &["forever()"]).unwrap_err();
        assert_eq!(
            error.kind,
            ErrorKind::ResourceLimit(ResourceKind::Steps),
            "{editor}: {error}"
        );
    }
}

#[test]
fn called_function_does_not_inherit_a_callers_captured_locals() {
    let mut runner = Run::new(
        "live.hpprgm",
        r#"
        helper() BEGIN RETURN EXPR("A"); END;
        EXPORT MAIN() BEGIN A:=19; LOCAL A:=7; RETURN {EXPR("helper()"),A}; END;
    "#,
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{19,7}");
}

#[test]
fn expired_block_locals_are_not_visible_and_shadowed_values_are_restored() {
    let mut runner = run(
        r#"LOCAL n:=7; IF 1 THEN LOCAL other:=4; END; IFERR RETURN EXPR("other"); THEN RETURN -99; END;"#,
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "-99");
    let mut runner = run(
        r#"LOCAL n:=7,values:={}; IF 1 THEN LOCAL n:=4; values:={EXPR("n")}; END; RETURN CONCAT(values,{EXPR("n")});"#,
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{4,7}");
}

#[test]
fn randomness_settings_and_graphics_are_not_detached() {
    let mut runner = run(r#"
        RANDSEED(31); LOCAL first:=RANDOM(), second:=RANDOM();
        RANDSEED(31); LOCAL again:=EXPR("RANDOM()"), next:=RANDOM();
        EXPR("HAngle:=1"); EXPR("PIXON_P(11,13,#12ABCDh)");
        RETURN {first==again,second==next,HAngle,GETPIX_P(11,13)};
    "#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{1,1,1,#12ABCDh}");
}

#[test]
fn linked_expressions_keep_module_private_variables_and_export_calls() {
    use ppl_vm::{
        HeadlessHost, Limits, Machine, ModuleSource, Settings, SourceIdentity, link_sources,
    };
    let sources = [
        ("A", r#"LOCAL count:=20; EXPORT MAIN() BEGIN LOCAL n:=3; IF 0 THEN LEAF(0); END; RETURN {EXPR("count:=count+n"),EXPR("LEAF(n)"),count}; END;"#),
        ("B", r#"LOCAL count:=100; EXPORT LEAF(x) BEGIN RETURN EXPR("count:=count+x"); END;"#),
    ].into_iter().map(|(name, source)| (name.to_owned(), ModuleSource { identity: SourceIdentity::new(name,source),source:source.to_owned() })).collect();
    let mut machine = Machine::new_linked(
        link_sources(&sources, "A").unwrap(),
        Box::new(HeadlessHost::new()),
        Settings::default(),
        Limits::default(),
    );
    assert_eq!(
        text(&machine.call_function("MAIN", vec![]).unwrap()),
        "{23,103,23}"
    );
    assert_eq!(
        text(&machine.call_function("MAIN", vec![]).unwrap()),
        "{26,106,26}"
    );
}

#[cfg(feature = "giac-integration")]
#[test]
fn measurement_expressions_use_live_variables_and_assignments() {
    let mut runner =
        run(r#"LOCAL n:=7; A:=3; LOCAL value:=EXPR("n*_m+A*_m"); RETURN STRING(value);"#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "10_m");
    let mut runner = run(r#"LOCAL n:=7; EXPR("n:=2_m"); RETURN STRING(n);"#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "2_m");
}

#[cfg(feature = "giac-integration")]
#[test]
fn cas_and_measurements_keep_their_genuine_g1_type_and_dialog_writeback() {
    use ppl_vm::{HeadlessHost, Limits, SessionContext};
    let mut runner = run(
        r#"LOCAL value:=EXPR("1_m"), algebra:=CAS.sqrt(2); LOCAL ok:=INPUT({{value,[9]},{algebra,[8]}}); RETURN {ok,TYPE(value),TYPE(algebra),STRING(value),STRING(algebra)};"#,
    );
    assert_eq!(
        text(&runner.call("MAIN", &["2_m", "sqrt(2)"]).unwrap()),
        "{1,9,8,\"2_m\",\"sqrt(2)\"}"
    );
    let mut session = SessionContext::default();
    let mut home = |text: &str| {
        session
            .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap()
    };
    assert_eq!(text(&home("TYPE(EXPR(\"1_m\"))")), "9");
    home("A:=EXPR(\"2_m\")");
    let image = session.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    assert_eq!(
        text(
            &restored
                .evaluate_home("TYPE(A)", Box::new(HeadlessHost::new()), Limits::default())
                .result
                .unwrap()
        ),
        "9"
    );
}

#[cfg(feature = "giac-integration")]
#[test]
fn measurement_evaluation_preserves_cas_answer_and_failed_bindings() {
    let mut runner = run(
        r#"CAS(42); A:=9; EXPR("A:=2_m"); IFERR EXPR("uncreated:=3_m"); THEN END; RETURN TYPE(A);"#,
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "9");
    assert_eq!(
        runner.machine.cas_context.last_value,
        Some(ppl_cas::CasValue::Integer("42".into()))
    );
    assert!(
        !runner
            .machine
            .cas_context
            .bindings
            .contains_key("uncreated")
    );
    assert!(!runner.machine.home_variables.contains_key("uncreated"));
}

#[cfg(feature = "giac-integration")]
#[test]
fn firmware_cas_type_controls_preserve_numeric_list_and_vector_identity() {
    for (expression, expected) in [
        (
            r#"{TYPE(CAS.sqrt(-1)),TYPE(CAS("1.5")),TYPE(CAS("3")),TYPE(CAS("[1,2]")),TYPE(CAS("[[1,2],[3,4]]"))}"#,
            "{3,0,0,4,4}",
        ),
        (r#"{TYPE(CAS(2^100)),TYPE(CAS("1234567890123"))}"#, "{0,0}"),
        (
            r#"{TYPE(CAS("solve(x^2=4,x)")),TYPE(CAS.sqrt(-2))}"#,
            "{6,8}",
        ),
        (
            r#"{TYPE(CAS("[x,2]")),TYPE(CAS("{1,2}")),TYPE(CAS("1/1"))}"#,
            "{6,6,0}",
        ),
        (
            r#"{TYPE(CAS({1,2})),TYPE(CAS("[]")),TYPE(CAS("[[x,2],[1,2]]")),TYPE(CAS("[1/2,2]"))}"#,
            "{6,6,6,4}",
        ),
        (
            r#"{TYPE(CAS.sqrt(2)),TYPE(CAS.x),TYPE(CAS("x")),TYPE(CAS(1/2)),TYPE(CAS("1/2")),TYPE(EXPR("1_m"))}"#,
            "{8,8,8,8,8,9}",
        ),
    ] {
        assert_eq!(
            text(
                &run(&format!("RETURN {expression};"))
                    .call("MAIN", &[])
                    .unwrap()
            ),
            expected,
            "{expression}"
        );
    }
}

#[cfg(feature = "giac-integration")]
#[test]
fn hp_cas_list_syntax_preserves_literal_strings_comments_and_home_lists() {
    let mut runner = run(r#"LOCAL value:=CAS("{\"{a}\",1}"); RETURN {TYPE(value),value};"#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), r#"{6,{"{a}",1}}"#);
    let mut runner = run(r#"RETURN TYPE(CAS("{1, // }\n2}"));"#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "6");
}

#[cfg(feature = "giac-integration")]
#[test]
fn old_profiles_recover_unit_types_without_executing_legacy_symbolic_text() {
    use ppl_cas::CasValue as C;
    use ppl_vm::{HeadlessHost, Limits, PortableValue, SessionContext};
    let mut legacy = SessionContext::default();
    for (name, value) in [
        ("A", C::Symbolic("-2_(m^2)".into())),
        ("B", C::Symbolic("quantity_m".into())),
        ("C", C::Integer("3".into())),
        (
            "D",
            C::Complex {
                real: Box::new(C::Integer("1".into())),
                imaginary: Box::new(C::Integer("2".into())),
            },
        ),
        ("E", C::Symbolic("side_m:=7".into())),
    ] {
        legacy
            .set_variable(name, PortableValue::Symbolic(value))
            .unwrap();
    }
    let mut image = legacy.memory_json();
    image["version"] = 1.into();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    let before = restored.snapshot().cas;
    let result = restored
        .evaluate_home(
            "{TYPE(A),TYPE(B),TYPE(C),TYPE(D),TYPE(E)}",
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap();
    assert_eq!(text(&result), "{9,8,0,3,8}");
    assert_eq!(restored.snapshot().cas.bindings, before.bindings);
    assert_eq!(restored.snapshot().cas.last_value, before.last_value);
    assert!(!restored.snapshot().globals.contains_key("side_m"));
    let source = "EXPORT OLDUNIT() BEGIN LOCAL a:=A; INPUT({{a,[9]}}); RETURN TYPE(a); END;";
    let mut runner = Run::new("old-unit.hpprgm", source);
    runner
        .machine
        .store_home_value("A", legacy.snapshot().globals["A"].to_value())
        .unwrap();
    assert_eq!(text(&runner.call("OLDUNIT", &["-2_(m^2)"]).unwrap()), "9");
}

#[test]
fn expression_new_home_slots_survive_without_resetting_existing_values() {
    let mut runner = run(r#"A:=7; EXPR("B:=4"); RETURN {EXPR("A+B"),A};"#);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{11,7}");
    assert_eq!(text(runner.machine.home_variables.get("B").unwrap()), "4");
    assert_eq!(text(&runner.machine.eval_text("B+1").unwrap()), "5");
}

fn install(runner: &mut Run, modules: &[(&str, &str)]) {
    let programs = modules
        .iter()
        .map(|(name, source)| (name.to_string(), source.to_string()))
        .collect();
    runner
        .machine
        .seed_object_stores(&Default::default(), &programs)
        .unwrap();
}

#[test]
fn expr_loads_installed_exports_in_the_same_machine_with_independent_private_state() {
    let mut runner = run(r#"LOCAL n:=3; RETURN {EXPR("LEAF(n)"),EXPR("LEAF(n)"),n};"#);
    install(
        &mut runner,
        &[(
            "library",
            r#"LOCAL count:=100; EXPORT LEAF(x) BEGIN count:=count+x; PRINT(count); RETURN count; END;"#,
        )],
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{103,106,3}");
    assert_eq!(runner.output(), ["103", "106"]);
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{109,112,3}");
    assert!(runner.machine.call_stack().is_empty());
}

#[test]
fn nested_expression_imports_preserve_outer_locals_code_and_imported_initialization() {
    let mut runner = run(
        r#"LOCAL count:=7,n:=4; LOCAL r:=EXPR("EXPR(\"BRANCH(n)\")"); RETURN {r,EXPR("LEAF(n)"),count,n};"#,
    );
    install(
        &mut runner,
        &[
            (
                "branch",
                r#"LOCAL count:=10; EXPORT BRANCH(x) BEGIN RETURN {EXPR("count:=count+x"),EXPR("LEAF(x)"),count}; END;"#,
            ),
            (
                "leaf",
                r#"LOCAL count:=100; EXPORT LEAF(x) BEGIN count:=count+x; RETURN count; END;"#,
            ),
        ],
    );
    assert_eq!(
        text(&runner.call("MAIN", &[]).unwrap()),
        "{{14,104,14},108,7,4}"
    );
    let dimensions = (
        runner.machine.program().globals.len(),
        runner.machine.program().functions.len(),
        runner.machine.program().consts.len(),
        runner.machine.program().names.len(),
    );
    for _ in 0..100 {
        runner.call("MAIN", &[]).unwrap();
    }
    assert_eq!(
        dimensions,
        (
            runner.machine.program().globals.len(),
            runner.machine.program().functions.len(),
            runner.machine.program().consts.len(),
            runner.machine.program().names.len()
        )
    );
    assert_eq!(
        text(&runner.call("MAIN", &[]).unwrap()),
        "{{418,912,418},916,7,4}"
    );
}

#[test]
fn dynamic_catalog_dependencies_initialize_and_exported_variables_share_their_owner() {
    let mut runner =
        run(r#"RETURN {EXPR("LEAF(2)"),EXPR("EXPORTED:=EXPORTED+1"),EXPR("LEAF(2)")};"#);
    install(
        &mut runner,
        &[
            ("alpha", "EXPORT EXPORTED:=7;"),
            (
                "beta",
                "LOCAL count:=EXPORTED+100; EXPORT LEAF(x) BEGIN count:=count+x; RETURN {count,EXPORTED}; END;",
            ),
        ],
    );
    assert_eq!(
        text(&runner.call("MAIN", &[]).unwrap()),
        "{{109,7},8,{111,8}}"
    );
}

#[test]
fn an_unselected_linked_catalog_module_is_available_to_dynamic_expr() {
    use ppl_vm::{
        HeadlessHost, Limits, Machine, ModuleSource, Settings, SourceIdentity, link_sources,
    };
    let sources = [
        (
            "root",
            r#"EXPORT MAIN() BEGIN RETURN EXPR("LEAF(5)"); END;"#,
        ),
        ("unused", r#"EXPORT LEAF(x) BEGIN RETURN x*3; END;"#),
    ]
    .into_iter()
    .map(|(name, source)| {
        (
            name.to_string(),
            ModuleSource {
                identity: SourceIdentity::new(name, source),
                source: source.into(),
            },
        )
    })
    .collect();
    let mut machine = Machine::new_linked(
        link_sources(&sources, "root").unwrap(),
        Box::new(HeadlessHost::new()),
        Settings::default(),
        Limits::default(),
    );
    assert_eq!(text(&machine.call_function("MAIN", vec![]).unwrap()), "15");
}

#[test]
fn dynamic_expr_preserves_syntax_atomicity_and_runtime_side_effects() {
    let mut runner = run(
        r#"LOCAL n:=7; IFERR EXPR("n:=1; LEAF(3)+("); THEN END; LOCAL first:=EXPR("LEAF(0)"); IFERR EXPR("LEAF(2)+1/0"); THEN END; RETURN {n,first,EXPR("LEAF(0)")};"#,
    );
    install(
        &mut runner,
        &[(
            "library",
            "LOCAL count:=100; EXPORT LEAF(x) BEGIN count:=count+x; RETURN count; END;",
        )],
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "{7,100,102}");
    assert!(runner.machine.call_stack().is_empty());
}

#[test]
fn dynamic_expr_ignores_broken_unrelated_tabs_but_refuses_ambiguous_exports() {
    let mut runner = run(r#"RETURN EXPR("LEAF(5)");"#);
    install(
        &mut runner,
        &[
            ("good", "EXPORT LEAF(x) BEGIN RETURN x+1; END;"),
            ("broken", "EXPORT BROKEN() BEGIN RETURN (;"),
        ],
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "6");
    let mut runner = run(r#"RETURN EXPR("LEAF(5)");"#);
    install(
        &mut runner,
        &[
            ("one", "EXPORT LEAF(x) BEGIN RETURN x+1; END;"),
            ("two", "EXPORT LEAF(x) BEGIN RETURN x+2; END;"),
        ],
    );
    assert_eq!(
        runner.call("MAIN", &[]).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn dynamically_loaded_programs_use_the_parent_execution_budget_and_input_host() {
    let mut runner = run(r#"RETURN EXPR("FOREVER()");"#);
    install(
        &mut runner,
        &[("forever", "EXPORT FOREVER() BEGIN WHILE 1 DO END; END;")],
    );
    runner.machine.limits.max_steps = Some(2000);
    assert_eq!(
        runner.call("MAIN", &[]).unwrap_err().kind,
        ErrorKind::ResourceLimit(ResourceKind::Steps)
    );
    assert!(runner.machine.call_stack().is_empty());
    let mut runner = run("LOCAL n:=7,answer:=0; INPUT(answer); RETURN {n,answer};");
    install(
        &mut runner,
        &[("leaf", "EXPORT LEAF(x) BEGIN PRINT(x); RETURN x*2; END;")],
    );
    assert_eq!(text(&runner.call("MAIN", &["LEAF(n)"]).unwrap()), "{7,14}");
    assert_eq!(runner.output(), ["7"]);
}

#[test]
fn dynamic_module_private_memory_survives_sessions_restore_and_exact_source_changes() {
    use ppl_vm::{HeadlessHost, Limits, SessionContext, SourceIdentity};
    use std::rc::Rc;
    fn invoke(session: &mut SessionContext) -> String {
        let source = r#"EXPORT MAIN() BEGIN RETURN EXPR("LEAF(3)"); END;"#;
        let (program, diagnostics) = ppl_compiler::compile_source(
            &ppl_syntax::SourceFile::new("root", source),
            &Default::default(),
        );
        assert!(!diagnostics.has_errors());
        text(
            &session
                .run(
                    SourceIdentity::new("root", source),
                    Rc::new(program.unwrap()),
                    Box::new(HeadlessHost::new()),
                    Limits::default(),
                    false,
                    |machine| machine.call_function("MAIN", vec![]),
                )
                .result
                .unwrap(),
        )
    }
    let mut session = SessionContext::default();
    session
        .store_program(
            "library",
            "LOCAL count:=100; EXPORT LEAF(x) BEGIN count:=count+x; RETURN count; END;".into(),
        )
        .unwrap();
    assert_eq!(invoke(&mut session), "103");
    assert_eq!(invoke(&mut session), "106");
    let mut restored = SessionContext::default();
    restored.restore_memory(&session.memory_json()).unwrap();
    assert_eq!(invoke(&mut restored), "109");
    restored
        .store_program(
            "library",
            "LOCAL count:=500; EXPORT LEAF(x) BEGIN count:=count+x; RETURN count; END;".into(),
        )
        .unwrap();
    assert_eq!(invoke(&mut restored), "503");
    assert!(!restored.snapshot().globals.contains_key("count"));
    assert!(
        !restored
            .memory_json()
            .to_string()
            .contains("retired expression")
    );
}

#[test]
fn original_g1_dynamic_catalog_controls_match_without_a_static_call_dependency() {
    use ppl_vm::{HeadlessHost, Limits, SessionContext, SourceIdentity, Value};
    use sha2::{Digest, Sha256};
    use std::rc::Rc;
    fn json(value: &Value) -> serde_json::Value {
        match value {
            Value::List(values) => values.iter().map(json).collect(),
            Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
            Value::Str(value) => serde_json::json!(value.as_ref()),
            other => panic!("unexpected G1 control result: {other:?}"),
        }
    }
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-expr-catalog-controls.json"
    ))
    .unwrap();
    let mut session = SessionContext::default();
    for module in controls["programs"].as_array().unwrap() {
        session
            .store_program(
                module["name"].as_str().unwrap(),
                module["source"].as_str().unwrap().into(),
            )
            .unwrap();
    }
    for case in controls["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            case["source_sha256"]
        );
        let (program, diagnostics) = ppl_compiler::compile_source(
            &ppl_syntax::SourceFile::new("G1 dynamic", source),
            &Default::default(),
        );
        assert!(!diagnostics.has_errors());
        let result = session
            .run(
                SourceIdentity::new(case["name"].as_str().unwrap(), source),
                Rc::new(program.unwrap()),
                Box::new(HeadlessHost::new()),
                Limits::default(),
                false,
                |machine| machine.call_function(case["entry"].as_str().unwrap(), vec![]),
            )
            .result
            .unwrap();
        assert_eq!(json(&result), case["expected"], "{}", case["name"]);
    }
}

#[test]
fn imported_expression_frames_keep_debugger_source_and_parent_call_stack() {
    use ppl_vm::{DebugAction, Debugger, Machine};
    use std::{cell::RefCell, rc::Rc};
    struct Record(Rc<RefCell<Vec<(String, usize)>>>);
    impl Debugger for Record {
        fn on_statement(
            &mut self,
            machine: &Machine,
            _: &str,
            _: ppl_syntax::Span,
            _: usize,
        ) -> DebugAction {
            if let Some(source) = machine.current_source() {
                self.0
                    .borrow_mut()
                    .push((source.name.clone(), machine.call_stack().len()));
            }
            DebugAction::Continue
        }
    }
    let mut runner = run(r#"RETURN EXPR("LEAF(5)");"#);
    install(
        &mut runner,
        &[(
            "library",
            "EXPORT LEAF(x) BEGIN LOCAL y:=x+1; RETURN y; END;",
        )],
    );
    let seen = Rc::new(RefCell::new(Vec::new()));
    runner
        .machine
        .set_debugger(Some(Box::new(Record(seen.clone()))));
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "6");
    assert!(
        seen.borrow()
            .iter()
            .any(|(name, depth)| name == "library" && *depth >= 3),
        "{:?}",
        seen.borrow()
    );
}

#[test]
fn dynamic_code_is_bounded_and_private_helpers_do_not_become_exports() {
    let mut runner = run(r#"RETURN EXPR("LEAF(5)");"#);
    install(
        &mut runner,
        &[("library", "EXPORT LEAF(x) BEGIN RETURN x+1; END;")],
    );
    runner.machine.limits.max_collection_len = 16;
    assert_eq!(
        runner.call("MAIN", &[]).unwrap_err().kind,
        ErrorKind::ResourceLimit(ResourceKind::Memory)
    );
    let mut runner = run(
        r#"LOCAL first:=EXPR("LEAF(5)"); IFERR EXPR("secret()"); THEN RETURN first; END; RETURN -1;"#,
    );
    install(
        &mut runner,
        &[(
            "library",
            "secret() BEGIN RETURN 999; END; EXPORT LEAF(x) BEGIN RETURN x+1; END;",
        )],
    );
    assert_eq!(text(&runner.call("MAIN", &[]).unwrap()), "6");
}

#[test]
fn failing_root_initializers_that_import_through_expr_are_retried_on_the_next_run() {
    use ppl_vm::{HeadlessHost, Limits, SessionContext, SourceIdentity};
    use std::rc::Rc;
    let mut session = SessionContext::default();
    session
        .store_program(
            "library",
            "EXPORT LEAF() BEGIN A:=A+1; IF A<2 THEN RETURN 1/0; END; RETURN A; END;".into(),
        )
        .unwrap();
    let source = r#"LOCAL cached:=EXPR("LEAF()"); EXPORT MAIN() BEGIN RETURN cached; END;"#;
    let (program, diagnostics) = ppl_compiler::compile_source(
        &ppl_syntax::SourceFile::new("root", source),
        &Default::default(),
    );
    assert!(!diagnostics.has_errors());
    let program = Rc::new(program.unwrap());
    let mut invoke = || {
        session
            .run(
                SourceIdentity::new("root", source),
                program.clone(),
                Box::new(HeadlessHost::new()),
                Limits::default(),
                false,
                |machine| machine.call_function("MAIN", vec![]),
            )
            .result
    };
    assert_eq!(invoke().unwrap_err().kind, ErrorKind::DivisionByZero);
    assert_eq!(text(&invoke().unwrap()), "2");
    assert_eq!(text(&invoke().unwrap()), "2");
}

#[test]
fn home_fast_path_keeps_uninstalled_editor_catalogs_lazy_and_restores_their_private_memory() {
    use ppl_vm::{HeadlessHost, Limits, ModuleSource, SessionContext, SourceIdentity};
    let mut session = SessionContext::default();
    let source = "LOCAL count:=100; EXPORT LEAF(x) BEGIN count:=count+x; RETURN count; END;";
    let mut sources = std::collections::BTreeMap::from([(
        "editor-library".into(),
        ModuleSource {
            identity: SourceIdentity::new("editor-library", "revision1"),
            source: source.into(),
        },
    )]);
    let mut evaluate = |text: &str| {
        session
            .evaluate_home_with_sources(
                text,
                &sources,
                Box::new(HeadlessHost::new()),
                Limits::default(),
            )
            .result
            .unwrap()
    };
    assert_eq!(text(&evaluate("2+3")), "5");
    assert_eq!(text(&evaluate(r#"EXPR("LEAF(5)")"#)), "105");
    assert_eq!(text(&evaluate(r#"EXPR("LEAF(5)")"#)), "110");
    assert!(session.snapshot().programs.is_empty());
    let mut restored = SessionContext::default();
    restored.restore_memory(&session.memory_json()).unwrap();
    assert_eq!(
        text(
            &restored
                .evaluate_home_with_sources(
                    r#"EXPR("LEAF(5)")"#,
                    &sources,
                    Box::new(HeadlessHost::new()),
                    Limits::default()
                )
                .result
                .unwrap()
        ),
        "115"
    );
    sources.get_mut("editor-library").unwrap().source = source.replace("100", "500");
    sources.get_mut("editor-library").unwrap().identity.revision = "revision2".into();
    assert_eq!(
        text(
            &restored
                .evaluate_home_with_sources(
                    r#"EXPR("LEAF(5)")"#,
                    &sources,
                    Box::new(HeadlessHost::new()),
                    Limits::default()
                )
                .result
                .unwrap()
        ),
        "505"
    );
}
