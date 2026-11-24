//! Compiler/VM transport tests. The recording host deliberately supplies typed
//! replies; calculator semantics are validated separately through the real GUI.
use ppl_compiler::{CompileOptions, compile_source};
use ppl_graphics::Grob;
use ppl_math::Real;
use ppl_syntax::SourceFile;
use ppl_vm::{
    ErrorKind, HeadlessHost, Host, Limits, Machine, ModuleSource, PortableValue, RuntimeError,
    SessionContext, SessionSnapshot, SourceIdentity,
    host::{AppVariableRequest, Dialog, DialogResult, Touch},
    link_sources,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};

type Seen = Rc<RefCell<Vec<(AppVariableRequest, SessionSnapshot)>>>;
struct RecordingHost {
    inner: HeadlessHost,
    seen: Seen,
    replies: VecDeque<Result<PortableValue, RuntimeError>>,
}
impl Host for RecordingHost {
    fn app_variable_with_context(
        &mut self,
        request: AppVariableRequest,
        context: SessionSnapshot,
    ) -> Result<PortableValue, RuntimeError> {
        self.seen.borrow_mut().push((request, context));
        self.replies.pop_front().expect("unexpected app request")
    }
    fn print(&mut self, text: &str) {
        self.inner.print(text);
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.inner.poll_key()
    }
    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.inner.is_key_down(key)
    }
    fn wait_key(&mut self, timeout: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.inner.wait_key(timeout)
    }
    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.inner.sleep(seconds)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.inner.touch()
    }
    fn dialog(&mut self, dialog: &Dialog, screen: &Grob) -> Result<DialogResult, RuntimeError> {
        self.inner.dialog(dialog, screen)
    }
    fn now_ms(&mut self) -> f64 {
        self.inner.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.inner.local_time()
    }
}
fn recording_host(replies: Vec<Result<PortableValue, RuntimeError>>) -> (Box<dyn Host>, Seen) {
    let seen = Seen::default();
    (
        Box::new(RecordingHost {
            inner: HeadlessHost::new(),
            seen: seen.clone(),
            replies: replies.into(),
        }),
        seen,
    )
}
#[test]
fn geometry_requests_capture_the_live_frame_and_evaluate_nested_ppl_calls_once() {
    let source = r#"EXPORT VALUE() BEGIN A:=A+1;RETURN 7;END;
        EXPORT GEO() BEGIN
        LOCAL GA:=7,p:=point(9,12);
        Geometry.Instruction(0):="point(0,0)";
        Geometry.GA:=point(3,4);
        RETURN {GA,Geometry.GA,distance(GA,0),distance(Geometry.GA,0),distance(p,0),distance(VALUE(),0),distance(Geometry.Instruction(1,4),0)};
        END;"#;
    let symbolic =
        |source: &str| PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source.into()));
    let (host, seen) = recording_host(vec![
        Ok(symbolic("point(9,12)")),
        Ok(number(1)),
        Ok(symbolic("point(3,4)")),
        Ok(symbolic("point(3,4)")),
        Ok(symbolic("point(3,4)")),
        Ok(number(7)),
        Ok(number(5)),
        Ok(symbolic("distance_data")),
        Ok(number(7)),
        Ok(symbolic("point(0,0)")),
        Ok(symbolic("distance_data")),
    ]);
    let mut session = SessionContext::default();
    let result = session.run(
        SourceIdentity::new("geometry", source),
        compiled(source),
        host,
        Limits::default(),
        false,
        |m| m.call_function("GEO", vec![]),
    );
    assert!(result.result.is_ok(), "{:?}", result.result);
    assert_eq!(
        session.snapshot().globals["A"],
        number(1),
        "A nested user call must execute exactly once"
    );
    let seen = seen.borrow();
    assert_eq!(seen.len(), 11);
    assert_eq!(
        (&*seen[0].0.app, &*seen[0].0.name),
        ("Geometry", "__Expression")
    );
    assert_eq!(seen[1].0.name, "Instruction");
    assert_eq!(
        seen[1].0.value,
        Some(PortableValue::String("point(0,0)".into()))
    );
    assert_eq!(seen[3].0.name, "GA");
    assert_eq!(seen[3].0.value, Some(symbolic("point(3,4)")));
    let local = &seen[5].1.globals;
    assert_eq!(local["GA"], number(7));
    assert_eq!(local["p"], symbolic("point(9,12)"));
    assert_eq!(
        seen[8].0.indices[0],
        PortableValue::List(vec![
            PortableValue::String("__GeometryArgument0".into()),
            number(7)
        ])
    );
    assert_eq!(seen[9].0.name, "Instruction");
    assert_eq!(seen[9].0.indices, vec![number(1), number(4)]);
    assert_eq!(
        seen[10].0.indices[0],
        PortableValue::List(vec![
            PortableValue::String("__GeometryArgument0".into()),
            symbolic("point(0,0)")
        ])
    );
    assert!(!session.snapshot().globals.contains_key("GA"));
    assert!(!session.snapshot().globals.contains_key("p"));
}
#[test]
fn geometry_names_preserve_graphics_commands_settings_and_declared_functions() {
    let source = "point(x) BEGIN RETURN x+1;END; EXPORT GO() BEGIN GridLines:=1;TRIANGLE();line(0,1);line(0,0,1,1);triangle(0,1,i);triangle(0,0,1,0,0,1);RETURN {point(4),GETKEY};END;";
    let program = compiled(source);
    assert!(
        !program
            .functions
            .iter()
            .flat_map(|f| &f.code)
            .any(|op| matches!(op, ppl_compiler::Op::AppVariable { .. })),
        "Declared functions, settings and uppercase graphics must retain their existing resolution"
    );
}
#[test]
fn installed_g_prefixed_exports_take_precedence_over_provisional_app_names() {
    let mut session = SessionContext::default();
    session
        .store_program(
            "GEOBRIDGE",
            "EXPORT GEOBRIDGE() BEGIN RETURN 42;END;".into(),
        )
        .unwrap();
    session
        .store_program("BROKEN", "EXPORT BROKEN( BEGIN".into())
        .unwrap();
    let result = session.evaluate_home(
        "GEOBRIDGE()",
        Box::new(HeadlessHost::new()),
        Limits::default(),
    );
    assert_eq!(PortableValue::from(&result.result.unwrap()), number(42));
    assert!(!session.snapshot().globals.contains_key("GEOBRIDGE"));
    session
        .store_program("OBJECTDATA", "EXPORT GA:=19;".into())
        .unwrap();
    let result = session.evaluate_home("GA", Box::new(HeadlessHost::new()), Limits::default());
    assert_eq!(PortableValue::from(&result.result.unwrap()), number(19));
    let result = session.evaluate_home("GA:=20", Box::new(HeadlessHost::new()), Limits::default());
    assert_eq!(PortableValue::from(&result.result.unwrap()), number(20));
    let result = session.evaluate_home("GA", Box::new(HeadlessHost::new()), Limits::default());
    assert_eq!(PortableValue::from(&result.result.unwrap()), number(20));
}
fn number(n: i64) -> PortableValue {
    PortableValue::Real(Real::from_i64(n))
}
fn compiled(source: &str) -> Rc<ppl_compiler::CompiledProgram> {
    let (program, diagnostics) =
        compile_source(&SourceFile::new("apps", source), &CompileOptions::default());
    assert!(!diagnostics.has_errors(), "{diagnostics:?}");
    Rc::new(program.unwrap())
}
#[test]
fn typed_app_reads_and_writes_observe_the_live_frame_without_creating_home_globals() {
    let source = "EXPORT GO() BEGIN A:=7; Spreadsheet.$A$1:=#FF:-8h; RETURN {Spreadsheet.a1,Spreadsheet.A1(3),EXPR(\"Spreadsheet.A1(1)\")}; END;";
    let (word_program, _) = compile_source(
        &SourceFile::new("word", "EXPORT WORD() BEGIN RETURN #FF:-8h; END;"),
        &CompileOptions::default(),
    );
    let mut word_machine = Machine::new(
        Rc::new(word_program.unwrap()),
        Box::new(HeadlessHost::new()),
        Default::default(),
        Limits::default(),
    );
    let word = PortableValue::from(&word_machine.call_function("WORD", vec![]).unwrap());
    let (host, seen) = recording_host(vec![
        Ok(word.clone()),
        Ok(word.clone()),
        Ok(number(-1)),
        Ok(number(7)),
    ]);
    let mut session = SessionContext::default();
    let result = session
        .run(
            SourceIdentity::new("apps", source),
            compiled(source),
            host,
            Limits::default(),
            false,
            |machine| machine.call_function("GO", vec![]),
        )
        .result
        .unwrap();
    assert_eq!(
        PortableValue::from(&result),
        PortableValue::List(vec![word.clone(), number(-1), number(7)])
    );
    let seen = seen.borrow();
    assert_eq!(seen.len(), 4);
    for (request, context) in seen.iter() {
        assert_eq!(request.app, "Spreadsheet");
        assert_eq!(context.globals["A"], number(7));
        assert!(
            !context
                .globals
                .keys()
                .any(|n| n.starts_with("Spreadsheet."))
        );
    }
    assert_eq!(seen[0].0.name, "$A$1");
    assert_eq!(seen[0].0.value, Some(word));
    assert_eq!(seen[1].0.name, "a1");
    assert_eq!(seen[2].0.indices, vec![number(3)]);
    assert_eq!(seen[3].0.indices, vec![number(1)]);
    assert_eq!(session.snapshot().globals["Ans"], number(0));
    assert!(session.snapshot().app_files.is_empty());
}
#[test]
fn indexed_assignment_evaluates_arguments_and_rhs_once_and_preserves_host_errors() {
    let source = "EXPORT GO() BEGIN A:=0; LOCAL i:=2; Spreadsheet.B2(i+1):=(A:=A+1); IFERR Spreadsheet.B2:=8; THEN RETURN A; END; RETURN 99; END;";
    let (host, seen) = recording_host(vec![
        Ok(number(1)),
        Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "Rejected worksheet write",
        )),
    ]);
    let mut machine = Machine::new(
        compiled(source),
        host,
        Default::default(),
        Limits::default(),
    );
    assert_eq!(
        machine
            .call_function("GO", vec![])
            .unwrap()
            .expect_i64("result")
            .unwrap(),
        1
    );
    let seen = seen.borrow();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0.indices, vec![number(3)]);
    assert_eq!(seen[0].0.value, Some(number(1)));
    assert_eq!(seen[0].1.globals["A"], number(1));
    assert_eq!(seen[1].0.value, Some(number(8)));
}
#[test]
fn linked_and_dynamic_catalog_app_requests_relocate_names_in_the_same_machine() {
    let mut session = SessionContext::default();
    let modules = BTreeMap::from([
        ("root".into(), ModuleSource { identity: SourceIdentity::new("root","r"), source:"LOCAL UNUSED() BEGIN RETURN CAS.X; END; EXPORT GO() BEGIN RETURN FROMCELL(); END;".into() }),
        ("reader".into(), ModuleSource { identity: SourceIdentity::new("reader","s"), source:"EXPORT FROMCELL() BEGIN RETURN Spreadsheet.A1; END;".into() })
    ]);
    let (host, seen) = recording_host(vec![Ok(number(17))]);
    let result = session
        .run_linked(
            link_sources(&modules, "root").unwrap(),
            host,
            Limits::default(),
            false,
            |m| m.call_function("GO", vec![]),
        )
        .result
        .unwrap();
    assert_eq!(result.expect_i64("linked").unwrap(), 17);
    assert_eq!(seen.borrow()[0].0.name, "A1");
    // This installed program is not initially linked: EXPR must load and
    // relocate it into the existing operation, retaining the same host.
    session.store_program("late-reader", "LOCAL NEVER() BEGIN RETURN CAS.Y; END; EXPORT LATECELL() BEGIN RETURN Spreadsheet.B2(1); END;".into()).unwrap();
    let source = "LOCAL UNUSED() BEGIN RETURN CAS.Z; END; EXPORT GO() BEGIN RETURN EXPR(\"LATECELL()\"); END;";
    let (host, seen) = recording_host(vec![Ok(number(23))]);
    let result = session
        .run(
            SourceIdentity::new("dynamic", source),
            compiled(source),
            host,
            Limits::default(),
            false,
            |m| m.call_function("GO", vec![]),
        )
        .result
        .unwrap();
    assert_eq!(result.expect_i64("dynamic").unwrap(), 23);
    assert_eq!(seen.borrow()[0].0.name, "B2");
    assert_eq!(seen.borrow()[0].0.indices, vec![number(1)]);
}
#[test]
fn headless_app_access_fails_explicitly_and_lowercase_app_qualifier_is_rejected() {
    let mut session = SessionContext::default();
    let error = session
        .evaluate_home(
            "Spreadsheet.A1",
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    let (_, diagnostics) = compile_source(
        &SourceFile::new("case", "EXPORT GO() BEGIN RETURN spreadsheet.A1; END;"),
        &CompileOptions::default(),
    );
    assert!(diagnostics.has_errors());
}

#[cfg(feature = "giac-integration")]
#[test]
fn actual_giac_uses_typed_app_values_once_in_scalar_and_algebra_arguments() {
    let source = "EXPORT GO() BEGIN RETURN {CAS(Spreadsheet.A1),CAS(Spreadsheet.A1(1)),CAS(Spreadsheet.A1+2)}; END;";
    let (host, seen) = recording_host(vec![Ok(number(3)), Ok(number(5)), Ok(number(7))]);
    let mut machine = Machine::new(
        compiled(source),
        host,
        Default::default(),
        Limits::default(),
    );
    let value = machine.call_function("GO", vec![]).unwrap();
    let ppl_vm::Value::List(values) = value else {
        panic!("Expected result list")
    };
    assert_eq!(
        values
            .iter()
            .map(|v| v.expect_i64("genuine Giac value").unwrap())
            .collect::<Vec<_>>(),
        vec![3, 5, 9]
    );
    assert_eq!(seen.borrow().len(), 3);
    assert_eq!(seen.borrow()[1].0.indices, vec![number(1)]);
}

#[test]
fn input_commits_actual_edited_values_and_cancel_does_not_freeze_an_app_formula() {
    use ppl_vm::host::ScriptedInput;
    for (cancel, app_first) in [(false, false), (true, false), (true, true)] {
        let seen = Seen::default();
        let mut inner = HeadlessHost::new();
        inner.inputs = if cancel {
            vec![ScriptedInput::Cancel]
        } else {
            vec![
                ScriptedInput::Text("42".into()),
                ScriptedInput::Text("43".into()),
            ]
        }
        .into();
        let replies = if cancel {
            vec![Ok(number(7))]
        } else {
            vec![Ok(number(7)), Ok(number(43))]
        };
        let host = Box::new(RecordingHost {
            inner,
            seen: seen.clone(),
            replies: replies.into(),
        });
        let fields = if app_first {
            "{Spreadsheet.A1,A}"
        } else {
            "{A,Spreadsheet.A1}"
        };
        let source =
            format!("EXPORT GO() BEGIN A:=3; LOCAL ok:=INPUT({fields}); RETURN {{ok,A}}; END;");
        let mut machine = Machine::new(
            compiled(&source),
            host,
            Default::default(),
            Limits::default(),
        );
        let result = machine.call_function("GO", vec![]).unwrap();
        assert_eq!(
            PortableValue::from(&result),
            PortableValue::List(vec![
                number(i64::from(!cancel)),
                number(if cancel { 3 } else { 42 })
            ])
        );
        let seen = seen.borrow();
        assert_eq!(seen.len(), if cancel { 1 } else { 2 });
        assert_eq!(seen[0].0.name, "A1");
        assert_eq!(seen[0].0.value, None);
        if !cancel {
            assert_eq!(seen[1].0.value, Some(number(43)));
        }
    }
}

#[test]
fn unqualified_function_slots_use_live_caller_while_qualified_owner_keeps_caller_context() {
    let source = "EXPORT GO() BEGIN A:=0; AAngle:=3; RETURN {F1(A:=A+1),Function.F2(A:=A+1),EXPR(\"F0(A:=A+1)\")}; END;";
    let (host, seen) = recording_host(vec![Ok(number(1)), Ok(number(2)), Ok(number(3))]);
    let mut machine = Machine::new(
        compiled(source),
        host,
        Default::default(),
        Limits::default(),
    );
    machine.active_app = "Spreadsheet".into();
    let result = machine.call_function("GO", vec![]).unwrap();
    assert_eq!(
        PortableValue::from(&result),
        PortableValue::List(vec![number(1), number(2), number(3)])
    );
    let seen = seen.borrow();
    assert_eq!(seen.len(), 3);
    for (i, (request, context)) in seen.iter().enumerate() {
        assert_eq!(context.active_app, "Spreadsheet");
        assert_eq!(context.app_settings["Spreadsheet"].angle, 3);
        assert_eq!(context.globals["A"], number(i as i64 + 1));
        assert_eq!(request.indices, vec![number(i as i64 + 1)]);
        assert_eq!(request.value, None);
    }
    assert_eq!(seen[0].0.app, "Spreadsheet");
    assert_eq!(seen[0].0.name, "F1");
    assert_eq!(seen[1].0.app, "Function");
    assert_eq!(seen[1].0.name, "F2");
    assert_eq!(seen[2].0.app, "Spreadsheet");
    assert_eq!(seen[2].0.name, "F0");
}

#[test]
fn declared_ppl_function_slot_names_preserve_existing_scoping_and_indexing() {
    for source in [
        "EXPORT GO() BEGIN LOCAL F1:={4,7}; RETURN F1(2); END;",
        "LOCAL F1:={4,7}; EXPORT GO() BEGIN RETURN F1(2); END;",
        "F1(x) BEGIN RETURN x+5; END; EXPORT GO() BEGIN RETURN F1(2); END;",
    ] {
        let (host, seen) = recording_host(vec![]);
        let mut machine = Machine::new(
            compiled(source),
            host,
            Default::default(),
            Limits::default(),
        );
        assert_eq!(
            machine
                .call_function("GO", vec![])
                .unwrap()
                .expect_i64("scope")
                .unwrap(),
            7
        );
        assert!(seen.borrow().is_empty());
    }
}

#[test]
fn app_function_slot_names_are_not_fabricated_in_headless_memory_or_lowercase_namespaces() {
    let mut session = SessionContext::default();
    for source in ["F1(2)", "Function.F0(2)", "Function.F1(2)"] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported, "{source}");
    }
    assert!(session.snapshot().app_files.is_empty());
    assert!(!session.snapshot().globals.contains_key("F1"));
    for source in ["function.F1(2)", "Function.F10(2)", "Function.f1(2)"] {
        let program = compiled(&format!("EXPORT GO() BEGIN RETURN {source}; END;"));
        let mut machine = Machine::new(
            program,
            Box::new(HeadlessHost::new()),
            Default::default(),
            Limits::default(),
        );
        let error = machine.call_function("GO", vec![]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported, "{source}");
    }
}
