# ─────────────────────────────────────────────────────────────────
#  String & Text Utilities and Expression Transpilation
# ─────────────────────────────────────────────────────────────────

import re
from .constants import _OPS, _PYTHON_RESERVED, BUILTINS, _STRUCTURAL, BUILTINS_ZERO_ARGS  # pyre-ignore

# Python class names emitted by _xform itself (e.g. {..} -> PPLList([..])).
# These must not be treated as PPL variable references in repl_var.
_XFORM_TYPES = frozenset(['PPLLIST', 'PPLSTRING', 'COERCE'])
_IDENT = r'[^\W\d]\w*'
_PPL_FACTORIAL = "__ppl_factorial__"

def _safe_name(name):
    """Prefix Python reserved words with '_ppl_' to avoid syntax errors."""
    if name.lower() in _PYTHON_RESERVED:
        return f"_ppl_{name.lower()}"
    return name

def _escape_content(s):
    """Escape backslashes and quotes for Python string literals."""
    return s.replace('\\', '\\\\').replace('"', '\\"')

def _strip_comment(line: str) -> str:
    """Remove a PPL // line comment, leaving string literals intact."""
    buf: list[str] = []
    in_str = False
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            if not in_str:
                in_str = True
                buf.append('"')
            elif i + 1 < len(line) and line[i + 1] == '"':
                buf.append('""')
                i += 1      # consume the second quote of ""
            else:
                in_str = False
                buf.append('"')
        elif in_str and ch == '\\' and i + 1 < len(line) and line[i + 1] == '"':
            buf.append('\\"')
            i += 1
        elif ch == '/' and line[i:i+2] == '//' and not in_str:
            break   # start of line comment — discard the rest
        else:
            buf.append(ch)
        i += 1
    return ''.join(buf).rstrip()

def _erase_strings(line):
    """Replace string literal content with spaces (keeps delimiters).

    Used for bracket-depth counting so characters inside strings don't
    count as real parentheses or operators.
    """
    res = []
    in_str = False
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            if not in_str:
                in_str = True
                res.append('"')
            elif i + 1 < len(line) and line[i + 1] == '"':
                res.append('  ')  # escaped quote inside string — erase both chars
                i += 1
            else:
                in_str = False
                res.append('"')
        elif in_str and ch == '\\' and i + 1 < len(line) and line[i + 1] == '"':
            res.append('  ')  # \" escape — erase both chars
            i += 1
        elif in_str:
            res.append(' ')   # inside string — replace with space
        else:
            res.append(ch)
        i += 1
    return ''.join(res)

def _split_locals(text):
    """Split LOCAL declarations on commas, respecting strings, braces and parentheses."""
    parts, cur, depth, in_s, i = [], [], 0, False, 0
    while i < len(text):
        ch = text[i]
        if ch == '"':
            if not in_s:
                in_s = True
                cur.append('"')
            else:
                if i + 1 < len(text) and text[i + 1] == '"':
                    cur.append('""')
                    i += 1
                else:
                    in_s = False
                    cur.append('"')
        elif not in_s and ch in '{[(':
            depth += 1
            cur.append(ch)
        elif not in_s and ch in '}])':
            depth -= 1
            cur.append(ch)
        elif not in_s and ch == ',' and depth == 0:
            parts.append(''.join(cur).strip())
            cur = []
        else:
            cur.append(ch)
        i += 1
    parts.append(''.join(cur).strip())
    return [p for p in parts if p]


def _has_open_string_literal(text: str) -> bool:
    in_str = False
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == '"':
            if not in_str:
                in_str = True
            elif i + 1 < len(text) and text[i + 1] == '"':
                i += 1
            else:
                in_str = False
        elif in_str and ch == '\\' and i + 1 < len(text) and text[i + 1] == '"':
            i += 1
        i += 1
    return in_str


def _slice_bound(val, is_start):
    """Convert a PPL 1-based slice bound to a Python 0-based bound."""
    s = val.strip()
    if not s:
        return ''
    if is_start:
        try:
            return str(int(s) - 1)
        except ValueError:
            return f'({s})-1'
    return s

def _ppl_to_py_slice(range_str):
    """Convert a PPL 'start:end' range string to a Python slice string."""
    parts = range_str.split(':', 1)
    start_py = _slice_bound(parts[0], True)
    end_py   = _slice_bound(parts[1] if len(parts) > 1 else '', False)
    return f'{start_py}:{end_py}'

def _split_top_level_args(s: str) -> list:
    """Split s on commas that are at bracket-depth 0 (ignoring strings)."""
    args: list = []
    cur:  list = []
    depth = 0
    in_str = False
    i = 0
    while i < len(s):
        ch = s[i]
        if ch == '"':
            in_str = not in_str
            cur.append(ch)
        elif not in_str and ch in '([{':
            depth += 1
            cur.append(ch)
        elif not in_str and ch in ')]}':
            depth -= 1
            cur.append(ch)
        elif not in_str and ch == ',' and depth == 0:
            args.append(''.join(cur).strip())
            cur = []
        else:
            cur.append(ch)
        i += 1
    if cur:
        args.append(''.join(cur).strip())
    return args


def _find_matching_open(text: str, close_idx: int) -> int:
    pairs = {')': '(', ']': '['}
    close_ch = text[close_idx]
    open_ch = pairs.get(close_ch)
    if open_ch is None:
        return -1
    depth = 1
    i = close_idx - 1
    while i >= 0:
        ch = text[i]
        if ch == close_ch:
            depth += 1
        elif ch == open_ch:
            depth -= 1
            if depth == 0:
                return i
        i -= 1
    return -1


def _find_inline_assignment_bounds(text: str, op_idx: int) -> tuple[int, int] | None:
    left = op_idx - 1
    while left >= 0 and text[left].isspace():
        left -= 1
    if left < 0:
        return None

    if text[left] in ')]':
        open_idx = _find_matching_open(text, left)
        if open_idx < 0:
            return None
        ident_end = open_idx - 1
        while ident_end >= 0 and text[ident_end].isspace():
            ident_end -= 1
        ident_start = ident_end
        while ident_start >= 0 and (text[ident_start] == '_' or text[ident_start].isalnum() or ord(text[ident_start]) > 127):
            ident_start -= 1
        lhs_start = ident_start + 1
        lhs_end = left + 1
    else:
        lhs_end = left + 1
        ident_start = left
        while ident_start >= 0 and (text[ident_start] == '_' or text[ident_start].isalnum() or ord(text[ident_start]) > 127):
            ident_start -= 1
        lhs_start = ident_start + 1

    if lhs_start >= lhs_end:
        return None

    rhs_start = op_idx + 2
    while rhs_start < len(text) and text[rhs_start].isspace():
        rhs_start += 1

    depth = 0
    in_str = False
    i = rhs_start
    while i < len(text):
        ch = text[i]
        if ch == '"':
            if not in_str:
                in_str = True
            elif i + 1 < len(text) and text[i + 1] == '"':
                i += 1
            else:
                in_str = False
        elif not in_str:
            if ch in '([{':
                depth += 1
            elif ch in ')]}':
                if depth == 0:
                    break
                depth -= 1
            elif ch in ',;' and depth == 0:
                break
        i += 1

    rhs_end = i
    while rhs_end > rhs_start and text[rhs_end - 1].isspace():
        rhs_end -= 1
    if rhs_start >= rhs_end:
        return None
    return lhs_start, rhs_end


def _inline_assignment_call(lhs: str, rhs: str, line_no) -> str:
    line_arg = "None" if line_no is None else str(line_no)
    simple = re.fullmatch(rf'({_IDENT})', lhs)
    if simple:
        return f"_rt._assign_expr('{simple.group(1).upper()}', {rhs}, {line_arg})"

    indexed = re.fullmatch(rf'({_IDENT})\s*\((.+)\)', lhs)
    if indexed:
        idx_exprs = ", ".join(arg.strip() for arg in _split_top_level_args(indexed.group(2)))
        return f"_rt._assign_index_expr('{indexed.group(1).upper()}', [{idx_exprs}], {rhs}, {line_arg})"

    bracketed = re.fullmatch(rf'({_IDENT})\s*\[(.+)\]', lhs)
    if bracketed:
        idx_exprs = ", ".join(arg.strip() for arg in _split_top_level_args(bracketed.group(2)))
        return f"_rt._assign_index_expr('{bracketed.group(1).upper()}', [{idx_exprs}], {rhs}, {line_arg})"

    return f"({lhs} := {rhs})"


def _rewrite_inline_assignments(e: str, line_no) -> str:
    while True:
        safe = _erase_strings(e)
        changed = False
        for op_idx in range(len(safe) - 1):
            if safe[op_idx:op_idx + 2] != ':=':
                continue
            bounds = _find_inline_assignment_bounds(e, op_idx)
            if not bounds:
                continue
            lhs_start, rhs_end = bounds
            lhs = e[lhs_start:op_idx].strip()
            rhs = e[op_idx + 2:rhs_end].strip()
            replacement = _inline_assignment_call(lhs, rhs, line_no)
            e = e[:lhs_start] + replacement + e[rhs_end:]
            changed = True
            break
        if not changed:
            return e


def _rewrite_makelist_calls(e: str, line_no, known_vars) -> str:
    """Find MAKELIST(expr, var, start, end[, step]) in e and rewrite expr as a lambda.

    This allows the runtime to iterate over the range and evaluate the expression
    for each value of var, correctly handling non-constant expressions like i^2.
    """
    # Find each MAKELIST( invocation
    out: list[str] = []
    i = 0
    while i < len(e):
        # Look for MAKELIST(
        m = re.search(r'\bMAKELIST\s*\(', e[i:], re.IGNORECASE)
        if not m:
            out.append(e[i:])
            break
        # Append everything before MAKELIST
        out.append(e[i: i + m.start()])
        i += m.start() + len(m.group())

        # Find the matching closing paren
        depth = 1
        j = i
        in_str = False
        while j < len(e) and depth > 0:
            ch = e[j]
            if ch == '"':
                in_str = not in_str
            elif not in_str:
                if ch == '(':
                    depth += 1
                elif ch == ')':
                    depth -= 1
            j += 1
        inner = e[i: j - 1]  # content between the parens
        i = j  # advance past closing )

        ml_args = _split_top_level_args(inner)
        if len(ml_args) >= 4:
            expr_raw  = ml_args[0]
            var_raw   = ml_args[1].strip()
            start_raw = ml_args[2]
            end_raw   = ml_args[3]
            step_raw  = ml_args[4] if len(ml_args) >= 5 else '1'

            # Derive the PPL variable name (strip array index, take identifier)
            var_upper = re.match(_IDENT, var_raw)
            var_upper = var_upper.group(0).upper() if var_upper else var_raw.upper()

            # Transform each part
            expr_py  = _xform(expr_raw,  line_no, known_vars)
            start_py = _xform(start_raw, line_no, known_vars)
            end_py   = _xform(end_raw,   line_no, known_vars)
            step_py  = _xform(step_raw,  line_no, known_vars)

            out.append(f"MAKELIST(lambda: {expr_py}, '{var_upper}', {start_py}, {end_py}, {step_py})")
        else:
            # Fewer than 4 args — pass through unchanged
            out.append(f"MAKELIST({inner})")
    return ''.join(out)


def _rewrite_raw_postfix_factorials(expr: str) -> str:
    """Rewrite Prime postfix factorial syntax before identifier wrapping."""
    patterns = [
        r'(\([^()]+\))\s*!(?!=)',
        r'((?:[A-Za-z_]\w*|\d+(?:\.\d+)?)(?:\([^()]*\)|\[[^\[\]]+\])*)\s*!(?!=)',
    ]
    prev = None
    while expr != prev:
        prev = expr
        for pat in patterns:
            expr = re.sub(pat, lambda m: f"{_PPL_FACTORIAL}({m.group(1)})", expr)
    return expr


def _xform(expr, line_no=None, known_vars=None):
    """Transform a PPL expression to valid Python, respecting strings."""
    if re.search(r'\bMAKELIST\s*\((?!\s*lambda:)', expr, re.IGNORECASE):
        return _rewrite_makelist_calls(expr, line_no, known_vars)

    parts, buf, in_str, i = [], [], False, 0
    while i < len(expr):
        ch = expr[i]
        if ch == '"':
            if not in_str:
                if buf:
                    parts.append(('code', ''.join(buf)))
                    buf = []
                in_str = True
                buf.append('"')
            else:
                if i + 1 < len(expr) and expr[i + 1] == '"':
                    buf.append('""')
                    i += 1  # consume second quote of escaped ""
                else:
                    buf.append('"')
                    in_str = False
                    parts.append(('str', ''.join(buf)))
                    buf = []
        elif in_str and ch == '\\' and i + 1 < len(expr) and expr[i + 1] == '"':
            buf.append('\\"')
            i += 1
        else:
            buf.append(ch)
        i += 1
    if buf:
        parts.append(('code', ''.join(buf)))

    res: list[str] = []
    for kind, val in parts:
        if kind == 'str':
            # PPL "" or \" -> Python \"
            content = str(val)[1:-1].replace('""', '"').replace('\\"', '"')  # type: ignore
            res.append(f'PPLString("{_escape_content(content)}")')  # type: ignore
        else:
            e = _rewrite_raw_postfix_factorials(val)
            single_literals: list[str] = []

            def stash_single_literal(m):
                content = m.group(0)[1:-1]
                token = f'@@{len(single_literals)}@@'
                single_literals.append(content)
                return token

            e = re.sub(r"'([^'\\]|\\.)*'", stash_single_literal, e)
            # { } or [ ] -> COERCE([ ]) for list literals; subscript [ ] are left as-is.
            # A single stack-based pass tracks each bracket's origin so the closer
            # only adds ')' when the matching opener was a list literal (not a subscript).
            _bstk: list = []
            _bout: list = []
            for _bch in e:
                if _bch == '{':
                    _bstk.append('c')
                    _bout.append('COERCE([')
                elif _bch == '}':
                    _bstk.pop() if _bstk else None
                    _bout.append('])')
                elif _bch == '[':
                    _tr = ''.join(_bout).rstrip()
                    if _tr and (_tr[-1].isalnum() or _tr[-1] in '_)]'):
                        _bstk.append('s')   # subscript bracket
                        _bout.append('[')
                    else:
                        _bstk.append('c')   # list literal
                        _bout.append('COERCE([')
                elif _bch == ']':
                    _bt = _bstk.pop() if _bstk else 's'
                    _bout.append('])')  if _bt == 'c' else _bout.append(']')
                else:
                    _bout.append(_bch)
            e = ''.join(_bout)
            e = _rewrite_inline_assignments(e, line_no)
            e = re.sub(r"'([^'\\]|\\.)*'", stash_single_literal, e)
            
            # Color literals
            # Strip leading zeros from plain decimal literals (e.g., 05 -> 5)
            # but leave # prefixed tokens alone — they are handled below.
            e = re.sub(r'(?<!\.)(?<!#)\b0+(\d+)\b', r'\1', e)
            # #AF:16h -> Prime base literal syntax
            def repl_base_literal(m):
                digits = m.group(1)
                base = int(m.group(2))
                try:
                    return str(int(digits, base))
                except ValueError:
                    return m.group(0)

            e = re.sub(r'#([0-9A-Za-z]+):([0-9]+)[hH]?\b', repl_base_literal, e)
            # #AFh  -> explicit hex
            e = re.sub(r'#([0-9A-Fa-f]+)[hH]\b', lambda m: str(int(m.group(1), 16)), e)
            # #1010b -> explicit binary
            e = re.sub(r'#([01]+)[bB]\b', lambda m: str(int(m.group(1), 2)), e)
            # #RRGGBB -> 6-digit colour constant (no suffix required)
            e = re.sub(r'#([0-9A-Fa-f]{6})\b', lambda m: str(int(m.group(1), 16)), e)
            # #XY (2-4 digit partial hex) -> fallback conversion
            e = re.sub(r'#([0-9A-Fa-f]{1,5})\b', lambda m: str(int(m.group(1), 16)), e)
            
            # PPL complex literal: (3, 4) -> complex(3, 4)
            # Match (number, number) NOT preceded by an identifier (which would be a call)
            e = re.sub(
                r'(?<![A-Za-z0-9_])\(\s*(-?[\d.]+(?:[eE][+-]?\d+)?)\s*,\s*(-?[\d.]+(?:[eE][+-]?\d+)?)\s*\)',
                r'complex(\1, \2)', e)

            # Comma indexing name[i, j] -> name[i][j]
            # Discrepancy 2 fix: PPLList/PPLMatrix.__getitem__ now handles 1-based conversion
            # internally, so we no longer subtract 1 here.
            def repl_comma_idx(m):
                name = m.group(1)
                indices = _split_top_level_args(m.group(2))
                return _safe_name(name) + "".join(f"[{idx}]" for idx in indices)
            e = re.sub(rf'\b({_IDENT})\s*\[([^\[\]]+?,[^\[\]]+?)\]', repl_comma_idx, e)
            
            # Paren indexing: name(i) -> name[i-1]
            # Special case: slice/range notation name(s:e) or name(r1:r2, c1:c2)
            def repl_paren_idx(m):
                name = m.group(1)
                name_up = name.upper()
                if name_up in BUILTINS or name_up in _STRUCTURAL: return m.group(0)
                
                # Known variables use bracket indexing so PPLList/PPLMatrix.__getitem__ handles 1-based.
                # Unknown identifiers keep parens and remain as function calls.
                safe = _safe_name(name)
                args_str = m.group(2)
                
                if ':' in args_str:
                    args = _split_top_level_args(args_str)
                    if len(args) == 1:
                        # Single range: m(1:2) -> m[0:2]
                        return f'{safe}[{_ppl_to_py_slice(args[0])}]'
                    if len(args) == 2 and ':' in args[0] and ':' in args[1]:
                        # Two ranges: m(1:2, 2:3) -> _ppl_slice_2d(m, 0, 2, 1, 3)
                        r = args[0].split(':', 1)
                        c_arg = args[1].split(':', 1)
                        r0 = _slice_bound(r[0], True)
                        r1 = _slice_bound(r[1], False)
                        c0 = _slice_bound(c_arg[0], True)
                        c1 = _slice_bound(c_arg[1], False)
                        return f'_ppl_slice_2d({safe}, {r0}, {r1}, {c0}, {c1})'
                
                if known_vars is not None and name_up in known_vars:
                    # Discrepancy 2 fix: pass index as-is; PPLList.__getitem__ handles 1-based.
                    args = _split_top_level_args(args_str)
                    return safe + "".join(f"[{a}]" for a in args)
                
                return safe + "(" + args_str + ")"
            e = re.sub(rf'\b({_IDENT})\s*\(([^()]+)\)', repl_paren_idx, e)
            
            # HP Prime Unicode-arrow function names: B→R, R→B, etc.
            e = re.sub(rf'({_IDENT})→({_IDENT})', r'\1_to_\2', e)
            # HP Prime native math symbols
            e = e.replace('\u2212', '-')          # − (U+2212) Unicode minus → ASCII minus
            e = e.replace('\ue003', '1j')          # U+E003 HP Prime imaginary unit → Python 1j
            e = e.replace('\u2148', '1j')          # ⅈ (U+2148) imaginary unit → Python 1j
            e = e.replace('\u03c0', 'pi')          # π (U+03C0) pi constant
            e = e.replace('\u03A0', 'pi')          # Π uppercase variant
            e = re.sub(r'\u221a([\w.]+)', r'SQRT(\1)', e)   # √x  → SQRT(x)
            e = re.sub(r'\u221a(\([^()]*\))', r'SQRT\1', e) # √(x) → SQRT(x)
            e = re.sub(r'(\w+)\u00b2', r'(\1)**2', e)         # X² → (X)**2
            # Ops
            for pat, rep in _OPS:
                if re.search(r'[A-Za-z]', pat):
                    e = re.sub(
                        pat,
                        lambda m, replacement=rep: m.group(0) if m.group(0).islower() else replacement,
                        e,
                        flags=re.IGNORECASE,
                    )
                else:
                    e = re.sub(pat, rep, e, flags=re.IGNORECASE)
            e = re.sub(r'(?<!\*)\^(?!\*)', '**', e)
            e = e.replace('₂', ' ').replace('₁₀', ' ').replace('₁₆', ' ')
            
            # Rename reserved Python names (already done above, but let's be thorough)
            # and wrap all other identifiers in _rt.GET_VAR(...)
            def repl_var(m):
                name = m.group(1)
                name_up = name.upper()
                ln_arg = f", {line_no}" if line_no is not None else ""
                tail = e[m.end():].lstrip()
                in_call = tail.startswith('(')

                # Declared local/global variables always use GET_VAR, even when they
                # shadow a builtin name (e.g. LOCAL roots shadows the ROOTS builtin).
                if name == '_rt':
                    return name
                if known_vars is not None and name_up in known_vars:
                    if name_up in _XFORM_TYPES:
                        return name
                    return f"_rt.GET_VAR('{name_up}'{ln_arg})"

                # Python logical operators are already lowercased by _OPS; do NOT uppercase
                # them back — `AND`/`OR`/`NOT` are not valid Python keywords (only lowercase).
                _PY_LOGICALS = {'AND': 'and', 'OR': 'or', 'NOT': 'not'}
                if name_up in _PY_LOGICALS:
                    return _PY_LOGICALS[name_up]

                # Builtins and structural keywords pass through unchanged (uppercase)
                if name_up in BUILTINS or name_up in _STRUCTURAL:
                    if name_up in BUILTINS_ZERO_ARGS and not in_call:
                        return f"{name_up}()"
                    return name_up  # force uppercase so _rt.BUILTIN bindings work
                if name.lower() in _PYTHON_RESERVED:
                    return f"_ppl_{name.lower()}"

                if in_call:
                    # Transpiler-generated Python types must not go through GET_VAR
                    if name_up in _XFORM_TYPES:
                        return name
                    # Unknown identifier in call position: treat as a direct Python function call.
                    return name

                return f"_rt.GET_VAR('{name_up}'{ln_arg}).value"

            # Keep dotted attribute access intact so CAS.diff() and similar calls
            # preserve the method name that the runtime actually implements.
            e = re.sub(rf'(?<!\.)\b({_IDENT})\b', repl_var, e)

            for idx, content in enumerate(single_literals):
                e = e.replace(f'@@{idx}@@', repr(content))
            
            # Post-process: dereference GET_VAR calls unless they are bare arguments in a call list.
            # We want: FUNC(_rt.GET_VAR('A')) if 'A' is passed alone.
            # Use regex to find GET_VAR().value and strip .value if preceded by ( or , and followed by ) or ,
            e = re.sub(r'([\(,]\s*)_rt\.GET_VAR\((.+?)\)\.value(\s*[,\)])', r'\1_rt.GET_VAR(\2)\3', e)
            e = e.replace(_PPL_FACTORIAL, '_ppl_factorial')
            
            res.append(e)
    
    return ''.join(res).strip()
