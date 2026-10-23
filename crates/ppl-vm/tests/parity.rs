use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::SourceFile;
use ppl_vm::{AngleMode, HeadlessHost, Limits, Machine, Settings, Value};
use std::rc::Rc;

fn run_ppl(code: &str) -> (Vec<String>, Value, Machine) {
    let file = SourceFile::new("<test>", code.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    if diags.has_errors() {
        for err in diags.errors() {
            eprintln!("Compile error: {}", err.message);
        }
        panic!("Compilation failed");
    }
    let prog = prog.expect("compiled program");
    let mut m = Machine::new(
        Rc::new(prog),
        Box::new(HeadlessHost::new()),
        Settings::default(),
        Limits {
            max_steps: Some(1_000_000),
            ..Limits::default()
        },
    );
    let val = m
        .call_function("TEST_PROG", Vec::new())
        .expect("runtime success");
    (m.terminal.clone(), val, m)
}

fn run_ppl_degrees(code: &str) -> (Vec<String>, Value, Machine) {
    let file = SourceFile::new("<test>", code.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    if diags.has_errors() {
        panic!("Compilation failed");
    }
    let prog = prog.expect("compiled program");
    let settings = Settings {
        angle: AngleMode::Degrees,
        ..Settings::default()
    };
    let mut m = Machine::new(
        Rc::new(prog),
        Box::new(HeadlessHost::new()),
        settings,
        Limits {
            max_steps: Some(1_000_000),
            ..Limits::default()
        },
    );
    let val = m
        .call_function("TEST_PROG", Vec::new())
        .expect("runtime success");
    (m.terminal.clone(), val, m)
}

fn try_run_ppl(code: &str) -> Result<(Vec<String>, Value), String> {
    let file = SourceFile::new("<test>", code.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    if diags.has_errors() {
        return Err(format!(
            "compile error: {}",
            diags.errors().next().unwrap().message
        ));
    }
    let prog = prog.expect("compiled program");
    let mut m = Machine::new(
        Rc::new(prog),
        Box::new(HeadlessHost::new()),
        Settings::default(),
        Limits {
            max_steps: Some(10_000),
            ..Limits::default()
        },
    );
    m.call_function("TEST_PROG", Vec::new())
        .map(|v| (m.terminal.clone(), v))
        .map_err(|e| format!("{e:?}"))
}

fn assert_compile_fails(code: &str, expected_substring: &str) {
    let file = SourceFile::new("<test>", code.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    assert!(
        prog.is_none() || diags.has_errors(),
        "Expected compilation failure for code:\n{code}"
    );
    let msgs: Vec<String> = diags.errors().map(|e| e.message.to_lowercase()).collect();
    let found = msgs
        .iter()
        .any(|m| m.contains(&expected_substring.to_lowercase()));
    assert!(
        found,
        "Expected error containing `{expected_substring}`, but got errors: {msgs:?}"
    );
}

fn assert_compile_warns(code: &str, expected_substring: &str) {
    let file = SourceFile::new("<test>", code.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    assert!(
        prog.is_some() && !diags.has_errors(),
        "Expected compilation success with warnings for code:\n{code}"
    );
    let msgs: Vec<String> = diags.warnings().map(|e| e.message.to_lowercase()).collect();
    let found = msgs
        .iter()
        .any(|m| m.contains(&expected_substring.to_lowercase()));
    assert!(
        found,
        "Expected warning containing `{expected_substring}`, but got warnings: {msgs:?}"
    );
}

// ============================================================================
// 17. COMPILER / PARSER / LINTER STRICTNESS CHECKS (Original: test_strict_mode.py)
// ============================================================================

#[test]
fn test_strict_lvalue_validation() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      5 := A;
    END;
    "#;
    assert_compile_fails(code, "l-value");
}

#[test]
fn test_strict_parameter_shadowing_fails() {
    let code = r#"
    EXPORT TEST_PROG(SIN)
    BEGIN
      PRINT(SIN);
    END;
    "#;
    assert_compile_fails(code, "shadows");
}

#[test]
fn test_local_shadowing_is_warning_only() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL SIN := 42;
      PRINT(SIN);
      RETURN SIN;
    END;
    "#;
    assert_compile_warns(code, "hides");
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["42"]);
}

#[test]
fn test_strict_for_loop_missing_do() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL I;
      FOR I FROM 1 TO 10 BEGIN
        PRINT(I);
      END;
    END;
    "#;
    assert_compile_fails(code, "DO");
}

#[test]
fn test_strict_unclosed_begin() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT("Hello");
    "#;
    assert_compile_fails(code, "BEGIN");
}

#[test]
fn test_strict_unclosed_string() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT("hello);
    END;
    "#;
    assert_compile_fails(code, "string");
}

// ============================================================================
// 18. BOOLEAN PRINTING QUIRK (Original: test_makelist_string_expression...)
// ============================================================================

#[test]
fn test_boolean_is_one_or_zero_not_true_false() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      // In Python emulator: PRINT(1 == 1) printed "True"
      // In HP Prime and Rust emulator: boolean comparisons yield numeric 1 or 0
      PRINT(1 == 1);
      PRINT(1 == 2);
      PRINT("hello" == "hello");
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["1", "0", "1"]);
}

// ============================================================================
// 19. GROB SIZING AND MANIPULATION (Original: test_runtime_parity.py)
// ============================================================================

#[test]
fn test_dimgrob_and_sizes() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      DIMGROB_P(G1, 40, 25, #00FF00h);
      PRINT(GROBW_P(G1));
      PRINT(GROBH_P(G1));
      BLIT_P(G1, 0, 0);
      PRINT(GETPIX_P(5, 5));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["40", "25", "65280"]); // 65280 is 0x00FF00
}

// ============================================================================
// 20. EMPTY CODE & UNICODE IDENTIFIERS
// ============================================================================

#[test]
fn test_empty_program_executes() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
    END;
    "#;
    let (_, val, _) = run_ppl(code);
    assert!(val.equals(&Value::zero()));
}

#[test]
fn test_unicode_identifiers() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL ρ := 10;
      ρ := ρ + 5;
      PRINT(ρ);
      RETURN ρ;
    END;
    "#;
    let (lines, val, _) = run_ppl(code);
    assert_eq!(lines, vec!["15"]);
    assert!(val.equals(&Value::from_i64(15)));
}

// ============================================================================
// 1. PRINT & ARITHMETIC TESTS (Original: TestPrint, TestBuiltins, TestNewBuiltins)
// ============================================================================

#[test]
fn test_print_number_and_expression() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(42);
      PRINT(2 + 3);
      PRINT("hello");
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["42", "5", "hello"]);
}

#[test]
fn test_math_operators() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(10 MOD 3);
      PRINT(10 DIV 3);
      PRINT(2^3);
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["1", "3", "8"]);
}

#[test]
fn test_basic_builtins_math() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(ABS(-7));
      PRINT(MAX(3, 9));
      PRINT(MIN(3, 9));
      PRINT(SQRT(16));
      PRINT(FLOOR(3.7));
      PRINT(CEILING(3.2));
      PRINT(SIGN(5));
      PRINT(SIGN(0));
      PRINT(SIGN(-3));
      PRINT(TRUNCATE(3.789, 2));
      PRINT(ROUND(3.456, 2));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(
        lines,
        vec!["7", "9", "3", "4", "3", "4", "1", "0", "-1", "3.78", "3.46"]
    );
}

#[test]
fn test_ifte() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(IFTE(1 > 0, "yes", "no"));
      PRINT(IFTE(1 < 0, "yes", "no"));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["yes", "no"]);
}

#[test]
fn test_postfix_factorial_and_powers() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := 5;
      PRINT(x!);
      LOCAL y := 3;
      PRINT(y²);
      PRINT(2³);
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["120", "9", "8"]);
}

// ============================================================================
// 2. VARIABLES & SCOPING (Original: TestVariables, TestLocalFunctions)
// ============================================================================

#[test]
fn test_local_variables_and_reassign() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := 10;
      LOCAL a := 1, b := 2;
      x := x + a + b;
      PRINT(x);
      RETURN x;
    END;
    "#;
    let (lines, val, _) = run_ppl(code);
    assert_eq!(lines, vec!["13"]);
    assert!(val.equals(&Value::from_i64(13)));
}

#[test]
fn test_store_operator() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x;
      42 ▶ x;
      PRINT(x);
      LOCAL L := {10, 20, 30};
      99 ▶ L(2);
      PRINT(L(2));
      RETURN x;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["42", "99"]);
}

#[test]
fn test_calculator_global_variables() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      A := 100;
      B := A + 50;
      PRINT(B);
      RETURN B;
    END;
    "#;
    let (lines, val, _) = run_ppl(code);
    assert_eq!(lines, vec!["150"]);
    assert!(val.equals(&Value::from_i64(150)));
}

// ============================================================================
// 3. CONTROL FLOW (Original: TestControlFlow)
// ============================================================================

#[test]
fn test_if_then_else() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := 5;
      IF x > 3 THEN
        PRINT("branch1");
      END;
      IF x > 10 THEN
        PRINT("big");
      ELSE
        PRINT("small");
      END;
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["branch1", "small"]);
}

#[test]
fn test_case_statement() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := 2;
      CASE
        IF x == 1 THEN PRINT("one"); END;
        IF x == 2 THEN PRINT("two"); END;
        DEFAULT PRINT("other");
      END;
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["two"]);
}

#[test]
fn test_for_loop_step_and_downto() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL s := 0;
      LOCAL i;
      FOR i FROM 1 TO 5 DO
        s := s + i;
      END;
      PRINT(s);

      LOCAL step_out := "";
      FOR i FROM 0 TO 6 STEP 2 DO
        step_out := step_out + STRING(i);
      END;
      PRINT(step_out);

      LOCAL down_out := "";
      FOR i FROM 5 DOWNTO 1 DO
        down_out := down_out + STRING(i);
      END;
      PRINT(down_out);

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["15", "0246", "54321"]);
}

#[test]
fn test_while_and_repeat_loops() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := 0;
      WHILE x < 3 DO
        x := x + 1;
      END;
      PRINT(x);

      LOCAL y := 0;
      REPEAT
        y := y + 2;
      UNTIL y >= 6;
      PRINT(y);

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["3", "6"]);
}

// ============================================================================
// 4. FUNCTIONS & RECURSION (Original: TestFunctions, TestLocalFunctions)
// ============================================================================

#[test]
fn test_helper_and_recursive_functions() {
    let code = r#"
    add(a, b)
    BEGIN
      RETURN a + b;
    END;

    fact(n)
    BEGIN
      IF n <= 1 THEN
        RETURN 1;
      END;
      RETURN n * fact(n - 1);
    END;

    EXPORT TEST_PROG()
    BEGIN
      PRINT(add(3, 4));
      PRINT(fact(5));
      PRINT(double_val(6));
      RETURN 0;
    END;

    double_val(x)
    BEGIN
      RETURN x * 2;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["7", "120", "12"]);
}

// ============================================================================
// 5. STRINGS (Original: TestNewBuiltins, strings.rs)
// ============================================================================

#[test]
fn test_string_builtins() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL s := "HelloWorld";
      PRINT(LEFT(s, 5));
      PRINT(RIGHT(s, 5));
      PRINT(MID(s, 6, 5));
      PRINT(DIM(s));
      PRINT(INSTRING(s, "World"));
      PRINT(UPPER("abc"));
      PRINT(LOWER("XYZ"));
      PRINT(CHR(65));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(
        lines,
        vec!["Hello", "World", "World", "10", "6", "ABC", "xyz", "A"]
    );
}

// ============================================================================
// 6. LISTS & MAKELIST (Original: TestLists, TestNewBuiltins, lists.rs)
// ============================================================================

#[test]
fn test_lists_and_makelist() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL L := {10, 20, 30};
      PRINT(L(1));
      PRINT(L(2));
      PRINT(L(3));
      PRINT(SIZE(L));

      L(2) := 99;
      PRINT(L(2));

      // MAKELIST(expr, var, start, end[, step])
      LOCAL k;
      LOCAL squares := MAKELIST(k * k, k, 1, 4);
      PRINT(SIZE(squares));
      PRINT(squares(1));
      PRINT(squares(2));
      PRINT(squares(3));
      PRINT(squares(4));

      // SORT and REVERSE return new lists
      LOCAL unsorted := {3, 1, 2};
      LOCAL sorted := SORT(unsorted);
      PRINT(sorted(1));
      PRINT(sorted(2));
      PRINT(sorted(3));

      LOCAL rev := REVERSE({1, 2, 3});
      PRINT(rev(1));
      PRINT(rev(2));
      PRINT(rev(3));

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(
        lines,
        vec![
            "10", "20", "30", "3", "99", "4", "1", "4", "9", "16", "1", "2", "3", "3", "2", "1"
        ]
    );
}

// ============================================================================
// 7. MATRICES (Original: ops.rs, matrices.rs)
// ============================================================================

#[test]
fn test_matrix_operations() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL M := [[1, 2], [3, 4]];
      PRINT(M(1, 1));
      PRINT(M(1, 2));
      PRINT(M(2, 1));
      PRINT(M(2, 2));

      PRINT(DET(M));

      LOCAL T := TRN(M);
      PRINT(T(1, 2));
      PRINT(T(2, 1));

      LOCAL zeroMat := MAKEMAT(0, 2, 2);
      PRINT(zeroMat(1, 1));

      LOCAL I := IDENMAT(2);
      PRINT(I(1, 1));
      PRINT(I(1, 2));

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(
        lines,
        vec!["1", "2", "3", "4", "-2", "3", "2", "0", "1", "0"]
    );
}

// ============================================================================
// 8. GRAPHICS (Original: TestGraphics, TestNewGraphics)
// ============================================================================

#[test]
fn test_graphics_primitives() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      RECT();
      PIXON_P(10, 20, #FF0000h);
      PRINT(GETPIX_P(10, 20));
      PIXOFF_P(10, 20);
      PRINT(GETPIX_P(10, 20));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines[0], "16711680");
    assert_eq!(lines[1], "16777215");
}

// ============================================================================
// 9. ERROR HANDLING & IFERR
// ============================================================================

#[test]
fn test_iferr_handling() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL ok := 1;
      IFERR
        LOCAL x := 1 / 0;
      THEN
        ok := 0;
      END;
      PRINT(ok);
      RETURN ok;
    END;
    "#;
    let (lines, val, _) = run_ppl(code);
    assert_eq!(lines, vec!["0"]);
    assert!(val.equals(&Value::from_i64(0)));
}

#[test]
fn test_division_by_zero_fails_without_iferr() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := 1 / 0;
      RETURN x;
    END;
    "#;
    assert!(try_run_ppl(code).is_err());
}

#[test]
fn test_index_out_of_bounds_fails() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL L := {1, 2, 3};
      RETURN L(10);
    END;
    "#;
    assert!(try_run_ppl(code).is_err());
}

#[test]
fn test_step_budget_limit() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL i := 0;
      WHILE 1 DO
        i := i + 1;
      END;
      RETURN i;
    END;
    "#;
    assert!(try_run_ppl(code).is_err());
}

// ============================================================================
// 10. ADVANCED MATH & ANGLE MODES
// ============================================================================

#[test]
fn test_trig_degrees() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(SIN(90));
      PRINT(COS(0));
      PRINT(COS(180));
      PRINT(TAN(45));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl_degrees(code);
    assert_eq!(lines, vec!["1", "1", "-1", "1"]);
}

#[test]
fn test_exp_and_log() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(LN(1));
      PRINT(LOG(100));
      PRINT(ALOG(2));
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["0", "2", "100"]);
}

#[test]
fn test_bitwise_and_base_conversions() {
    let code = r##"
    EXPORT TEST_PROG()
    BEGIN
      PRINT(BITSL(3, 2));
      PRINT(BITSR(16, 2));
      PRINT(STRING(R→B(10)));
      PRINT(B→R(#1010b));
      PRINT(B→R(#Ah));
      RETURN 0;
    END;
    "##;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["12", "4", "#Ah", "10", "10"]);
}

// ============================================================================
// 11. ADVANCED MATRIX OPERATIONS (Original: ops.rs)
// ============================================================================

#[test]
fn test_matrix_arithmetic_mul_add() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL A := [[1, 2], [3, 4]];
      LOCAL B := [[2, 0], [1, 2]];
      LOCAL C := A + B;
      PRINT(C(1, 1));
      PRINT(C(1, 2));
      PRINT(C(2, 1));
      PRINT(C(2, 2));

      // Matrix multiplication
      LOCAL D := A * B;
      PRINT(D(1, 1));
      PRINT(D(1, 2));
      PRINT(D(2, 1));
      PRINT(D(2, 2));

      // Scalar multiplication
      LOCAL S := A * 2;
      PRINT(S(1, 1));
      PRINT(S(2, 2));

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(
        lines,
        vec![
            "3", "2", "4", "6", // C = A + B
            "4", "4", "10", "8", // D = A * B
            "2", "8" // S = A * 2
        ]
    );
}

// ============================================================================
// 12. ADVANCED LIST & COLLECTION OPERATIONS
// ============================================================================

#[test]
fn test_list_head_tail_append_pos() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL L := {10, 20, 30};
      PRINT(HEAD(L));
      LOCAL T := TAIL(L);
      PRINT(T(1));
      PRINT(T(2));

      LOCAL A := APPEND(L, 40);
      PRINT(SIZE(A));
      PRINT(A(4));

      PRINT(POS(L, 20));
      PRINT(POS(L, 99));

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["10", "20", "30", "4", "40", "2", "0"]);
}

// ============================================================================
// 13. BREAK & CONTINUE IN LOOPS
// ============================================================================

#[test]
fn test_break_and_continue() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL i;
      LOCAL out_break := "";
      FOR i FROM 1 TO 10 DO
        IF i == 4 THEN
          BREAK;
        END;
        out_break := out_break + STRING(i);
      END;
      PRINT(out_break);

      LOCAL out_cont := "";
      FOR i FROM 1 TO 5 DO
        IF i == 3 THEN
          CONTINUE;
        END;
        out_cont := out_cont + STRING(i);
      END;
      PRINT(out_cont);

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines, vec!["123", "1245"]);
}

// ============================================================================
// 14. 12-DIGIT DECIMAL PRECISION (HP Prime BCD vs Python float)
// ============================================================================

#[test]
fn test_precision_12_digits() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      // 1/3 in 12-digit decimal Real is exactly 0.333333333333
      PRINT(1/3);
      // Multiplying by 3 yields 0.999999999999 in 12-digit decimal (unlike Python float which gives 1.0)
      PRINT(1/3 * 3);
      // Exact decimal addition: 0.1 + 0.2 is exactly 0.3 (unlike IEEE float 0.30000000000000004)
      PRINT(0.1 + 0.2);
      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines[0], "0.333333333333");
    assert_eq!(lines[1], "0.999999999999");
    assert_eq!(lines[2], "0.3");
}

// ============================================================================
// 15. ERROR CONDITIONS: DIMENSION MISMATCH & DOMAIN ERRORS
// ============================================================================

#[test]
fn test_dimension_mismatch_error() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL A := [[1, 2]];
      LOCAL B := [[1, 2], [3, 4]];
      LOCAL C := A + B;
      RETURN 0;
    END;
    "#;
    assert!(try_run_ppl(code).is_err());
}

#[test]
fn test_sqrt_negative_in_real_mode() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      LOCAL x := SQRT(-4);
      RETURN x;
    END;
    "#;
    assert!(try_run_ppl(code).is_err());
}

// ============================================================================
// 16. GRAPHICS: LINE, CIRCLE, INVERT, GROB
// ============================================================================

#[test]
fn test_graphics_draw_primitives() {
    let code = r#"
    EXPORT TEST_PROG()
    BEGIN
      RECT();
      LINE_P(0, 0, 10, 0, #000000h);
      PRINT(GETPIX_P(5, 0));
      PRINT(GETPIX_P(5, 5));

      FILLCIRCLE_P(50, 50, 10, #0000FFh);
      PRINT(GETPIX_P(50, 50));

      INVERT_P(50, 50, 50, 50);
      PRINT(GETPIX_P(50, 50));

      RETURN 0;
    END;
    "#;
    let (lines, _, _) = run_ppl(code);
    assert_eq!(lines[0], "0"); // Black line pixel
    assert_eq!(lines[1], "16777215"); // White off-line pixel
    assert_eq!(lines[2], "255"); // Blue pixel (0x0000FF)
    assert_eq!(lines[3], "16776960"); // Inverted blue pixel (0xFFFF00)
}
