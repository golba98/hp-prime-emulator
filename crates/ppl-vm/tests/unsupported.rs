//! Valid HP Prime programs that use commands the emulator lacks: they must
//! compile and run, and fail only when the unsupported call executes, with
//! a runtime error that points at the call.

use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::SourceFile;
use ppl_vm::{ErrorKind, HeadlessHost, Limits, Machine, Settings};
use std::rc::Rc;

fn machine(src: &str) -> (Machine, SourceFile) {
    let file = SourceFile::new("<test>", src.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    assert!(!diags.has_errors(), "{:#?}", diags.items);
    let m = Machine::new(
        Rc::new(prog.expect("compiled")),
        Box::new(HeadlessHost::new()),
        Settings::default(),
        Limits {
            max_steps: Some(10_000),
            ..Limits::default()
        },
    );
    (m, file)
}

const SRC: &str = "EXPORT SOLVER(n)\nBEGIN\n  IF n > 0 THEN\n    RETURN SOLVE(X^2-X-2, X, 3);\n  END;\n  RETURN 7;\nEND;\n";

#[test]
fn unexecuted_unsupported_call_does_not_prevent_running() {
    let (mut m, _) = machine(SRC);
    let v = m
        .call_function("SOLVER", vec![ppl_vm::Value::from_i64(0)])
        .expect("branch without SOLVE runs");
    assert_eq!(v.display(m.settings.format()), "7");
}

#[test]
fn executing_unsupported_call_is_a_runtime_error_at_the_call() {
    let (mut m, file) = machine(SRC);
    let err = m
        .call_function("SOLVER", vec![ppl_vm::Value::from_i64(1)])
        .expect_err("SOLVE is not emulated");
    assert_eq!(err.kind, ErrorKind::Unsupported);
    let frame = err.trace.first().expect("error carries a source location");
    let lc = file.line_col(frame.span.start);
    assert_eq!(lc.line, 4, "{err:?}");
}

/// Runs `EXPORT TST() BEGIN <body> END;` and returns the result or error kind.
fn run_body(body: &str) -> Result<String, ErrorKind> {
    let (mut m, _) = machine(&format!("EXPORT TST()\nBEGIN\n{body}\nEND;\n"));
    m.call_function("TST", Vec::new())
        .map(|v| v.display(m.settings.format()))
        .map_err(|e| e.kind)
}

#[test]
fn documented_short_forms_run() {
    // User Guide 3rd ed.: RANDINT() is 0 or 1, RANDINT(a) is in 0..a.
    for _ in 0..20 {
        let v: i64 = run_body("RETURN RANDINT();").unwrap().parse().unwrap();
        assert!((0..=1).contains(&v));
        let v: i64 = run_body("RETURN RANDINT(3);").unwrap().parse().unwrap();
        assert!((0..=3).contains(&v));
    }
    // MAKEMAT(expression, elements) makes a vector.
    assert_eq!(run_body("RETURN MAKEMAT(7, 3);").unwrap(), "[7,7,7]");
    // SUBGROB_P(srcGRB, trgtGRB) copies the whole graphic.
    assert_eq!(
        run_body("DIMGROB_P(G1, 12, 5);\nSUBGROB_P(G1, G2);\nRETURN GROBW_P(G2) + GROBH_P(G2);")
            .unwrap(),
        "17"
    );
}

#[test]
fn documented_but_unemulated_forms_raise_unsupported() {
    for body in [
        "LINE_P({[0,0,0],[1,1,1]}, {{1,2}}, [[1,0,0],[0,1,0],[0,0,1]]);",
        "TRIANGLE_P();",
        "DIMGROB_P(G1, {1, 2});",
        "RETURN STRING(1, 2, 3, \",\");",
    ] {
        assert_eq!(run_body(body), Err(ErrorKind::Unsupported), "{body}");
    }
}
