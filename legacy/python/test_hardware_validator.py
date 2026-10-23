import pytest, sys, os
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from src.ppl_emulator.hardware_validator import hardware_validate


def _code(body):
    parts = ["EXPORT T()", "BEGIN", body, "END;"]
    return chr(10).join(parts)


def _errs(code):
    return [i for i in hardware_validate(code) if i.severity =="ERROR"]


def _warns(code):
    return [i for i in hardware_validate(code) if i.severity =="WARNING"]


# --- Color literal checks ---

def test_hash_color_flagged():
    issues = _errs(_code("  LOCAL c := #FF0000;"))
    assert len(issues) == 1
    assert "Invalid color literal" in issues[0].message
    assert "0xFF0000" in issues[0].message


def test_0x_color_ok():
    assert _errs(_code("  LOCAL c := 0xFF0000;")) == []


def test_hash_in_string_ignored():
    body = "  PRINT(" + chr(34) + "color is #FF0000" + chr(34) + ");"
    assert _errs(_code(body)) == []


# --- Unknown function checks ---

def test_unknown_function_suggestion():
    issues = _errs(_code("  FILLRECT(0, 0, 100, 100);"))
    assert len(issues) == 1
    assert "FILLRECT" in issues[0].message
    assert "RECT_P" in issues[0].message


def test_unknown_function_no_suggestion():
    issues = _errs(_code("  NONEXISTENT_FN(1, 2);"))
    assert len(issues) == 1
    assert "NONEXISTENT_FN" in issues[0].message


def test_known_function_ok():
    assert _errs(_code("  RECT_P(0, 0, 319, 239, 0);")) == []


def test_user_defined_function_ok():
    body = chr(10).join([
        "EXPORT T()",
        "BEGIN",
        "  MyHelper(1);",
        "END;",
        "PROCEDURE MyHelper(x)",
        "BEGIN",
        "  PRINT(x);",
        "END;",
    ])
    assert _errs(body) == []


# --- Non-_P graphic command warnings ---

def test_rect_warns():
    warns = _warns(_code("  RECT(0, 0, 319, 239);"))
    assert any("RECT_P" in w.message for w in warns)


def test_rect_p_no_pixel_warn():
    warns = _warns(_code("  RECT_P(0, 0, 319, 239, 0);"))
    assert not any("pixel" in w.message.lower() for w in warns)


# --- Argument count checks ---

def test_rect_p_too_many_args():
    issues = _errs(_code("  RECT_P(1, 2, 3, 4, 5, 6, 7, 8);"))
    assert len(issues) == 1
    assert "Wrong number" in issues[0].message
    assert "got 8" in issues[0].message


def test_rect_p_too_few_args():
    issues = _errs(_code("  RECT_P(0, 0, 100);"))
    assert any("got 3" in i.message for i in issues)


def test_print_zero_args_ok():
    assert _errs(_code("  PRINT();")) == []


def test_abs_wrong_arg_count():
    issues = _errs(_code("  ABS(1, 2);"))
    assert len(issues) == 1
    assert "got 2, expected 1" in issues[0].message


def test_rgb_correct_args():
    assert _errs(_code("  LOCAL c := RGB(255, 0, 128);")) == []


def test_ifte_wrong_args():
    issues = _errs(_code("  LOCAL x := IFTE(1 > 0, 5);"))
    assert issues


# --- Combo test ---

def test_test_strict_hpprgm():
    path = os.path.join(os.path.dirname(__file__), "test_strict.hpprgm")
    if not os.path.exists(path):
        pytest.skip("test_strict.hpprgm not found")
    with open(path, encoding="utf-8", errors="replace") as f:
        code = f.read()
    issues = hardware_validate(code)
    errors = [i for i in issues if i.severity =="ERROR"]
    warns  = [i for i in issues if i.severity =="WARNING"]
    assert len(errors) == 3, f"Expected 3 errors, got {[(i.line_no, i.message) for i in errors]}"
    assert len(warns)  == 1, f"Expected 1 warning, got {[(i.line_no, i.message) for i in warns]}"
