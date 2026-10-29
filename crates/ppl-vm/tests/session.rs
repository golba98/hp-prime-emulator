use ppl_vm::{HeadlessHost, Limits, PortableValue, SessionContext, SourceIdentity, Value};
use std::{
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

fn program(session: &mut SessionContext, name: &str, source: &str) -> ppl_vm::SessionRun {
    let file = ppl_syntax::SourceFile::new(name, source);
    let (compiled, diags) = ppl_compiler::compile_source(&file, &Default::default());
    assert!(!diags.has_errors(), "{diags:?}");
    session.run(
        SourceIdentity::new(name, source),
        Rc::new(compiled.unwrap()),
        Box::new(HeadlessHost::new()),
        Limits {
            max_steps: Some(5000),
            ..Limits::default()
        },
        false,
        |machine| machine.call_function("MAIN", Vec::new()),
    )
}
fn home(session: &mut SessionContext, text: &str) -> Value {
    session
        .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap()
}
fn number(value: Value) -> i64 {
    value.expect_i64("test").unwrap()
}

#[test]
fn home_uses_real_typed_ans_assignments_and_persistent_settings() {
    let mut session = SessionContext::default();
    assert_eq!(number(home(&mut session, "2+3")), 5);
    assert_eq!(number(home(&mut session, "Ans*2")), 10);
    assert_eq!(number(home(&mut session, "A:=7")), 7);
    assert_eq!(number(home(&mut session, "A+1")), 8);
    home(&mut session, "HAngle:=1");
    assert_eq!(number(home(&mut session, "SIN(90)")), 1);
    assert_eq!(session.snapshot().generation, 6);
    assert_eq!(
        session.snapshot().globals["Ans"],
        PortableValue::Real(ppl_math::Real::ONE)
    );
    let failed = session.evaluate_home("2+", Box::new(HeadlessHost::new()), Limits::default());
    assert!(failed.result.is_err());
    assert_eq!(number(home(&mut session, "Ans")), 1);
}

#[test]
fn modules_initialize_once_and_are_isolated_by_source_and_revision() {
    let mut session = SessionContext::default();
    let source = "LOCAL count:=10; EXPORT MAIN() BEGIN count:=count+1; RETURN count; END;";
    assert_eq!(
        number(program(&mut session, "a.ppl", source).result.unwrap()),
        11
    );
    assert_eq!(
        number(program(&mut session, "a.ppl", source).result.unwrap()),
        12
    );
    assert_eq!(
        number(program(&mut session, "b.ppl", source).result.unwrap()),
        11
    );
    assert_eq!(
        number(
            program(&mut session, "a.ppl", &source.replace(":=10", ":=20"))
                .result
                .unwrap()
        ),
        21
    );
    // Catalog results are available but never silently become Home Ans.
    assert_eq!(number(home(&mut session, "Ans")), 0);
}

#[test]
fn calculator_values_notes_programs_and_graphics_survive_invocations() {
    let mut session = SessionContext::default();
    let write = "EXPORT MAIN() BEGIN A:=41; Notes(\"ZZT_N\"):=\"hello\"; Programs(\"ZZT_P\"):=\"source\"; DIMGROB_P(G1,2,3); RETURN 0; END;";
    program(&mut session, "write.ppl", write).result.unwrap();
    let read = "EXPORT MAIN() BEGIN RETURN {A,Notes(\"ZZT_N\"),Programs(\"ZZT_P\"),GROBW_P(G1),GROBH_P(G1)}; END;";
    let value = program(&mut session, "read.ppl", read).result.unwrap();
    assert_eq!(
        PortableValue::from(&value),
        PortableValue::List(vec![
            PortableValue::Real(ppl_math::Real::from_i64(41)),
            PortableValue::String("hello".into()),
            PortableValue::String("source".into()),
            PortableValue::Real(ppl_math::Real::from_i64(2)),
            PortableValue::Real(ppl_math::Real::from_i64(3))
        ])
    );
    home(&mut session, "A:=7");
    let shadow = "LOCAL privateValue:=99; EXPORT MAIN() BEGIN RETURN privateValue; END;";
    assert_eq!(
        number(program(&mut session, "shadow.ppl", shadow).result.unwrap()),
        99
    );
    assert_eq!(number(home(&mut session, "A")), 7);
}

#[test]
fn runtime_errors_limits_and_interrupts_leave_a_reusable_session() {
    let mut session = SessionContext::default();
    home(&mut session, "42");
    let error = "EXPORT MAIN() BEGIN A:=A+1; RETURN 1/0; END;";
    assert!(program(&mut session, "error.ppl", error).result.is_err());
    assert_eq!(
        session.snapshot().globals["Ans"],
        PortableValue::Real(ppl_math::Real::from_i64(42))
    );
    assert_eq!(number(home(&mut session, "A")), 1);
    let looping = "EXPORT MAIN() BEGIN IFERR WHILE 1 DO A:=A+1; END; THEN RETURN 9; END; END;";
    assert!(program(&mut session, "loop.ppl", looping).result.is_err());
    let file = ppl_syntax::SourceFile::new("loop.ppl", looping);
    let (compiled, _) = ppl_compiler::compile_source(&file, &Default::default());
    let mut host = HeadlessHost::new();
    host.interrupt = Some(Arc::new(AtomicBool::new(true)));
    let interrupted = session.run(
        SourceIdentity::new("loop.ppl", looping),
        Rc::new(compiled.unwrap()),
        Box::new(host),
        Limits::default(),
        false,
        |m| m.call_function("MAIN", Vec::new()),
    );
    assert!(interrupted.result.is_err());
    assert_eq!(number(home(&mut session, "2+3")), 5);
}

#[test]
fn failed_file_initializer_is_retried_on_the_next_invocation() {
    let mut session = SessionContext::default();
    let source = "RINIT() BEGIN A:=A+1; IF A==1 THEN RETURN 1/0; END; RETURN 7; END; LOCAL count:=RINIT(); EXPORT MAIN() BEGIN RETURN count; END;";
    assert!(program(&mut session, "init.ppl", source).result.is_err());
    assert_eq!(
        number(program(&mut session, "init.ppl", source).result.unwrap()),
        7
    );
    assert_eq!(number(home(&mut session, "A")), 2);
}

#[test]
fn portable_matrix_dimensions_vector_flags_and_value_copies_survive_transfer() {
    let mut session = SessionContext::default();
    home(&mut session, "M1:=[[1,2],[3,4]]");
    home(&mut session, "M2:=M1");
    home(&mut session, "M2(1,1):=9");
    let snapshot = session.snapshot();
    let PortableValue::Matrix {
        rows,
        cols,
        vector,
        data,
    } = &snapshot.globals["M1"]
    else {
        panic!("matrix")
    };
    assert_eq!((*rows, *cols, *vector), (2, 2, false));
    assert_eq!(data[0], PortableValue::Real(ppl_math::Real::ONE));
    let Value::Matrix(mut restored) = snapshot.globals["M1"].to_value() else {
        panic!("matrix")
    };
    Rc::make_mut(&mut restored).data[0] = Value::from_i64(88);
    assert_eq!(data[0], PortableValue::Real(ppl_math::Real::ONE));
    home(&mut session, "M1:=[1,2,3]");
    let PortableValue::Matrix {
        rows,
        cols,
        vector,
        data,
    } = session.snapshot().globals["M1"].clone()
    else {
        panic!("vector")
    };
    assert_eq!((rows, cols, vector, data.len()), (1, 3, true, 3));
}

#[test]
fn argument_evaluation_reads_calculator_state_without_temporary_ans_or_history() {
    let mut session = SessionContext::default();
    home(&mut session, "5");
    let argument = session
        .evaluate_argument("Ans+1", Box::new(HeadlessHost::new()), Limits::default())
        .unwrap();
    assert_eq!(number(argument), 6);
    session
        .evaluate_argument("A:=7", Box::new(HeadlessHost::new()), Limits::default())
        .unwrap();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.generation, 1);
    assert_eq!(
        snapshot.last_value,
        Some(PortableValue::Real(ppl_math::Real::from_i64(5)))
    );
    assert_eq!(
        snapshot.globals["Ans"],
        PortableValue::Real(ppl_math::Real::from_i64(5))
    );
    assert_eq!(
        snapshot.globals["A"],
        PortableValue::Real(ppl_math::Real::from_i64(7))
    );
}
