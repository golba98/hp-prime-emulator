import re
from dataclasses import dataclass

from src.ppl_emulator.hpprime_specs import (
    COMMAND_ARITY as HARDWARE_FUNCTIONS,
    command_accepts_arity,
    command_expected_arity,
    HARDWARE_SUGGESTIONS as _SUGGESTIONS,
    PIXEL_VARIANTS as _HAS_PIXEL_VARIANT,
    STRUCTURAL_KEYWORDS as _STRUCTURAL_KEYWORDS,
)


@dataclass
class HardwareIssue:
    severity: str
    message:  str
    line_no:  int


def _strip_strings(text):
    """Replace the contents of string literals with spaces so regex
    patterns never accidentally match text inside quoted strings.
    Handles PPL double-escaped quotes (two consecutive double-quotes).
    """
    result = list(text)
    in_str = False
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == '"':  # quote character
            if not in_str:
                in_str = True
            else:
                # Check for PPL escaped quote: ""
                if i + 1 < len(text) and text[i + 1] == '"':
                    result[i] = result[i + 1] = " "
                    i += 2
                    continue
                else:
                    in_str = False
        elif in_str:
            result[i] = " "
        i += 1
    return "".join(result)


def _strip_comment(line):
    """Remove a PPL // line comment, leaving string literals intact."""
    in_str = False
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            in_str = not in_str
        elif not in_str and ch == "/" and i + 1 < len(line) and line[i + 1] == "/":
            return line[:i]
        i += 1
    return line


def _count_args(args_str):
    """Count top-level comma-separated arguments in an argument string."""
    s = args_str.strip()
    if not s:
        return 0
    depth = 0
    in_str = False
    count = 1
    for ch in s:
        if ch == '"':
            in_str = not in_str
        elif not in_str:
            if ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
            elif ch == "," and depth == 0:
                count += 1
    return count


def _collect_user_functions(code):
    """Scan for EXPORT/PROCEDURE function declarations and return their names (uppercase)."""
    names = set()
    for line in code.splitlines():
        stripped = _strip_comment(line).strip()
        m = re.match(r"^(?:EXPORT|PROCEDURE)\s+(\w+)\s*\(", stripped, re.IGNORECASE)
        if m:
            names.add(m.group(1).upper())
    return names


def hardware_validate(code):
    """Run all hardware compatibility checks. Returns list of HardwareIssue."""
    issues = []
    user_fns = _collect_user_functions(code)

    for line_no, raw_line in enumerate(code.splitlines(), 1):
        no_comment = _strip_comment(raw_line)
        clean = _strip_strings(no_comment)

        # Check 1: invalid color literals starting with #
        for m in re.finditer(r"(?<![0-9A-Za-z_])#([0-9A-Fa-f]{3,8})\b", clean):
            hex_val = m.group(1)
            issues.append(HardwareIssue(
                severity="ERROR",
                message=(
                    f"Invalid color literal '#{hex_val}'. "
                    f"Use '0x' prefix for hex colors on HP Prime hardware "
                    f"(e.g. 0x{hex_val.upper()})"
                ),
                line_no=line_no,
            ))

        # Check 2 & 3: function call validation
        for m in re.finditer(r"\b([A-Za-z_]\w*)\s*\(", clean):
            fn_name = m.group(1).upper()
            if fn_name in _STRUCTURAL_KEYWORDS:
                continue
            if fn_name in user_fns:
                continue

            # 2a: unknown function
            if fn_name not in HARDWARE_FUNCTIONS:
                if fn_name in _SUGGESTIONS:
                    issues.append(HardwareIssue(
                        severity="ERROR",
                        message=f"Unknown function '{m.group(1)}'. Did you mean '{_SUGGESTIONS[fn_name]}'?",
                        line_no=line_no,
                    ))
                else:
                    issues.append(HardwareIssue(
                        severity="ERROR",
                        message=f"Unknown function '{m.group(1)}' is not a valid HP Prime G1/G2 built-in",
                        line_no=line_no,
                    ))
                continue

            # 2b: non-_P graphic command warning
            if fn_name in _HAS_PIXEL_VARIANT:
                pixel_v = _HAS_PIXEL_VARIANT[fn_name]
                issues.append(HardwareIssue(
                    severity="WARNING",
                    message=f"'{m.group(1)}' uses screen coordinates. Consider '{pixel_v}' for pixel-exact rendering on hardware",
                    line_no=line_no,
                ))

            # 3: argument count validation
            call_start = m.end()
            # Walk forward past the argument list to find the closing paren
            depth = 1
            in_string = False
            i = call_start
            while i < len(clean) and depth > 0:
                ch = clean[i]
                if ch == '"':
                    in_string = not in_string
                elif not in_string:
                    if ch == "(":
                        depth += 1
                    elif ch == ")":
                        depth -= 1
                i += 1
            if depth == 0:
                raw_args = no_comment[call_start: i - 1]
                nargs = _count_args(raw_args)
                min_a, max_a = HARDWARE_FUNCTIONS[fn_name]
                if not command_accepts_arity(fn_name, nargs):
                    expected = command_expected_arity(fn_name)
                    issues.append(HardwareIssue(
                        severity="ERROR",
                        message=f"Wrong number of arguments for '{m.group(1)}': got {nargs}, expected {expected}",
                        line_no=line_no,
                    ))

    return issues
