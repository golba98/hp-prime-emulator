import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.ppl_emulator.linter import lint  # pyre-ignore


def _errors(code):
    return [issue for issue in lint(code) if issue.severity == "ERROR"]


def _warnings(code):
    return [issue for issue in lint(code) if issue.severity == "WARNING"]


def test_hash_color_literals_warn_but_do_not_fail_lint():
    code = """
    EXPORT T()
    BEGIN
      RECT_P(0, 0, 10, 10, #FF0000);
    END;
    """
    assert _errors(code) == []
    warns = _warnings(code)
    assert any("hardware compatibility" in issue.message.lower() for issue in warns)


def test_cas_dot_methods_are_not_flagged_as_lowercase_builtin_errors():
    code = """
    EXPORT T()
    BEGIN
      LOCAL d := CAS.diff("X^2", "X");
      LOCAL i := CAS.integrate("X^2", "X");
      LOCAL s := CAS.simplify("(X+1)*(X-1)");
      PRINT(d + i + s);
    END;
    """
    issues = lint(code)
    assert [issue for issue in issues if issue.severity == "ERROR"] == []
    assert not any("UPPERCASE" in issue.message for issue in issues)


def test_bare_function_headers_do_not_require_semicolons():
    code = """
    HELPER()
    BEGIN
      RETURN 1;
    END;

    EXPORT T()
    BEGIN
      PRINT(HELPER());
    END;
    """
    assert _errors(code) == []


def test_pragma_and_forward_declarations_do_not_fail_lint():
    code = """
    #pragma mode(separator(.,;) integer(h32))

    Helper();

    Helper()
    BEGIN
      RETURN 1
    END;

    EXPORT T()
    BEGIN
      PRINT(Helper());
    END;
    """
    errs = _errors(code)
    assert any("Missing semicolon" in issue.message for issue in errs)


def test_single_equals_in_if_is_valid_ppl_equality():
    code = """
    EXPORT T()
    BEGIN
      IF A=2 THEN
        PRINT(1);
      END;
    END;
    """
    assert _errors(code) == []


def test_multiline_if_header_is_linted_as_valid():
    code = """
    EXPORT T()
    BEGIN
      IF
      INPUT({{A,[0]}})
      THEN
        PRINT(A);
      END;
    END;
    """
    assert _errors(code) == []


def test_bare_function_header_with_comment_lines_before_begin_is_valid():
    code = """
    CSV_Parse1(ST, POS1, POSN)
    // comment
    // another comment
    BEGIN
      RETURN ST;
    END;
    """
    assert _errors(code) == []


def test_prime_hash_literals_are_accepted_by_linter():
    code = """
    EXPORT T()
    BEGIN
      LOCAL a := #0;
      LOCAL b := #AF;
      LOCAL c := #AF:16h;
      PRINT(a + b + c);
    END;
    """
    assert _errors(code) == []


def test_lowercase_mod_variable_does_not_look_like_trailing_operator():
    code = """
    EXPORT T()
    BEGIN
      LOCAL mod := 7;
      LOCAL diferencia := 10-mod;
      PRINT(diferencia);
    END;
    """
    assert _errors(code) == []


def test_system_globals_and_helper_builtins_are_not_flagged_unknown():
    code = """
    EXPORT T()
    BEGIN
      LOCAL x := Ans;
      LOCAL langs := MYLANGS;
      LOCAL items := APPEND({}, 1);
      LOCAL head := HEAD("1>Hello");
      LOCAL tail := TAIL("1>Hello");
      LOCAL mask := BITSL(3, 2) + BITSR(16, 2);
      LOCAL mat := MAKEMAT(0, 2, 2);
      TEXT_CLEAR();
      TEXT_AT(1, 1, "OK");
      EDITMAT(mat);
      PRINT(x + SIZE(langs) + SIZE(items) + EXPR(head) + mask + mat(1,1) + SIZE(tail));
    END;
    """
    assert _errors(code) == []
    assert not any("unknown function" in issue.message.lower() for issue in lint(code))


def test_square_bracket_indexing_warns_for_hardware_compatibility():
    code = """
    EXPORT T()
    BEGIN
      LOCAL xs := {1,2,3};
      LOCAL m := {{1,2,3,4,5}};
      xs[2] := 9;
      PRINT(m[1,1]);
    END;
    """
    assert _errors(code) == []
    warns = _warnings(code)
    assert any("invalid input" in issue.message.lower() for issue in warns)
    assert any("expects parentheses" in issue.message.lower() for issue in warns)
