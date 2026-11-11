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
            ppl_vm::Dialog::Input {
                title,
                fields,
                error,
                ..
            } => {
                assert!(error.is_none(), "unexpected re-prompt: {error:?}");
                if title == "inspect_positions" {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(
                        fields[0].position,
                        Some(ppl_vm::host::InputPosition {
                            start_percent: 25.0,
                            width_percent: 50.0,
                            line: 7,
                        })
                    );
                    assert_eq!(fields[0].page_title.as_deref(), Some("page two"));
                    assert_eq!(fields[1].position.unwrap().line, 0);
                    assert_eq!(fields[1].page_title.as_deref(), Some("inspect_positions"));
                }
                if title == "inspect_pages" {
                    assert_eq!(fields.len(), 8);
                    assert!(fields.iter().all(|f| f.position.is_none()));
                    assert_eq!(fields[6].page_title.as_deref(), Some("inspect_pages"));
                    assert_eq!(fields[7].page_title.as_deref(), Some("page two"));
                }
                Ok(ppl_vm::DialogResult::Entered(
                    fields
                        .iter()
                        .map(|f| {
                            if title == "inspect_reset" {
                                f.reset_text.clone().expect("reset metadata")
                            } else {
                                f.current.clone()
                            }
                        })
                        .collect(),
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

fn enter_defaults(body: &str) -> String {
    let file = ppl_syntax::SourceFile::new("<defaults>", prog(body));
    let (program, diagnostics) = ppl_compiler::compile_source(&file, &Default::default());
    assert!(!diagnostics.has_errors(), "{diagnostics:?}");
    let mut machine = ppl_vm::Machine::new(
        std::rc::Rc::new(program.unwrap()),
        Box::new(EnterHost),
        Default::default(),
        Default::default(),
    );
    text(&machine.call_function("TEST_PROG", vec![]).unwrap())
}

#[test]
fn initial_values_override_display_without_confusing_scalar_lists() {
    assert_eq!(
        enter_defaults("LOCAL a:=9; INPUT(a,\"T\",\"A\",\"Help\",1,5); RETURN a;"),
        "5"
    );
    assert_eq!(
        enter_defaults(
            "LOCAL a:=9,b:=8; INPUT({a,b},\"T\",{\"A\",\"B\"},{},{1,2},{5,6}); RETURN {a,b};"
        ),
        "{5,6}"
    );
    assert_eq!(
        enter_defaults("LOCAL a:={9}; INPUT(a,\"T\",\"A\",\"Help\",{1},{5,6}); RETURN a;"),
        "{5,6}"
    );
    assert_eq!(
        enter_defaults(
            "LOCAL a:=\"old\"; INPUT({{a,[2]}},\"T\",{\"A\"},{},{\"reset\"},{\"hello\"}); RETURN a;"
        ),
        "hello"
    );
}

#[test]
fn cancelling_initial_value_keeps_original_variable() {
    let src = prog("LOCAL a:=9; INPUT(a,\"T\",\"A\",\"help\",1,5); RETURN a;");
    let mut r = Run::new("cancel initial", &src);
    assert_eq!(text(&r.call("TEST_PROG", &["<ESC>"]).unwrap()), "9");
}

#[test]
fn input_writeback_obeys_native_string_limit() {
    let src = prog("LOCAL s:=\"\"; INPUT({{s,[2]}}); RETURN s;");
    let mut r = Run::new("limits", &src);
    r.machine.limits.max_string_bytes = 4;
    assert!(r.call("TEST_PROG", &["oversize"]).is_err());
}

#[test]
fn reset_metadata_preserves_scalar_and_multiple_field_values() {
    assert_eq!(
        enter_defaults("LOCAL a:=9; INPUT(a,\"inspect_reset\",\"A\",\"\",2,5); RETURN a;"),
        "2"
    );
    assert_eq!(
        enter_defaults(
            "LOCAL a:=9,b:=8; INPUT({a,b},\"inspect_reset\",{\"A\",\"B\"},{},{1,2},{5,6}); RETURN {a,b};"
        ),
        "{1,2}"
    );
}

#[test]
fn positioned_fields_preserve_expressions_titles_and_variable_order() {
    assert_eq!(
        enter_defaults(
            "LOCAL a:=11,b:=22,pos:={25,50,7}; INPUT({{a,[0],pos},{b,[0],{25,50,0}}},{\"inspect_positions\",\"page two\"}); RETURN {a,b};"
        ),
        "{11,22}"
    );
}

#[test]
fn unpositioned_eighth_field_gets_second_page_title() {
    assert_eq!(
        enter_defaults(
            "LOCAL a,b,c,d,e,f,g,h; INPUT({a,b,c,d,e,f,g,h},{\"inspect_pages\",\"page two\"}); RETURN 1;"
        ),
        "1"
    );
}

#[test]
fn mixed_and_unsupported_positions_fail_before_opening_dialog() {
    let mut r = Run::new(
        "<positions>",
        &prog("LOCAL a,b; INPUT({{a,[0],{25,50,0}},b});"),
    );
    let err = r.call("TEST_PROG", &[]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::BadArgValue);
    assert!(err.detail.contains("all fields or none"));
    for position in [
        "0",
        "{}",
        "{25,50}",
        "{-1,50,0}",
        "{-1E-499,50,0}",
        "{25,0,0}",
        "{75,50,0}",
        "{25,50,-1}",
        "{25,50,0.5}",
        "{25,50,1E100}",
    ] {
        let mut r = Run::new(
            "<positions>",
            &prog(&format!("LOCAL a; INPUT({{{{a,[0],{position}}}}});")),
        );
        let err = r.call("TEST_PROG", &[]).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported, "{position}: {err:?}");
        assert!(err.detail.contains("Firmware mode"));
    }
}

#[test]
fn positioned_cancel_retains_original_variables() {
    let mut r = Run::new(
        "<positions>",
        &prog(
            "LOCAL a:=5,b:=8; LOCAL ok:=INPUT({{a,[0],{25,50,0}},{b,[0],{25,50,7}}},{\"One\",\"Two\"},{},{},{1,2},{3,4}); RETURN {ok,a,b};",
        ),
    );
    assert_eq!(text(&r.call("TEST_PROG", &["<ESC>"]).unwrap()), "{0,5,8}");
}

#[test]
fn explicit_unsupported_or_malformed_descriptors_never_open_or_write_back() {
    for (spec, kind) in [
        ("\"checkbox\"", ErrorKind::Unsupported),
        ("[0,1.5]", ErrorKind::BadArgValue),
        ("[0,1E-499]", ErrorKind::BadArgValue),
        ("[1E100]", ErrorKind::BadArgValue),
        ("[7]", ErrorKind::Unsupported),
        ("[-1,7]", ErrorKind::Unsupported),
        ("[[0,2]]", ErrorKind::BadArgValue),
    ] {
        let mut r = Run::new(
            "<descriptors>",
            &prog(&format!("A:=17; B:=23; INPUT({{{{A,[0]}},{{B,{spec}}}}});")),
        );
        let err = r.call("TEST_PROG", &["100", "200"]).unwrap_err();
        assert_eq!(err.kind, kind, "{spec}: {err:?}");
        assert!(err.detail.contains("INPUT"), "{spec}: {err:?}");
        assert!(r.dialogs().is_empty(), "{spec}: dialog must not open");
        assert_eq!(r.unused_inputs(), 2, "{spec}: host input consumed");
        for (name, expected) in [("A", "17"), ("B", "23")] {
            let value = r
                .machine
                .globals()
                .into_iter()
                .find(|(n, _)| n == name)
                .unwrap()
                .1;
            assert_eq!(text(&value), expected, "{spec}: {name} changed");
        }
    }
}

#[test]
fn dynamic_choice_descriptors_are_delivered_as_choices() {
    let mut r = Run::new(
        "<dynamic descriptor>",
        &prog("LOCAL a:=17,spec:={0,\"bad\"}; INPUT({{a,spec}}); RETURN a;"),
    );
    let result = r.call("TEST_PROG", &["2"]).unwrap();
    assert_eq!(text(&result), "2");
    assert_eq!(r.unused_inputs(), 0);
}

#[test]
fn duplicate_type_codes_preserve_string_only_semantics() {
    for spec in ["[2,2]", "[2,2,2]"] {
        let mut r = Run::new(
            "<duplicate types>",
            &prog(&format!(
                "LOCAL a:=\"old\"; INPUT({{{{a,{spec}}}}}); RETURN a;"
            )),
        );
        let result = r.call("TEST_PROG", &["1+1"]).unwrap();
        assert!(matches!(result, ppl_vm::Value::Str(_)));
        assert_eq!(text(&result), "1+1");
    }
}

#[test]
fn omitted_descriptor_and_wildcard_keep_unrestricted_edit_fields() {
    for field in ["a".to_string(), "{a}".into(), "{a,[-1,2]}".into()] {
        let mut r = Run::new(
            "<unrestricted fields>",
            &prog(&format!("LOCAL a:=17; INPUT({{{field}}}); RETURN a;")),
        );
        assert_eq!(text(&r.call("TEST_PROG", &["1+1"]).unwrap()), "2");
    }
}

#[test]
fn checkbox_radio_and_choice_descriptors_write_back_typed_values() {
    let mut r = Run::new(
        "<controls>",
        &prog(
            "LOCAL a:=0,b:=1,c:=0,d:=1; INPUT({{a,1},{b,2},{c,0},{d,{\"First\",\"Second\"}}}); RETURN {a,b,c,d};",
        ),
    );
    assert_eq!(
        text(&r.call("TEST_PROG", &["1", "0", "1", "2"]).unwrap()),
        "{1,0,1,2}"
    );
}
#[test]
fn checkbox_and_choice_invalid_entries_reopen_without_partial_writeback() {
    let mut r = Run::new(
        "<controls>",
        &prog("LOCAL a:=0,b:=1; INPUT({{a,1},{b,{\"First\",\"Second\"}}}); RETURN {a,b};"),
    );
    assert_eq!(
        text(&r.call("TEST_PROG", &["2", "1", "1", "2"]).unwrap()),
        "{1,2}"
    );
    assert!(r.dialogs().iter().any(|s| s.contains("Checkbox requires")));
}
#[test]
fn radio_group_rejects_multiple_checked_values_and_cancel_preserves_originals() {
    let mut r = Run::new(
        "<radio>",
        &prog("LOCAL a:=0,b:=1; LOCAL ok:=INPUT({{a,2},{b,0}}); RETURN {ok,a,b};"),
    );
    assert_eq!(
        text(&r.call("TEST_PROG", &["1", "1", "<ESC>"]).unwrap()),
        "{0,0,1}"
    );
    assert!(r.dialogs().iter().any(|s| s.contains("Only one radio")));
}
