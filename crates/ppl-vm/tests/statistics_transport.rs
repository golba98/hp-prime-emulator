//! Transport-only recording-host tests. Real Statistics results and UI behavior
//! are verified separately; the recording replies do not emulate statistics.
use ppl_compiler::{CompileOptions, compile_source};
use ppl_graphics::Grob;
use ppl_math::Real;
use ppl_syntax::SourceFile;
use ppl_vm::{
    ErrorKind, HeadlessHost, Host, Limits, PortableValue, RuntimeError, SessionContext,
    SessionSnapshot, SourceIdentity,
    host::{AppVariableRequest, Dialog, DialogResult, Touch},
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
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
fn number(n: i64) -> PortableValue {
    PortableValue::Real(Real::from_i64(n))
}
fn compiled(source: &str) -> Rc<ppl_compiler::CompiledProgram> {
    let (program, diagnostics) =
        compile_source(&SourceFile::new("apps", source), &CompileOptions::default());
    assert!(!diagnostics.has_errors(), "{diagnostics:?}");
    Rc::new(program.unwrap())
}

fn run(
    session: &mut SessionContext,
    source: &str,
    host: Box<dyn Host>,
) -> Result<ppl_vm::Value, RuntimeError> {
    session
        .run(
            SourceIdentity::new("statistics", source),
            compiled(source),
            host,
            Limits::default(),
            false,
            |machine| machine.call_function("GO", vec![]),
        )
        .result
}
fn syntax(value: &PortableValue) -> &str {
    match value {
        PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) => source,
        _ => panic!("expected unevaluated PPL syntax: {value:?}"),
    }
}
#[test]
fn commands_capture_live_identifiers_and_expressions_without_running_them() {
    let source = r#"TOUCH() BEGIN A:=99; RETURN {1,2}; END;
        EXPORT GO() BEGIN LOCAL P:={7,8,9}; A:=3;
        SetSample(H5,D1+1); SetFreq(H5,1+1); SetSample(H5,P);
        SetSample(H5,TOUCH()); RETURN {A,Do1VStats(H5)}; END;"#;
    let (host, seen) = recording_host(vec![Ok(number(1)); 5]);
    let mut session = SessionContext::default();
    session.active_app = "Statistics 1Var".into();
    session
        .set_variable(
            "D1",
            PortableValue::List(vec![number(1), number(2), number(4)]),
        )
        .unwrap();
    session.app_settings.insert(
        "Statistics 1Var".into(),
        ppl_vm::AppSettings {
            angle: 2,
            ..Default::default()
        },
    );
    let result = run(&mut session, source, host).unwrap();
    assert_eq!(result.string_form(ppl_vm::value::default_format()), "{3,1}");
    let seen = seen.borrow();
    assert_eq!(seen.len(), 5);
    for ((request, snapshot), expected) in seen[..4].iter().zip(["D1+1", "1+1", "P", "TOUCH()"]) {
        assert_eq!(request.app, "Statistics 1Var");
        assert!(request.value.is_none());
        assert_eq!(syntax(&request.indices[0]), "H5");
        assert_eq!(syntax(&request.indices[1]), expected);
        assert_eq!(snapshot.active_app, "Statistics 1Var");
        assert_eq!(snapshot.settings.angle, ppl_math::AngleMode::Radians);
        assert_eq!(snapshot.app_settings["Statistics 1Var"].angle, 2);
        assert_eq!(
            snapshot.globals["D1"],
            PortableValue::List(vec![number(1), number(2), number(4)])
        );
        assert!(!snapshot.globals.contains_key("P"));
    }
    assert_eq!(seen[0].0.name, "SetSample");
    assert_eq!(seen[1].0.name, "SetFreq");
    assert_eq!(seen[4].0.name, "Do1VStats");
    assert_eq!(seen[4].0.indices.len(), 1);
    assert_eq!(syntax(&seen[4].0.indices[0]), "H5");
    for name in ["H5", "SetSample", "SetFreq", "Do1VStats"] {
        assert!(!session.snapshot().globals.contains_key(name));
        assert!(!session.snapshot().programs.contains_key(name));
    }
}
#[test]
fn qualified_config_get_set_and_results_keep_the_actual_caller_context() {
    let source = r#"EXPORT GO() BEGIN A:=7;
        Statistics_1Var.H3:={"D1","",2,1,#FF:24h};
        Statistics_1Var.H3(3):=10;
        RETURN {Statistics_1Var.H3,Statistics_1Var.H3(1),Statistics_1Var.MeanX}; END;"#;
    let definition = PortableValue::List(vec![
        PortableValue::String("D1".into()),
        PortableValue::String(String::new()),
        number(2),
        number(1),
        PortableValue::Integer(ppl_math::HpInt::new(255, 24, ppl_math::IntBase::Hex)),
    ]);
    let (host, seen) = recording_host(vec![
        Ok(definition.clone()),
        Ok(number(10)),
        Ok(definition),
        Ok(PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(
            "D1".into(),
        ))),
        Ok(number(2)),
    ]);
    let mut session = SessionContext::default();
    let result = run(&mut session, source, host).unwrap();
    assert_eq!(result.type_code(), 6);
    let seen = seen.borrow();
    assert_eq!(seen.len(), 5);
    for (request, context) in seen.iter() {
        assert_eq!(request.app, "Statistics 1Var");
        assert_eq!(context.active_app, "Function");
        assert_eq!(context.globals["A"], number(7));
    }
    assert_eq!(seen[0].0.name, "H3");
    assert!(matches!(seen[0].0.value, Some(PortableValue::List(_))));
    assert_eq!(seen[1].0.indices, vec![number(3)]);
    assert_eq!(seen[1].0.value, Some(number(10)));
    assert_eq!(seen[4].0.name, "MeanX");
    assert!(!session.snapshot().globals.contains_key("H3"));
    assert!(!session.snapshot().globals.contains_key("MeanX"));
}
#[test]
fn declared_names_shadow_bare_app_fields_and_explicit_qualifiers_still_dispatch() {
    let source = "H2() BEGIN RETURN 8; END; EXPORT GO() BEGIN LOCAL H1:={4},MeanX:=9; RETURN {H1(1),H2(),MeanX,Statistics_1Var.MeanX}; END;";
    let (host, seen) = recording_host(vec![Ok(number(2))]);
    let mut session = SessionContext::default();
    let result = run(&mut session, source, host).unwrap();
    assert_eq!(
        result.string_form(ppl_vm::value::default_format()),
        "{4,8,9,2}"
    );
    assert_eq!(seen.borrow().len(), 1);
    assert_eq!(seen.borrow()[0].0.name, "MeanX");
}
#[test]
fn host_errors_are_catchable_and_headless_never_fabricates_app_success() {
    let source = "EXPORT GO() BEGIN LOCAL caught:=0; A:=7; IFERR SetFreq(H1,3); THEN caught:=1; END; RETURN {caught,A}; END;";
    let (host, seen) = recording_host(vec![Err(RuntimeError::kind(ErrorKind::InvalidInput))]);
    let mut session = SessionContext::default();
    session.active_app = "Statistics 1Var".into();
    let result = run(&mut session, source, host).unwrap();
    assert_eq!(result.string_form(ppl_vm::value::default_format()), "{1,7}");
    assert_eq!(syntax(&seen.borrow()[0].0.indices[1]), "3");
    let error = run(
        &mut session,
        "EXPORT GO() BEGIN RETURN Do1VStats(H1); END;",
        Box::new(HeadlessHost::new()),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
}
#[test]
fn captured_lists_strings_and_long_literals_keep_ppl_syntax_and_command_arity() {
    let source = r#"EXPORT GO() BEGIN LOCAL caught:=0;
        SetSample(H1,{1,2,(3,4)}); SetFreq(H1,1234567890123456789);
        IFERR SetSample(H1,"D1"); THEN caught:=1; END; RETURN caught; END;"#;
    let (host, seen) = recording_host(vec![
        Ok(number(1)),
        Ok(number(1)),
        Err(RuntimeError::kind(ErrorKind::BadArgType)),
    ]);
    let mut session = SessionContext::default();
    let result = run(&mut session, source, host).unwrap();
    assert_eq!(result.expect_real("caught").unwrap(), Real::ONE);
    let seen = seen.borrow();
    assert_eq!(syntax(&seen[0].0.indices[1]), "{1,2,(3,4)}");
    assert_eq!(syntax(&seen[1].0.indices[1]), "1234567890123456789");
    assert_eq!(syntax(&seen[2].0.indices[1]), "\"D1\"");
    for call in ["Do1VStats()", "SetSample(H1)", "SetFreq(H1,2,3)"] {
        let (_, diagnostics) = compile_source(
            &SourceFile::new("arity", format!("EXPORT GO() BEGIN RETURN {call}; END;")),
            &CompileOptions::default(),
        );
        assert!(
            diagnostics.errors().any(|d| d.code == "wrong-arg-count"),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn interruption_preserves_the_pre_request_frame_and_cannot_become_a_success() {
    let source = "EXPORT GO() BEGIN A:=7; SetSample(H1,D1+1); A:=99; RETURN A; END;";
    let (host, seen) = recording_host(vec![Err(RuntimeError::kind(ErrorKind::Interrupted))]);
    let mut session = SessionContext::default();
    session.active_app = "Statistics 1Var".into();
    let error = run(&mut session, source, host).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Interrupted);
    assert_eq!(seen.borrow().len(), 1);
    assert_eq!(seen.borrow()[0].1.globals["A"], number(7));
    assert_eq!(session.snapshot().globals["A"], number(7));
    assert!(!session.snapshot().globals.contains_key("H1"));
}
