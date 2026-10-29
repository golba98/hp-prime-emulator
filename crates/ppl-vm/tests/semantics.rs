//! Characterization tests: they pin what the emulator does **today** for
//! PPL behaviour that has not been checked on a physical HP Prime G1.
//!
//! Every test names its hypothesis in an `UNVERIFIED on G1` comment. A
//! failure here means behaviour changed, not that the calculator was
//! consulted; update a test only together with hardware evidence or a
//! deliberate semantic change.

use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::SourceFile;
use ppl_vm::{HeadlessHost, Limits, Machine, Settings};
use std::rc::Rc;

/// Compiles and runs `TEST_PROG`, returning the terminal lines, or the
/// error kind (`Err("DivisionByZero")`) with whatever was printed before.
fn run_src(code: &str, settings: Settings) -> Result<Vec<String>, (String, Vec<String>)> {
    let file = SourceFile::new("<test>", code.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    if diags.has_errors() {
        let msgs: Vec<String> = diags.errors().map(|e| e.message.clone()).collect();
        panic!("compile errors: {msgs:?}\n{code}");
    }
    let prog = prog.expect("compiled program");
    let mut m = Machine::new(
        Rc::new(prog),
        Box::new(HeadlessHost::new()),
        settings,
        Limits {
            max_steps: Some(100_000),
            ..Limits::default()
        },
    );
    match m.call_function("TEST_PROG", Vec::new()) {
        Ok(_) => Ok(m.terminal.clone()),
        Err(e) => Err((format!("{:?}", e.kind), m.terminal.clone())),
    }
}

/// Wraps statements in `EXPORT TEST_PROG() BEGIN … END;`.
fn body(stmts: &str) -> String {
    format!("EXPORT TEST_PROG()\nBEGIN\n{stmts}\nRETURN 0;\nEND;\n")
}

fn lines(stmts: &str) -> Vec<String> {
    run_src(&body(stmts), Settings::default()).unwrap_or_else(|e| panic!("{e:?}\n{stmts}"))
}

fn error_kind(stmts: &str) -> String {
    match run_src(&body(stmts), Settings::default()) {
        Ok(out) => panic!("expected a runtime error, got {out:?}\n{stmts}"),
        Err((kind, _)) => kind,
    }
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

#[test]
fn mod_and_div_with_negative_operands() {
    // MOD: floored (result takes the divisor's sign), so -7 MOD 3 = 2.
    // Real operands are ACCEPTED by the G1 for MOD (hardware-verified
    // 2026-10-23: batches containing `3.14159 MOD 1.5` compiled past that
    // line; the UNVERIFIED parts are the exact values).
    assert_eq!(
        lines("PRINT(-7 MOD 3); PRINT(7 MOD -3); PRINT(-7 MOD -3); PRINT(-7.5 MOD 2);"),
        v(&["2", "-2", "-1", "0.5"])
    );
    // DIV is FLOOR(a/b) for integers. Hardware-verified 2026-10-23: real
    // literals as DIV operands are a *syntax error* on the G1 (mirrored by
    // the compiler as `div-requires-integers`), so only integer DIV runs.
    assert_eq!(lines("PRINT(-7 DIV 3); PRINT(7 DIV -3);"), v(&["-3", "-3"]));
    assert_eq!(error_kind("PRINT(7 MOD 0);"), "DivisionByZero");
    assert_eq!(error_kind("PRINT(7 DIV 0);"), "DivisionByZero");
}

#[test]
fn round_halves_away_from_zero() {
    // UNVERIFIED on G1: ROUND rounds ties away from zero (not to even), on
    // the decimal value, so ROUND(2.45,1) = 2.5.
    assert_eq!(
        lines(
            "PRINT(ROUND(2.5,0)); PRINT(ROUND(-2.5,0)); PRINT(ROUND(0.5,0));
             PRINT(ROUND(3.5,0)); PRINT(ROUND(-3.5,0)); PRINT(ROUND(-0.5,0));
             PRINT(ROUND(2.5)); PRINT(ROUND(2.45,1)); PRINT(ROUND(1.005,2));"
        ),
        v(&["3", "-3", "1", "4", "-4", "-1", "3", "2.5", "1.01"])
    );
    // UNVERIFIED on G1: negative places count significant digits.
    assert_eq!(lines("PRINT(ROUND(1234.5678,-2));"), v(&["1200"]));
}

#[test]
fn round_with_out_of_range_places_does_not_wrap() {
    // 2^32 places used to wrap to 0 through `as i32` and round to 3.
    // UNVERIFIED on G1: a place count beyond 12 digits leaves the value as is.
    assert_eq!(
        lines("PRINT(ROUND(2.5,4294967296)); PRINT(ROUND(2.5,-4294967296));"),
        v(&["2.5", "2.5"])
    );
}

#[test]
fn sqrt_with_large_odd_exponent_coefficient() {
    // Regression for the u128 overflow in Real::sqrt (debug builds panicked,
    // release builds printed 1.31172905731 and 1787.27520588). Expected
    // values from Python's decimal module.
    assert_eq!(
        lines("PRINT(√(5.12345678901)); PRINT(√(9999999.99999)); PRINT(√(2));"),
        v(&["2.26350542058", "3162.27766017", "1.41421356237"])
    );
}

#[test]
fn hp_integers_compare_as_their_unsigned_word() {
    // A 64-bit word with the top bit set compared as -1 before.
    // UNVERIFIED on G1: implicit integer→real conversion uses the unsigned
    // word (matching the `#…` display) regardless of the Signed setting.
    assert_eq!(
        lines(
            "PRINT(#FFFFFFFFFFFFFFFF:64h > 0); PRINT(#FFFFFFFFFFFFFFFF:64h == -1);
             PRINT(#FFFFFFFFh > 0); PRINT(B→R(#FFFFFFFFh));"
        ),
        v(&["1", "0", "1", "4294967295"])
    );
    // B→R honours the Signed setting (two's complement).
    let signed = Settings {
        int_signed: true,
        ..Settings::default()
    };
    assert_eq!(
        run_src(
            &body("PRINT(B→R(#FFFFFFFBh)); PRINT(#FFFFFFFBh > 0);"),
            signed
        )
        .unwrap(),
        v(&["-5", "1"])
    );
}

// ---------------------------------------------------------------------------
// Types and strings
// ---------------------------------------------------------------------------

#[test]
fn type_codes_per_value_kind() {
    // UNVERIFIED on G1: TYPE codes 0 real, 1 integer, 2 string, 3 complex,
    // 4 matrix/vector, 6 list, 8 graphic (forum-reported table).
    assert_eq!(
        lines(
            "PRINT(TYPE(1)); PRINT(TYPE(3.5)); PRINT(TYPE(#5h)); PRINT(TYPE(\"a\"));
             PRINT(TYPE(1+2*ⅈ)); PRINT(TYPE((1,2))); PRINT(TYPE([1,2]));
             PRINT(TYPE([[1,2],[3,4]])); PRINT(TYPE({1})); PRINT(TYPE(G0));"
        ),
        v(&["0", "0", "1", "2", "3", "3", "4", "4", "6", "8"])
    );
}

#[test]
fn string_indexing_returns_character_codes() {
    // UNVERIFIED on G1: s(n) is the character code of the n-th character
    // (a real), and indexes outside 1..SIZE(s) are "Invalid dimension".
    assert_eq!(
        lines("LOCAL s:=\"hello\"; PRINT(s(2)); PRINT(s(5)); PRINT(TYPE(s(2)));"),
        v(&["101", "111", "0"])
    );
    assert_eq!(
        error_kind("LOCAL s:=\"hello\"; PRINT(s(6));"),
        "InvalidDimension"
    );
    assert_eq!(
        error_kind("LOCAL s:=\"hello\"; PRINT(s(0));"),
        "InvalidDimension"
    );
}

#[test]
fn asc_returns_a_list_of_codes() {
    // UNVERIFIED on G1: ASC always returns a list, even for one character.
    assert_eq!(
        lines(
            "PRINT(ASC(\"AB\")); PRINT(ASC(\"A\")); PRINT(TYPE(ASC(\"A\")));
             PRINT(ASC(\"\")); PRINT(CHAR(65));"
        ),
        v(&["{65,66}", "{65}", "6", "{}", "A"])
    );
}

#[test]
fn string_plus_value_concatenates_display_forms() {
    // UNVERIFIED on G1: `+` with a string operand concatenates the display
    // form in the current format; lists distribute element-wise.
    assert_eq!(
        lines(
            "PRINT(\"ab\"+1); PRINT(1+\"ab\"); PRINT(\"ab\"+1.5); PRINT(\"ab\"+(1/3));
             PRINT(\"\"+1E20); PRINT(\"ab\"+#1Fh); PRINT(\"x\"+{1,2}); PRINT(\"v\"+[1,2]);"
        ),
        v(&[
            "ab1",
            "1ab",
            "ab1.5",
            "ab0.333333333333",
            "1E20",
            "ab#1Fh",
            "{\"x1\",\"x2\"}",
            "v[1,2]",
        ])
    );
}

#[test]
fn list_and_matrix_display() {
    // UNVERIFIED on G1: PRINT shows lists as {a,b} and matrices as [[a,b],[c,d]]
    // with no spaces, strings inside collections quoted.
    assert_eq!(
        lines(
            "PRINT({1,2,{3,\"a\"}}); PRINT([1,2,3]); PRINT([[1,2],[3,4]]);
             PRINT({}); PRINT({1.5,-2}); PRINT(1+2*ⅈ);"
        ),
        v(&[
            "{1,2,{3,\"a\"}}",
            "[1,2,3]",
            "[[1,2],[3,4]]",
            "{}",
            "{1.5,-2}",
            "1+2*i"
        ])
    );
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn list_index_zero_appends() {
    // UNVERIFIED on G1: L(0):=x appends, an existing index replaces,
    // L(SIZE+1):=x also appends, and anything further out is
    // "Invalid dimension".
    assert_eq!(
        lines(
            "LOCAL L:={1,2}; L(0):=9; PRINT(L); L(0):=10; PRINT(L); L(4):=11; PRINT(L);
             L(5):=12; PRINT(L); LOCAL E:={}; E(0):=1; PRINT(E); PRINT(SIZE(E));"
        ),
        v(&[
            "{1,2,9}",
            "{1,2,9,10}",
            "{1,2,9,11}",
            "{1,2,9,11,12}",
            "{1}",
            "1"
        ])
    );
    assert_eq!(error_kind("LOCAL L:={1,2}; L(5):=9;"), "InvalidDimension");
}

// ---------------------------------------------------------------------------
// FOR loops
// ---------------------------------------------------------------------------

#[test]
fn for_variable_after_the_loop() {
    // UNVERIFIED on G1: after a FOR loop the variable holds the first value
    // that failed the test (one STEP past the bound); an empty range leaves
    // it at the start value without running the body.
    assert_eq!(
        lines(
            "LOCAL i;
             FOR i FROM 1 TO 3 DO END; PRINT(i);
             FOR i FROM 3 DOWNTO 1 DO END; PRINT(i);
             FOR i FROM 1 TO 5 STEP 2 DO END; PRINT(i);
             FOR i FROM 1 TO 3 STEP 0.5 DO END; PRINT(i);
             FOR i FROM 10 DOWNTO 1 STEP 3 DO END; PRINT(i);
             FOR i FROM 5 TO 1 DO PRINT(\"body\"); END; PRINT(i);
             FOR i FROM 1 DOWNTO 5 DO PRINT(\"body\"); END; PRINT(i);
             i:=7; FOR i FROM 1 TO 0 DO END; PRINT(i);"
        ),
        v(&["4", "0", "7", "3.5", "-2", "5", "1", "1"])
    );
}

#[test]
fn for_bounds_and_step_are_evaluated_once() {
    // UNVERIFIED on G1: TO and STEP are evaluated once before the first
    // iteration; changing their variables in the body has no effect.
    assert_eq!(
        lines(
            "LOCAL n:=3, c:=0, i;
             FOR i FROM 1 TO n DO n:=n+1; c:=c+1; IF c>10 THEN BREAK; END; END;
             PRINT(c); PRINT(n);
             LOCAL s:=1;
             FOR i FROM 1 TO 3 STEP s DO s:=s+1; PRINT(i); END;"
        ),
        v(&["3", "6", "1", "2", "3"])
    );
}

#[test]
fn for_variable_assigned_in_body() {
    // UNVERIFIED on G1: assigning the loop variable in the body changes the
    // next iteration's value (the step is added to the current value).
    assert_eq!(
        lines("LOCAL i; FOR i FROM 1 TO 3 DO i:=i+1; PRINT(i); END; PRINT(i);"),
        v(&["2", "4", "5"])
    );
}

// ---------------------------------------------------------------------------
// Evaluation order and strictness
// ---------------------------------------------------------------------------

#[test]
fn and_or_evaluate_both_operands() {
    // UNVERIFIED on G1: AND / OR are not short-circuit, so the right operand
    // is evaluated (and can fail) even when the left decides the result.
    assert_eq!(
        error_kind("IF 0 AND (1/0) THEN PRINT(1); END;"),
        "DivisionByZero"
    );
    assert_eq!(
        error_kind("IF 1 OR (1/0) THEN PRINT(1); END;"),
        "DivisionByZero"
    );
    // UNVERIFIED on G1: results are 1/0 (not the operand values).
    assert_eq!(
        lines("PRINT(0 AND 2); PRINT(3 OR 0); PRINT(NOT 5); PRINT(2 XOR 3);"),
        v(&["0", "1", "0", "0"])
    );
}

#[test]
fn ifte_evaluates_both_branches() {
    // UNVERIFIED on G1: IFTE is an ordinary function with eager arguments,
    // so the unused branch still runs.
    assert_eq!(error_kind("PRINT(IFTE(1, 5, 1/0));"), "DivisionByZero");
    assert_eq!(
        lines("PRINT(IFTE(0, 5, 6)); PRINT(IFTE(1, 5, 6));"),
        v(&["6", "5"])
    );
}

#[test]
fn uninitialised_locals_are_zero() {
    // UNVERIFIED on G1: a LOCAL without an initialiser is the real 0, and a
    // later initialiser in the same LOCAL statement sees earlier ones.
    assert_eq!(
        lines(
            "LOCAL x; PRINT(x); PRINT(TYPE(x)); LOCAL y,z; PRINT(y+z); LOCAL a:=1, b:=a+1; PRINT(b);"
        ),
        v(&["0", "0", "0", "2"])
    );
}

#[test]
fn arguments_and_operands_evaluate_left_to_right() {
    // UNVERIFIED on G1: binary operands, call arguments and list elements
    // are evaluated left to right.
    let src = "SIDE(x)
BEGIN
  PRINT(x);
  RETURN x;
END;
PAIR(a,b)
BEGIN
  RETURN a*10+b;
END;
EXPORT TEST_PROG()
BEGIN
  PRINT(SIDE(1)+SIDE(2));
  PRINT(PAIR(SIDE(3),SIDE(4)));
  PRINT({SIDE(5),SIDE(6)});
  PRINT(SIDE(7)*10+SIDE(8));
  PRINT(SIDE(9)<SIDE(10));
  RETURN 0;
END;
";
    assert_eq!(
        run_src(src, Settings::default()).unwrap(),
        v(&[
            "1", "2", "3", "3", "4", "34", "5", "6", "{5,6}", "7", "8", "78", "9", "10", "1",
        ])
    );
}

// ---------------------------------------------------------------------------
// Copy-on-write value semantics
// ---------------------------------------------------------------------------

#[test]
fn assignment_copies_collections() {
    // UNVERIFIED on G1: `B:=A` copies; changing B leaves A unchanged, for
    // lists and matrices alike.
    assert_eq!(
        lines(
            "LOCAL A:={1,2}; LOCAL B; B:=A; B(1):=9; PRINT(A); PRINT(B);
             LOCAL M:=[[1,2],[3,4]]; LOCAL N:=M; N(1,1):=9; PRINT(M); PRINT(N);"
        ),
        v(&["{1,2}", "{9,2}", "[[1,2],[3,4]]", "[[9,2],[3,4]]"])
    );
}

#[test]
fn function_arguments_are_passed_by_value() {
    // UNVERIFIED on G1: a function changing its parameter (number, list or
    // matrix) does not change the caller's variable.
    let src = "MUTL(L)
BEGIN
  L(1):=99;
  RETURN L;
END;
MUTM(M)
BEGIN
  M(1,1):=99;
  RETURN M;
END;
INC(x)
BEGIN
  x:=x+1;
  RETURN x;
END;
EXPORT TEST_PROG()
BEGIN
  LOCAL A:={1,2};
  LOCAL B:=MUTL(A);
  PRINT(A); PRINT(B);
  LOCAL X:=[[1,2],[3,4]];
  LOCAL Y:=MUTM(X);
  PRINT(X); PRINT(Y);
  LOCAL a:=1;
  PRINT(INC(a)); PRINT(a);
  RETURN 0;
END;
";
    assert_eq!(
        run_src(src, Settings::default()).unwrap(),
        v(&[
            "{1,2}",
            "{99,2}",
            "[[1,2],[3,4]]",
            "[[99,2],[3,4]]",
            "2",
            "1"
        ])
    );
}

#[test]
fn nested_list_mutation_and_copies() {
    // UNVERIFIED on G1: K(2,2):=8 changes the inner list in place; a copy
    // of K, an element taken out of K, and a list stored inside another
    // list are all independent afterwards.
    assert_eq!(
        lines(
            "LOCAL K:={{1,2},{3,4}}; K(2,2):=8; PRINT(K);
             LOCAL J:=K; J(1,1):=0; PRINT(K); PRINT(J);
             LOCAL I:=K(1); I(1):=7; PRINT(K); PRINT(I);
             LOCAL L:={1,2}; LOCAL W:={L,L}; L(1):=5; PRINT(W);
             W(1,1):=7; PRINT(W); PRINT(L);"
        ),
        v(&[
            "{{1,2},{3,8}}",
            "{{1,2},{3,8}}",
            "{{0,2},{3,8}}",
            "{{1,2},{3,8}}",
            "{7,2}",
            "{{1,2},{1,2}}",
            "{{7,2},{1,2}}",
            "{5,2}",
        ])
    );
}

#[test]
fn equality_across_kinds() {
    // UNVERIFIED on G1: `==` compares numbers by value across kinds and
    // collections structurally, returning a single 1/0.
    assert_eq!(
        lines("PRINT(5 == 5.0); PRINT(\"a\" == \"a\"); PRINT({1,2} == {1,2});"),
        v(&["1", "1", "1"])
    );
}

// ---------------------------------------------------------------------------
// MOUSE idle shape (regression: FIREWORKS-style guards)
//
// UNVERIFIED on G1 (shape inferred from real-world programs): with no
// touch active, MOUSE() must be *empty* — programs written for hardware
// guard with `SIZE(m) > 0` before indexing m[1,1]. The emulator used to
// return [[],[]] (SIZE 2), crashing such programs with
// "Invalid dimension (index 1 is outside 1..0)".

#[test]
fn mouse_idle_returns_empty_list_not_two_empty_lists() {
    let out = lines(
        "LOCAL m := MOUSE; PRINT(SIZE(m));
         IF SIZE(m) > 0 THEN PRINT(m[1,1]); ELSE PRINT(\"idle\"); END;",
    );
    assert_eq!(out, v(&["0", "idle"]));
}

#[test]
fn mouse_idle_size_guard_pattern_survives() {
    // the exact guard pattern used by tests/fixtures/FIREWORKS.hpprgm
    let out = lines(
        "LOCAL m := MOUSE, mx := 0;
         IF SIZE(m) > 0 THEN mx := m[1,1]; END;
         PRINT(mx);",
    );
    assert_eq!(out, v(&["0"]));
}

// ---------------------------------------------------------------------------
// STRING() of a string quotes it
//
// HARDWARE-VERIFIED (2026-10-23, G1 build 15157, note-channel evidence for
// suite gen_0135289e): STRING("0|−0.5") on the calculator yields
// "0|−0.5" WITH surrounding double quotes; the emulator used to return the
// bare text, which the differential comparator flagged as FAILED.

#[test]
fn string_of_string_is_quoted() {
    let out = lines(r#"LOCAL s := STRING("abc"); PRINT(s); PRINT(SIZE(s));"#);
    assert_eq!(out, v(&["\"abc\"", "5"]));
}

// Raw note ZZT_F0E093, source71e75578..., tag0c68690f, G1 15157,
// Home0|0|0: full ASC lists distinguish program data from displayed glyphs.
#[test]
fn g1_string_escapes_double_quotes_without_changing_the_original_value() {
    let out = run_src(
        &body(
            r#"
            LOCAL v:="a"+CHAR(34)+"b",s:=STRING(v),q:=STRING(CHAR(34));
            PRINT(v);
            PRINT({SIZE(v),ASC(v),SIZE(s),ASC(s)});
            PRINT({SIZE(q),ASC(q)});
            PRINT({SIZE(STRING("")),ASC(STRING(""))});
            PRINT(STRING(" A "));
        "#,
        ),
        Settings {
            hdigits: 0,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        out,
        v(&[
            "a\"b",
            "{3,{97,34,98},6,{34,97,92,34,98,34}}",
            "{4,{34,92,34,34}}",
            "{2,{34,34}}",
            "\" A \"",
        ])
    );
}

#[test]
fn string_of_string_concat_matches_hardware_record() {
    // the exact shape of the device-program record for a string case
    let out = lines(r#"PRINT(STRING(0) + "|" + STRING(-0.5));"#);
    assert_eq!(out, v(&["0|-0.5"]));
}

// ---------------------------------------------------------------------------
// Notes / Programs system variables
//
// The ASSIGNMENT syntax `Notes("X"):=value` is HARDWARE-VERIFIED (2026-10-23,
// G1 build 15157): every campaign result note was written by it. Reading a
// note written this way is verified the same way. The DELETION semantics
// (`Notes("X"):=""` / `Programs("X"):=""`, HP Prime User Guide ch. 28) are
// DOCUMENTED-ONLY here until the physical probes confirm them.

#[test]
fn notes_assignment_and_read_roundtrip() {
    let out = lines("Notes(\"N1\"):=\"hello\";\n         PRINT(Notes(\"N1\"));");
    assert_eq!(out, v(&["hello"]));
}

#[test]
fn notes_variable_rhs_assignment() {
    // the exact form used by the hardware-verified campaign writers
    let out =
        lines("LOCAL r := \"body\";\n         Notes(\"N2\"):=r;\n         PRINT(Notes(\"N2\"));");
    assert_eq!(out, v(&["body"]));
}

#[test]
fn notes_read_missing_raises() {
    // HARDWARE-VERIFIED (2026-10-23, G1 build 15157, probe ZZT_PROBE2):
    // reading a note that does not exist raises a catchable runtime error.
    // (An earlier session recorded the opposite; the direct IFERR probe is
    // the stronger evidence. IFERR catches it.)
    let kind = error_kind("PRINT(Notes(\"ZZT_NO_SUCH_NOTE\"));");
    assert_eq!(kind, "Undefined");
}

#[test]
fn iferr_catches_missing_note_read() {
    // the exact probe pattern that proved the behaviour on the calculator
    let out = lines(
        "LOCAL r,ok;\n         r:=\"?\";\n         ok:=1;\n         IFERR r:=Notes(\"ZZT_NO_SUCH_NOTE\"); THEN ok:=0; END;\n         IF ok THEN PRINT(\"readback:[\"+r+\"]\"); ELSE PRINT(\"readback:ERR\"); END;",
    );
    assert_eq!(out, v(&["readback:ERR"]));
}

#[test]
fn notes_empty_assignment_deletes() {
    // HARDWARE-VERIFIED on G1 (probe ZZT_PROBE2): both the literal and the
    // variable-empty forms delete; the deletion is confirmed by the
    // missing-note read raising afterwards.
    let out = lines(
        "Notes(\"N3\"):=\"gone\";\n         Notes(\"N3\"):=\"\";\n         Notes(\"N3\"):=\"x\";\n         PRINT(Notes(\"N3\"));",
    );
    assert_eq!(out, v(&["x"]));
}

#[test]
fn notes_without_argument_lists_names() {
    // DOCUMENTED (User Guide ch. 28), UNVERIFIED on G1: `Notes` alone is the
    // list of note names.
    let out = lines(
        "Notes(\"N4\"):=\"a\";\n         Notes(\"N5\"):=\"b\";\n         PRINT(SIZE(Notes()));",
    );
    assert_eq!(out, v(&["2"]));
}

#[test]
fn programs_assignment_read_and_delete() {
    // HARDWARE-VERIFIED on G1 (probe ZZT_PROBE3): Programs() assignment
    // with an empty value deletes the program. Reads of missing programs
    // raise like missing notes.
    let out = lines(
        "Programs(\"P1\"):=\"body\";\n         PRINT(Programs(\"P1\"));\n         Programs(\"P1\"):=\"\";\n         PRINT(\"deleted\");",
    );
    assert_eq!(out, v(&["body", "deleted"]));
    assert_eq!(
        error_kind("Programs(\"P1\"):=\"body\"; Programs(\"P1\"):=\"\"; PRINT(Programs(\"P1\"));"),
        "Undefined"
    );
}

#[test]
fn notes_and_programs_namespaces_are_separate() {
    let out = lines(
        "Notes(\"S\"):=\"note\";\n         Programs(\"S\"):=\"prog\";\n         PRINT(Notes(\"S\"));\n         PRINT(Programs(\"S\"));",
    );
    assert_eq!(out, v(&["note", "prog"]));
}

// ---------------------------------------------------------------------------
// String commands used by digit-by-digit converters (Baseconv)
// ---------------------------------------------------------------------------

#[test]
fn string_edge_cases() {
    // Positions are 1-based and count characters, not bytes.
    // UNVERIFIED on G1: out-of-range lengths clamp instead of erroring;
    // INSTRING with an empty needle returns 0; UPPER maps non-ASCII.
    assert_eq!(
        lines(
            r#"PRINT(DIM(""));
               PRINT(MID("FF",2,1)); PRINT(MID("ABC",4)); PRINT(DIM(MID("ABC",4,2)));
               PRINT(MID("ABC",2)); PRINT(LEFT("A",0) + "|"); PRINT(LEFT("AB",9));
               PRINT(INSTRING("0123456789ABCDEF","G")); PRINT(INSTRING("0123456789ABCDEF","A"));
               PRINT(INSTRING("abc","")); PRINT(INSTRING("","a"));
               PRINT(UPPER("7b.8")); PRINT(UPPER("äb")); PRINT(DIM("₂é"));
               PRINT(MID("₂é",2,1));"#
        ),
        v(&[
            "0", "F", "", "0", "BC", "|", "AB", "0", "11", "0", "0", "7B.8", "ÄB", "2", "é",
        ])
    );
    // UNVERIFIED on G1: a start position below 1 is an argument error.
    assert_eq!(error_kind(r#"PRINT(MID("ABC",0));"#), "BadArgValue");
}

#[test]
fn string_concatenation_and_conversion() {
    // `+` with a string operand concatenates; STRING of a number is its
    // terminal form; IP truncates toward zero.
    assert_eq!(
        lines(
            r#"PRINT("Base " + STRING(16)); PRINT("." + "1000");
               PRINT(STRING(10.5)); PRINT(IP(-2.5)); PRINT(IP(7/2));
               PRINT(255 MOD 16); PRINT(IP(255/16));"#
        ),
        v(&["Base 16", ".1000", "10.5", "-2", "3", "15", "15"])
    );
}
