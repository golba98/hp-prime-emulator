import re
from contextlib import contextmanager
from .constants import BUILTINS, _PPL_KEYWORDS, _SYSTEM_GLOBALS
from .expressions import _safe_name, _strip_comment, _erase_strings, _has_open_string_literal, _split_locals, _split_top_level_args, _xform

_IDENT = r'[^\W\d]\w*'


def _begin_follows(lines: list[str], start_idx: int, max_ahead: int = 12) -> bool:
    blockers = r'^(EXPORT|PROCEDURE|LOCAL|VAR|IF|FOR|WHILE|REPEAT|CASE|END|THEN|ELSE|DEFAULT)\b'
    for j in range(start_idx, min(start_idx + max_ahead, len(lines))):
        nxt = _strip_comment(lines[j]).strip()
        if not nxt:
            continue
        if re.match(r'^BEGIN;?$', nxt, re.IGNORECASE):
            return True
        if re.match(blockers, nxt, re.IGNORECASE):
            return False
    return False


def _header_needs_continuation(erased_stmt: str) -> bool:
    stmt = erased_stmt.strip().upper()
    if not stmt:
        return False
    # If a malformed header already rolls into a block opener/closer, let the
    # normal parser raise the real syntax error instead of buffering forever.
    if re.search(r'\b(?:BEGIN|END)\b', stmt):
        return False
    if re.match(r'^IF(?:\s+.*)?$', stmt) and 'THEN' not in stmt:
        return True
    if re.match(r'^ELSE\s+IF(?:\s+.*)?$', stmt) and 'THEN' not in stmt:
        return True
    if re.match(r'^WHILE(?:\s+.*)?$', stmt) and 'DO' not in stmt:
        return True
    if re.match(r'^FOR\s+.+$', stmt) and ' DO' not in f' {stmt}':
        return True
    return False


def _find_top_level_operator(text: str, operator: str) -> int:
    safe = _erase_strings(text)
    depth = 0
    i = 0
    op_len = len(operator)
    while i <= len(safe) - op_len:
        ch = safe[i]
        if ch in '([{':
            depth += 1
        elif ch in ')]}':
            depth = max(0, depth - 1)
        if depth == 0 and safe[i:i + op_len] == operator:
            return i
        i += 1
    return -1


def _split_inline_statements(text: str) -> list[str]:
    parts: list[str] = []
    cur: list[str] = []
    depth = 0
    in_string = False
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == '"':
            if not in_string:
                in_string = True
                cur.append(ch)
            elif i + 1 < len(text) and text[i + 1] == '"':
                cur.append('""')
                i += 1
            else:
                in_string = False
                cur.append(ch)
        elif in_string and ch == '\\' and i + 1 < len(text) and text[i + 1] == '"':
            cur.append('\\"')
            i += 1
        elif not in_string and ch in '([{':
            depth += 1
            cur.append(ch)
        elif not in_string and ch in ')]}':
            depth = max(0, depth - 1)
            cur.append(ch)
        elif not in_string and depth == 0 and ch == ';':
            part = ''.join(cur).strip()
            if part:
                parts.append(part)
            cur = []
        else:
            cur.append(ch)
        i += 1
    tail = ''.join(cur).strip()
    if tail:
        parts.append(tail)
    return parts


def _key_header_to_procedure(stmt: str) -> str | None:
    m = re.match(rf'^KEY\s+({_IDENT})\s*\((.*?)\)\s*;?$', stmt, re.IGNORECASE)
    if not m:
        return None
    return f'PROCEDURE {m.group(1)}({m.group(2)})'

class Transpiler:
    """
    PPL → Python transpiler.

    Three-phase process:
      1. _preprocess  — expand one-liner IF/FOR/WHILE/REPEAT into multi-line form.
      2. _first_pass  — scan for function/variable declarations so we know which
                        names are locals vs. globals before emitting any code.
      3. transpile    — walk the preprocessed source line-by-line, emit Python.
    """

    def __init__(self):
        self._out: list[str] = []
        self._block_stack: list[tuple[str, int]] = []  # (block_type, source_line_no)
        self.indent_level: int = 0
        self._cur_line_raw: int = 0        # current PPL source line number
        self._cur_fn: str | None = None    # function we're currently inside
        self._fn_order: list[tuple[str, str]] = []
        self._export: str | None = None
        self._export_params: list[str] = []
        self._locals: dict[str, set[str]] = {}
        self._globals: dict[str, set[str]] = {}
        self._module_vars: set[str] = set()
        self._iferr_stack: list[int] = []
        self._iferr_in_except: set[int] = set()
        self._case_stack: list[dict[str, bool | int]] = []
        self._fn_params: dict[str, list[str]] = {}  # raw param names per function

    @contextmanager
    def _indent_block(self):
        """Context manager to push/pop indentation levels."""
        self.indent_level += 1
        yield
        self.indent_level -= 1

    def _pad(self):
        return '    ' * self.indent_level

    def _xf(self, expr):
        """Transform a PPL expression using the current function's symbol table.

        Passes known local+global variable names to _xform so that
        paren-indexed variable access (e.g. my_list(2)) is correctly
        rewritten to _rt.GET_VAR('MY_LIST')(2) rather than left as a
        bare Python identifier that would raise NameError at runtime.
        """
        known = None
        raw = set(self._module_vars)
        if self._cur_fn:
            raw |= (self._locals.get(self._cur_fn, set()) |
                    self._globals.get(self._cur_fn, set()))
        if raw:
            # _first_pass stores names via _safe_name (lowercase); normalise to
            # uppercase so repl_var's name_up comparisons work correctly.
            known = {v.upper() for v in raw}
        return _xform(expr, self._cur_line_raw, known)

    def _emit(self, line=''):
        """Emitter helper that handles indentation automatically."""
        self._out.append(self._pad() + line if line else '')

    def _emit0(self, line):
        """Append a line at column 0 (no indentation)."""
        self._out.append(line)

    def _last_out_is_block_header(self) -> bool:
        """Return True if the most recent non-blank output line ends with ':'."""
        last = next((l for l in reversed(self._out) if l.strip()), '')
        return last.strip().endswith(':')

    def _validate_lvalue(self, lhs):
        lhs = lhs.strip()
        # Variable or bracket-indexed element (e.g., A or A[1] or A[1,2])
        if re.match(rf'^{_IDENT}(?:\s*\[.+\])?$', lhs):
            return True
        # Paren-indexed element (e.g., A(1) or A(1,2)) — PPL list/matrix access
        if re.match(rf'^{_IDENT}\s*\(.+\)$', lhs):
            return True
        raise SyntaxError(f"Line {self._cur_line_raw}: L-value must be a variable or array element: '{lhs}'")

    # ─────────────────────────────────────────────────────────────────
    # Phase 1: Preprocessing
    # ─────────────────────────────────────────────────────────────────

    def _preprocess(self, code):
        """Expand procedure-style calls and one-liner control structures."""
        lines = code.replace('\r\n', '\n').splitlines()
        result = []
        for i, line in enumerate(lines):
            nc = _strip_comment(line).strip()
            indent = line[: len(line) - len(line.lstrip())]
            key_header = _key_header_to_procedure(nc)
            if key_header:
                end_idx_key = min(i + 10, len(lines))
                is_key_proc = False
                for j in range(i + 1, end_idx_key):
                    ns = _strip_comment(lines[j]).strip()
                    if not ns:
                        continue
                    if re.match(r'^BEGIN;?$', ns, re.IGNORECASE):
                        is_key_proc = True
                    break
                if is_key_proc:
                    line = line.replace(nc, key_header)
                    nc = key_header
            m_inline_fn = re.match(rf'^(EXPORT|PROCEDURE)\s+({_IDENT})\s*\((.*?)\)\s+BEGIN;?\s*$', nc, re.IGNORECASE)
            if m_inline_fn:
                result.append(indent + f'{m_inline_fn.group(1)} {m_inline_fn.group(2)}({m_inline_fn.group(3)})')
                result.append(indent + 'BEGIN')
                continue
            # Handle procedure calls followed by BEGIN
            m = re.match(rf'^({_IDENT})\s*\(([^)]*)\)\s*;?$', nc)
            if m and m.group(1).upper() not in _PPL_KEYWORDS:
                if _begin_follows(lines, i + 1):
                    line = line.replace(nc, 'PROCEDURE ' + nc.rstrip(';'))
            # Handle LOCAL function definitions: LOCAL func(params) [+ BEGIN] → PROCEDURE
            elif re.match(rf'^LOCAL\s+{_IDENT}\s*\(', nc, re.IGNORECASE):
                m_lfn_inline = re.match(rf'^LOCAL\s+({_IDENT})\s*\(([^)]*)\)\s+BEGIN;?\s*$', nc, re.IGNORECASE)
                if m_lfn_inline:
                    result.append(indent + f'PROCEDURE {m_lfn_inline.group(1)}({m_lfn_inline.group(2)})')
                    result.append(indent + 'BEGIN')
                    continue
                m_lfn = re.match(rf'^LOCAL\s+({_IDENT})\s*\(([^)]*)\)\s*;?$', nc, re.IGNORECASE)
                if m_lfn:
                    if _begin_follows(lines, i + 1):
                        line = line.replace(nc, f'PROCEDURE {m_lfn.group(1)}({m_lfn.group(2)})')
            result.append(line)
        
        expanded: list[str] = []
        i = 0
        while i < len(result):
            line = result[i]
            nc     = _strip_comment(line).strip()
            indent = line[: len(line) - len(line.lstrip())]
            # Expansion for one-liners
            m = re.match(r'^(IF\s+.+?(?:\s+THEN|(?<![A-Za-z_])\s*THEN))\s+(.+?)\s*(?:ELSE\s+(.+?)\s*)?END;?\s*$', nc, re.IGNORECASE)
            if m:
                expanded.append(indent + m.group(1))
                for s in m.group(2).split(';'):
                    if s.strip(): expanded.append(indent + '  ' + s.strip() + ';')
                if m.group(3):
                    expanded.append(indent + 'ELSE')
                    for s in m.group(3).split(';'):
                        if s.strip():
                            expanded.append(indent + '  ' + s.strip() + ';')
                expanded.append(indent + 'END;')
                i += 1
                continue
            m_if_header = re.match(r'^(IF\s+.+?(?:\s+THEN|(?<![A-Za-z_])\s*THEN))\s*$', nc, re.IGNORECASE)
            if m_if_header and i + 1 < len(result):
                next_line = result[i + 1]
                next_nc = _strip_comment(next_line).strip()
                m_next_inline = re.match(r'^(.+?)\s*(?:ELSE\s+(.+?)\s*)?END;?\s*$', next_nc, re.IGNORECASE)
                if m_next_inline:
                    expanded.append(indent + m_if_header.group(1))
                    for s in m_next_inline.group(1).split(';'):
                        if s.strip():
                            expanded.append(indent + '  ' + s.strip() + ';')
                    if m_next_inline.group(2):
                        expanded.append(indent + 'ELSE')
                        for s in m_next_inline.group(2).split(';'):
                            if s.strip():
                                expanded.append(indent + '  ' + s.strip() + ';')
                    expanded.append(indent + 'END;')
                    i += 2
                    continue
            m_iferr_inline = re.match(r'^(IFERR\b.+?\bTHEN)\s+(.+?)\s*(?:ELSE\s+(.+?)\s*)?END;?\s*$', nc, re.IGNORECASE)
            if m_iferr_inline:
                header = re.sub(r'\bTHEN\s*$', '', m_iferr_inline.group(1), flags=re.IGNORECASE).rstrip()
                expanded.append(indent + header)
                expanded.append(indent + 'THEN')
                for s in _split_inline_statements(m_iferr_inline.group(2)):
                    if s.strip():
                        expanded.append(indent + '  ' + s.strip() + ';')
                if m_iferr_inline.group(3):
                    expanded.append(indent + 'ELSE')
                    for s in _split_inline_statements(m_iferr_inline.group(3)):
                        if s.strip():
                            expanded.append(indent + '  ' + s.strip() + ';')
                expanded.append(indent + 'END;')
                i += 1
                continue
            m = re.match(r'^(WHILE\s+.+?\s+DO)\s+(.+?)\s*END;?\s*$', nc, re.IGNORECASE)
            if m:
                expanded.append(indent + m.group(1))
                for s in m.group(2).split(';'):
                    if s.strip(): expanded.append(indent + '  ' + s.strip() + ';')
                expanded.append(indent + 'END;')
                i += 1
                continue
            m = re.match(r'^(FOR\s+.+?\s+DO)\s+(.+?)\s*END;?\s*$', nc, re.IGNORECASE)
            if m:
                expanded.append(indent + m.group(1))
                for s in m.group(2).split(';'):
                    if s.strip(): expanded.append(indent + '  ' + s.strip() + ';')
                expanded.append(indent + 'END;')
                i += 1
                continue
            m = re.match(r'^REPEAT\s*(.*?)\s*UNTIL\s+(.+?);?\s*$', nc, re.IGNORECASE)
            if m:
                expanded.append(indent + 'REPEAT')
                for s in m.group(1).split(';'):
                    if s.strip(): expanded.append(indent + '  ' + s.strip() + ';')
                expanded.append(indent + 'UNTIL ' + m.group(2) + ';')
                i += 1
                continue
            expanded.append(line)
            i += 1
        return '\n'.join(expanded)

    # ─────────────────────────────────────────────────────────────────
    # Phase 2: First pass — collect locals/globals per function
    # ─────────────────────────────────────────────────────────────────

    def _first_pass(self, code):
        """Scan declarations so we know each variable's scope before emitting."""
        cur: str | None = None
        loc: set[str] = set()   # LOCAL-declared names in current function
        asgn: set[str] = set()  # assigned names (potential globals) in current function
        self._module_vars = set()
        for raw in code.splitlines():
            line = _strip_comment(raw).strip()
            if not line: continue
            m = re.match(r'(EXPORT|PROCEDURE)\s+(\w+)(?:\s*\((.*?)\))?', line, re.IGNORECASE)
            if m and m.group(3) is None and line.rstrip().endswith(';'):
                m = None
            if m:
                if cur:
                    self._locals[cur] = loc  # type: ignore
                    self._globals[cur] = asgn - loc  # type: ignore
                cur = m.group(2)
                loc = set()
                asgn = set()
                params = [_safe_name(p.strip()) for p in (m.group(3) or '').split(',') if p.strip()]
                loc.update(params)
                self._fn_order.append((cur, ", ".join(params)))
                if m.group(1).upper() == 'EXPORT':
                    self._export, self._export_params = cur, params
                continue
            m = re.match(r'LOCAL\s+(.+?);?\s*$', line, re.IGNORECASE)
            if m:
                target_scope = loc if cur else self._module_vars
                for d in _split_locals(m.group(1)):
                    match = re.match(r'(\w+)', d)
                    if match:
                        target_scope.add(_safe_name(match.group(1)))
                continue
            m_export_assign = re.match(r'^EXPORT\s+(\w+)\s*:=', line, re.IGNORECASE)
            if m_export_assign and not cur:
                self._module_vars.add(_safe_name(m_export_assign.group(1)))
            for ma in re.finditer(r'(\w+)(?:\[.+?\])?\s*:=', line):
                if cur:
                    asgn.add(_safe_name(ma.group(1)))
                else:
                    self._module_vars.add(_safe_name(ma.group(1)))
        if cur:
            self._locals[cur] = loc
            self._globals[cur] = asgn - loc

    # ─────────────────────────────────────────────────────────────────
    # Header / Footer emission
    # ─────────────────────────────────────────────────────────────────

    def _emit_header(self):
        self._emit0("# Auto-generated by HP PPL Emulator")
        self._emit0("import sys, math, sympy")
        self._emit0("from src.ppl_emulator.runtime.engine import HPPrimeRuntime")
        self._emit0("from src.ppl_emulator.runtime.types import PPLList, PPLString, PPLMatrix")
        self._emit0("_rt = HPPrimeRuntime()")
        self._emit0("HVars = _rt.HVarsCall")
        self._emit0("DelHVars = _rt.DelHVars")
        self._emit0("Programs = _rt.ProgramsCall")
        self._emit0("TICKS = _rt.TICKS")
        # Bind every builtin directly so PPL code can call them without _rt. prefix
        for fn in BUILTINS:
            self._emit0(f"{fn} = _rt.{fn}")
            safe = _safe_name(fn)
            if safe != fn:
                self._emit0(f"{safe} = _rt.{fn}")
        self._emit0("CAS = _rt.CAS")
        self._emit0("COERCE = _rt.COERCE")
        self._emit0("Finance = _rt.Finance")
        self._emit0("pi = math.pi")      # HP Prime π constant
        self._emit0("e = math.e")        # HP Prime e constant
        self._emit0("_ppl_factorial = lambda x: math.factorial(int(x))")
        # CAS symbolic variables — used by PPL CAS calls
        self._emit0("x, y, z, t, a, b, c = sympy.symbols('x y z t a b c')")
        self._emit0("n = k = s = r = m = 0")   # numeric loop/scratch vars
        self._emit0("Ans = 0")                  # HP Prime last-result register
        # 2-D matrix slice helper: emitted into every transpiled program
        self._emit0("def _ppl_slice_2d(mat, r0, r1, c0, c1):")
        self._emit0("    return PPLList([row[c0:c1] for row in mat[r0:r1]])")
        self._emit0("")

    def _emit_footer(self, out_path):
        if self._export:
            nparams = len(self._export_params)
            if nparams > 0:
                # Pull runtime-supplied entry args (from --args flag); fall back to 0
                args = ", ".join(f"_rt._get_entry_arg({i})" for i in range(nparams))
            else:
                args = ""
            self._emit0(f"_ppl_result = {self._export}({args})")
            # If the function returned a value, display it (mirrors HP Prime home-screen history)
            self._emit0("if _ppl_result is not None:")
            self._emit0(f"    _rt.PRINT(_ppl_result)")
        self._emit0(f"_rt.save({repr(out_path)})")

    # ─────────────────────────────────────────────────────────────────
    # Phase 3: Line-by-line transpilation
    # ─────────────────────────────────────────────────────────────────

    def _transpile_line(self, line):
        line = line.strip()
        if not line:
            return
        # Skip forward declarations: bare user-function calls at module level
        if self.indent_level == 0 and self._cur_fn is None:
            m_fwd = re.match(r'^([A-Za-z_]\w*)\s*\(([^()]*)\)\s*;?$', line)
            if m_fwd:
                fn_names = {name for name, _ in self._fn_order}
                if m_fwd.group(1) in fn_names or m_fwd.group(1).upper() in {n.upper() for n, _ in self._fn_order}:
                    return  # forward declaration — skip
        m = re.match(r'^(EXPORT|PROCEDURE)\s+(\w+)(?:\s*\((.*?)\))?;?$', line, re.IGNORECASE)
        if m and m.group(3) is None and line.rstrip().endswith(';'):
            m = None
        if m:
            fn_name = m.group(2)
            raw_params = [p.strip() for p in (m.group(3) or '').split(',') if p.strip()]
            self._cur_fn = fn_name
            self.indent_level = 0
            params = [_safe_name(p) for p in raw_params]
            self._fn_params[fn_name.upper()] = raw_params  # stored for BEGIN injection
            self._emit(f"def {fn_name}({', '.join(params)}):")
            self.indent_level = 1
            self._block_stack = [('PROCEDURE', self._cur_line_raw)]
            gvars = self._globals.get(fn_name, set())
            if gvars:
                self._emit(f"global {', '.join(sorted(gvars))}")
            # Register arity so CHECK_ARITY can validate call sites at runtime
            self._emit(f"_rt.REGISTER_FN('{fn_name}', {len(raw_params)})")
            return
        m_exportvars = re.match(r'^EXPORT\s+([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)+)\s*;?$', line, re.IGNORECASE)
        if m_exportvars and '(' not in line and ':=' not in line:
            return
        m_exportsym = re.match(r'^EXPORT\s+([A-Za-z_]\w*)\s*;?$', line, re.IGNORECASE)
        if m_exportsym and '(' not in line and ':=' not in line:
            return
        # EXPORT variable := value  (exported global, not a function)
        m_expvar = re.match(r'^EXPORT\s+(.+)$', line, re.IGNORECASE)
        if m_expvar:
            line = m_expvar.group(1).strip()

        if re.match(r'^BEGIN;?$', line, re.IGNORECASE):
            self._emit("_rt.PUSH_BLOCK()")
            # If this BEGIN opens a function body, inject parameters into the scope
            if self._cur_fn and len(self._block_stack) == 1 and self._block_stack[0][0] == 'PROCEDURE':
                fn_key = self._cur_fn.upper()
                for p in self._fn_params.get(fn_key, []):
                    safe = _safe_name(p)
                    self._emit(f"_rt.SET_VAR('{p.upper()}', {safe}, is_local=True)")
            self._block_stack.append(('BEGIN', self._cur_line_raw))
            return
        if re.match(r'^END;?$', line, re.IGNORECASE):
            if self._last_out_is_block_header(): self._emit("pass")
            self._emit("_rt.POP_BLOCK()")
            _bts_popped = self._block_stack.pop() if self._block_stack else ('UNKNOWN', 0)
            if _bts_popped[0] != 'CASE':
                self.indent_level = max(0, self.indent_level - 1)
            if self._iferr_stack and self._iferr_stack[-1] == self.indent_level:
                self._iferr_stack.pop()
                self._iferr_in_except.discard(self.indent_level)

            if _bts_popped[0] == 'DEFAULT' and self._block_stack and self._block_stack[-1][0] == 'CASE':
                # PPL CASE's END closes BOTH the DEFAULT branch AND the enclosing CASE
                self._block_stack.pop()
                if self._case_stack:
                    self._case_stack.pop()
            elif _bts_popped[0] == 'CASE':
                if self._case_stack:
                    self._case_stack.pop()

            if self.indent_level == 0:
                if self._block_stack and self._block_stack[-1][0] == 'PROCEDURE':
                    self._block_stack.pop()
                elif self._block_stack:
                    btype, bline = self._block_stack[-1]
                    raise SyntaxError(f"Line {self._cur_line_raw}: Unexpected END; unclosed {btype} block at line {bline}")
                self._cur_fn = None
                self._emit()        # blank line after function
                self._block_stack = []
            return
        
        # CASE statement
        if re.match(r'^CASE;?$', line, re.IGNORECASE):
            self._case_stack.append({'first': True, 'has_default': False, 'indent': self.indent_level})
            self._block_stack.append(('CASE', self._cur_line_raw))
            return
        
        # DEFAULT (inside CASE)
        m = re.match(r'^DEFAULT\b\s*(.*)$', line, re.IGNORECASE)
        if m and self._case_stack:
            verb = "if True:" if self._case_stack[-1]['first'] else "else:"
            self._emit(verb)
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('DEFAULT', self._cur_line_raw))
            self._case_stack[-1]['has_default'] = True
            self._case_stack[-1]['first'] = False
            if m.group(1):
                self._transpile_line(m.group(1))
            return

        m = re.match(r'^LOCAL\b\s+(.+?);?$', line, re.IGNORECASE)
        if m:
            for d in _split_locals(m.group(1)):
                name_match = re.match(r'^(\w+)', d)
                if name_match:
                    name = name_match.group(1)
                    if name.upper() in _SYSTEM_GLOBALS:
                        raise SyntaxError(f"Line {self._cur_line_raw}: Cannot shadow system global '{name}' in LOCAL declaration")
                am = re.match(r'(\w+)\[(\d+)\]\s*(?::=\s*(.+))?', d)
                if am:
                    # LOCAL name[size]  →  pre-allocated zero list
                    self._emit(f"_rt.SET_VAR('{am.group(1).upper()}', PPLList([0] * {am.group(2)}), is_local=True)")
                else:
                    im = re.match(r'(\w+)\s*:=\s*(.+)', d)
                    if im:
                        # LOCAL name := expr  →  initialise with expression
                        self._emit(f"_rt.SET_VAR('{im.group(1).upper()}', {self._xf(im.group(2))}, is_local=True)")
                    else:
                        # LOCAL name  →  initialise to 0
                        self._emit(f"_rt.SET_VAR('{d.strip().upper()}', 0, is_local=True)")
            return
        # FOR ... DOWNTO (descending loop)
        # Supports: FROM start STEP step DOWNTO stop DO  and  FROM start DOWNTO stop STEP step DO
        m = re.match(r'^FOR\s+(\w+)\s+FROM\s+(.+?)\s+STEP\s+(.+?)\s+DOWNTO\s+(.+?)\s+DO\s*(.*)$', line, re.IGNORECASE)
        if m:
            start = self._xf(m.group(2))
            step  = self._xf(m.group(3))
            stop  = self._xf(m.group(4))
            self._emit(f"for {_safe_name(m.group(1))} in range(int({start}), int({stop}) + (1 if -int({step}) > 0 else -1), -int({step})):")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('FOR', self._cur_line_raw))
            self._emit(f"_rt.SET_VAR('{m.group(1).upper()}', {_safe_name(m.group(1))})")
            if m.group(5):
                self._transpile_line(m.group(5))
            return
        m = re.match(r'^FOR\s+(\w+)\s*:=\s*(.+?)\s+STEP\s+(.+?)\s+DOWNTO\s+(.+?)\s+DO\s*(.*)$', line, re.IGNORECASE)
        if m:
            start = self._xf(m.group(2))
            step  = self._xf(m.group(3))
            stop  = self._xf(m.group(4))
            self._emit(f"for {_safe_name(m.group(1))} in range(int({start}), int({stop}) + (1 if -int({step}) > 0 else -1), -int({step})):")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('FOR', self._cur_line_raw))
            self._emit(f"_rt.SET_VAR('{m.group(1).upper()}', {_safe_name(m.group(1))})")
            if m.group(5):
                self._transpile_line(m.group(5))
            return
        m = re.match(r'^FOR\s+(\w+)\s+FROM\s+(.+?)\s+DOWNTO\s+(.+?)(?:\s+STEP\s+(.+?))?\s+DO\s*(.*)$', line, re.IGNORECASE)
        if m:
            start = self._xf(m.group(2))
            stop  = self._xf(m.group(3))
            step  = self._xf(m.group(4)) if m.group(4) else '1'
            self._emit(f"for {_safe_name(m.group(1))} in range(int({start}), int({stop}) + (1 if -int({step}) > 0 else -1), -int({step})):")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('FOR', self._cur_line_raw))
            self._emit(f"_rt.SET_VAR('{m.group(1).upper()}', {_safe_name(m.group(1))})")
            if m.group(5):
                self._transpile_line(m.group(5))
            return
        m = re.match(r'^FOR\s+(\w+)\s*:=\s*(.+?)\s+DOWNTO\s+(.+?)(?:\s+STEP\s+(.+?))?\s+DO\s*(.*)$', line, re.IGNORECASE)
        if m:
            start = self._xf(m.group(2))
            stop  = self._xf(m.group(3))
            step  = self._xf(m.group(4)) if m.group(4) else '1'
            self._emit(f"for {_safe_name(m.group(1))} in range(int({start}), int({stop}) + (1 if -int({step}) > 0 else -1), -int({step})):")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('FOR', self._cur_line_raw))
            self._emit(f"_rt.SET_VAR('{m.group(1).upper()}', {_safe_name(m.group(1))})")
            if m.group(5):
                self._transpile_line(m.group(5))
            return
        # FOR ... TO (ascending): check for missing DO keyword
        if re.match(r'^FOR\b', line, re.IGNORECASE):
            # PPL supports both orderings:
            #   FROM start STEP step TO stop DO  (STEP before TO)
            #   FROM start TO stop STEP step DO  (STEP after TO)
            m = re.match(r'^FOR\s+(\w+)\s+FROM\s+(.+?)\s+STEP\s+(.+?)\s+TO\s+(.+?)\s+DO\b\s*(.*)$', line, re.IGNORECASE)
            if m:
                # STEP before TO form
                start = self._xf(m.group(2))
                step  = self._xf(m.group(3))
                stop  = self._xf(m.group(4))
                tail  = m.group(5)
            else:
                m = re.match(r'^FOR\s+(\w+)\s+FROM\s+(.+?)\s+TO\s+(.+?)(?:\s+STEP\s+(.+?))?\s+DO\b\s*(.*)$', line, re.IGNORECASE)
                if m:
                    # STEP after TO form (or no STEP)
                    start = self._xf(m.group(2))
                    stop  = self._xf(m.group(3))
                    step  = self._xf(m.group(4)) if m.group(4) else '1'
                    tail  = m.group(5)
                else:
                    m = re.match(r'^FOR\s+(\w+)\s*:=\s*(.+?)\s+STEP\s+(.+?)\s+TO\s+(.+?)\s+DO\b\s*(.*)$', line, re.IGNORECASE)
                    if m:
                        start = self._xf(m.group(2))
                        step  = self._xf(m.group(3))
                        stop  = self._xf(m.group(4))
                        tail  = m.group(5)
                    else:
                        m = re.match(r'^FOR\s+(\w+)\s*:=\s*(.+?)\s+TO\s+(.+?)(?:\s+STEP\s+(.+?))?\s+DO\b\s*(.*)$', line, re.IGNORECASE)
                        if not m:
                            raise SyntaxError(f"Line {self._cur_line_raw}: Expected 'DO' in FOR loop")
                        start = self._xf(m.group(2))
                        stop  = self._xf(m.group(3))
                        step  = self._xf(m.group(4)) if m.group(4) else '1'
                        tail  = m.group(5)
            self._emit(f"for {_safe_name(m.group(1))} in range(int({start}), int({stop}) + (1 if int({step}) > 0 else -1), int({step})):")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('FOR', self._cur_line_raw))
            self._emit(f"_rt.SET_VAR('{m.group(1).upper()}', {_safe_name(m.group(1))})")
            if tail:
                self._transpile_line(tail)
            return
        
        m = re.match(r'^WHILE\s+(.+?)\s+DO\b\s*(.*)$', line, re.IGNORECASE)
        if m:
            self._emit(f"while {self._xf(m.group(1))}:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('WHILE', self._cur_line_raw))
            if m.group(2):
                self._transpile_line(m.group(2))
            return
        m_rep = re.match(r'^REPEAT\s*(.*?)\s*UNTIL\s+(.+?);?\s*$', line, re.IGNORECASE)
        if m_rep:
            self._emit('while True:')
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('REPEAT', self._cur_line_raw))
            body = m_rep.group(1).strip()
            if body:
                for s in body.split(';'):
                    if s.strip():
                        self._transpile_line(s.strip())
            self._emit(f'if {self._xf(m_rep.group(2))}: break')
            self._emit("_rt.POP_BLOCK()")
            self.indent_level -= 1
            if self._block_stack: self._block_stack.pop()
            return
        m_repeat_inline = re.match(r'^REPEAT\s+(.+)$', line, re.IGNORECASE)
        if m_repeat_inline:
            self._emit("while True:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('REPEAT', self._cur_line_raw))
            for stmt in _split_inline_statements(m_repeat_inline.group(1)):
                self._transpile_line(stmt)
            return
        if re.match(r'^REPEAT;?$', line, re.IGNORECASE):
            self._emit("while True:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('REPEAT', self._cur_line_raw))
            return
        m = re.match(r'^UNTIL\s+(.+?);?$', line, re.IGNORECASE)
        if m:
            if self._last_out_is_block_header():
                self._emit("pass")
            self._emit(f"if {self._xf(m.group(1))}: break")
            self._emit("_rt.POP_BLOCK()")
            self.indent_level = max(0, self.indent_level - 1)
            if self._block_stack:
                self._block_stack.pop()
            return
        
        m = re.match(r'^IF\s+(.+?)(?:\s+THEN\b|(?<![A-Za-z_])\s*THEN\b)\s*(.*)$', line, re.IGNORECASE)
        if m:
            inline_tail = m.group(2).strip()
            m_inline = re.match(r'^(.+?)\s*(?:ELSE\s+(.+?)\s*)?END;?$', inline_tail, re.IGNORECASE)
            if m_inline:
                self._transpile_line(f"IF {m.group(1)} THEN")
                for stmt in _split_inline_statements(m_inline.group(1)):
                    self._transpile_line(stmt)
                if m_inline.group(2):
                    self._transpile_line("ELSE")
                    for stmt in _split_inline_statements(m_inline.group(2)):
                        self._transpile_line(stmt)
                self._transpile_line("END")
                return
            # Only treat as a CASE branch if this IF is at the CASE's own indent level
            if self._case_stack and self.indent_level == self._case_stack[-1]['indent']:
                verb = "if" if self._case_stack[-1]['first'] else "elif"
                self._case_stack[-1]['first'] = False
                self._emit(f"{verb} {self._xf(m.group(1))}:")
            else:
                self._emit(f"if {self._xf(m.group(1))}:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('IF', self._cur_line_raw))
            if m.group(2):
                self._transpile_line(m.group(2))
            return
        m = re.match(r'^IFERR\b\s*(.+?)\s+THEN\b\s*(.*)$', line, re.IGNORECASE)
        if m:
            self._emit("try:")
            self._iferr_stack.append(self.indent_level)
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('IFERR', self._cur_line_raw))
            self._transpile_line(m.group(1))
            base_indent = self._iferr_stack[-1]
            self._emit("_rt.POP_BLOCK()")
            self.indent_level -= 1
            if self._block_stack:
                self._block_stack.pop()
            self._emit("except:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('IF', self._cur_line_raw))
            self._iferr_in_except.add(base_indent)
            if m.group(2):
                self._transpile_line(m.group(2))
            return
        m = re.match(r'^IFERR\b\s*(.*)$', line, re.IGNORECASE)
        if m:
            self._emit("try:")
            self._iferr_stack.append(self.indent_level)
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('IFERR', self._cur_line_raw))
            if m.group(1):
                self._transpile_line(m.group(1))
            return
        m = re.match(r'^THEN\b\s*(.*)$', line, re.IGNORECASE)
        if m:
            if self._iferr_stack and self._iferr_stack[-1] == self.indent_level - 1:
                base_indent = self._iferr_stack[-1]
                self._emit("_rt.POP_BLOCK()")
                self.indent_level -= 1
                if self._block_stack:
                    self._block_stack.pop()
                self._emit("except:")
                self.indent_level += 1
                self._emit("_rt.PUSH_BLOCK()")
                # The except block is tracked as 'IF' so the matching END closes it
                self._block_stack.append(('IF', self._cur_line_raw))
                self._iferr_in_except.add(base_indent)
                if m.group(1):
                    self._transpile_line(m.group(1))
                return
        m = re.match(r'^ELSE\s+IF\s+(.+?)\s+THEN\b\s*(.*)$', line, re.IGNORECASE)
        if m:
            if self._last_out_is_block_header(): self._emit("pass")
            self._emit("_rt.POP_BLOCK()")
            prev = self.indent_level
            self.indent_level = max(1, self.indent_level - 1)
            if self.indent_level < prev and self._block_stack:
                self._block_stack.pop()
            self._emit(f"elif {self._xf(m.group(1))}:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('IF', self._cur_line_raw))
            if m.group(2):
                self._transpile_line(m.group(2))
            return
        m = re.match(r'^ELSE\b\s*(.*)$', line, re.IGNORECASE)
        if m:
            if (
                self._iferr_stack
                and self._iferr_stack[-1] == self.indent_level - 1
                and self._iferr_stack[-1] in self._iferr_in_except
            ):
                base_indent = self._iferr_stack[-1]
                if self._last_out_is_block_header():
                    self._emit("pass")
                self._emit("_rt.POP_BLOCK()")
                prev = self.indent_level
                self.indent_level = max(0, self.indent_level - 1)
                if self.indent_level < prev and self._block_stack:
                    self._block_stack.pop()
                self._emit("else:")
                self.indent_level += 1
                self._emit("_rt.PUSH_BLOCK()")
                self._block_stack.append(('IFERR_ELSE', self._cur_line_raw))
                self._iferr_in_except.discard(base_indent)
                if m.group(1):
                    self._transpile_line(m.group(1))
                return
            if self._last_out_is_block_header(): self._emit("pass")
            self._emit("_rt.POP_BLOCK()")
            prev = self.indent_level
            self.indent_level = max(1, self.indent_level - 1)
            if self.indent_level < prev and self._block_stack:
                self._block_stack.pop()
            self._emit("else:")
            self.indent_level += 1
            self._emit("_rt.PUSH_BLOCK()")
            self._block_stack.append(('IF', self._cur_line_raw))
            if m.group(1):
                self._transpile_line(m.group(1))
            return
        m = re.match(r'^RETURN\s*(.*?);?$', line, re.IGNORECASE)
        if m:
            # Ensure return expressions are evaluated before popping scopes.
            _push_block_types = {'BEGIN', 'IF', 'WHILE', 'FOR', 'REPEAT', 'IFERR', 'DEFAULT'}
            ret_expr = m.group(1).strip() if m.group(1) else ""
            if ret_expr:
                self._emit(f"_ppl_ret = {self._xf(ret_expr)}")
            for btype, _ in reversed(self._block_stack):
                if btype in _push_block_types:
                    self._emit("_rt.POP_BLOCK()")
            if ret_expr:
                self._emit("return _ppl_ret")
            else:
                self._emit("return None")
            return
        if re.match(r'^BREAK;?$', line, re.IGNORECASE):
            has_loop = any(b[0] in ('FOR', 'WHILE', 'REPEAT') for b in self._block_stack)
            self._emit("break" if has_loop else "pass")
            return
        if re.match(r'^CONTINUE;?$', line, re.IGNORECASE):
            has_loop = any(b[0] in ('FOR', 'WHILE', 'REPEAT') for b in self._block_stack)
            self._emit("continue" if has_loop else "pass")
            return
        # Handle ▶ (STO) operator: expr▶target
        sto_idx = _find_top_level_operator(line, '▶')
        if sto_idx != -1:
            lhs_expr = line[:sto_idx].strip()
            rhs_target = line[sto_idx + 1 :].strip().rstrip(';').strip()
            if rhs_target.upper() not in _PPL_KEYWORDS:
                rhs = self._xf(lhs_expr)
                try:
                    self._validate_lvalue(rhs_target)
                except SyntaxError:
                    pass
                else:
                    m_paren = re.match(rf'^({_IDENT})\s*\((.+)\)$', rhs_target)
                    m_bracket = re.match(rf'^({_IDENT})\s*\[(.+)\]$', rhs_target)
                    if m_paren:
                        var = m_paren.group(1).upper()
                        indices = _split_top_level_args(m_paren.group(2))
                        chain = ''.join(f'[{self._xf(idx)}]' for idx in indices)
                        self._emit(f"_rt.GET_VAR('{var}', {self._cur_line_raw}).value{chain} = {rhs}")
                    elif m_bracket:
                        var = m_bracket.group(1).upper()
                        raw_idx = m_bracket.group(2)
                        if ',' in raw_idx:
                            indices = _split_top_level_args(raw_idx)
                            chain = ''.join(f'[{self._xf(idx)}]' for idx in indices)
                            self._emit(f"_rt.GET_VAR('{var}', {self._cur_line_raw}).value{chain} = {rhs}")
                        else:
                            self._emit(f"_rt.GET_VAR('{var}', {self._cur_line_raw}).value[{self._xf(raw_idx)}] = {rhs}")
                    else:
                        self._emit(f"_rt.SET_VAR('{rhs_target.upper()}', {rhs})")
                    return
        # Import _slice_bound at the top level or use it here
        from .expressions import _slice_bound
        assign_idx = _find_top_level_operator(line, ':=')
        if assign_idx != -1:
            lhs = line[:assign_idx].strip()
            rhs = self._xf(line[assign_idx + 2 :].strip().rstrip(';').strip())
            self._validate_lvalue(lhs)
            m_paren   = re.match(rf'^({_IDENT})\s*\((.+)\)$', lhs)   # paren-indexed: name(i) or name(i,j)
            m_bracket = re.match(rf'^({_IDENT})\s*\[(.+)\]$', lhs)   # bracket-indexed: name[i]
            if m_paren:
                var = m_paren.group(1).upper()
                # Discrepancy 2: PPLList/PPLMatrix.__setitem__ handles 1-based conversion
                # internally, so we pass the raw PPL index without subtracting 1.
                indices = _split_top_level_args(m_paren.group(2))
                chain = ''.join(f'[{self._xf(idx)}]' for idx in indices)
                self._emit(f"_rt.GET_VAR('{var}', {self._cur_line_raw}).value{chain} = {rhs}")
            elif m_bracket:
                var = m_bracket.group(1).upper()
                raw_idx = m_bracket.group(2)
                # Discrepancy 2: same — PPLList handles 1-based internally
                if ',' in raw_idx:
                    indices = _split_top_level_args(raw_idx)
                    chain = ''.join(f'[{self._xf(idx)}]' for idx in indices)
                    self._emit(f"_rt.GET_VAR('{var}', {self._cur_line_raw}).value{chain} = {rhs}")
                else:
                    self._emit(f"_rt.GET_VAR('{var}', {self._cur_line_raw}).value[{self._xf(raw_idx)}] = {rhs}")
            else:
                self._emit(f"_rt.GET_VAR('{lhs.upper()}', {self._cur_line_raw}).value = {rhs}")
            return
        m = re.match(r'^CHOOSE\s*\(\s*(\w+)\s*,\s*(.+?)\);?$', line, re.IGNORECASE)
        if m:
            self._emit(f"_rt.SET_VAR('{m.group(1).upper()}', _rt.CHOOSE({self._xf(m.group(2))}))")
            return
        
        # INPUT(var, ...)
        m_input = re.match(r'^INPUT\s*\(\s*(\w+)\s*(?:,\s*(.+?))?\s*\);?$', line, re.IGNORECASE)
        if m_input:
            var = m_input.group(1).upper()
            args = f", {self._xf(m_input.group(2))}" if m_input.group(2) else ""
            self._emit(f"_rt.INPUT('{var}'{args})")
            return

        # DIMGROB_P(Gk, w, h [, color])
        m_dimgrob = re.match(r'^DIMGROB_P\s*\(\s*(\w+)\s*,\s*(.*?)\);?$', line, re.IGNORECASE)
        if m_dimgrob:
            target = m_dimgrob.group(1).upper()
            args = m_dimgrob.group(2)
            self._emit(f"_rt.SET_VAR('{target}', _rt.DIMGROB_P({self._xf(args)}))")
            return
        
        self._emit(self._xf(line.rstrip(';')))

    # ─────────────────────────────────────────────────────────────────
    # Main entry point
    # ─────────────────────────────────────────────────────────────────

    def transpile(self, ppl_code, out_path='screen.png'):
        """Transpile a PPL source string to Python and return it as a string."""
        code = self._preprocess(ppl_code)
        self._first_pass(code)
        self._emit_header()

        # Accumulate continuation lines (unclosed parens/braces or trailing operator)
        pending: list[str] = []

        for line_num, raw in enumerate(code.splitlines(), 1):
            self._cur_line_raw = line_num
            cleaned = _strip_comment(raw).strip()

            if not cleaned:
                if not pending:
                    self._emit()  # preserve blank lines between functions
                continue

            pending.append(cleaned)
            combined = " ".join(pending)

            # Count unmatched brackets in the string-erased copy to detect continuations
            erased      = _erase_strings(combined)
            paren_depth = erased.count('(') - erased.count(')')
            brace_depth = erased.count('{') - erased.count('}')
            trailing_op = bool(
                re.search(r'(?:[-+*/^]|\b(?:AND|OR|XOR|MOD|DIV))\s*$', erased, re.IGNORECASE)
            )

            # Expression is still open — wait for more input lines
            if paren_depth > 0 or brace_depth > 0 or _has_open_string_literal(combined) or combined.endswith(',') \
                    or combined.endswith(':=') or trailing_op \
                    or _header_needs_continuation(erased):
                continue

            # Split the accumulated line(s) on semicolons, respecting strings and brackets
            stmts: list[str] = []
            stmt_chars: list[str] = []
            in_string  = False
            nest_depth = 0
            i = 0
            while i < len(combined):
                ch = combined[i]
                if ch == '"':
                    if not in_string:
                        in_string = True
                        stmt_chars.append('"')
                    else:
                        # "" inside a string is an escaped quote, not end-of-string
                        if i + 1 < len(combined) and combined[i + 1] == '"':
                            stmt_chars.append('""')
                            i += 1
                        else:
                            in_string = False
                            stmt_chars.append('"')
                elif in_string and ch == '\\' and i + 1 < len(combined) and combined[i + 1] == '"':
                    stmt_chars.append('\\"')
                    i += 1
                elif not in_string and ch in '([{':
                    nest_depth += 1
                    stmt_chars.append(ch)
                elif not in_string and ch in ')]}':  
                    nest_depth = max(0, nest_depth - 1)
                    stmt_chars.append(ch)
                elif ch == ';' and not in_string and nest_depth == 0:
                    # Semicolon at the top level — statement boundary
                    s = ''.join(stmt_chars).strip()
                    if s:
                        stmts.append(s)
                    stmt_chars = []
                else:
                    stmt_chars.append(ch)
                i += 1

            last_stmt = ''.join(stmt_chars).strip()
            if last_stmt:
                stmts.append(last_stmt)

            for s in stmts:
                self._transpile_line(s)

            pending = []
        
        if self._block_stack:
            btype, bline = self._block_stack[-1]
            raise SyntaxError(f"End of file: {btype} block at line {bline} was never closed")

        self._emit0("")
        self._emit_footer(out_path)
        return '\n'.join(self._out)

def transpile(ppl_code, out_path='screen.png'): return Transpiler().transpile(ppl_code, out_path)
