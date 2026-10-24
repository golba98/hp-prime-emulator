//! Diagnostic classification: valid HP Prime programs that use features
//! the emulator lacks must compile with *compatibility* diagnostics only,
//! while genuinely malformed programs must produce syntax or semantic
//! errors at exact positions.

use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::{Diagnostic, DiagnosticClass, Severity, SourceFile};

struct Checked {
    file: SourceFile,
    compiled: bool,
    diags: Vec<Diagnostic>,
}

fn check(src: &str) -> Checked {
    let file = SourceFile::new("<test>", src.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    Checked {
        compiled: prog.is_some(),
        diags: diags.items,
        file,
    }
}

impl Checked {
    fn of_class(&self, class: DiagnosticClass) -> Vec<&Diagnostic> {
        self.diags.iter().filter(|d| d.class() == class).collect()
    }

    fn find(&self, code: &str) -> &Diagnostic {
        self.diags
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("no `{code}` diagnostic in {:#?}", self.diags))
    }

    fn line_col(&self, d: &Diagnostic) -> (u32, u32) {
        let lc = self.file.line_col(d.span.start);
        (lc.line, lc.col)
    }

    /// The program compiles and every diagnostic is a compatibility one.
    fn assert_valid_with_compat_only(&self) {
        assert!(self.compiled, "valid PPL must compile: {:#?}", self.diags);
        let other: Vec<_> = self
            .diags
            .iter()
            .filter(|d| d.class() != DiagnosticClass::Compatibility)
            .collect();
        assert!(
            other.is_empty(),
            "unexpected non-compat diagnostics: {other:#?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The four diagnostics from the reported screenshot.
// ---------------------------------------------------------------------------

#[test]
fn input_with_six_arguments_is_valid() {
    // User Guide 3rd ed. p.652:
    // INPUT(var,["title"],["label"],["help"],[reset_value],[initial_value])
    let c = check(
        "EXPORT ASK()\nBEGIN\n  LOCAL r;\n  INPUT(r,\"Radius\",\"r =\",\"Enter radius\",1,5);\n  RETURN r;\nEND;\n",
    );
    c.assert_valid_with_compat_only();
    assert!(c.diags.iter().all(|d| d.code != "wrong-arg-count"));
    // The emulator's dialog ignores the two defaults; say so as a note.
    let note = c.find("input-defaults-ignored");
    assert_eq!(note.severity, Severity::Note);
    assert_eq!(c.line_col(note), (4, 41));
}

#[test]
fn input_with_seven_arguments_is_still_an_error() {
    let c = check("EXPORT ASK()\nBEGIN\n  LOCAL r;\n  INPUT(r,\"t\",\"l\",\"h\",1,5,9);\nEND;\n");
    assert!(!c.compiled);
    let d = c.find("wrong-arg-count");
    assert_eq!(d.class(), DiagnosticClass::Semantic);
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(c.line_col(d), (4, 3));
}

#[test]
fn lowercase_append_is_a_compat_note_not_an_error() {
    let c = check(
        "EXPORT LST()\nBEGIN\n  LOCAL l := {1, 2};\n  l := append(l, 3);\n  RETURN l;\nEND;\n",
    );
    c.assert_valid_with_compat_only();
    let d = c.find("lowercase-command");
    assert_eq!(d.severity, Severity::Note);
    assert_eq!(c.line_col(d), (4, 8));
    assert!(d.message.contains("APPEND"), "{}", d.message);
}

#[test]
fn solve_in_every_documented_spelling_compiles() {
    // SOLVE: Solve app function (p.428); solve: CAS command; CAS.solve:
    // explicit CAS call (p.654).
    let c = check(
        "EXPORT SLV()\nBEGIN\n  LOCAL a, b, c;\n  a := SOLVE(X^2-X-2, X, 3);\n  b := solve(x^2-4 = 0, x);\n  c := CAS.solve(\"x^2=9\", \"x\");\n  RETURN a;\nEND;\n",
    );
    c.assert_valid_with_compat_only();
    let unsupported: Vec<_> = c
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .map(|d| (d.code, c.line_col(d)))
        .collect();
    assert_eq!(
        unsupported,
        vec![
            ("unsupported-command", (4, 8)),
            ("unsupported-command", (5, 8)),
            ("cas-unsupported", (6, 8)),
        ]
    );
    // `x` inside the CAS call is a symbol, not an undeclared variable.
    assert!(c.diags.iter().all(|d| d.code != "undeclared-identifier"));
}

#[test]
fn eval_is_a_compat_warning() {
    let c = check("EXPORT EV()\nBEGIN\n  RETURN EVAL(\"1+2\");\nEND;\n");
    c.assert_valid_with_compat_only();
    let d = c.find("unsupported-command");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(c.line_col(d), (3, 10));
}

#[test]
fn other_documented_features_are_compat_not_errors() {
    for (src, code) in [
        (
            "EXPORT PROG()\nBEGIN\n  RETURN %CHANGE(20, 50);\nEND;\n",
            "unsupported-command",
        ),
        (
            "EXPORT PROG()\nBEGIN\n  RETURN MAXREAL;\nEND;\n",
            "unsupported-command",
        ),
        (
            "EXPORT PROG()\nBEGIN\n  RETURN ∞;\nEND;\n",
            "unsupported-value",
        ),
        (
            "EXPORT PROG()\nBEGIN\n  RETURN 'X^2';\nEND;\n",
            "symbolic-unsupported",
        ),
        (
            "#cas\nf(x):=x^2;\n#end\nEXPORT PROG()\nBEGIN\n  RETURN 1;\nEND;\n",
            "cas-block-unsupported",
        ),
    ] {
        let c = check(src);
        c.assert_valid_with_compat_only();
        assert_eq!(
            c.find(code).class(),
            DiagnosticClass::Compatibility,
            "{src}"
        );
    }
}

// ---------------------------------------------------------------------------
// Genuinely invalid programs keep their errors, at exact positions.
// ---------------------------------------------------------------------------

#[test]
fn unknown_names_are_still_semantic_errors() {
    let c = check("EXPORT PROG()\nBEGIN\n  FROBNICATE(1);\n  RETURN y2k;\nEND;\n");
    assert!(!c.compiled);
    let f = c.find("unknown-function");
    assert_eq!(
        (f.class(), c.line_col(f)),
        (DiagnosticClass::Semantic, (3, 3))
    );
    let u = c.find("undeclared-identifier");
    assert_eq!(
        (u.class(), c.line_col(u)),
        (DiagnosticClass::Semantic, (4, 10))
    );
}

#[test]
fn wrong_builtin_arity_is_semantic() {
    let c = check("EXPORT PROG()\nBEGIN\n  PRINT(1, 2);\nEND;\n");
    assert!(!c.compiled);
    let d = c.find("wrong-arg-count");
    assert_eq!(
        (d.class(), c.line_col(d)),
        (DiagnosticClass::Semantic, (3, 3))
    );
}

#[test]
fn malformed_source_gives_syntax_errors() {
    let missing_paren = check("EXPORT PROG()\nBEGIN\n  PRINT(1;\nEND;\n");
    assert!(!missing_paren.compiled);
    assert!(!missing_paren.of_class(DiagnosticClass::Syntax).is_empty());

    let missing_end = check("EXPORT PROG()\nBEGIN\n  IF 1 THEN PRINT(1);\nEND;\n");
    assert!(!missing_end.compiled);
    assert!(!missing_end.of_class(DiagnosticClass::Syntax).is_empty());
}

#[test]
fn unclosed_string_error_stays_on_its_line() {
    let src = "EXPORT PROG()\nBEGIN\n  PRINT(\"oops);\n  PRINT(2);\nEND;\n";
    let c = check(src);
    assert!(!c.compiled);
    let d = c.find("unclosed-string");
    assert_eq!(d.class(), DiagnosticClass::Syntax);
    assert_eq!(c.line_col(d), (3, 9));
    // The span covers the rest of line 3 only.
    let end = c.file.line_col(d.span.end);
    assert_eq!(end.line, 3);
    // Later lines are still parsed: no cascade of errors after line 3
    // other than the unbalanced `(` the missing quote really causes.
    let after: Vec<_> = c
        .diags
        .iter()
        .filter(|d| d.is_error() && c.line_col(d).0 > 5)
        .collect();
    assert!(after.is_empty(), "{after:#?}");
}

#[test]
fn closed_multiline_string_is_legal_with_a_note() {
    let c = check("EXPORT PROG()\nBEGIN\n  PRINT(\"two\nlines\");\nEND;\n");
    assert!(c.compiled, "{:#?}", c.diags);
    let d = c.find("multiline-string");
    assert_eq!(d.severity, Severity::Note);
    assert_eq!(c.line_col(d), (3, 9));
}

// ---------------------------------------------------------------------------
// LOCAL placement and typed INPUT (Baseconv regression).
// ---------------------------------------------------------------------------

#[test]
fn local_declarations_anywhere_in_a_block_are_valid() {
    // LOCAL is a statement: after other statements, inside loops, and
    // re-declared in sibling blocks (the original Baseconv does all three).
    let c = check(
        "EXPORT PROG()\nBEGIN\n  LOCAL a := 1;\n  PRINT(a);\n  LOCAL b := a + 1;\n\
         FOR a FROM 1 TO 2 DO LOCAL d := a; PRINT(d); END;\n\
         FOR a FROM 1 TO 2 DO LOCAL d := b; PRINT(d); END;\n\
         RETURN b;\nEND;\n",
    );
    assert!(c.compiled, "{:#?}", c.diags);
    assert!(c.diags.is_empty(), "{:#?}", c.diags);
}

#[test]
fn duplicate_local_in_one_block_warns() {
    let c = check("EXPORT PROG()\nBEGIN\n  LOCAL a;\n  LOCAL a := 2;\n  RETURN a;\nEND;\n");
    assert!(c.compiled);
    let d = c.find("duplicate-local");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(c.line_col(d), (4, 9));
}

#[test]
fn typed_input_forms_compile_cleanly() {
    let c = check(
        "EXPORT PROG()\nBEGIN\n  LOCAL s := \"\", b := 10, p;\n\
         IF INPUT({{s,[2]}}, \"Number\", {\"N\"}) == 0 THEN RETURN; END;\n\
         INPUT(b, \"Base\", \"B\");\n\
         INPUT({b, {p,[0,-1]}}, \"T\", {\"B\",\"P\"});\n\
         RETURN s;\nEND;\n",
    );
    assert!(c.compiled, "{:#?}", c.diags);
    assert!(c.diags.is_empty(), "{:#?}", c.diags);
}

#[test]
fn typed_input_errors_point_at_the_field() {
    // A typed field must name a variable.
    let c = check("EXPORT PROG()\nBEGIN\n  INPUT({{1,[2]}});\nEND;\n");
    assert!(!c.compiled);
    let d = c.find("expected-variable");
    assert_eq!(d.class(), DiagnosticClass::Semantic);
    assert_eq!(c.line_col(d), (3, 11));
    // A missing comma makes `s [2]` an index expression, not a field.
    let c = check("EXPORT PROG()\nBEGIN\n  LOCAL s;\n  INPUT({{s [2]}});\nEND;\n");
    assert!(!c.compiled);
    assert_eq!(c.line_col(c.find("expected-variable")), (4, 11));
    // Undeclared variables are still reported.
    let c = check("EXPORT PROG()\nBEGIN\n  INPUT({{zz,[2]}});\nEND;\n");
    assert!(!c.compiled);
    assert_eq!(c.line_col(c.find("undeclared-identifier")), (3, 11));
}

#[test]
fn unknown_input_type_code_warns_and_position_is_noted() {
    let c = check("EXPORT PROG()\nBEGIN\n  LOCAL s;\n  INPUT({{s,[7]}});\nEND;\n");
    assert!(c.compiled);
    let d = c.find("input-unknown-type");
    assert_eq!(d.severity, Severity::Warning);
    assert!(d.unverified);
    assert_eq!(c.line_col(d), (4, 14));
    let c = check("EXPORT PROG()\nBEGIN\n  LOCAL s;\n  INPUT({{s,[2],{20,50,1}}});\nEND;\n");
    c.assert_valid_with_compat_only();
    c.find("input-field-position-ignored");
}

#[test]
fn malformed_control_flow_is_still_rejected() {
    for (src, code) in [
        (
            "EXPORT PROG()\nBEGIN\n  REPEAT PRINT(1); END;\nEND;\n",
            "expected-until",
        ),
        (
            "EXPORT PROG()\nBEGIN\n  WHILE 1 PRINT(1); END;\nEND;\n",
            "expected-do",
        ),
        (
            "EXPORT PROG()\nBEGIN\n  IF 1 THEN PRINT(1);\n",
            "unclosed-block",
        ),
    ] {
        let c = check(src);
        assert!(!c.compiled, "{src}");
        let d = c.find(code);
        assert_eq!(d.class(), DiagnosticClass::Syntax, "{src}");
    }
}
