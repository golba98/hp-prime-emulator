//! INPUT semantics: scalar and list forms, typed fields, write-back,
//! cancellation and the re-prompt that replaces a program abort.
//!
//! Typed `[2]` fields follow HP's statements on the HP Museum forum (the
//! entry is stored as a string without quotes and never evaluated);
//! UNVERIFIED on G1 by this project's own hardware evidence.

mod common;

use common::{Run, text};
use ppl_vm::ErrorKind;

/// `EXPORT TEST_PROG()` around `body`.
fn prog(body: &str) -> String {
    format!("EXPORT TEST_PROG()\nBEGIN\n{body}\nEND;")
}

#[test]
fn string_only_field_stores_text_verbatim() {
    let src = prog(
        "LOCAL s := \"\";\n\
         IF INPUT({{s,[2]}}, \"T\", {\"N\"}) == 0 THEN RETURN \"cancel\"; END;\n\
         RETURN s;",
    );
    // (typed, stored): letters, digits and dots are never evaluated.
    for (typed, want) in [
        ("FF", "FF"),
        ("A3", "A3"),
        ("7B.8", "7B.8"),
        ("255", "255"),
        ("1+1", "1+1"),
        ("\"q\"", "\"q\""),
        ("", ""),
    ] {
        let mut r = Run::new("<t>", &src);
        let v = r.call("TEST_PROG", &[typed]).unwrap();
        assert!(matches!(v, ppl_vm::Value::Str(_)), "{typed:?} gave {v:?}");
        assert_eq!(text(&v), want, "typed {typed:?}");
    }
}

#[test]
fn untyped_field_evaluates_and_writes_back() {
    let src = prog(
        "LOCAL a := 5;\n\
         LOCAL ok := INPUT(a, \"T\", \"A\");\n\
         RETURN {ok, a};",
    );
    // (typed, returned {status, a}).
    for (typed, want) in [
        ("12", "{1,12}"),
        ("2^3+1", "{1,9}"),
        ("\"hi\"", "{1,\"hi\"}"),
        ("{1,2}", "{1,{1,2}}"),
        // An empty entry keeps the current value.
        ("", "{1,5}"),
        ("<ESC>", "{0,5}"),
    ] {
        let mut r = Run::new("<t>", &src);
        let v = r.call("TEST_PROG", &[typed]).unwrap();
        assert_eq!(text(&v), want, "typed {typed:?}");
    }
}

#[test]
fn list_of_variables_writes_each_field() {
    let src = prog(
        "LOCAL a := 1, b := 2, s := \"x\";\n\
         LOCAL ok := INPUT({a, {s,[2]}, b}, \"T\", {\"A\",\"S\",\"B\"});\n\
         RETURN {ok, a, s, b};",
    );
    let mut r = Run::new("<t>", &src);
    let v = r.call("TEST_PROG", &["10", "c0de", "30"]).unwrap();
    assert_eq!(text(&v), "{1,10,\"c0de\",30}");
    let mut r = Run::new("<t>", &src);
    let v = r.call("TEST_PROG", &["<ESC>"]).unwrap();
    assert_eq!(text(&v), "{0,1,\"x\",2}", "cancel leaves every variable");
}

/// Presses Enter on every dialog without editing: each field returns the
/// initial text the VM offered.
struct EnterHost;

impl ppl_vm::Host for EnterHost {
    fn print(&mut self, _: &str) {}
    fn poll_key(&mut self) -> Result<Option<i64>, ppl_vm::RuntimeError> {
        Ok(None)
    }
    fn is_key_down(&mut self, _: i64) -> Result<bool, ppl_vm::RuntimeError> {
        Ok(false)
    }
    fn wait_key(&mut self, _: Option<f64>) -> Result<Option<i64>, ppl_vm::RuntimeError> {
        Ok(None)
    }
    fn sleep(&mut self, _: f64) -> Result<(), ppl_vm::RuntimeError> {
        Ok(())
    }
    fn touch(&mut self) -> Result<Option<ppl_vm::host::Touch>, ppl_vm::RuntimeError> {
        Ok(None)
    }
    fn dialog(
        &mut self,
        dialog: &ppl_vm::Dialog,
        _: &ppl_graphics::Grob,
    ) -> Result<ppl_vm::DialogResult, ppl_vm::RuntimeError> {
        match dialog {
            ppl_vm::Dialog::Input { fields, error, .. } => {
                assert!(error.is_none(), "unexpected re-prompt: {error:?}");
                Ok(ppl_vm::DialogResult::Entered(
                    fields.iter().map(|f| f.current.clone()).collect(),
                ))
            }
            _ => Ok(ppl_vm::DialogResult::Ok),
        }
    }
    fn now_ms(&mut self) -> f64 {
        0.0
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        (2026, 1, 1, 0, 0, 0.0)
    }
}

#[test]
fn untouched_fields_round_trip() {
    // Each field starts from a text that reproduces its value, so pressing
    // Enter without editing must not re-evaluate a bare string as an
    // identifier (or quote a string-only field).
    let src = prog(
        "LOCAL s := \"abc\", t := \"FF\", n := 2.5, l := {1,\"a\"};\n\
         INPUT({s, {t,[2]}, n, l});\n\
         RETURN {s, t, n, l};",
    );
    let file = ppl_syntax::SourceFile::new("<t>", src);
    let (prog, d) = ppl_compiler::compile_source(&file, &Default::default());
    assert!(!d.has_errors());
    let mut m = ppl_vm::Machine::new(
        std::rc::Rc::new(prog.unwrap()),
        Box::new(EnterHost),
        ppl_vm::Settings::default(),
        ppl_vm::Limits::default(),
    );
    let v = m.call_function("TEST_PROG", Vec::new()).unwrap();
    assert_eq!(text(&v), "{\"abc\",\"FF\",2.5,{1,\"a\"}}");
}

#[test]
fn rejected_entries_reopen_the_dialog() {
    let src = prog(
        "LOCAL b := 10;\n\
         IF INPUT(b, \"Base\", \"B\") == 0 THEN RETURN -1; END;\n\
         RETURN b;",
    );
    let mut r = Run::new("<t>", &src);
    // `abc` is undeclared and `1+` does not parse: both re-prompt.
    assert_eq!(
        text(&r.call("TEST_PROG", &["abc", "1+", "16"]).unwrap()),
        "16"
    );
    let d = r.dialogs();
    let errors: Vec<&String> = d.iter().filter(|l| l.starts_with("INPUT-ERROR")).collect();
    assert_eq!(errors.len(), 2, "{d:?}");
    assert!(errors[0].contains("Syntax error"), "{d:?}");
    assert_eq!(r.unused_inputs(), 0);
}

#[test]
fn disallowed_type_reopens_the_dialog() {
    let src = prog(
        "LOCAL x := 0;\n\
         INPUT({{x,[0]}}, \"T\", {\"X\"});\n\
         RETURN x;",
    );
    let mut r = Run::new("<t>", &src);
    assert_eq!(
        text(&r.call("TEST_PROG", &["\"str\"", "{1}", "2.5"]).unwrap()),
        "2.5"
    );
    let d = r.dialogs();
    assert_eq!(
        d.iter().filter(|l| l.contains("Invalid input")).count(),
        2,
        "{d:?}"
    );
}

#[test]
fn rejected_entry_keeps_earlier_fields_and_refocuses() {
    let src = prog(
        "LOCAL a := 0, b := 0;\n\
         INPUT({a, b}, \"T\", {\"A\",\"B\"});\n\
         RETURN {a, b};",
    );
    let mut r = Run::new("<t>", &src);
    // First attempt: b is invalid. Second attempt types both again.
    let v = r.call("TEST_PROG", &["1", "zz", "1", "2"]).unwrap();
    assert_eq!(text(&v), "{1,2}");
}

#[test]
fn cancel_after_a_rejected_entry_returns_zero() {
    let src = prog(
        "LOCAL b := 7;\n\
         LOCAL ok := INPUT(b, \"T\", \"B\");\n\
         RETURN {ok, b};",
    );
    let mut r = Run::new("<t>", &src);
    assert_eq!(
        text(&r.call("TEST_PROG", &["abc", "<ESC>"]).unwrap()),
        "{0,7}"
    );
}

#[test]
fn exhausted_script_is_a_deterministic_error() {
    let src = prog("LOCAL b; INPUT(b); RETURN b;");
    let mut r = Run::new("<t>", &src);
    let e = r.call("TEST_PROG", &["abc"]).unwrap_err();
    assert_eq!(e.kind, ErrorKind::ScriptExhausted);
}

#[test]
fn runtime_error_after_input_points_at_its_line() {
    let src = prog(
        "LOCAL s := \"\";\n\
         INPUT({{s,[2]}});\n\
         RETURN 1/0;",
    );
    let mut r = Run::new("<t>", &src);
    let e = r.call("TEST_PROG", &["FF"]).unwrap_err();
    assert_eq!(e.kind, ErrorKind::DivisionByZero);
    let frame = e.trace.first().expect("trace");
    let lc = ppl_syntax::SourceFile::new("<t>", src.clone()).line_col(frame.span.start);
    assert_eq!(lc.line, 5);
}
