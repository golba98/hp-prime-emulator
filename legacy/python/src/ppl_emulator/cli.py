#!/usr/bin/env python3
# ─────────────────────────────────────────────────────────────────
#  HP Prime PPL Emulator — CLI
#
#  Usage:
#    ppl program.hpprgm
#    ppl program.hpprgm --output out.png
#    ppl --code "EXPORT Hello() BEGIN PRINT(42); END;"
#    ppl program.hpprgm --dump-python
# ─────────────────────────────────────────────────────────────────

import sys
import os
import argparse
import traceback
import re
import threading
import time
import itertools

# Force UTF-8 output so Unicode characters in error messages display correctly
if hasattr(sys.stdout, 'reconfigure'):
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')  # type: ignore
if hasattr(sys.stderr, 'reconfigure'):
    sys.stderr.reconfigure(encoding='utf-8', errors='replace')  # type: ignore

# ── Ensure 0-App root is on sys.path (works both as script and -m module) ──
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))   # .../src/ppl_emulator
_APP_ROOT   = os.path.dirname(os.path.dirname(_SCRIPT_DIR)) # .../0-App
if _APP_ROOT not in sys.path:
    sys.path.insert(0, _APP_ROOT)

from src.ppl_emulator.linter import lint  # pyre-ignore
from src.ppl_emulator.source_loader import read_ppl_file  # pyre-ignore


# ─────────────────────────────────────────────────────────────────
# ─────────────────────────────────────────────────────────────────
#  Colors & UI
# ─────────────────────────────────────────────────────────────────

def _color_enabled():
    """Enable ANSI escape sequences on Windows if possible."""
    if not sys.stdout.isatty():
        return False
    if os.name == 'nt':
        # Standard way to enable ANSI on Windows 10+
        import ctypes
        kernel32 = ctypes.windll.kernel32  # type: ignore
        # ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x0004
        kernel32.SetConsoleMode(kernel32.GetStdHandle(-11), 7)
    return True

_HAS_COLOR = _color_enabled()

_CLR_RED = '\033[91m' if _HAS_COLOR else ''
_CLR_GRN = '\033[92m' if _HAS_COLOR else ''
_CLR_YEL = '\033[93m' if _HAS_COLOR else ''
_CLR_BLU = '\033[94m' if _HAS_COLOR else ''
_CLR_CYN = '\033[96m' if _HAS_COLOR else ''
_CLR_GRY = '\033[90m' if _HAS_COLOR else ''
_CLR_BLD = '\033[1m'  if _HAS_COLOR else ''
_CLR_RST = '\033[0m'  if _HAS_COLOR else ''

# Width of the output panel
_W = 64

def _divider(char='-', color=_CLR_GRY):
    print(f"{color}{char * _W}{_CLR_RST}")


# ─────────────────────────────────────────────────────────────────
#  Spinner / loading animation
# ─────────────────────────────────────────────────────────────────

class _Spinner:
    """
    Animated spinner that runs on a background thread while the PPL
    program executes.  Writes to stderr so it doesn't interleave with
    PRINT() output on stdout.
    """
    # Braille particle frames — gives a smooth "loading" feel
    _FRAMES = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏']

    def __init__(self, label: str = 'running'):
        self._label  = label
        self._stop   = threading.Event()
        self._thread = threading.Thread(target=self._spin, daemon=True)

    def _spin(self):
        frames = itertools.cycle(self._FRAMES)
        while not self._stop.is_set():
            frame = next(frames)
            sys.stderr.write(
                f'\r  {_CLR_CYN}{frame}{_CLR_RST}  {_CLR_GRY}{self._label}…{_CLR_RST}   '
            )
            sys.stderr.flush()
            time.sleep(0.08)

    def start(self):
        self._thread.start()

    def stop(self):
        self._stop.set()
        self._thread.join(timeout=0.5)
        # Erase the spinner line completely
        sys.stderr.write('\r' + ' ' * _W + '\r')
        sys.stderr.flush()


def _find_col(source_line: str, issue_msg: str) -> int:
    """
    Best-effort: return 0-based column of the problem on this line.
    Returns -1 if we can't determine a column.
    """
    safe = source_line

    # '=' instead of ':=' — find the bare = sign
    if 'Use ":="' in issue_msg:
        m = re.search(r'(?<![:<>!=])([A-Za-z_]\w*)\s*=(?!=)', safe)
        if m:
            return m.start(0) + len(m.group(1).rstrip())

    # Unbalanced ( — find the first unclosed (
    if 'unclosed "("' in issue_msg:
        depth = 0
        in_str = False
        for idx, ch in enumerate(safe):
            if ch == '"':
                in_str = not in_str
            elif not in_str:
                if ch == '(':
                    depth += 1
                elif ch == ')':
                    depth -= 1
                    if depth < 0:
                        return idx
        # Find last ( if all unclosed
        return safe.rfind('(')

    # Extra ) — find the first )  that closes too early
    if 'extra ")"' in issue_msg:
        depth = 0
        in_str = False
        for idx, ch in enumerate(safe):
            if ch == '"':
                in_str = not in_str
            elif not in_str:
                if ch == '(':
                    depth += 1
                elif ch == ')':
                    depth -= 1
                    if depth < 0:
                        return idx
        return -1

    # Unbalanced { — find the first unclosed {
    if 'unclosed "{"' in issue_msg or 'unclosed "{' in issue_msg:
        return safe.rfind('{')

    # Extra } — find the extra }
    if 'extra "}"' in issue_msg or 'extra "}"' in issue_msg:
        depth = 0
        for idx, ch in enumerate(safe):
            if ch == '{':
                depth += 1
            elif ch == '}':
                depth -= 1
                if depth < 0:
                    return idx
        return -1

    return -1   # no column info


def _show_issue(filename: str, lines: list, issue) -> None:
    """Print one issue in a clean, professional format."""
    is_err = (issue.severity == 'ERROR')
    tag    = 'error' if is_err else 'warning'
    color  = _CLR_RED if is_err else _CLR_YEL
    msg    = issue.message
    ln     = issue.line_no

    # Header:   error: Duplicate function name "foo"
    print(f'  {color}{_CLR_BLD}{tag}:{_CLR_RST} {_CLR_BLD}{msg}{_CLR_RST}')

    # Path:     --> path/to/file:12
    short_path = os.path.relpath(filename) if os.path.exists(filename) else filename
    issue_file = getattr(issue, 'filename', '') or filename
    short_path = os.path.relpath(issue_file) if issue_file and os.path.exists(issue_file) else issue_file or short_path
    col = getattr(issue, 'column', 0) or 0
    if ln > 0 and col > 0:
        loc_info = f'{short_path}:{ln}:{col}'
    elif ln > 0:
        loc_info = f'{short_path}:{ln}'
    else:
        loc_info = short_path
    print(f'     {_CLR_BLU}-->{_CLR_RST} {_CLR_GRY}{loc_info}{_CLR_RST}')
    hint = getattr(issue, 'hint', '')
    if hint:
        print(f'     {_CLR_CYN}hint:{_CLR_RST} {hint}')

    # Source snippet removed as per request ("remove the line results")
    pass

    print()


def _show_lint_report(filename: str, source: str, issues: list) -> None:
    """Pretty-print all lint issues with source context."""
    lines   = source.splitlines()
    errors  = [x for x in issues if x.severity == 'ERROR']
    warns   = [x for x in issues if x.severity == 'WARNING']

    if not issues:
        return

    print()
    for issue in issues:
        _show_issue(filename, lines, issue)

    if errors:
        summary_color = _CLR_RED
        status = 'fix errors before running'
    elif warns:
        summary_color = _CLR_YEL
        status = 'some warnings to check'
    else:
        summary_color = _CLR_GRN
        status = 'CLEAN'

    print(f'  {summary_color}{_CLR_BLD}{len(errors)} error(s), {len(warns)} warning(s){_CLR_RST}'
          f'  --  {status}\n')
    _divider()
    print()


# ─────────────────────────────────────────────────────────────────
#  Runtime error display
# ─────────────────────────────────────────────────────────────────

def _show_runtime_error(filename: str, exc: Exception, py_code: str) -> None:
    """Print a runtime crash in a readable way."""
    short = os.path.basename(filename) if filename else '<code>'
    from src.ppl_emulator.runtime.resource_budget import ResourceLimitExceeded
    if isinstance(exc, ResourceLimitExceeded):
        print(f'  {_CLR_RED}{_CLR_BLD}runtime error:{_CLR_RST} {_CLR_BLD}resource limit exceeded{_CLR_RST}')
        print(f'     {_CLR_BLU}-->{_CLR_RST} {_CLR_GRY}{filename}{_CLR_RST}')
        print()
        print(f'  {_CLR_RED}{type(exc).__name__}:{_CLR_RST} {exc}')
        print()
        _divider()
        return
    print(f'  {_CLR_RED}{_CLR_BLD}runtime error:{_CLR_RST} {_CLR_BLD}{short}{_CLR_RST}')
    print(f'     {_CLR_BLU}-->{_CLR_RST} {_CLR_GRY}{filename}{_CLR_RST}')
    print()
    print(f'  {_CLR_RED}{type(exc).__name__}:{_CLR_RST} {exc}')
    print()
    _divider()



# ─────────────────────────────────────────────────────────────────
#  PPL language detection (for .txt files)
# ─────────────────────────────────────────────────────────────────

def _looks_like_ppl(code: str) -> bool:
    """
    Score a text file to decide if it contains HP Prime PPL code.
    Returns True if the score is high enough to be confident.
    """
    # Normalise for matching (upper-case copy, keep original for neg checks)
    up = code.upper()

    score = 0

    # ── Strong PPL indicators ─────────────────────────────────────
    # EXPORT FuncName() BEGIN  — the canonical PPL function header
    if re.search(r'EXPORT\s+\w+\s*\(', up):                 score += 4
    # HP Prime pragma header
    if '#PRAGMA MODE(' in up:                                    score += 4
    # := assignment  (Pascal/PPL, not Python/JS/C)
    if ':=' in code:                                             score += 3
    # BEGIN / END block structure
    if re.search(r'\bBEGIN\b', up):                            score += 2
    if re.search(r'\bEND;', up):                                score += 2
    # PPL-specific keywords
    for kw in ('LOCAL ', 'IFERR ', 'THEN\n', 'THEN ', 'UNTIL '):
        if kw in up:                                             score += 1
    # Common PPL built-ins unlikely to appear in other languages
    for fn in ('PRINT(', 'MSGBOX(', 'RECT(', 'PIXON(', 'DISP(', 'WAIT(', 'MAKELIST('):
        if fn in up:                                             score += 1

    # ── Negative indicators — other languages ────────────────────
    lines = code.splitlines()
    for line in lines[:40]:              # only check the top of the file
        s = line.strip()
        if s.startswith('def ')    or s.startswith('import ') or s.startswith('from '):
            score -= 3               # Python
        if s.startswith('#include') or s.startswith('int main'):
            score -= 3               # C / C++
        if s.startswith('function ') or s.startswith('const ') or s.startswith('let ') or s.startswith('var '):
            score -= 3               # JavaScript / TypeScript
        if s.startswith('public ') or s.startswith('private ') or s.startswith('class '):
            score -= 3               # Java / C#
        if s.startswith('fn ') or s.startswith('use ') or s.startswith('mod '):
            score -= 3               # Rust

    return score >= 5


# ─────────────────────────────────────────────────────────────────
#  Main
# ─────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description='HP Prime PPL Emulator',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  ppl BSTVisualizer.hpprgm
  ppl BSTVisualizer.hpprgm --output bst_screen.png
  ppl --code "EXPORT T() BEGIN PRINT(1+1); END;"
  ppl BSTVisualizer.hpprgm --dump-python
  ppl Menu.hpprgm --max-elapsed-seconds 20
  ppl Menu.hpprgm --no-time-limit
        """
    )
    parser.add_argument('file',            nargs='?',            help='PPL source file (.hpprgm or .txt)')
    parser.add_argument('--code',  '-c',                         help='Inline PPL code string')
    parser.add_argument('--output','-o',   default='screen.png', help='Output PNG path (default: screen.png)')
    parser.add_argument('--dump-python',   action='store_true',  help='Print transpiled Python to stderr')
    parser.add_argument('--no-lint',       action='store_true',  help='Skip linting, run directly')
    parser.add_argument('--save',          action='store_true',  help='Force saving output image even when live pygame window is active')
    parser.add_argument('--warnings-only', action='store_true',  help='Show warnings but do not fail on them')
    parser.add_argument('--input',         action='append',      help='Provide a value for INPUT() calls in headless mode (can be used multiple times)')
    parser.add_argument('--args',          default='',           help='Comma-separated arguments for the EXPORT function (e.g. --args "50" or --args "50,100")')
    parser.add_argument('--max-elapsed-seconds', type=float, default=None,
                        help='Override the emulator runtime time budget for this run')
    parser.add_argument('--no-time-limit', action='store_true',
                        help='Disable the emulator runtime time budget for this run')
    parser.add_argument('--print-mode',   default='both',       choices=['screen', 'terminal', 'both'],
                        help='Where PRINT() output appears: '
                             '"screen" = HP Prime display only (Option A), '
                             '"terminal" = stdout only, graphics on screen (Option B), '
                             '"both" = screen + terminal (default)')
    args = parser.parse_args()

    # ── Populate headless input queue ────────────────────────────
    if args.input:
        from src.ppl_emulator.runtime.engine import HPPrimeRuntime
        HPPrimeRuntime._pending_input_queue = args.input

    # ── Populate EXPORT function entry arguments ──────────────────
    if args.args:
        from src.ppl_emulator.runtime.engine import HPPrimeRuntime
        HPPrimeRuntime._entry_args = [a.strip() for a in args.args.split(',') if a.strip()]

    # ── Set PRINT output routing ──────────────────────────────────
    from src.ppl_emulator.runtime.engine import HPPrimeRuntime
    HPPrimeRuntime._print_mode = getattr(args, 'print_mode', 'both')
    if args.no_time_limit:
        HPPrimeRuntime._pending_elapsed_seconds = None
        HPPrimeRuntime._pending_elapsed_seconds_set = True
    elif args.max_elapsed_seconds is not None:
        HPPrimeRuntime._pending_elapsed_seconds = args.max_elapsed_seconds
        HPPrimeRuntime._pending_elapsed_seconds_set = True

    # ── Read source ──────────────────────────────────────────────
    filename = args.file or '<inline>'
    if args.code:
        ppl_code = args.code
    elif args.file:
        try:
            ppl_code = read_ppl_file(args.file)
        except FileNotFoundError:
            print(f'  {_CLR_RED}{_CLR_BLD}error:{_CLR_RST} File not found: {_CLR_BLD}{args.file}{_CLR_RST}')
            sys.exit(1)
    else:
        ppl_code = sys.stdin.read()

    if not ppl_code.strip():
        print(f'  {_CLR_RED}{_CLR_BLD}error:{_CLR_RST} No PPL code provided.')
        sys.exit(1)

    # ── .txt files: verify the content is actually PPL ───────────
    if args.file and os.path.splitext(args.file)[1].lower() == '.txt':
        if not _looks_like_ppl(ppl_code):
            print(f'  {_CLR_RED}{_CLR_BLD}error:{_CLR_RST} {_CLR_BLD}{args.file}{_CLR_RST} does not appear to be PPL code.')
            print(f'  {_CLR_GRY}Hint: PPL files should contain EXPORT functions with BEGIN/END blocks and := assignments.{_CLR_RST}')
            sys.exit(1)

    # ── Stage 1: Front-end validation (always required) ──────────
    issues = lint(ppl_code, filename=filename)
    errors = [x for x in issues if x.severity == 'ERROR']
    report_issues = errors if args.no_lint else issues

    # Always print diagnostics when present.
    _show_lint_report(filename, ppl_code, report_issues)

    if errors:
        sys.exit(1)   # stop here — don't transpile or execute invalid PPL

    # ── Stage 2: Transpile ───────────────────────────────────────
    # Resolve output path relative to the current working directory.
    out_path = os.path.abspath(args.output)
    from src.ppl_emulator.transpiler.core import transpile  # pyre-ignore
    try:
        python_code = transpile(ppl_code, out_path=out_path)
    except Exception as e:
        print(f'  {_CLR_RED}{_CLR_BLD}transpile error:{_CLR_RST} {os.path.basename(filename)}')
        print(f'     {_CLR_BLU}-->{_CLR_RST} {_CLR_GRY}{filename}{_CLR_RST}')
        print(f'\n  {_CLR_RED}SyntaxError:{_CLR_RST} {e}\n')
        _divider()
        if args.dump_python:
            traceback.print_exc()
        sys.exit(1)

    if args.dump_python:
        _divider('=', _CLR_BLU)
        print(f' {_CLR_BLD}TRANSPILED PYTHON{_CLR_RST}', file=sys.stderr)
        _divider('=', _CLR_BLU)
        for i, ln in enumerate(python_code.splitlines(), 1):
            print(f'{_CLR_GRY}{i:4}:{_CLR_RST} {ln}', file=sys.stderr)
        _divider('=', _CLR_BLU)

    # ── Stage 3: Execute ─────────────────────────────────────────
    from src.ppl_emulator.runtime.engine import HPPrimeRuntime  # pyre-ignore
    # Enable strict compiled-mode: undeclared variable access raises NameError
    # rather than silently creating a zero-initialised global (Discrepancy 3 fix).
    HPPrimeRuntime._compiled_mode = True
    HPPrimeRuntime._force_save_output_default = bool(args.save)
    ns = {'__name__': '__main__', '__file__': args.file or '<ppl>'}

    short    = os.path.basename(filename) if filename else 'inline code'

    print(f"  {_CLR_CYN}{_CLR_BLD}RUNNING{_CLR_RST}  {short}")
    _divider()
    print()

    try:
        exec(compile(python_code, '<ppl_transpiled>', 'exec'), ns)

    except (KeyboardInterrupt, SystemExit):
        pass   # clean stop — still show FINISHED banner

    except Exception as e:
        _show_runtime_error(filename, e, python_code)
        if args.dump_python:
            traceback.print_exc(file=sys.stderr)
        sys.exit(1)

    finally:
        HPPrimeRuntime._compiled_mode = False  # reset for subsequent uses in same process
        HPPrimeRuntime._force_save_output_default = False
        HPPrimeRuntime._entry_args = None
        HPPrimeRuntime._print_mode = 'both'

    # ── FINISHED banner ──────────────────────────────────────────
    print()
    _divider('═', _CLR_GRN)
    print(f"  {_CLR_GRN}{_CLR_BLD}✓  FINISHED{_CLR_RST}  {_CLR_GRY}{short}{_CLR_RST}")
    _divider('═', _CLR_GRN)

    # ── Optional output image notice ─────────────────────────────
    rt = ns.get('_rt')
    live_window = bool(rt is not None and getattr(rt, "_pg_enabled", False))
    saved_path = None
    if rt is not None and hasattr(rt, "_last_saved_path"):
        saved_path = rt._last_saved_path
    if (args.save or not live_window) and saved_path and os.path.exists(saved_path):
        size_kb = os.path.getsize(saved_path) / 1024
        print(f"\n  {_CLR_BLU}→  Output image:{_CLR_RST}  {_CLR_BLD}{saved_path}{_CLR_RST}")
        print(f"     {_CLR_GRY}320×240 px  ·  {size_kb:.1f} KB{_CLR_RST}")

    # Keep live pygame window open until user closes it, then clean up.
    if rt is not None and hasattr(rt, "close"):
        try:
            if getattr(rt, "_pg_enabled", False):
                try:
                    while getattr(rt, "_pg_enabled", False):
                        rt.WAIT(0.016)  # ~60 FPS idle refresh loop
                except (KeyboardInterrupt, SystemExit):
                    pass
            rt.close()
        except Exception:
            pass
        # Hard-exit to avoid Windows CMD's "Terminate batch job (Y/N)?" prompt
        # when the pygame window is closed or Ctrl+C is pressed.
        os._exit(0)



if __name__ == '__main__':
    main()
