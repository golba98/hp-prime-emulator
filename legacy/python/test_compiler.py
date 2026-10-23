#!/usr/bin/env python3
"""
HP Prime PPL Test Suite
=======================
Run:  py -m pytest test_ppl.py -v
      py -m pytest test_ppl.py -v -k "test_print"   (run one test)

Each test writes PPL code, transpiles it, executes it, and checks:
  - stdout PRINT output  (via captured stdout)
  - no transpile/runtime crashes
  - optionally, pixel colors on the 320x240 screen
"""

import sys, os, io, math, contextlib, tempfile
import pytest  # pyre-ignore

# Ensure project dir is on path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..')))

from src.ppl_emulator.transpiler import transpile  # pyre-ignore
from src.ppl_emulator.runtime import HPPrimeRuntime, PPLList  # pyre-ignore
from src.ppl_emulator.linter import lint  # pyre-ignore


# ── Helper ────────────────────────────────────────────────────────

def run_ppl(code, out_png=None):
    """
    Transpile + execute PPL code.
    Returns (stdout_text, runtime_instance, transpiled_python).
    Raises on transpile or execution error.
    """
    if out_png is None:
        out_png = os.path.join(tempfile.gettempdir(), "_ppl_test_screen.png")

    py_code = transpile(code, out_path=out_png)

    # Capture stdout (PRINT output)
    buf = io.StringIO()
    ns = {"__name__": "__main__", "__file__": "<test>"}

    with contextlib.redirect_stdout(buf):
        exec(compile(py_code, "<ppl_test>", "exec"), ns)

    # Fish out the runtime instance for pixel checks
    rt = ns.get("_rt")
    return buf.getvalue(), rt, py_code


def printed_lines(stdout_text):
    """Split captured stdout into non-empty lines, stripping whitespace."""
    return [l.strip() for l in stdout_text.splitlines() if l.strip()]


# ── PRINT / math tests ───────────────────────────────────────────

class TestPrint:

    def test_print_number(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(42);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "42" in out

    def test_print_expression(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(2+3);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "5" in out

    def test_print_string(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT("hello");
        END;
        """
        out, _, _ = run_ppl(code)
        assert "hello" in out

    def test_print_multiple(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(1);
          PRINT(2);
          PRINT(3);
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["1", "2", "3"]

    def test_math_ops(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(10 MOD 3);
          PRINT(10 DIV 3);
          PRINT(2^3);
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["1", "3", "8"]


# ── Variable / assignment tests ───────────────────────────────────

class TestVariables:

    def test_local_var(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 10;
          PRINT(x);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "10" in out

    def test_reassign(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 5;
          x := x + 1;
          PRINT(x);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "6" in out

    def test_multiple_locals(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL a := 1, b := 2;
          PRINT(a + b);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "3" in out


# ── Control flow tests ────────────────────────────────────────────

class TestControlFlow:

    def test_if_true(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 5;
          IF x > 3 THEN
            PRINT("yes");
          END;
        END;
        """
        out, _, _ = run_ppl(code)
        assert "yes" in out

    def test_if_false(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 1;
          IF x > 3 THEN
            PRINT("yes");
          END;
          PRINT("done");
        END;
        """
        out, _, _ = run_ppl(code)
        assert "yes" not in out
        assert "done" in out

    def test_if_else(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 2;
          IF x > 5 THEN
            PRINT("big");
          ELSE
            PRINT("small");
          END;
        END;
        """
        out, _, _ = run_ppl(code)
        assert "small" in out

    def test_for_loop(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL s := 0;
          LOCAL i;
          FOR i FROM 1 TO 5 DO
            s := s + i;
          END;
          PRINT(s);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "15" in out

    def test_for_step(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL i;
          FOR i FROM 0 TO 10 STEP 2 DO
            PRINT(i);
          END;
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["0", "2", "4", "6", "8", "10"]

    def test_while_loop(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 0;
          WHILE x < 3 DO
            x := x + 1;
          END;
          PRINT(x);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "3" in out

    def test_repeat_until(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 0;
          REPEAT
            x := x + 1;
          UNTIL x >= 5;
          PRINT(x);
        END;
        """
        out, _, _ = run_ppl(code)
        assert "5" in out


# ── Function call tests ──────────────────────────────────────────

class TestFunctions:

    def test_helper_function(self):
        code = """
        add(a, b)
        BEGIN
          RETURN a + b;
        END;

        EXPORT T()
        BEGIN
          PRINT(add(3, 4));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "7" in out

    def test_recursive_function(self):
        code = """
        fact(n)
        BEGIN
          IF n <= 1 THEN
            RETURN 1;
          END;
          RETURN n * fact(n - 1);
        END;

        EXPORT T()
        BEGIN
          PRINT(fact(5));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "120" in out

    def test_makelist_string_expression_transpiles_and_executes(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL tokensSize := 3;
          LOCAL k;
          LOCAL tokens := MAKELIST("", k, 1, tokensSize);
          PRINT(SIZE(tokens));
          PRINT(tokens(1) == "");
        END;
        """
        out, _, py = run_ppl(code)
        assert "MAKELIST(lambda: PPLString(\"\")" in py
        lines = printed_lines(out)
        assert lines == ["3", "True"]

    def test_postfix_factorial_on_variable_transpiles_and_executes(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL x := 5;
          PRINT(x!);
        END;
        """
        out, _, py = run_ppl(code)
        assert "120" in out
        assert "math.factorial" in py


# ── Built-in math function tests ─────────────────────────────────

class TestBuiltins:

    def test_abs(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(ABS(-7));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "7" in out

    def test_max_min(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(MAX(3, 9));
          PRINT(MIN(3, 9));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["9", "3"]

    def test_sqrt(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(SQRT(16));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "4" in out

    def test_floor_ceiling(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(FLOOR(3.7));
          PRINT(CEILING(3.2));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["3", "4"]

    def test_ifte(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(IFTE(1 > 0, "yes", "no"));
          PRINT(IFTE(1 < 0, "yes", "no"));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["yes", "no"]

    def test_rgb(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(RGB(255, 0, 0));
        END;
        """
        out, _, _ = run_ppl(code)
        assert str(0xFF0000) in out


# ── Graphics tests ────────────────────────────────────────────────

class TestGraphics:

    def test_rect_clears_screen(self):
        """RECT() with no args should clear screen to white."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
        END;
        """
        _, rt, _ = run_ppl(code)
        # Center pixel should be white
        px = rt.img.getpixel((160, 120))  # pyre-ignore
        assert px == (255, 255, 255)

    def test_pixon(self):
        """PIXON_P should set a specific pixel."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          PIXON_P(50, 50, #FF0000h);
        END;
        """
        _, rt, _ = run_ppl(code)
        px = rt.img.getpixel((50, 50))  # pyre-ignore
        assert px == (255, 0, 0)

    def test_line(self):
        """LINE_P should draw a black line."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          LINE_P(0, 0, 100, 0, #000000h);
        END;
        """
        _, rt, _ = run_ppl(code)
        # Pixel on the line should be black
        px = rt.img.getpixel((50, 0))  # pyre-ignore
        assert px == (0, 0, 0)
        # Pixel off the line should be white
        px2 = rt.img.getpixel((50, 50))  # pyre-ignore
        assert px2 == (255, 255, 255)

    def test_filled_rect(self):
        """RECT_P with fill color."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          RECT_P(10, 10, 50, 50, #0000FFh, #0000FFh);
        END;
        """
        _, rt, _ = run_ppl(code)
        px = rt.img.getpixel((30, 30))  # pyre-ignore
        assert px == (0, 0, 255)

    def test_textout(self):
        """TEXTOUT_P should not crash."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          TEXTOUT_P("Hello", 10, 10, 1, #000000h);
        END;
        """
        _, rt, _ = run_ppl(code)
        # Just verify it didn't crash and screen exists
        assert rt.img.size == (320, 240)  # pyre-ignore

    def test_circle(self):
        """FILLCIRCLE_P should fill pixels."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          FILLCIRCLE_P(100, 100, 20, #FF0000h);
        END;
        """
        _, rt, _ = run_ppl(code)
        # Center of circle should be red
        px = rt.img.getpixel((100, 100))  # pyre-ignore
        assert px == (255, 0, 0)


# ── PPL List (1-indexed array) tests ─────────────────────────────

class TestLists:

    def test_list_literal(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL L := {10, 20, 30};
          PRINT(L(1));
          PRINT(L(2));
          PRINT(L(3));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["10", "20", "30"]

    def test_local_array(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL A[5];
          A(1) := 99;
          A(2) := 42;
          PRINT(A(1));
          PRINT(A(2));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["99", "42"]


# ── Transpiler error handling ─────────────────────────────────────

class TestTranspiler:

    def test_empty_code_still_runs(self):
        """An empty EXPORT function should not crash."""
        code = """
        EXPORT T()
        BEGIN
        END;
        """
        out, _, _ = run_ppl(code)
        # No crash = pass

    def test_transpile_produces_python(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(1);
        END;
        """
        py = transpile(code)
        assert "def T(" in py
        assert "PRINT" in py


# ── New Builtins tests ────────────────────────────────────────────

class TestNewBuiltins:

    def test_integer_conversion(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(INTEGER(3.9));
          PRINT(INTEGER(-3.9));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines[0] == "3"
        assert lines[1] == "-3"

    def test_real_conversion(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(REAL(3.14));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "3.14" in out

    def test_sign(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(SIGN(5));
          PRINT(SIGN(0));
          PRINT(SIGN(-3));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["1", "0", "-1"]

    def test_truncate(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(TRUNCATE(3.789, 2));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "3.78" in out

    def test_asc_chr(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(ASC("A"));
          PRINT(CHR(65));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines[0] == "65"
        assert lines[1] == "A"

    def test_trim(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(TRIM("  hello  "));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "hello" in out
        assert "  hello  " not in out

    def test_startswith_endswith(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(STARTSWITH("hello", "he"));
          PRINT(ENDSWITH("hello", "lo"));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["1", "1"]

    def test_sort(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL L := {3, 1, 2};
          SORT(L);
          PRINT(L(1));
          PRINT(L(2));
          PRINT(L(3));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["1", "2", "3"]

    def test_reverse(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL R := REVERSE({1, 2, 3});
          PRINT(R(1));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "3" in printed_lines(out)

    def test_addtail(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL L := {1, 2};
          ADDTAIL(L, 99);
          PRINT(SIZE(L));
          PRINT(L(3));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines[0] == "3"
        assert lines[1] == "99"

    def test_bitshift(self):
        code = """
        EXPORT T()
        BEGIN
          PRINT(BITSHIFT(1, 3));
          PRINT(BITSHIFT(8, -2));
        END;
        """
        out, _, _ = run_ppl(code)
        lines = printed_lines(out)
        assert lines == ["8", "2"]


# ── New Graphics tests ────────────────────────────────────────────

class TestNewGraphics:

    def test_textout_renders_pixels(self):
        """TEXTOUT_P should draw at least one dark pixel in the text region."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          TEXTOUT_P("X", 10, 10, 1, #000000h);
        END;
        """
        _, rt, _ = run_ppl(code)
        found_dark = False
        for y in range(10, 26):
            for x in range(10, 26):
                px = rt.img.getpixel((x, y))
                # Newer Pillow load_default() returns an anti-aliased TrueType font,
                # so pixels may be grey rather than pure black.  Accept any channel
                # value < 128 as "dark enough".
                if all(ch < 128 for ch in px[:3]):
                    found_dark = True
                    break
        assert found_dark, "TEXTOUT_P did not render any dark pixels"

    def test_invert_p(self):
        """INVERT_P should flip pixel colors."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          INVERT_P(0, 0, 10, 10);
        END;
        """
        _, rt, _ = run_ppl(code)
        px = rt.img.getpixel((5, 5))
        assert px == (0, 0, 0), f"Expected black, got {px}"

    def test_arc_p(self):
        """ARC_P should run without error."""
        code = """
        EXPORT T()
        BEGIN
          RECT();
          ARC_P(100, 100, 30, 0, 180, #FF0000h, 1);
        END;
        """
        _, rt, _ = run_ppl(code)
        assert rt.img.size == (320, 240)


# ── New Linter checks tests ───────────────────────────────────────

class TestLinterNewChecks:

    def test_unreachable_after_return(self):
        code = "EXPORT T() BEGIN RETURN; PRINT(1); END;"
        issues = lint(code)
        msgs = [i.message.lower() for i in issues]
        assert any("unreachable" in m for m in msgs), f"Expected unreachable warning, got: {issues}"

    def test_deep_nesting_warning(self):
        code = """
        EXPORT T()
        BEGIN
          IF 1 THEN
            IF 1 THEN
              IF 1 THEN
                IF 1 THEN
                  IF 1 THEN
                    PRINT(1);
                  END;
                END;
              END;
            END;
          END;
        END;
        """
        issues = lint(code)
        msgs = [i.message.lower() for i in issues]
        assert any("deeply" in m or "nest" in m or "depth" in m for m in msgs),             f"Expected nesting warning, got: {[i.message for i in issues]}"


# ── Run directly ──────────────────────────────────────────────────


# ── LOCAL function tests ──────────────────────────────────────────────────────

class TestLocalFunctions:

    def test_local_func_after_export(self):
        """LOCAL function defined after EXPORT function is callable."""
        code = """
        EXPORT T()
        BEGIN
          PRINT(double(5));
        END;

        LOCAL double(n)
        BEGIN
          RETURN n * 2;
        END;
        """
        out, _, _ = run_ppl(code)
        assert "10" in printed_lines(out)

    def test_local_func_before_export(self):
        """LOCAL function defined before EXPORT function is callable."""
        code = """
        LOCAL triple(n)
        BEGIN
          RETURN n * 3;
        END;

        EXPORT T()
        BEGIN
          PRINT(triple(4));
        END;
        """
        out, _, _ = run_ppl(code)
        assert "12" in printed_lines(out)

    def test_local_func_no_false_errors(self):
        """LOCAL function definition must not generate spurious lint errors."""
        code = """
        EXPORT MAIN_FUNC()
        BEGIN
          PRINT(sub_func(5));
        END;

        LOCAL sub_func(param)
        BEGIN
          RETURN param * 2;
        END;
        """
        issues = lint(code)
        errors = [i for i in issues if i.severity == "ERROR"]
        assert not errors, f"Unexpected lint errors: {errors}"

    def test_local_func_param_not_reported_as_unused_local(self):
        """Parameter of a LOCAL function must not appear as unused local of parent."""
        code = """
        EXPORT T()
        BEGIN
          PRINT(add(3, 4));
        END;

        LOCAL add(a, b)
        BEGIN
          RETURN a + b;
        END;
        """
        issues = lint(code)
        param_warnings = [i for i in issues if "unused" in i.message.lower() and
                          ("'A'" in i.message or "'B'" in i.message)]
        assert not param_warnings, f"False unused-param warnings: {param_warnings}"
        out, _, _ = run_ppl(code)
        assert "7" in printed_lines(out)

    def test_unicode_identifier_assignments_transpile_and_execute(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL ρ:=0;
          ρ:=ρ+1;
          PRINT(ρ);
        END;
        """
        py = transpile(code)
        assert "_rt.SET_VAR('Ρ', 0, is_local=True)" in py
        out, _, _ = run_ppl(code)
        assert printed_lines(out) == ["1"]

    def test_store_operator_supports_indexed_targets(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL L:={0,0,0};
          7▶L(2);
          PRINT(L(2));
        END;
        """
        py = transpile(code)
        assert ".value[2] = 7" in py
        out, _, _ = run_ppl(code)
        assert printed_lines(out) == ["7"]

    def test_module_scope_variables_are_known_inside_local_functions(self):
        code = """
        LOCAL lines:={"abc"};
        LOCAL F()
        BEGIN
          PRINT(lines(1));
        END;
        EXPORT T()
        BEGIN
          F();
        END;
        """
        py = transpile(code)
        assert "_rt.GET_VAR('LINES'" in py
        out, _, _ = run_ppl(code)
        assert printed_lines(out) == ["abc"]

    def test_inline_if_with_store_operator_else_branch_transpiles(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL s;
          IF 1==1 THEN "A"▶s ELSE "B"▶s;END;
          PRINT(s);
        END;
        """
        out, _, _ = run_ppl(code)
        assert printed_lines(out) == ["A"]

    def test_split_line_if_header_followed_by_inline_else_body_transpiles(self):
        code = """
        EXPORT T()
        BEGIN
          LOCAL s,f:=1;
          IF 1==1 THEN
          "A"▶s;f+1▶f ELSE "B"▶s;END;
          PRINT(s);
        END;
        """
        out, _, _ = run_ppl(code)
        assert printed_lines(out) == ["A"]

    def test_not_equal_does_not_rewrite_as_factorial(self):
        code = """
        EXPORT T()
        BEGIN
          WHILE GETKEY() != 4 DO
            RETURN 1;
          END;
        END;
        """
        py = transpile(code)
        assert "_ppl_factorial(GETKEY())" not in py
        assert "!= 4" in py or "!=4" in py


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-v"]))
