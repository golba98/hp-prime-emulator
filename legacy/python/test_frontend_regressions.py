import os
import subprocess
import sys
import tempfile

import pytest

from src.ppl_emulator.linter import lint


def _run_cli(code: str, extra_args: list[str] | None = None, timeout: int = 30) -> subprocess.CompletedProcess[str]:
    out_png = os.path.join(tempfile.gettempdir(), "_ppl_frontend_regression.png")
    return subprocess.run(
        [
            sys.executable,
            "-m",
            "src.ppl_emulator.cli",
            "--code",
            code,
            "--dump-python",
            "--output",
            out_png,
            *(extra_args or []),
        ],
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
    )


INVALID_CASES = [
    ("missing_semicolon", "EXPORT T()\nBEGIN\nPRINT(1)\nEND;\n", "PARSER", ["missing semicolon"]),
    ("extra_semicolon_after_export_header", "EXPORT TEST10();\nBEGIN\nPRINT(1);\nEND;\n", "PARSER", ["export header"]),
    ("missing_parentheses_in_export_header", "EXPORT TEST11\nBEGIN\nPRINT(1);\nEND;\n", "PARSER", ["export header"]),
    ("invalid_identifier_starts_with_digit", "EXPORT 2TEST()\nBEGIN\nPRINT(1);\nEND;\n", "LEXER", ["invalid identifier"]),
    ("invalid_identifier_contains_hyphen", "EXPORT MY-PROG()\nBEGIN\nPRINT(1);\nEND;\n", "LEXER", ["invalid identifier"]),
    ("missing_end_for_begin_block", "EXPORT T()\nBEGIN\nPRINT(1);\n", "SYNTAX", ["missing end", "unclosed begin"]),
    ("malformed_if_missing_then", "EXPORT T()\nBEGIN\nIF 1=1\nPRINT(1);\nEND;\nEND;\n", "SYNTAX", ["malformed if", "then"]),
    ("malformed_for_missing_do", "EXPORT T()\nBEGIN\nLOCAL I;\nFOR I FROM 1 TO 2\nPRINT(I);\nEND;\nEND;\n", "SYNTAX", ["malformed for", "do"]),
    ("malformed_while_missing_do", "EXPORT T()\nBEGIN\nWHILE 1\nPRINT(1);\nEND;\nEND;\n", "SYNTAX", ["malformed while", "do"]),
    ("repeat_missing_until", "EXPORT T()\nBEGIN\nREPEAT\nPRINT(1);\nEND;\nEND;\n", "SYNTAX", ["repeat", "until"]),
    ("end_used_to_close_repeat", "EXPORT T()\nBEGIN\nREPEAT\nPRINT(1);\nEND;\nEND;\n", "SYNTAX", ["repeat", "until"]),
    ("unclosed_string_literal", 'EXPORT T()\nBEGIN\nPRINT("hello);\nEND;\n', "SYNTAX", ["unclosed string"]),
    ("single_quoted_string_literal", "EXPORT T()\nBEGIN\nMSGBOX('Hello');\nEND;\n", "LEXER", ["double quotes"]),
    ("unmatched_opening_parenthesis", "EXPORT T()\nBEGIN\nPRINT((1+2);\nEND;\n", "SYNTAX", ["unclosed", "("]),
    ("unmatched_closing_parenthesis", "EXPORT T()\nBEGIN\nPRINT(1+2));\nEND;\n", "SYNTAX", ["extra", ")"]),
    ("malformed_local_missing_comma", "EXPORT T()\nBEGIN\nLOCAL A B, C;\nEND;\n", "PARSER", ["local declaration", "comma"]),
    ("undeclared_variable_use", "EXPORT T()\nBEGIN\nLOCAL A;\nB:=7;\nRETURN A+B;\nEND;\n", "SEMANTIC", ["undeclared variable"]),
    ("invalid_assignment_equals_instead_of_colon_equals", "EXPORT T()\nBEGIN\nLOCAL A;\nA=5;\nEND;\n", "PARSER", ["did you mean ':='", "invalid assignment"]),
    ("trailing_comma_in_function_call", 'EXPORT T()\nBEGIN\nMSGBOX("Hi",);\nEND;\n', "PARSER", ["trailing comma"]),
    ("return_outside_program_scope", "RETURN 1;\nEXPORT T()\nBEGIN\nPRINT(1);\nEND;\n", "SYNTAX", ["return outside"]),
    ("invalid_top_level_tokens_before_after_program", "GARBAGE_TOKEN;\nEXPORT T()\nBEGIN\nPRINT(1);\nEND;\nAFTER_TOKEN;\n", "PARSER", ["top-level token"]),
]


@pytest.mark.parametrize("name,code,category,phrases", INVALID_CASES)
def test_invalid_programs_are_rejected_before_transpile(name, code, category, phrases):
    issues = lint(code, filename=f"{name}.hpprgm")
    errors = [i for i in issues if i.severity == "ERROR"]
    assert errors, f"{name}: expected compile errors"
    assert any((i.category or "").upper() == category for i in errors), f"{name}: expected category {category}, got {[i.category for i in errors]}"
    combined_messages = " ".join((i.message + " " + (i.hint or "")).lower() for i in errors)
    for phrase in phrases:
        assert phrase.lower() in combined_messages, f"{name}: expected phrase '{phrase}' in '{combined_messages}'"

    cli = _run_cli(code)
    assert cli.returncode != 0, f"{name}: expected non-zero exit code"
    combined_output = f"{cli.stdout}\n{cli.stderr}".upper()
    assert "TRANSPILED PYTHON" not in combined_output, f"{name}: should fail before transpile"
    assert "✓  FINISHED" not in combined_output, f"{name}: should fail before execution"


def test_multiple_syntax_errors_reports_more_than_one():
    code = "EXPORT 2BAD()\nBEGIN\nIF 1=1\nPRINT('x')\n"
    errors = [i for i in lint(code, filename="multi.hpprgm") if i.severity == "ERROR"]
    assert len(errors) >= 2


def test_shadowing_is_warning_only():
    code = "EXPORT T()\nBEGIN\nLOCAL X;\nX:=1;\nPRINT(X);\nEND;\n"
    issues = lint(code, filename="shadow.hpprgm")
    errors = [i for i in issues if i.severity == "ERROR"]
    warnings = [i for i in issues if i.severity == "WARNING"]
    assert errors == []
    assert warnings

    cli = _run_cli(code)
    assert cli.returncode == 0


def test_valid_control_program_compiles_and_runs():
    code = """
EXPORT T()
BEGIN
  LOCAL I,S:=0;
  FOR I FROM 1 TO 4 DO
    S:=S+I;
  END;
  IF S=10 THEN
    PRINT("OK");
  END;
END;
"""
    issues = lint(code, filename="valid_control.hpprgm")
    assert [i for i in issues if i.severity == "ERROR"] == []
    cli = _run_cli(code)
    assert cli.returncode == 0


def test_valid_local_and_msgbox_program_compiles_and_runs():
    code = """
EXPORT T()
BEGIN
  LOCAL NAME:="Jordan";
  MSGBOX(NAME);
  PRINT(NAME);
END;
"""
    issues = lint(code, filename="valid_msgbox.hpprgm")
    assert [i for i in issues if i.severity == "ERROR"] == []
    cli = _run_cli(code)
    assert cli.returncode == 0


def test_cli_can_override_runtime_time_budget():
    code = """
EXPORT T()
BEGIN
  WHILE 1 DO
  END;
END;
"""
    cli = _run_cli(code, extra_args=["--max-elapsed-seconds", "0.01"], timeout=30)
    assert cli.returncode != 0
    combined_output = f"{cli.stdout}\n{cli.stderr}"
    assert "RESOURCE LIMIT EXCEEDED" in combined_output.upper()
    assert "(TIME)" in combined_output.upper()


def test_cli_can_disable_runtime_time_budget():
    code = """
EXPORT T()
BEGIN
  WAIT(0.05);
  PRINT("OK");
END;
"""
    cli = _run_cli(code, extra_args=["--max-elapsed-seconds", "0.01", "--no-time-limit"], timeout=30)
    assert cli.returncode == 0
    combined_output = f"{cli.stdout}\n{cli.stderr}"
    assert "OK" in combined_output


def test_direct_recursion_emits_warning():
    code = """
EXPORT T()
BEGIN
  T();
END;
"""
    warnings = [i for i in lint(code, filename="recursive.hpprgm") if i.severity == "WARNING"]
    assert any("recursion" in i.message.lower() for i in warnings)


def test_large_string_literal_emits_warning():
    code = "EXPORT T()\nBEGIN\nPRINT(\"" + ("x" * 5000) + "\");\nEND;\n"
    warnings = [i for i in lint(code, filename="large_string.hpprgm") if i.severity == "WARNING"]
    assert any("string literal" in i.message.lower() for i in warnings)


def test_repeated_concatenation_emits_warning():
    code = """
EXPORT T()
BEGIN
  LOCAL s := "";
  s := s + "a";
  s := s + "b";
  s := s + "c";
END;
"""
    warnings = [i for i in lint(code, filename="concat.hpprgm") if i.severity == "WARNING"]
    assert any("concatenation" in i.message.lower() for i in warnings)
