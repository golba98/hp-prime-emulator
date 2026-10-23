"""
HP Prime runtime engine.

Provides ``HPPrimeRuntime`` — the Python object that is bound to ``_rt`` in
every transpiled program.  It emulates the HP Prime built-in environment:

  * Graphics (RECT_P, LINE_P, CIRCLE_P, TEXTOUT_P, …) via live pygame window
  * Math builtins (SIN/COS take degrees, matching HP Prime behaviour)
  * String functions (LEFT, RIGHT, MID, INSTRING, …)
  * I/O (PRINT, INPUT, CHOOSE, GETKEY) with headless-safe defaults
  * Variable scoping via ScopeStack (locals shadow globals, PPLVar boxing)
"""
import os
import sys
import math
import random
import time
import re
from PIL import Image, ImageDraw, ImageFont  # type: ignore
try:
    import pygame  # type: ignore
except Exception:
    pygame = None
from src.ppl_emulator.runtime.types import PPLList, PPLString, PPLVar, PPLMatrix
from src.ppl_emulator.runtime.resource_budget import ResourceBudget, ResourceLimitExceeded
from src.ppl_emulator.transpiler.constants import BUILTINS, _STRUCTURAL, _SYSTEM_GLOBALS
from src.ppl_emulator.runtime.ppl_runtime import CAS, ppl_expr, DET

_APP_MISSING = object()

def _coerce_list(value):
    """Wrap a plain Python list in the appropriate PPL type.

    A flat list becomes PPLList; a list-of-lists becomes PPLMatrix
    (Discrepancy 1: matrix elements must be mutable).
    PPLVar items are unwrapped to their current value so that list literals
    like [i] store a snapshot of i's value, not a live reference to the variable.
    """
    if isinstance(value, PPLVar):
        # Unwrap the boxed variable so list literals capture the value, not the
        # live PPLVar reference (which would mutate as the variable changes).
        return _coerce_list(value.value)
    if isinstance(value, (PPLList, PPLMatrix, PPLString)):
        if isinstance(value, PPLList):
            # If a PPLList contains list-like rows, treat it as a 2-D matrix.
            if value and all(isinstance(item, (list, PPLList, PPLMatrix)) for item in value):
                return PPLMatrix([[_coerce_list(x) for x in row] for row in value])
        return value
    if isinstance(value, list):
        # Discrepancy 1 fix: a Python list-of-lists (i.e. a 2-D structure) becomes a
        # PPLMatrix so that element assignment is allowed but dimension changes are not.
        if value and all(isinstance(item, (list, PPLList, PPLMatrix)) for item in value):
            return PPLMatrix([[_coerce_list(x) for x in row] for row in value])
        return PPLList([_coerce_list(item) for item in value])
    return value

class HP_Grob:
    """An HP Prime GROB (graphic object) backed by a Pillow Image."""

    def __init__(self, width, height, color=None, runtime=None):
        self.width = int(width)
        self.height = int(height)
        bg = runtime._color(color) if (color is not None and runtime) else (255, 255, 255)
        if runtime is not None and getattr(runtime, "_budget", None) is not None and runtime._budget.active:
            runtime._budget.account_value(self, runtime=runtime, label="grob")
        self.img = Image.new('RGB', (self.width, self.height), bg)
        self.draw = ImageDraw.Draw(self.img)

    def clear(self, color, runtime):
        self.draw.rectangle([0, 0, self.width-1, self.height-1], fill=runtime._color(color))

    def draw_rect(self, x1, y1, x2, y2, color, fill=None):
        self.draw.rectangle([x1, y1, x2, y2], outline=color, fill=fill)

    def draw_line(self, x1, y1, x2, y2, color):
        self.draw.line([x1, y1, x2, y2], fill=color)

    def draw_point(self, x, y, color):
        self.draw.point([x, y], fill=color)

    def blit(self, source, x, y, w=None, h=None, sx=0, sy=0, sw=None, sh=None):
        """Copy a portion of source GROB onto this GROB."""
        if not isinstance(source, HP_Grob):
            return
        
        # Source rectangle
        src_w = sw if sw is not None else source.width
        src_h = sh if sh is not None else source.height
        box = (sx, sy, sx + src_w, sy + src_h)
        region = source.img.crop(box)
        
        # Target scaling
        target_w = w if w is not None else src_w
        target_h = h if h is not None else src_h
        if target_w != src_w or target_h != src_h:
            region = region.resize((int(target_w), int(target_h)), Image.NEAREST)
        
        self.img.paste(region, (int(x), int(y)))

    def copy(self, runtime=None):
        clone = HP_Grob(self.width, self.height, runtime=runtime)
        clone.img = self.img.copy()
        clone.draw = ImageDraw.Draw(clone.img)
        return clone


class _CatalogStoreProxy:
    def __init__(self, runtime, kind):
        self._rt = runtime
        self._kind = kind

    def _key(self, key):
        return str(self._rt._val(key))

    def __len__(self):
        if self._kind == "HVARS":
            return len(self._rt.HVarsCall())
        if self._kind == "PROGRAMS":
            return len(self._rt.ProgramsCall())
        if self._kind == "NOTES":
            return len(self._rt.Notes())
        if self._kind == "AFILES":
            return len(self._rt.AFiles())
        return 0

    def __iter__(self):
        if self._kind == "HVARS":
            return iter(self._rt.HVarsCall())
        if self._kind == "PROGRAMS":
            return iter(self._rt.ProgramsCall())
        if self._kind == "NOTES":
            return iter(self._rt.Notes())
        if self._kind == "AFILES":
            return iter(self._rt.AFiles())
        return iter(())

    def __getitem__(self, key):
        if self._kind == "HVARS":
            return self._rt.HVarsCall(key)
        if self._kind == "PROGRAMS":
            return self._rt.ProgramsCall(key)
        if self._kind == "NOTES":
            return self._rt.Notes(key)
        if self._kind == "AFILES":
            return self._rt.AFiles(key)
        raise KeyError(key)

    def __setitem__(self, key, value):
        if self._kind == "HVARS":
            self._rt.SET_VAR(self._key(key), self._rt._val(value))
            self._rt._refresh_catalog_vars()
            return
        if self._kind == "PROGRAMS":
            self._rt._program_store[self._key(key)] = str(self._rt._val(value))
            self._rt._refresh_catalog_vars()
            return
        if self._kind == "NOTES":
            self._rt._notes_store[self._key(key)] = str(self._rt._val(value))
            self._rt._refresh_catalog_vars()
            return
        if self._kind == "AFILES":
            resolved = self._rt._val(value)
            if isinstance(resolved, HP_Grob):
                self._rt._afile_store[self._key(key)] = resolved.copy(runtime=self._rt)
            else:
                self._rt._afile_store[self._key(key)] = _coerce_list(resolved)
            self._rt._refresh_catalog_vars()
            return
        raise KeyError(key)

class PPLError(Exception):
    """Runtime error raised by PPL built-ins (e.g. arity mismatch)."""

    def __init__(self, message, line_no=None):
        super().__init__(message)
        self.message = message
        self.line_no = line_no

class ScopeStack:
    """Stack of variable scopes.

    Each PUSH_BLOCK call adds a new scope frame; POP_BLOCK removes it.
    Variable lookup walks from innermost to outermost scope.
    In compiled_mode, accessing an undeclared variable raises NameError
    instead of silently creating a zero-initialised global.
    """

    def __init__(self, runtime=None, compiled_mode=False):
        self.stack = [{}]
        self.runtime = runtime
        # Discrepancy 3 fix: when compiled_mode is True (i.e. we are running a compiled
        # .hpprgm program), referencing an undeclared variable is a hard NameError rather
        # than silently creating a global initialised to 0.  Interactive/REPL callers
        # leave compiled_mode=False to preserve the tolerant behaviour.
        self.compiled_mode = compiled_mode

    def push(self):
        self.stack.append({})
        if self.runtime is not None and getattr(self.runtime, "_budget", None) is not None:
            self.runtime._budget.push_block()

    def pop(self):
        if len(self.stack) > 1:
            self.stack.pop()
        if self.runtime is not None and getattr(self.runtime, "_budget", None) is not None:
            self.runtime._budget.pop_block()

    def get(self, name, line_no=None):
        name = name.upper()
        for scope in reversed(self.stack):
            if name in scope:
                return scope[name]
        if self.compiled_mode:
            loc = f" (line {line_no})" if line_no is not None else ""
            raise NameError(
                f"Undeclared variable '{name}' referenced{loc}. "
                "Declare it with LOCAL or assign it before use."
            )
        # Interactive / tolerant mode: auto-initialise to 0 in global scope.
        from src.ppl_emulator.runtime.types import PPLVar
        new_var = PPLVar(0)
        self.stack[0][name] = new_var
        return new_var

    def set(self, name, value, is_local=False):
        name = name.upper()
        from src.ppl_emulator.runtime.types import PPLVar, PPLList, PPLString

        if isinstance(value, str) and not isinstance(value, PPLString):
            value = PPLString(value)
        if isinstance(value, list) and not isinstance(value, (PPLList, PPLMatrix)):
            value = _coerce_list(value)

        if is_local:
            if name in self.stack[-1] and isinstance(self.stack[-1][name], PPLVar):
                self.stack[-1][name].value = value.value if isinstance(value, PPLVar) else value
            else:
                self.stack[-1][name] = value if isinstance(value, PPLVar) else PPLVar(value)
        else:
            for scope in reversed(self.stack):
                if name in scope:
                    if isinstance(scope[name], PPLVar):
                        scope[name].value = value.value if isinstance(value, PPLVar) else value
                    else:
                        scope[name] = value if isinstance(value, PPLVar) else PPLVar(value)
                    return
            self.stack[0][name] = value if isinstance(value, PPLVar) else PPLVar(value)

class HPPrimeRuntime:
    """Central emulation object — bound to ``_rt`` in every transpiled program.

    Manages variable scopes, graphics output, and all HP Prime built-in
    functions. Graphics are rendered in a 320×240 pygame window and mirrored
    to a Pillow image for compatibility and PNG export.
    """

    # Mutable defaults use None sentinels to avoid the shared-list Python anti-pattern.
    # The CLI sets these before exec() and reads them in __init__.
    _pending_input_queue: list | None = None
    # When True, always allow PNG save even if a live pygame window exists.
    _force_save_output_default: bool = False
    # Discrepancy 3: set to True before exec()ing transpiled code to enable strict
    # compiled-mode variable lookup (undeclared vars raise NameError).
    # The CLI sets this via HPPrimeRuntime._compiled_mode = True before exec().
    _compiled_mode: bool = False
    # CLI --args values fed into the EXPORT function's default call.
    _entry_args: list | None = None
    # Print output routing: 'both' | 'screen' | 'terminal'
    # 'screen'   → PRINT renders on pygame display only (silent terminal)
    # 'terminal' → PRINT goes to stdout only (no pygame text rendering)
    # 'both'     → PRINT does both (default)
    _print_mode: str = 'both'
    _pending_choice_queue: list | None = None
    _pending_key_queue: list | None = None
    _pending_mouse_queue: list | None = None
    _GETKEY_MAX_CALLS: int = 30
    _ISKEYDOWN_MAX_CALLS: int = 30
    _WAIT_MAX_CALLS: int = 10
    _DEFAULT_TOTAL_BYTES: int = 32 * 1024 * 1024
    _DEFAULT_SINGLE_OBJECT_BYTES: int = 8 * 1024 * 1024
    _DEFAULT_OUTPUT_CHARS: int = 256 * 1024
    _DEFAULT_CALL_DEPTH: int = 128
    _DEFAULT_BLOCK_DEPTH: int = 128
    _DEFAULT_LINE_EVENTS: int = 1_000_000
    _DEFAULT_ELAPSED_SECONDS: float = 8.0
    _pending_elapsed_seconds: float | None = None
    _pending_elapsed_seconds_set: bool = False

    @staticmethod
    def _stream_is_tty(stream) -> bool:
        try:
            return bool(stream) and bool(stream.isatty())
        except Exception:
            return False

    @classmethod
    def _should_enable_pygame(cls) -> bool:
        headless = os.environ.get("PPL_EMULATOR_HEADLESS", "").strip().lower()
        if headless and headless not in {"0", "false", "no", "off"}:
            return False

        force_gui = os.environ.get("PPL_EMULATOR_GUI", "").strip().lower()
        if force_gui and force_gui not in {"0", "false", "no", "off"}:
            return True

        # Redirected stdout/stderr means we're in a non-interactive run such as
        # tests, scripted execution, or captured subprocess output. Keep those
        # deterministic and headless instead of opening modal pygame dialogs.
        return cls._stream_is_tty(sys.stdout) and cls._stream_is_tty(sys.stderr)

    def __init__(self, compiled_mode=None):
        self.width  = 320
        self.height = 240
        elapsed_seconds = HPPrimeRuntime._pending_elapsed_seconds
        elapsed_seconds_set = HPPrimeRuntime._pending_elapsed_seconds_set
        HPPrimeRuntime._pending_elapsed_seconds = None
        HPPrimeRuntime._pending_elapsed_seconds_set = False
        if not elapsed_seconds_set:
            elapsed_seconds = HPPrimeRuntime._DEFAULT_ELAPSED_SECONDS
        # Track whether any graphics call has mutated the framebuffer.
        self.screen_is_dirty = False
        # Track last saved output path (if any).
        self._last_saved_path = None
        self._force_save_output = HPPrimeRuntime._force_save_output_default
        # Optional real-time pygame display.
        self._pg_enabled = False
        self._pg_should_close = False
        self._pg_window = None
        self._pg_window_size = (560, 460)
        self._pg_screen = None
        self._pg_font = None
        self._pg_fonts: dict[int, object] = {}
        self.grobs = [HP_Grob(self.width, self.height, runtime=self)] + [None]*9
        self.G0 = self.grobs[0]
        self.img = self.G0.img
        self.draw = self.G0.draw
        self._getkey_calls = 0
        self._wait_calls   = 0
        self._input_cancelled = 0
        self._iskeydown_calls = 0
        self._choose_calls = 0
        self._program_store: dict[str, str] = {}
        self._notes_store: dict[str, str] = {}
        self._afile_store: dict[str, object] = {}
        self._input_queue: list = list(HPPrimeRuntime._pending_input_queue or [])
        HPPrimeRuntime._pending_input_queue = None
        self._choice_queue: list = list(HPPrimeRuntime._pending_choice_queue or [])
        HPPrimeRuntime._pending_choice_queue = None
        self._key_queue: list[int] = [int(v) for v in (HPPrimeRuntime._pending_key_queue or [])]
        HPPrimeRuntime._pending_key_queue = None
        self._mouse_queue: list = list(HPPrimeRuntime._pending_mouse_queue or [])
        HPPrimeRuntime._pending_mouse_queue = None
        self._held_keys: set[int] = set()
        # Terminal text buffer — lines printed via PRINT() shown on the pygame screen.
        self._terminal_lines: list[str] = []
        self._terminal_font = None   # initialised lazily after pygame.font.init()
        # Set to True once any explicit graphics call (RECT_P, LINE_P, etc.) is made.
        # When True, _render_terminal does NOT clear the screen so graphics are preserved.
        self._graphics_mode: bool = False
        self._budget = ResourceBudget(
            max_total_bytes=HPPrimeRuntime._DEFAULT_TOTAL_BYTES,
            max_single_object_bytes=HPPrimeRuntime._DEFAULT_SINGLE_OBJECT_BYTES,
            max_output_chars=HPPrimeRuntime._DEFAULT_OUTPUT_CHARS,
            max_call_depth=HPPrimeRuntime._DEFAULT_CALL_DEPTH,
            max_block_depth=HPPrimeRuntime._DEFAULT_BLOCK_DEPTH,
            max_line_events=HPPrimeRuntime._DEFAULT_LINE_EVENTS,
            max_elapsed_seconds=elapsed_seconds,
        )
        self.CAS    = CAS(self)
        self.Finance = _FinanceMock()
        self._fn_registry: dict[str, int] = {}
        self._app_state = {
            "current_app": "",
            "current_view": "Home",
            "apps": {},
        }
        # Discrepancy 3: resolve compiled_mode — explicit arg > class flag > default False.
        if compiled_mode is None:
            compiled_mode = HPPrimeRuntime._compiled_mode
        self.scopes = ScopeStack(runtime=self, compiled_mode=compiled_mode)
        self.COERCE = _coerce_list
        self._refresh_catalog_vars()
        self.SET_VAR('APROGRAM', PPLString(''))
        self.SET_VAR('MYLANGS', _coerce_list([]))
        self._budget.activate(self)

        # Initialize pygame window if available.
        if pygame is not None and self._should_enable_pygame():
            try:
                pygame.init()
                pygame.font.init()
                self._pg_window = pygame.display.set_mode(self._pg_window_size, pygame.RESIZABLE)
                self._pg_screen = pygame.Surface((self.width, self.height)).convert()
                pygame.display.set_caption("HP Prime Emulator")
                self._pg_enabled = True
                self._pg_font = self._get_pg_font(14)
                # Start with a white background like the PIL buffer.
                self._pg_screen.fill((255, 255, 255))
                self._present_display()
                pygame.display.flip()
            except Exception:
                self._pg_enabled = False

        for i in range(10):
            name = f"G{i}"
            setattr(self, name, self.grobs[i])
            self.SET_VAR(name, self.grobs[i])
        
        # Pre-seed HP Prime's named single-letter variables (A-Z)
        # and standard collections (L0-L9, M0-M9, C0-C9).
        import string
        from src.ppl_emulator.runtime.types import PPLList, PPLMatrix
        for v in string.ascii_uppercase:
            # X, Y, Z, T, N, K are special in PPL, but all single letters are globals.
            self.SET_VAR(v, 0)
        
        for i in range(10):
            self.SET_VAR(f"L{i}", PPLList([]))
            self.SET_VAR(f"M{i}", PPLMatrix([[0]]))
            self.SET_VAR(f"C{i}", complex(0))

        # System settings (H-prefix)
        self.SET_VAR('HANGLE', 1)   # 1 = Degrees (matching current hardcoded behavior)
        self.SET_VAR('HFORMAT', 0)  # 0 = Standard
        self.SET_VAR('HSIZE', 1)    # 1 = Medium font
        self.SET_VAR('ANS', 0)
        
        self.SET_VAR('PI', math.pi)
        self.SET_VAR('E', math.e)

    # Builtins that fall through to the CAS engine when no concrete method exists.
    # Only true symbolic/algebra operations belong here — everything else should
    # have an explicit implementation above.
    _CAS_DELEGATED = frozenset([
        'DIFF', 'INTEGRATE', 'SOLVE', 'EXPAND', 'FACTOR', 'SIMPLIFY',
        'ZEROS', 'CZEROS', 'LIMIT', 'SERIES', 'TAYLOR', 'LAPLACE', 'INVLAPLACE',
        'PARTFRAC', 'NDERIV', 'NINT', 'FSOLVE', 'POLY', 'DEGREE', 'COEFF',
        'ROOTS', 'FACTORS', 'EXACT', 'VAR',
        'CORR', 'COV', 'MEAN', 'MEDIAN', 'STDDEV', 'REG', 'PREDX', 'PREDY',
        'FFT', 'IFFT',
    ])

    def __getattr__(self, name):
        name_up = name.upper()
        if name_up in HPPrimeRuntime._CAS_DELEGATED:
            def cas_wrapper(*args):
                return getattr(self.CAS, name.lower())(*args)
            return cas_wrapper
        if name_up in _STRUCTURAL or name_up in _SYSTEM_GLOBALS:
            # Structural keywords (LOCAL, EXPORT, etc.) and system globals (VAR, EXACT, etc.)
            # are bound in the transpiled header for pass-through; never called as functions.
            return lambda *a, **kw: None
        if name_up in BUILTINS:
            raise PPLError(
                f"'{name_up}()' is advertised as a builtin but has no runtime implementation. "
                f"File a bug or use a workaround."
            )
        raise AttributeError(f"'HPPrimeRuntime' object has no attribute '{name}'")

    def REGISTER_FN(self, name: str, arity: int):
        self._fn_registry[name.upper()] = arity

    def CHECK_ARITY(self, name: str, got: int, line_no=None):
        key = name.upper()
        if key in self._fn_registry:
            expected = self._fn_registry[key]
            if got != expected:
                raise PPLError(f'Function "{name}" expects {expected} argument(s), got {got}.', line_no)

    def GET_VAR(self, name, line_no=None):
        return self.scopes.get(name, line_no)

    def SET_VAR(self, name, value, is_local=False):
        self.scopes.set(name, value, is_local)
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_value(self._val(value), runtime=self, label=str(name))
            self._budget.recalculate(self)
        self._refresh_catalog_vars()

    def _val(self, x):
        """Unwrap a PPLVar to its underlying value."""
        if isinstance(x, PPLVar):
            return x.value
        return x

    def _refresh_catalog_vars(self):
        home_vars = []
        for scope in self.scopes.stack:
            for name, value in scope.items():
                if name in {"HVARS", "PROGRAMS", "NOTES", "AFILES", "APROGRAM"}:
                    continue
                if name in _SYSTEM_GLOBALS:
                    continue
                if re.fullmatch(r'[GLM]\d', name):
                    continue
                if isinstance(value, PPLVar):
                    home_vars.append(name)
        self.scopes.stack[0]["PROGRAMS"] = PPLVar(_CatalogStoreProxy(self, "PROGRAMS"))
        self.scopes.stack[0]["HVARS"] = PPLVar(_CatalogStoreProxy(self, "HVARS"))
        self.scopes.stack[0]["NOTES"] = PPLVar(_CatalogStoreProxy(self, "NOTES"))
        self.scopes.stack[0]["AFILES"] = PPLVar(_CatalogStoreProxy(self, "AFILES"))

    def _safe_console_print(self, text) -> None:
        rendered = str(text)
        stream = sys.stdout
        try:
            print(rendered)
            return
        except UnicodeEncodeError:
            pass

        encoding = getattr(stream, "encoding", None) or "utf-8"
        fallback = rendered.encode(encoding, errors="backslashreplace").decode(encoding, errors="ignore")
        try:
            stream.write(fallback + "\n")
        except Exception:
            print(rendered.encode("ascii", errors="backslashreplace").decode("ascii"))
        try:
            stream.flush()
        except Exception:
            pass

    def _emit_terminal_line(self, text) -> str:
        rendered = str(text)
        self._safe_console_print(rendered)
        self._terminal_lines.append(rendered)
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_output(rendered, self)
        return rendered

    def _assign_expr(self, name, value, line_no=None):
        self.SET_VAR(name, self._val(value))
        return self.GET_VAR(name, line_no).value

    def _assign_index_expr(self, name, indices, value, line_no=None):
        name_up = name.upper()
        resolved_indices = [self._val(idx) for idx in indices]
        resolved_value = self._val(value)

        if name_up == "PROGRAMS":
            key = str(resolved_indices[0]) if resolved_indices else ""
            self._program_store[key] = str(resolved_value)
            self._refresh_catalog_vars()
            return PPLString(self._program_store[key])

        if name_up == "HVARS":
            key = str(resolved_indices[0]) if resolved_indices else ""
            self.SET_VAR(key, resolved_value)
            return self.GET_VAR(key, line_no).value

        if name_up == "NOTES":
            key = str(resolved_indices[0]) if resolved_indices else ""
            self._notes_store[key] = str(resolved_value)
            return PPLString(self._notes_store[key])

        if name_up == "AFILES":
            key = str(resolved_indices[0]) if resolved_indices else ""
            if isinstance(resolved_value, HP_Grob):
                self._afile_store[key] = resolved_value.copy(runtime=self)
                return self._afile_store[key].copy(runtime=self)
            stored = _coerce_list(resolved_value)
            self._afile_store[key] = stored
            return stored

        container = self.GET_VAR(name_up, line_no).value
        ref = container
        for idx in resolved_indices[:-1]:
            ref = ref[idx]
        ref[resolved_indices[-1]] = resolved_value
        return ref[resolved_indices[-1]]

    def HVarsCall(self, name=None):
        if name is None:
            home_vars = []
            for scope in self.scopes.stack:
                for var_name, value in scope.items():
                    if var_name in {"HVARS", "PROGRAMS", "NOTES", "AFILES", "APROGRAM"}:
                        continue
                    if var_name in _SYSTEM_GLOBALS:
                        continue
                    if re.fullmatch(r'[GLM]\d', var_name):
                        continue
                    if isinstance(value, PPLVar):
                        home_vars.append(var_name)
            return PPLList(sorted(set(home_vars)))
        return self.GET_VAR(str(self._val(name)).upper()).value

    def HVARS(self, name=None):
        return self.HVarsCall(name)

    def DelHVars(self, name):
        name_up = str(self._val(name)).upper()
        for scope in reversed(self.scopes.stack):
            if name_up in scope:
                del scope[name_up]
                break
        self._refresh_catalog_vars()
        return 1

    def DELHVARS(self, name):
        return self.DelHVars(name)

    def ProgramsCall(self, name=None):
        if name is None:
            return PPLList(sorted(self._program_store))
        return PPLString(self._program_store.get(str(self._val(name)), ""))

    def PROGRAMS(self, name=None):
        return self.ProgramsCall(name)

    def NOTES(self, name=None):
        return self.Notes(name)

    def Notes(self, name=None):
        if name is None:
            return PPLList(sorted(self._notes_store))
        return PPLString(self._notes_store.get(str(self._val(name)), ""))

    def AFILES(self, name=None):
        return self.AFiles(name)

    def AFiles(self, name=None):
        if name is None:
            return PPLList(sorted(self._afile_store))
        value = self._afile_store.get(str(self._val(name)))
        if isinstance(value, HP_Grob):
            return value.copy(runtime=self)
        return value if value is not None else PPLString("")

    def DelAFiles(self, name):
        self._afile_store.pop(str(self._val(name)), None)
        return 1

    def DELAFILES(self, name):
        return self.DelAFiles(name)

    def TICKS(self):
        return int(time.monotonic() * 1000)

    def MYLANGS(self):
        return self.MyLangs()

    def MyLangs(self):
        return PPLList(sorted(self._notes_store))

    def _normalise_app_name(self, name=None) -> str:
        if name is None:
            return self._app_state["current_app"] or "Home"
        app_name = str(self._val(name)).strip()
        return app_name or "Home"

    def _ensure_app_slot(self, app_name: str):
        apps = self._app_state["apps"]
        return apps.setdefault(
            app_name,
            {
                "current_view": "Symb" if app_name != "Home" else "Home",
                "custom_views": {},
                "symb_setup": {},
                "plot_setup": {},
                "num_setup": {},
            },
        )

    def _set_current_app(self, name=None) -> str:
        app_name = self._normalise_app_name(name)
        slot = self._ensure_app_slot(app_name)
        self._app_state["current_app"] = app_name
        self._app_state["current_view"] = slot["current_view"]
        self.SET_VAR("APROGRAM", PPLString("" if app_name == "Home" else app_name))
        return app_name

    def _set_current_view(self, view_name: str) -> PPLString:
        app_name = self._set_current_app(self._app_state["current_app"] or "Home")
        slot = self._ensure_app_slot(app_name)
        slot["current_view"] = view_name
        self._app_state["current_view"] = view_name
        return PPLString(view_name)

    def _app_setup_value(self, bucket: str, key=None, value=_APP_MISSING):
        app_name = self._set_current_app(self._app_state["current_app"] or "Home")
        slot = self._ensure_app_slot(app_name)
        store = slot[bucket]
        if key is None:
            return PPLList([PPLString(k) for k in sorted(store)])
        key_name = str(self._val(key))
        if value is _APP_MISSING:
            return store.get(key_name, 0)
        store[key_name] = self._val(value)
        return store[key_name]

    def START(self, name=None):
        app_name = self._set_current_app(name)
        return PPLString("" if app_name == "Home" else app_name)

    def RESET(self, name=None):
        app_name = self._normalise_app_name(name or self._app_state["current_app"] or "Home")
        self._app_state["apps"][app_name] = {
            "current_view": "Symb" if app_name != "Home" else "Home",
            "custom_views": {},
            "symb_setup": {},
            "plot_setup": {},
            "num_setup": {},
        }
        self._set_current_app(app_name)
        return 1

    def VIEW(self, name=None, handler=_APP_MISSING):
        if name is None:
            return PPLString(self._app_state["current_view"])
        app_name = self._set_current_app(self._app_state["current_app"] or "Home")
        slot = self._ensure_app_slot(app_name)
        label = str(self._val(name))
        if handler is not _APP_MISSING:
            slot["custom_views"][label] = self._val(handler)
        self._set_current_view(label)
        registered = slot["custom_views"].get(label, _APP_MISSING)
        if callable(registered):
            return registered()
        if registered is not _APP_MISSING:
            return registered
        return PPLString(label)

    def SYMB(self):
        return self._set_current_view("Symb")

    def PLOT(self):
        return self._set_current_view("Plot")

    def INFO(self):
        return self._set_current_view("Info")

    def SYMBSETUP(self, key=None, value=_APP_MISSING):
        return self._app_setup_value("symb_setup", key, value)

    def PLOTSETUP(self, key=None, value=_APP_MISSING):
        return self._app_setup_value("plot_setup", key, value)

    def NUMSETUP(self, key=None, value=_APP_MISSING):
        return self._app_setup_value("num_setup", key, value)

    def _set_ans(self, value):
        self.SET_VAR('ANS', value)

    def queue_input(self, *values):
        self._input_queue.extend(values)

    def queue_choice(self, *values):
        self._choice_queue.extend(values)

    def queue_key(self, *keycodes):
        for code in keycodes:
            ic = int(code)
            self._key_queue.append(ic)
            self._held_keys.add(ic)

    def queue_mouse(self, *events):
        self._mouse_queue.extend(events)

    def _normalise_mouse_event(self, event):
        if isinstance(event, PPLVar):
            event = event.value
        if event is None:
            return PPLList([])
        if isinstance(event, dict):
            values = [
                event.get('x', -1),
                event.get('y', -1),
                event.get('ox', event.get('x', -1)),
                event.get('oy', event.get('y', -1)),
                event.get('type', 0),
            ]
        elif isinstance(event, (list, tuple, PPLList)):
            values = list(event)
        else:
            return PPLList([])
        while len(values) < 5:
            values.append(-1 if len(values) < 4 else 0)
        return PPLList(values[:5])

    def _resolve_grob_slot(self, raw):
        value = self._val(raw)
        if isinstance(raw, int) and 0 <= raw <= 9:
            return int(raw)
        if isinstance(raw, float) and raw.is_integer() and 0 <= int(raw) <= 9:
            return int(raw)
        if isinstance(value, int) and 0 <= value <= 9:
            return int(value)
        return None

    def _ensure_grob_slot(self, slot, width=None, height=None, color=None):
        if slot is None:
            return None
        if self.grobs[slot] is None:
            grob = HP_Grob(width or self.width, height or self.height, color, self)
            self.grobs[slot] = grob
            name = f"G{slot}"
            setattr(self, name, grob)
            self.SET_VAR(name, grob)
            if slot == 0:
                self.G0 = grob
                self.img = grob.img
                self.draw = grob.draw
            return grob
        return self.grobs[slot]

    def _is_explicit_grob_slot_ref(self, raw):
        if isinstance(raw, int) and 0 <= raw <= 9:
            return True
        if isinstance(raw, float) and raw.is_integer() and 0 <= int(raw) <= 9:
            return True
        if isinstance(raw, str):
            text = raw.strip().upper()
            return len(text) == 2 and text.startswith("G") and text[1].isdigit()
        return False

    def _resolve_grob_ref(self, raw):
        slot = self._resolve_grob_slot(raw)
        if slot is not None:
            return self._ensure_grob_slot(slot), slot
        value = self._val(raw)
        if isinstance(value, HP_Grob):
            return value, None
        return None, None

    def _truncate_text_to_width(self, text, width, measure):
        if width is None or width <= 0:
            return text
        if measure(text) <= width:
            return text
        lo, hi = 0, len(text)
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if measure(text[:mid]) <= width:
                lo = mid
            else:
                hi = mid - 1
        return text[:lo]

    def PUSH_BLOCK(self):
        self.scopes.push()

    def POP_BLOCK(self):
        self.scopes.pop()

    # ── Pygame helpers ───────────────────────────────────────────────

    def _get_pg_font(self, size):
        if pygame is None:
            return None
        size = max(1, int(size))
        cached = self._pg_fonts.get(size)
        if cached is not None:
            return cached

        font_obj = None
        for name in ("consolas", "couriernew", "courier", "dejavusansmono", "liberationmono", "monospace"):
            try:
                path = pygame.font.match_font(name)
                if path:
                    font_obj = pygame.font.Font(path, size)
                    break
            except Exception:
                continue
        if font_obj is None:
            font_obj = pygame.font.SysFont("monospace", size)

        self._pg_fonts[size] = font_obj
        return font_obj

    def _present_display(self):
        if not self._pg_enabled or pygame is None or self._pg_window is None or self._pg_screen is None:
            return

        ww, wh = self._pg_window.get_size()

        # ── HP Prime G1 screen-only view ──────────────────────────────
        # Dark aluminium background (matches G1 body colour).
        BG       = (28, 30, 36)
        BEZEL    = (44, 46, 54)       # outer bezel face
        BEZEL_HI = (72, 76, 88)       # highlight rim
        BEZEL_SH = (18, 19, 23)       # shadow rim
        INNER    = (10, 11, 14)       # recessed screen surround
        HDR_BG   = (22, 24, 30)       # title-bar strip
        HDR_TXT  = (180, 185, 200)    # title-bar text
        ACCENT   = (0, 113, 197)      # HP blue accent line

        self._pg_window.fill(BG)

        # Outer bezel — rounded rectangle with a subtle 3-D rim.
        margin = max(18, min(ww, wh) // 18)
        bezel_rect = pygame.Rect(margin, margin, ww - 2 * margin, wh - 2 * margin)
        pygame.draw.rect(self._pg_window, BEZEL, bezel_rect, border_radius=18)
        # Highlight (top-left edge)
        pygame.draw.rect(self._pg_window, BEZEL_HI, bezel_rect, width=1, border_radius=18)
        # Shadow inset (bottom-right feel via a second rect 1 px smaller)
        shadow_r = bezel_rect.inflate(-2, -2)
        pygame.draw.rect(self._pg_window, BEZEL_SH, shadow_r, width=1, border_radius=16)

        # Title bar strip at top of bezel.
        hdr_h = max(22, bezel_rect.height // 14)
        hdr_rect = pygame.Rect(bezel_rect.x + 1, bezel_rect.y + 1,
                               bezel_rect.width - 2, hdr_h)
        # Clip to rounded top corners by drawing over bezel background first.
        pygame.draw.rect(self._pg_window, HDR_BG, hdr_rect)
        # HP blue accent line under the title bar.
        accent_y = hdr_rect.bottom
        pygame.draw.line(self._pg_window, ACCENT,
                         (bezel_rect.x + 4, accent_y),
                         (bezel_rect.right - 4, accent_y), 2)
        # "HP Prime" label in title bar.
        try:
            label_font = pygame.font.SysFont("segoeui,arial,sans-serif", max(11, hdr_h - 6), bold=True)
        except Exception:
            label_font = pygame.font.SysFont("monospace", max(11, hdr_h - 6))
        lbl = label_font.render("HP Prime", True, HDR_TXT)
        self._pg_window.blit(lbl, (bezel_rect.x + 10, hdr_rect.y + (hdr_h - lbl.get_height()) // 2))

        # Recessed screen area below title bar.
        pad = max(10, bezel_rect.width // 28)
        screen_area_top = accent_y + pad
        screen_area = pygame.Rect(
            bezel_rect.x + pad,
            screen_area_top,
            bezel_rect.width - 2 * pad,
            bezel_rect.bottom - screen_area_top - pad,
        )
        pygame.draw.rect(self._pg_window, INNER, screen_area, border_radius=6)
        pygame.draw.rect(self._pg_window, BEZEL_SH, screen_area, width=2, border_radius=6)

        # Scale 320×240 content to fit screen_area, keeping 4:3.
        target_w = screen_area.width - 8
        target_h = int(target_w * 3 / 4)
        if target_h > screen_area.height - 8:
            target_h = screen_area.height - 8
            target_w = int(target_h * 4 / 3)
        target_w = max(4, target_w)
        target_h = max(3, target_h)

        screen_rect = pygame.Rect(
            screen_area.centerx - target_w // 2,
            screen_area.centery - target_h // 2,
            target_w,
            target_h,
        )
        scaled = pygame.transform.scale(self._pg_screen, (target_w, target_h))
        self._pg_window.blit(scaled, screen_rect.topleft)

    def _pg_pump(self):
        if not self._pg_enabled or pygame is None:
            return
        pygame.event.pump()
        for event in pygame.event.get([pygame.QUIT, pygame.VIDEORESIZE]):
            if event.type == pygame.QUIT:
                self._pg_should_close = True
                break
            if event.type == pygame.VIDEORESIZE and self._pg_window is not None:
                new_w = max(380, int(event.w))
                new_h = max(320, int(event.h))
                self._pg_window = pygame.display.set_mode((new_w, new_h), pygame.RESIZABLE)
                self._present_display()
                pygame.display.flip()
        if self._pg_should_close:
            self.close()
            raise SystemExit(0)

    # ── Dialog helpers ───────────────────────────────────────────────

    def _dlg_font(self, size=15):
        try:
            return pygame.font.SysFont("segoeui,arial,sans-serif", size)
        except Exception:
            return pygame.font.Font(None, size + 2)

    def _render_dialog_on_window(self, lines, title=None, footer=None, input_text=None):
        """Draw a modal dialog directly on the pygame window surface (above the bezel).

        Calls _present_display() first so the HP Prime content is visible behind
        the semi-transparent shade, then overlays the dialog box.  Flips at the end.
        Does nothing in headless (no pygame) mode.
        """
        if not self._pg_enabled or pygame is None or self._pg_window is None:
            return
        self._present_display()  # draw current content + bezel into window
        ww, wh = self._pg_window.get_size()

        # Semi-transparent darkening shade over the whole window
        shade = pygame.Surface((ww, wh), pygame.SRCALPHA)
        shade.fill((0, 0, 0, 160))
        self._pg_window.blit(shade, (0, 0))

        # Dialog box geometry
        dlg_w = min(ww - 60, 420)
        dlg_h = min(wh - 60, 320)
        dlg_x = (ww - dlg_w) // 2
        dlg_y = (wh - dlg_h) // 2

        BG      = (22, 25, 35)
        BORDER  = (0, 113, 197)
        TITLE_C = (0, 160, 230)
        TEXT_C  = (210, 218, 228)
        HINT_C  = (100, 110, 130)
        FIELD_BG = (38, 42, 56)

        pygame.draw.rect(self._pg_window, BG,     (dlg_x, dlg_y, dlg_w, dlg_h), border_radius=10)
        pygame.draw.rect(self._pg_window, BORDER, (dlg_x, dlg_y, dlg_w, dlg_h), width=2, border_radius=10)

        font = self._dlg_font(15)
        small = self._dlg_font(12)
        lh = font.get_linesize() + 3
        y = dlg_y + 12

        if title:
            ts = font.render(str(title), True, TITLE_C)
            self._pg_window.blit(ts, (dlg_x + 12, y))
            y += lh
            pygame.draw.line(self._pg_window, BORDER,
                             (dlg_x + 6, y), (dlg_x + dlg_w - 6, y))
            y += 8

        for line in lines:
            ls = font.render(str(line), True, TEXT_C)
            self._pg_window.blit(ls, (dlg_x + 12, y))
            y += lh
            if y > dlg_y + dlg_h - lh * 2:
                break  # clip if too many lines

        if input_text is not None:
            fld_y = dlg_y + dlg_h - lh - 28
            pygame.draw.rect(self._pg_window, FIELD_BG,
                             (dlg_x + 12, fld_y, dlg_w - 24, lh + 6), border_radius=4)
            pygame.draw.rect(self._pg_window, BORDER,
                             (dlg_x + 12, fld_y, dlg_w - 24, lh + 6), width=1, border_radius=4)
            cursor = '|' if (pygame.time.get_ticks() // 500) % 2 == 0 else ' '
            isurf = font.render(input_text + cursor, True, (255, 255, 255))
            self._pg_window.blit(isurf, (dlg_x + 16, fld_y + 3))

        if footer:
            fs = small.render(str(footer), True, HINT_C)
            self._pg_window.blit(fs, (dlg_x + 12, dlg_y + dlg_h - small.get_linesize() - 8))

        pygame.display.flip()

    def _pg_input_dialog(self, label):
        """Show a text-input dialog on the pygame window. Returns typed string."""
        typed = ''
        while True:
            self._render_dialog_on_window(
                [],
                title=label,
                footer="Enter → confirm   ESC → cancel",
                input_text=typed,
            )
            pygame.time.wait(16)
            self._pg_pump()
            for event in pygame.event.get([pygame.KEYDOWN, pygame.QUIT]):
                if event.type == pygame.QUIT:
                    raise SystemExit(0)
                if event.type == pygame.KEYDOWN:
                    if event.key == pygame.K_RETURN:
                        return typed
                    elif event.key == pygame.K_ESCAPE:
                        return ''
                    elif event.key == pygame.K_BACKSPACE:
                        typed = typed[:-1]
                    elif event.unicode and event.unicode.isprintable():
                        typed += event.unicode

    def _pg_choose_dialog(self, title, options):
        """Show a numbered-list CHOOSE dialog on the pygame window. Returns 1-based index."""
        selected = 0  # 0-based cursor
        n = len(options)
        while True:
            rows = []
            for i, opt in enumerate(options):
                prefix = '▶ ' if i == selected else '  '
                rows.append(f"{prefix}{i + 1}.  {opt}")
            self._render_dialog_on_window(
                rows,
                title=title,
                footer="↑ ↓ to navigate   Enter / 1–9 to choose   ESC = cancel",
            )
            pygame.time.wait(16)
            self._pg_pump()
            for event in pygame.event.get([pygame.KEYDOWN, pygame.QUIT]):
                if event.type == pygame.QUIT:
                    raise SystemExit(0)
                if event.type == pygame.KEYDOWN:
                    if event.key == pygame.K_UP:
                        selected = (selected - 1) % n
                    elif event.key == pygame.K_DOWN:
                        selected = (selected + 1) % n
                    elif event.key == pygame.K_RETURN:
                        return selected + 1
                    elif event.key == pygame.K_ESCAPE:
                        return 0
                    elif pygame.K_1 <= event.key <= pygame.K_9:
                        idx = event.key - pygame.K_0
                        if 1 <= idx <= n:
                            return idx

    # ── Output ───────────────────────────────────────────────────────

    def PRINT(self, *args):
        text = ' '.join(str(a) for a in args)
        mode = HPPrimeRuntime._print_mode
        self.screen_is_dirty = True
        if mode in ('both', 'terminal'):
            self._safe_console_print(text)
        if args:
            self._set_ans(args[0] if len(args) == 1 else PPLString(text))
        self._terminal_lines.append(text)
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_output(text, self)
        if mode in ('both', 'screen'):
            if self._pg_enabled and pygame is not None and self._pg_screen is not None:
                self._render_terminal()
                self._present_display()
                pygame.display.flip()

    def DISP(self, text='', row=None):
        """HP Prime DISP — show text on the home screen, optionally at a specific row.

        Behaves like PRINT: feeds into the terminal buffer and renders on screen.
        The optional row argument is accepted for compatibility but is ignored in
        the emulator (the terminal buffer scrolls naturally).
        """
        self.PRINT(text)

    def _render_terminal(self):
        """Draw all buffered PRINT lines onto the 320×240 _pg_screen.

        Pure terminal mode (no graphics calls): clears to white first, like the
        HP Prime Home view.  Graphics mode: overlays text on existing content.
        """
        if pygame is None or self._pg_screen is None:
            return
        # Lazily build a small monospace font
        if self._terminal_font is None:
            try:
                self._terminal_font = pygame.font.SysFont("consolas,courier new,monospace", 14)
            except Exception:
                self._terminal_font = pygame.font.Font(None, 16)
        font   = self._terminal_font
        line_h = font.get_linesize() + 2
        pad_x, pad_y = 4, 4
        max_w = max(1, self.width - 2 * pad_x)
        max_lines = max(1, (self.height - 2 * pad_y) // line_h)

        if not self._graphics_mode:
            self._pg_screen.fill((255, 255, 255))

        # Build a flat list of display rows (each logical PRINT line may word-wrap)
        rows: list[str] = []
        for line in self._terminal_lines:
            if font.size(line)[0] <= max_w:
                rows.append(line)
            else:
                # Simple character-level wrap
                cur = ''
                for ch in line:
                    if font.size(cur + ch)[0] <= max_w:
                        cur += ch
                    else:
                        rows.append(cur)
                        cur = ch
                if cur:
                    rows.append(cur)

        # Show only the bottom max_lines rows (scroll)
        visible_rows = rows[-max_lines:]
        for i, row in enumerate(visible_rows):
            surf = font.render(row, True, (0, 0, 0))
            self._pg_screen.blit(surf, (pad_x, pad_y + i * line_h))

    def MSGBOX(self, msg, *extra):
        text = f"[MSG] {msg}"
        mode = HPPrimeRuntime._print_mode
        self.screen_is_dirty = True
        if mode in ('both', 'terminal'):
            self._safe_console_print(text)
        self._terminal_lines.append(text)
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_output(text, self)
        if mode in ('both', 'screen'):
            if self._pg_enabled and pygame is not None and self._pg_screen is not None:
                self._render_terminal()
                self._present_display()
                pygame.display.flip()
        return 1

    def _get_entry_arg(self, idx: int):
        """Return the idx-th CLI --args value, coerced to int/float/str, defaulting to 0."""
        args = HPPrimeRuntime._entry_args or []
        if idx < len(args):
            val = args[idx]
            try:
                return int(val)
            except (ValueError, TypeError):
                try:
                    return float(val)
                except (ValueError, TypeError):
                    return val
        return 0

    def INPUT(self, vars_spec, title="", label=None, help_text=None, reset_value=None, *extra):
        label_text = label if isinstance(label, str) and label else title
        label_text = label_text if label_text else (vars_spec if isinstance(vars_spec, str) else "?")

        # 1. Pre-queued values (from --input flag)
        if self._input_queue:
            val = self._input_queue.pop(0)
            msg = f"[INPUT] '{label_text}' ← '{val}'"
            self._emit_terminal_line(msg)
            if isinstance(vars_spec, str):
                self.SET_VAR(vars_spec, val)
            return 1

        # 2. MOCK_INPUTS environment variable
        if 'MOCK_INPUTS' in os.environ:
            mock = os.environ['MOCK_INPUTS'].split(',')
            if mock:
                val = mock.pop(0)
                os.environ['MOCK_INPUTS'] = ','.join(mock)
                msg = f"[INPUT] mocked '{label_text}' ← '{val}'"
                self._emit_terminal_line(msg)
                if isinstance(vars_spec, str):
                    self.SET_VAR(vars_spec, val)
                return 1

        # 3. Live pygame window — show on-screen input dialog
        if self._pg_enabled and pygame is not None:
            val = self._pg_input_dialog(str(label_text))
            msg = f"[INPUT] '{label_text}' ← '{val}'"
            print(msg)
            self._terminal_lines.append(msg)
            if isinstance(vars_spec, str):
                self.SET_VAR(vars_spec, val)
            return 1 if val != '' else 0

        # 4. Piped stdin
        if sys.stdin and not sys.stdin.isatty():
            line = sys.stdin.readline()
            if line:
                val = line.strip()
                msg = f"[INPUT] '{label_text}' (stdin) ← '{val}'"
                self._emit_terminal_line(msg)
                if isinstance(vars_spec, str):
                    self.SET_VAR(vars_spec, val)
                return 1

        # 5. Interactive terminal
        if sys.stdin and sys.stdin.isatty():
            try:
                val = input(f"[INPUT] {label_text}: ")
                if isinstance(vars_spec, str):
                    self.SET_VAR(vars_spec, val)
                return 1
            except (EOFError, KeyboardInterrupt):
                print()

        # 6. Headless fallback
        self._input_cancelled += 1
        fallback = "" if reset_value is None else reset_value
        msg = f"[INPUT] headless — '{label_text}' defaulting to \"{fallback}\""
        self._emit_terminal_line(msg)
        if isinstance(vars_spec, str):
            self.SET_VAR(vars_spec, fallback)
        return 0

    def CHOOSE(self, title, options, *extra):
        self._choose_calls += 1
        if self._choice_queue:
            return int(self._choice_queue.pop(0))
        if not self._pg_enabled and self._choose_calls > 20:
            raise SystemExit(0)

        # Flatten options into a plain list of strings
        all_opts: list = []
        for item in ([options] + list(extra)):
            if hasattr(item, '__iter__') and not isinstance(item, str):
                all_opts.extend(str(x) for x in item)
            elif item is not None:
                all_opts.append(str(item))

        if self._pg_enabled and pygame is not None:
            return self._pg_choose_dialog(str(title), all_opts)

        msg = f"[CHOOSE] headless — '{title}' → 1 (first option)"
        self._emit_terminal_line(msg)
        return 1

    def WAIT(self, t=0):
        self._wait_calls += 1
        if not self._pg_enabled and self._wait_calls > self._WAIT_MAX_CALLS:
            raise SystemExit(0)
        if self._pg_enabled and pygame is not None:
            self._present_display()
            pygame.display.flip()
            pygame.event.pump()
            self._pg_pump()
            try:
                pygame.time.delay(max(0, int(float(t) * 1000)))
            except Exception:
                time.sleep(float(t))
            return 4
        try:
            delay = max(0.0, float(t))
        except Exception:
            delay = 0.0
        if delay > 0:
            time.sleep(delay)
        return 4 

    def FREEZE(self):
        """Flush the current screen content to the live window and hold it."""
        if self._pg_enabled and pygame is not None:
            self._present_display()
            pygame.display.flip()
            pygame.event.pump()

    def DISP_FREEZE(self):
        """Alias for FREEZE — present current display."""
        self.FREEZE()

    def DRAWMENU(self, *labels):
        """Draw an HP Prime-style F1–F6 menu bar at the bottom of the screen.

        Each positional argument is the label for the next soft-key button.
        Empty string hides that slot.  Labels are rendered in a dark bar with
        HP blue borders, mirroring the real calculator's bottom menu strip.
        """
        self.screen_is_dirty = True
        self._graphics_mode = True
        n_slots = 6
        slot_w  = self.width  // n_slots   # 320 / 6 ≈ 53 px per slot
        bar_h   = 16
        bar_y   = self.height - bar_h      # 240 - 16 = 224

        # ── Pillow ──────────────────────────────────────────────────
        from PIL import ImageFont as _IF
        _bar_fill = (30, 35, 50)
        _border   = (0, 113, 197)
        _text_c   = (200, 210, 220)
        self.draw.rectangle([0, bar_y, self.width - 1, self.height - 1], fill=_bar_fill)
        self.draw.line([0, bar_y, self.width - 1, bar_y], fill=_border, width=1)
        _fnt = _IF.load_default()
        for i in range(n_slots):
            label = str(labels[i]) if i < len(labels) else ''
            if label:
                tx = i * slot_w + 2
                self.draw.text((tx, bar_y + 2), label, fill=_text_c, font=_fnt)
            if i > 0:
                self.draw.line([i * slot_w, bar_y, i * slot_w, self.height - 1],
                               fill=_border, width=1)

        # ── pygame ───────────────────────────────────────────────────
        if self._pg_enabled and pygame is not None and self._pg_screen is not None:
            self._pg_pump()
            _pg_bar_fill = (30, 35, 50)
            _pg_border   = (0, 113, 197)
            _pg_text_c   = (200, 210, 220)
            pygame.draw.rect(self._pg_screen, _pg_bar_fill,
                             (0, bar_y, self.width, bar_h))
            pygame.draw.line(self._pg_screen, _pg_border,
                             (0, bar_y), (self.width - 1, bar_y), 1)
            try:
                _sfont = pygame.font.SysFont("consolas,monospace", 11)
            except Exception:
                _sfont = pygame.font.Font(None, 13)
            for i in range(n_slots):
                label = str(labels[i]) if i < len(labels) else ''
                if label:
                    ts = _sfont.render(label, True, _pg_text_c)
                    self._pg_screen.blit(ts, (i * slot_w + 2, bar_y + 2))
                if i > 0:
                    pygame.draw.line(self._pg_screen, _pg_border,
                                     (i * slot_w, bar_y), (i * slot_w, self.height - 1), 1)

    # HP Prime G1 keycode mapping
    _KEY_MAP = {
        # Navigation
        'up': 2, 'down': 12, 'left': 7, 'right': 8,
        # Functions / Softkeys
        'f1': 15, 'f2': 16, 'f3': 17, 'f4': 18, 'f5': 19, 'f6': 20,
        # Control
        'escape': 4, 'return': 30, 'backspace': 1, 'space': 31,
        # Numbers
        '0': 48, '1': 49, '2': 50, '3': 51, '4': 52,
        '5': 53, '6': 54, '7': 55, '8': 56, '9': 57,
        # Operators
        'plus': 10, 'minus': 11, 'asterisk': 12, 'slash': 13,
        # Letters (approximate G1 keycodes)
        'a': 58, 'b': 59, 'c': 60, 'd': 61, 'e': 62, 'f': 63, 'g': 64,
        'h': 65, 'i': 66, 'j': 67, 'k': 68, 'l': 69, 'm': 70, 'n': 71,
        'o': 72, 'p': 73, 'q': 74, 'r': 75, 's': 76, 't': 77, 'u': 78,
        'v': 79, 'w': 80, 'x': 81, 'y': 82, 'z': 83
    }

    def _pygame_to_ppl_key(self, pg_key):
        """Map a pygame key integer to an HP Prime keycode."""
        if pygame is None: return 0
        name = pygame.key.name(pg_key)
        return self._KEY_MAP.get(name, 0)

    def GETKEY(self):
        if self._pg_enabled and pygame is not None:
            self._pg_pump()
            # Yield 5 ms to the OS so tight key-polling loops don't pin a CPU
            pygame.time.wait(5)
            for event in pygame.event.get(pygame.KEYDOWN):
                keycode = self._pygame_to_ppl_key(event.key)
                if keycode > 0:
                    self._held_keys.add(keycode)
                    return keycode

        if self._key_queue:
            return int(self._key_queue.pop(0))
        
        self._getkey_calls += 1
        if not self._pg_enabled and self._getkey_calls > self._GETKEY_MAX_CALLS:
            return 4   # ESC
        return -1

    def ISKEYDOWN(self, key_code):
        if self._pg_enabled and pygame is not None:
            self._pg_pump()
            pygame.time.wait(1)
            keys = pygame.key.get_pressed()
            # Check all pygame keys and see if any map to this HP Prime key_code
            for pg_idx, pressed in enumerate(keys):
                if pressed:
                    if self._pygame_to_ppl_key(pg_idx) == int(key_code):
                        return True
            return False

        if int(key_code) in self._held_keys:
            return True
        self._iskeydown_calls += 1
        if not self._pg_enabled and self._iskeydown_calls > self._ISKEYDOWN_MAX_CALLS:
            raise SystemExit(0)
        return False

    def SIZE(self, obj):
        """Return the number of elements; 0 for non-sequences."""
        try:
            return len(obj)
        except TypeError:
            return 0

    def DIM(self, obj):
        """Alias for SIZE (HP Prime uses both names)."""
        return self.SIZE(obj)

    def MOUSE(self, idx=0):
        event = self._normalise_mouse_event(self._mouse_queue.pop(0) if self._mouse_queue else None)
        try:
            idx = int(idx)
        except Exception:
            idx = 0
        if idx:
            return event[idx] if event else -1
        return event

    def _color(self, c):
        """Convert an HP Prime packed colour integer to an (R, G, B) tuple."""
        if isinstance(c, tuple) and len(c) >= 3:
            return (int(c[0]) & 255, int(c[1]) & 255, int(c[2]) & 255)
        if isinstance(c, str):
            s = c.strip().lower()
            if s.startswith('#') and s.endswith('h'):
                s = f"0x{s[1:-1]}"
            elif s.endswith('h'):
                s = f"0x{s[:-1]}"
            try:
                c = int(s, 0)
            except (TypeError, ValueError):
                return (0, 0, 0)
        try:
            c = int(c)
        except (TypeError, ValueError):
            return (0, 0, 0)
        c = c & 0xFFFFFF
        return ((c >> 16) & 255, (c >> 8) & 255, c & 255)

    def RGB(self, r, g, b, a=255):
        return (int(r) << 16) | (int(g) << 8) | int(b)

    # ── Graphics ─────────────────────────────────────────────────────

    def RECT_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        # Handle optional GROB argument
        target = self.G0
        args_list = list(args)
        if args_list:
            first, _ = self._resolve_grob_ref(args_list[0])
            if isinstance(first, HP_Grob):
                target = first
                args_list.pop(0)
        
        if not args_list:
            # Clear to white
            target.clear((255, 255, 255), self)
            if target == self.G0 and self._pg_enabled and pygame is not None:
                self._pg_screen.fill((255, 255, 255))
            return

        if len(args_list) == 1:
            fill = self._color(self._val(args_list[0]))
            target.clear(fill, self)
            if target == self.G0 and self._pg_enabled and pygame is not None:
                self._pg_screen.fill(fill)
            return
            
        if len(args_list) < 4:
            return

        x1, y1 = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        x2, y2 = int(float(self._val(args_list[2]))), int(float(self._val(args_list[3])))
        if x2 < x1:
            x1, x2 = x2, x1
        if y2 < y1:
            y1, y2 = y2, y1
        color = self._color(self._val(args_list[4])) if len(args_list) > 4 else (0,0,0)
        fill = self._color(self._val(args_list[5])) if len(args_list) > 5 else None
        
        # Always keep Pillow in sync
        target.draw_rect(x1, y1, x2, y2, color, fill)
        
        # Mirror to pygame ONLY if drawing to G0
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            left, top = min(x1, x2), min(y1, y2)
            w, h = abs(x2 - x1), abs(y2 - y1)
            if fill is not None:
                pygame.draw.rect(self._pg_screen, fill, (left, top, w, h), 0)
            pygame.draw.rect(self._pg_screen, color, (left, top, w, h), 1)

    def LINE_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        args_list = list(args)
        target = self.G0
        if args_list:
            first = self._val(args_list[0])
            if isinstance(first, HP_Grob):
                target = first
                args_list.pop(0)
        
        if len(args_list) < 4:
            return
            
        x1, y1 = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        x2, y2 = int(float(self._val(args_list[2]))), int(float(self._val(args_list[3])))
        color = self._val(args_list[4]) if len(args_list) > 4 else 0
        c = self._color(color)
        
        # Always keep Pillow in sync
        target.draw_line(x1, y1, x2, y2, c)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            pygame.draw.line(
                self._pg_screen, c, (x1, y1), (x2, y2), 1
            )

    def PIXON_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        args_list = list(args)
        target = self.G0
        if args_list:
            first = self._val(args_list[0])
            if isinstance(first, HP_Grob):
                target = first
                args_list.pop(0)
                
        if len(args_list) < 2:
            return
            
        x, y = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        color = self._val(args_list[2]) if len(args_list) > 2 else 0
        c = self._color(color)
        
        # Always keep Pillow in sync
        target.draw_point(x, y, c)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_screen.set_at((x, y), c)

    def PIXOFF_P(self, *args):
        # On HP Prime, PIXOFF_P usually sets the pixel to the background color (white)
        args_list = list(args)
        # We can just call PIXON_P with white color
        # PIXOFF_P([grob,] x, y)
        if args_list:
            first = self._val(args_list[0])
            if isinstance(first, HP_Grob):
                args_list = [first] + args_list[1:] + [0xFFFFFF]
            else:
                args_list = args_list + [0xFFFFFF]
        self.PIXON_P(*args_list)

    def RECT(self, *args): self.RECT_P(*args)
    def LINE(self, *args): self.LINE_P(*args)
    def PIXON(self, *args): self.PIXON_P(*args)
    def PIXOFF(self, *args): self.PIXOFF_P(*args)
    def CIRCLE(self, *args): self.CIRCLE_P(*args)
    def FILLCIRCLE(self, *args): self.FILLCIRCLE_P(*args)
    def BLIT(self, *args): self.BLIT_P(*args)
    def ARC(self, *args): self.ARC_P(*args)
    def TRIANGLE(self, *args): self.TRIANGLE_P(*args)
    def TEXTOUT(self, *args): self.TEXTOUT_P(*args)
    def INVERT(self, *args): self.INVERT_P(*args)
    def GETPIX_P(self, *args): return self.GETPIX(*args)
    def DIMGROB(self, *args): return self.DIMGROB_P(*args)
    def GROBW(self, grob): return self.GROBW_P(grob)
    def GROBH(self, grob): return self.GROBH_P(grob)

    def FILLCIRCLE_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        args_list = list(args)
        target = self.G0
        if args_list:
            first = self._val(args_list[0])
            if isinstance(first, HP_Grob):
                target = first
                args_list.pop(0)
                
        if len(args_list) < 3:
            return
            
        x, y = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        r = int(float(self._val(args_list[2])))
        color = self._val(args_list[3]) if len(args_list) > 3 else 0
        c = self._color(color)
        
        # Always keep Pillow in sync
        target.draw.ellipse([x-r, y-r, x+r, y+r], fill=c, outline=c)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            pygame.draw.circle(self._pg_screen, c, (x, y), r, 0)

    def CIRCLE_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        args_list = list(args)
        target = self.G0
        if args_list:
            first = self._val(args_list[0])
            if isinstance(first, HP_Grob):
                target = first
                args_list.pop(0)
                
        if len(args_list) < 3:
            return
            
        x, y = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        r = int(float(self._val(args_list[2])))
        color = self._val(args_list[3]) if len(args_list) > 3 else 0
        c = self._color(color)
        
        # Always keep Pillow in sync
        target.draw.ellipse([x-r, y-r, x+r, y+r], outline=c)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            pygame.draw.circle(self._pg_screen, c, (x, y), r, 1)

    def TEXTOUT_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        args_list = list(args)
        target = self.G0
        if args_list and isinstance(self._val(args_list[0]), HP_Grob):
            target = self._val(args_list.pop(0))
        elif len(args_list) > 1 and isinstance(self._val(args_list[1]), HP_Grob):
            target = self._val(args_list[1])
            args_list.pop(1)
                
        if len(args_list) < 3:
            return 0
            
        text = str(self._val(args_list[0]))
        x, y = int(float(self._val(args_list[1]))), int(float(self._val(args_list[2])))
        # HP Prime G1 Font mapping: 0=Small, 1=Medium, 2=Large
        f_idx = int(float(self._val(args_list[3]))) if len(args_list) > 3 else 0
        color = self._val(args_list[4]) if len(args_list) > 4 else 0
        width = int(float(self._val(args_list[5]))) if len(args_list) > 5 else None
        background = self._val(args_list[6]) if len(args_list) > 6 else None
        c = self._color(color)
        
        # Determine pixel size
        current_font = int(self._val(self.GET_VAR('HSIZE')))
        size_map = {0: {0: 10, 1: 14, 2: 18}.get(current_font, 14), 1: 10, 2: 18}
        px_size = size_map.get(f_idx, 14)
        pil_font = ImageFont.load_default()
        measure_line = lambda s: int(target.draw.textlength(s, font=pil_font))
        raw_lines = text.splitlines() or [text]
        draw_lines = [self._truncate_text_to_width(line, width, measure_line) for line in raw_lines]
        draw_text = "\n".join(draw_lines)
        rendered_width = max((measure_line(line) for line in draw_lines), default=0)
        if background is not None:
            if "\n" in draw_text:
                bbox = target.draw.multiline_textbbox((x, y), draw_text, font=pil_font)
            else:
                bbox = target.draw.textbbox((x, y), draw_text, font=pil_font)
            target.draw.rectangle(bbox, fill=self._color(background))
        
        # Always write to Pillow
        if "\n" in draw_text:
            target.draw.multiline_text((x, y), draw_text, fill=c, font=pil_font, spacing=0)
        else:
            target.draw.text((x, y), draw_text, fill=c, font=pil_font)
        
        if target == self.G0 and self._pg_enabled and pygame is not None:
            # Also render via pygame font for the live window
            try:
                self._pg_pump()
                pg_font = self._get_pg_font(px_size)
                if pg_font is None:
                    pg_font = pygame.font.SysFont("monospace", px_size)
                line_height = max(1, pg_font.get_linesize())
                render_lines = [self._truncate_text_to_width(line, width, lambda s: pg_font.size(s)[0]) for line in raw_lines]
                for idx, render_line in enumerate(render_lines):
                    surf = pg_font.render(render_line, True, c)
                    top = y + idx * line_height
                    if background is not None:
                        self._pg_screen.fill(self._color(background), surf.get_rect(topleft=(x, top)))
                    if width is not None and width > 0 and surf.get_width() > width:
                        surf = surf.subsurface((0, 0, width, surf.get_height()))
                    self._pg_screen.blit(surf, (x, top))
            except Exception:
                pass
        return rendered_width

    def INVERT_P(self, x1=0, y1=0, x2=319, y2=239):
        self.screen_is_dirty = True
        x1, y1, x2, y2 = int(float(x1)), int(float(y1)), int(float(x2)), int(float(y2))
        # Ensure bounds are within image and ordered
        ix1 = max(0, min(x1, x2, self.width - 1))
        ix2 = max(0, min(max(x1, x2), self.width - 1))
        iy1 = max(0, min(y1, y2, self.height - 1))
        iy2 = max(0, min(max(y1, y2), self.height - 1))
        
        # Always invert the Pillow buffer (canonical store)
        pixels = self.img.load()
        for py in range(iy1, iy2 + 1):
            for px in range(ix1, ix2 + 1):
                r, g, b = pixels[px, py][:3]
                pixels[px, py] = (255 - r, 255 - g, 255 - b)
        # Mirror to pygame if live
        if self._pg_enabled and pygame is not None:
            self._pg_pump()
            for py in range(iy1, iy2 + 1):
                for px in range(ix1, ix2 + 1):
                    r, g, b, _ = self._pg_screen.get_at((px, py))
                    self._pg_screen.set_at((px, py), (255 - r, 255 - g, 255 - b))

    # ── Math ─────────────────────────────────────────────────────────

    def IP(self, x): return int(float(x))
    def FP(self, x): return float(x) - int(float(x))
    def ABS(self, x):
        r = abs(float(x))
        return int(r) if r.is_integer() else r
    def MAX(self, *args): return max(args)
    def MIN(self, *args): return min(args)
    def FLOOR(self, x): return math.floor(float(x))
    def CEILING(self, x): return math.ceil(float(x))
    def ROUND(self, x, n=0):
        n = int(n)
        r = round(float(x), n)
        return int(r) if n == 0 else r
    def SQ(self, x):
        r = float(x) * float(x)
        return int(r) if r.is_integer() else r
    def SQRT(self, x): return math.sqrt(float(x))
    def LOG(self, x): return math.log10(float(x))
    def LN(self, x): return math.log(float(x))
    def EXP(self, x): return math.exp(float(x))
    def _to_rad(self, x):
        """Convert input x to radians based on HANGLE (0=Rad, 1=Deg)."""
        mode = self._val(self.GET_VAR('HANGLE'))
        if mode == 1:  # Degrees
            return math.radians(float(x))
        return float(x)  # Already radians

    def _from_rad(self, rad):
        """Convert radians to output based on HANGLE (0=Rad, 1=Deg)."""
        mode = self._val(self.GET_VAR('HANGLE'))
        if mode == 1:  # Degrees
            return math.degrees(float(rad))
        return float(rad)

    # HP Prime trig operates in degrees by default (HANGLE=1)
    def SIN(self, x): return math.sin(self._to_rad(x))
    def COS(self, x): return math.cos(self._to_rad(x))
    def TAN(self, x): return math.tan(self._to_rad(x))
    def ASIN(self, x): return self._from_rad(math.asin(float(x)))
    def ACOS(self, x): return self._from_rad(math.acos(float(x)))
    def ATAN(self, x): return self._from_rad(math.atan(float(x)))
    def IFTE(self, cond, a, b): return a if cond else b
    def RANDOM(self, *args): return random.random()
    def RANDINT(self, a, b): return random.randint(int(a), int(b))

    def INTEGER(self, x): return int(float(x))
    def REAL(self, x): return float(x)
    def APPROX(self, x):
        """Convert to float approximation (N() equivalent)."""
        try: return float(x)
        except: return x
    def SIGN(self, x):
        x = float(x)
        return 1 if x > 0 else (-1 if x < 0 else 0)
    def TRUNCATE(self, x, n=0):
        n = int(n)
        if n == 0:
            return math.trunc(float(x))
        factor = 10**n
        r = math.trunc(float(x) * factor) / factor
        return int(r) if r.is_integer() else r
    def MANT(self, x):
        """Significand (mantissa) of x in base 10."""
        x = float(x)
        if x == 0: return 0.0
        exp = math.floor(math.log10(abs(x)))
        return x / (10 ** exp)
    def XPON(self, x):
        """Exponent of x in base 10."""
        x = float(x)
        if x == 0: return 0
        return math.floor(math.log10(abs(x)))
    # HP Prime hyperbolic functions operate in radians (unlike SIN/COS/TAN)
    def SINH(self, x): return math.sinh(float(x))
    def COSH(self, x): return math.cosh(float(x))
    def TANH(self, x): return math.tanh(float(x))
    def ASINH(self, x): return math.asinh(float(x))
    def ACOSH(self, x): return math.acosh(float(x))
    def ATANH(self, x): return math.atanh(float(x))

    def TYPE(self, x):
        """Return HP Prime type code: 0=real/int, 2=string, 3=list, 5=matrix."""
        if isinstance(x, PPLVar): x = x.value
        if isinstance(x, PPLMatrix): return 5
        if isinstance(x, (PPLList, list)): return 3
        if isinstance(x, (PPLString, str)): return 2
        return 0  # real / integer
    def CHAR(self, n): return chr(int(n))

    # ── Statistics ────────────────────────────────────────────────

    def _unwrap_list(self, lst):
        """Flatten a PPLVar/PPLList to a Python list of floats."""
        if isinstance(lst, PPLVar): lst = lst.value
        return [float(v.value if isinstance(v, PPLVar) else v) for v in lst]

    def SIGMALIST(self, lst): return sum(self._unwrap_list(lst))
    def PILIST(self, lst):
        result = 1
        for v in self._unwrap_list(lst): result *= v
        return result
    def SUM(self, lst): return self.SIGMALIST(lst)
    def PRODUCT(self, lst): return self.PILIST(lst)
    def MEAN(self, lst):
        vals = self._unwrap_list(lst)
        return sum(vals) / len(vals) if vals else 0
    def MEDIAN(self, lst):
        vals = sorted(self._unwrap_list(lst))
        n = len(vals)
        if n == 0: return 0
        mid = n // 2
        return vals[mid] if n % 2 else (vals[mid - 1] + vals[mid]) / 2
    def STDDEV(self, lst):
        vals = self._unwrap_list(lst)
        if len(vals) < 2: return 0.0
        mu = sum(vals) / len(vals)
        return math.sqrt(sum((v - mu) ** 2 for v in vals) / len(vals))

    # ── Graphics stubs (unimplemented — no-op with warning) ─────────────────

    def _unimplemented(self, name, *args):
        raise PPLError(f"{name}() is not implemented yet.")

    def BLIT_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        
        args_list = list(args)
        if not args_list:
            return 0

        target = self.G0

        def _int_arg(idx, default=0):
            if idx >= len(args_list):
                return default
            return int(float(self._val(args_list[idx])))

        first, first_slot = self._resolve_grob_ref(args_list[0]) if args_list else (None, None)
        second, second_slot = self._resolve_grob_ref(args_list[1]) if len(args_list) > 1 else (None, None)

        source = None
        x = y = 0
        w = h = None
        sx = sy = 0
        sw = sh = None

        # BLIT_P(target_slot, source_slot, x, y [, w, h, sx, sy, sw, sh])
        # Fireworks and similar Prime programs use numeric GROB slots directly.
        if self._is_explicit_grob_slot_ref(args_list[0]) and self._is_explicit_grob_slot_ref(args_list[1]):
            target = first
            source = second
            x = _int_arg(2, 0)
            y = _int_arg(3, 0)
            if len(args_list) == 6:
                x2 = _int_arg(4, x)
                y2 = _int_arg(5, y)
                sx = x
                sy = y
                sw = abs(x2 - x) + 1
                sh = abs(y2 - y) + 1
                w = sw
                h = sh
                x = min(x, x2)
                y = min(y, y2)
                sx = min(sx, x2)
                sy = min(sy, y2)
            else:
                w = _int_arg(4, None) if len(args_list) > 4 else None
                h = _int_arg(5, None) if len(args_list) > 5 else None
                sx = _int_arg(6, 0)
                sy = _int_arg(7, 0)
                sw = _int_arg(8, None) if len(args_list) > 8 else None
                sh = _int_arg(9, None) if len(args_list) > 9 else None
        # BLIT_P([target,] source, x, y [, w, h, sx, sy, sw, sh])
        elif len(args_list) >= 4 and isinstance(first, HP_Grob) and (
            isinstance(args_list[1], HP_Grob) or self._is_explicit_grob_slot_ref(args_list[1])
        ) and isinstance(second, HP_Grob):
            target = first
            source = second
            x = _int_arg(2, 0)
            y = _int_arg(3, 0)
            w = _int_arg(4, None) if len(args_list) > 4 else None
            h = _int_arg(5, None) if len(args_list) > 5 else None
            sx = _int_arg(6, 0)
            sy = _int_arg(7, 0)
            sw = _int_arg(8, None) if len(args_list) > 8 else None
            sh = _int_arg(9, None) if len(args_list) > 9 else None
        # BLIT_P(target, x, y, w, h, source, sx, sy, sw, sh)
        elif isinstance(first, HP_Grob) and len(args_list) > 5 and isinstance(self._resolve_grob_ref(args_list[5])[0], HP_Grob):
            target = first
            x = _int_arg(1, 0)
            y = _int_arg(2, 0)
            w = _int_arg(3, None)
            h = _int_arg(4, None)
            source = self._resolve_grob_ref(args_list[5])[0]
            sx = _int_arg(6, 0)
            sy = _int_arg(7, 0)
            sw = _int_arg(8, None) if len(args_list) > 8 else None
            sh = _int_arg(9, None) if len(args_list) > 9 else None
        # BLIT_P(x, y, w, h, source, sx, sy, sw, sh)
        elif len(args_list) > 4 and isinstance(self._resolve_grob_ref(args_list[4])[0], HP_Grob):
            x = _int_arg(0, 0)
            y = _int_arg(1, 0)
            w = _int_arg(2, None)
            h = _int_arg(3, None)
            source = self._resolve_grob_ref(args_list[4])[0]
            sx = _int_arg(5, 0)
            sy = _int_arg(6, 0)
            sw = _int_arg(7, None) if len(args_list) > 7 else None
            sh = _int_arg(8, None) if len(args_list) > 8 else None
        # BLIT_P(source, x, y [, w, h, sx, sy, sw, sh])
        elif isinstance(first, HP_Grob):
            source = first
            x = _int_arg(1, 0)
            y = _int_arg(2, 0)
            w = _int_arg(3, None) if len(args_list) > 3 else None
            h = _int_arg(4, None) if len(args_list) > 4 else None
            sx = _int_arg(5, 0)
            sy = _int_arg(6, 0)
            sw = _int_arg(7, None) if len(args_list) > 7 else None
            sh = _int_arg(8, None) if len(args_list) > 8 else None

        if not isinstance(source, HP_Grob):
            return 0
        
        target.blit(source, x, y, w, h, sx, sy, sw, sh)
        
        # Mirror to pygame ONLY if target is G0
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            # Efficiently mirror by pasting the updated region or full screen
            pg_img = pygame.image.fromstring(self.img.tobytes(), self.img.size, self.img.mode)
            self._pg_screen.blit(pg_img, (0, 0))
        return 1

    def ARC_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        args_list = list(args)
        target = self.G0
        if args_list and isinstance(self._val(args_list[0]), HP_Grob):
            target = self._val(args_list.pop(0))
        if len(args_list) < 3:
            return
        x, y = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        r = int(float(self._val(args_list[2])))
        if len(args_list) >= 5:
            a1 = float(self._val(args_list[3])) # start angle (degrees)
            a2 = float(self._val(args_list[4])) # end angle (degrees)
            c = self._color(self._val(args_list[5])) if len(args_list) > 5 else (0,0,0)
        else:
            a1 = 0.0
            a2 = 360.0
            c = self._color(self._val(args_list[3])) if len(args_list) > 3 else (0,0,0)
        
        # Pillow: chord or arc? PPL ARC_P is just the arc line.
        # Pillow angles are 0=East, CCW. HP Prime is same.
        target.draw.arc([x-r, y-r, x+r, y+r], start=a1, end=a2, fill=c)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            rect = pygame.Rect(x-r, y-r, r*2, r*2)
            pygame.draw.arc(self._pg_screen, c, rect, math.radians(a1), math.radians(a2), 1)

    def TRIANGLE_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        args_list = list(args)
        target = self.G0
        if args_list and isinstance(self._val(args_list[0]), HP_Grob):
            target = self._val(args_list.pop(0))
        if len(args_list) < 6: return
        p1 = (int(float(self._val(args_list[0]))), int(float(self._val(args_list[1]))))
        p2 = (int(float(self._val(args_list[2]))), int(float(self._val(args_list[3]))))
        p3 = (int(float(self._val(args_list[4]))), int(float(self._val(args_list[5]))))
        c = self._color(self._val(args_list[6])) if len(args_list) > 6 else (0,0,0)
        
        target.draw.polygon([p1, p2, p3], outline=c, fill=c)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            pygame.draw.polygon(self._pg_screen, c, [p1, p2, p3])

    def _normalise_polygon_points(self, raw_points):
        points = self._val(raw_points)
        if isinstance(points, (PPLList, PPLMatrix, list, tuple)):
            if points and all(isinstance(self._val(item), (PPLList, list, tuple)) for item in points):
                return [
                    (
                        int(float(self._val(self._val(item)[0]))),
                        int(float(self._val(self._val(item)[1]))),
                    )
                    for item in points
                    if len(self._val(item)) >= 2
                ]
            flat = list(points)
            if len(flat) % 2 != 0:
                raise PPLError("FILLPOLY_P() requires an even number of coordinates.")
            return [
                (
                    int(float(self._val(flat[i]))),
                    int(float(self._val(flat[i + 1]))),
                )
                for i in range(0, len(flat), 2)
            ]
        raise PPLError("FILLPOLY_P() expects a list of points or flat coordinates.")

    def SUBGROB(self, *args):
        args_list = list(args)
        if not args_list:
            return None

        target_slot = None
        target_var = None
        target_ref = args_list[-1]
        maybe_target_slot = self._resolve_grob_slot(target_ref)
        if len(args_list) >= 2 and maybe_target_slot is not None and maybe_target_slot != 0:
            target_slot = maybe_target_slot
            args_list.pop()
        elif len(args_list) >= 2 and isinstance(target_ref, PPLVar):
            target_var = target_ref
            args_list.pop()

        source = self.G0
        if args_list and isinstance(self._val(args_list[0]), HP_Grob):
            source = self._val(args_list.pop(0))

        x1 = int(float(self._val(args_list[0]))) if len(args_list) > 0 else 0
        y1 = int(float(self._val(args_list[1]))) if len(args_list) > 1 else 0
        x2 = int(float(self._val(args_list[2]))) if len(args_list) > 2 else source.width - 1
        y2 = int(float(self._val(args_list[3]))) if len(args_list) > 3 else source.height - 1

        new_grob = HP_Grob(max(1, abs(x2 - x1) + 1), max(1, abs(y2 - y1) + 1), runtime=self)
        box = (min(x1, x2), min(y1, y2), max(x1, x2) + 1, max(y1, y2) + 1)
        region = source.img.crop(box)
        new_grob.img.paste(region, (0, 0))
        if target_slot is not None:
            self.grobs[target_slot] = new_grob
            name = f"G{target_slot}"
            setattr(self, name, new_grob)
            self.SET_VAR(name, new_grob)
        elif target_var is not None:
            target_var.value = new_grob
            self._refresh_catalog_vars()
        return new_grob

    def SUBGROB_P(self, *args):
        return self.SUBGROB(*args)

    def GETPIX(self, *args):
        args_list = list(args)
        target = self.G0
        if args_list:
            first = self._val(args_list[0])
            if isinstance(first, HP_Grob):
                target = first
                args_list.pop(0)
                
        if len(args_list) < 2:
            return 0
            
        x, y = int(float(self._val(args_list[0]))), int(float(self._val(args_list[1])))
        if 0 <= x < target.width and 0 <= y < target.height:
            r, g, b = target.img.getpixel((x, y))[:3]
            return (r << 16) | (g << 8) | b
        return 0

    def GROBW_P(self, grob):
        value = self._val(grob)
        if not isinstance(value, HP_Grob):
            raise PPLError("GROBW_P() expects a GROB.")
        return value.width

    def GROBH_P(self, grob):
        value = self._val(grob)
        if not isinstance(value, HP_Grob):
            raise PPLError("GROBH_P() expects a GROB.")
        return value.height

    def GROB(self, *args):
        return self.DIMGROB_P(*args)

    def FILLPOLY_P(self, *args):
        self.screen_is_dirty = True
        self._graphics_mode = True
        args_list = list(args)
        target = self.G0
        if args_list and isinstance(self._val(args_list[0]), HP_Grob):
            target = self._val(args_list.pop(0))
        if len(args_list) < 2:
            raise PPLError("FILLPOLY_P() expects at least points and color.")
        points = self._normalise_polygon_points(args_list[0])
        if len(points) < 2:
            return 0
        color = self._color(self._val(args_list[1]))
        target.draw.polygon(points, fill=color, outline=color)
        if target == self.G0 and self._pg_enabled and pygame is not None:
            self._pg_pump()
            pygame.draw.polygon(self._pg_screen, color, points)
        return 1

    def FILLPOLY(self, *a):   return self.FILLPOLY_P(*a)
    def DIMGROB_P(self, *args):
        args_list = list(args)
        if not args_list:
            return
            
        # DIMGROB_P([grob,] width, height [, color])
        target_idx = self._resolve_grob_slot(args_list[0])
        if target_idx is not None:
            args_list.pop(0)
            
        if len(args_list) < 2:
            return
            
        w = int(float(self._val(args_list[0])))
        h = int(float(self._val(args_list[1])))
        color = self._val(args_list[2]) if len(args_list) > 2 else None
        
        new_grob = HP_Grob(w, h, color, self)
        
        if target_idx is not None:
            self.grobs[target_idx] = new_grob
            name = f"G{target_idx}"
            setattr(self, name, new_grob)
            self.SET_VAR(name, new_grob)
            if target_idx == 0:
                self.G0 = new_grob
                self.img = self.G0.img
                self.draw = self.G0.draw
                if self._pg_enabled and pygame is not None:
                    self._pg_screen = pygame.Surface((w, h)).convert()
                    self._pg_screen.fill(self._color(color) if color is not None else (255, 255, 255))
        
        return new_grob
    def DIMGROB(self, *a):    return self.DIMGROB_P(*a)

    # ── Strings ──────────────────────────────────────────────────────

    def ASC(self, s):
        s = str(s)
        return ord(s[0]) if s else 0
    def CHR(self, n): return chr(int(n))
    def TRIM(self, s): return str(s).strip()
    def STARTSWITH(self, s, sub): return 1 if str(s).startswith(str(sub)) else 0
    def ENDSWITH(self, s, sub): return 1 if str(s).endswith(str(sub)) else 0
    def CONTAINS(self, s, sub): return 1 if str(sub) in str(s) else 0

    # ── Bit operations ───────────────────────────────────────────────

    def BITSHIFT(self, n, s):
        """Shift n left by s bits (or right if s is negative)."""
        n, s = int(n), int(s)
        if s >= 0:
            return n << s
        else:
            return n >> abs(s)
    def BITSL(self, n, s): return self.BITSHIFT(n, abs(int(s)))
    def BITSR(self, n, s): return self.BITSHIFT(n, -abs(int(s)))

    # ── List / collection operations ─────────────────────────────────

    def SORT(self, obj):
        if isinstance(obj, PPLVar): obj = obj.value
        if isinstance(obj, list):
            obj.sort()
        return obj

    def REVERSE(self, obj):
        if isinstance(obj, PPLVar): obj = obj.value
        if isinstance(obj, list):
            result = PPLList(reversed(obj))
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.account_value(result, runtime=self, label="reversed list")
            return result
        if isinstance(obj, str):
            result = str(obj)[::-1]
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.account_value(result, runtime=self, label="reversed string")
            return result
        return obj

    def ADDTAIL(self, obj, val):
        if isinstance(obj, PPLVar): obj = obj.value
        if isinstance(obj, list):
            obj.append(val)
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.recalculate(self)
        return obj
    def APPEND(self, obj, val): return self.ADDTAIL(obj, val)
    def HEAD(self, obj):
        if isinstance(obj, PPLVar): obj = obj.value
        if isinstance(obj, (PPLString, str)):
            return PPLString(str(obj)[:1])
        if isinstance(obj, (PPLList, list, tuple)):
            return obj[0] if obj else 0
        return obj
    def TAIL(self, obj):
        if isinstance(obj, PPLVar): obj = obj.value
        if isinstance(obj, (PPLString, str)):
            result = PPLString(str(obj)[1:])
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.account_value(result, runtime=self, label="string tail")
            return result
        if isinstance(obj, (PPLList, list, tuple)):
            result = PPLList(list(obj)[1:])
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.account_value(result, runtime=self, label="list tail")
            return result
        return obj
    def MAKEMAT(self, *args):
        if len(args) == 2:
            value = 0
            rows, cols = args
        elif len(args) == 3:
            value, rows, cols = args
            value = self._val(value)
        else:
            raise PPLError("MAKEMAT() expects 2 or 3 arguments.")
        rows_i = max(0, int(float(self._val(rows))))
        cols_i = max(0, int(float(self._val(cols))))
        result = PPLMatrix([[value for _ in range(cols_i)] for _ in range(rows_i)])
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_value(result, runtime=self, label="matrix")
        return result
    def MAT2LIST(self, obj):
        if isinstance(obj, PPLVar): obj = obj.value
        if isinstance(obj, PPLMatrix):
            return PPLList([PPLList(list(row)) for row in obj])
        if isinstance(obj, list):
            return PPLList(list(obj))
        return PPLList([])
    def EDITMAT(self, matrix, *args): return self._val(matrix)

    def INSTRING(self, target, pattern, start=1):
        target, pattern = str(target), str(pattern)
        idx = target.find(pattern, int(start) - 1)
        return idx + 1 if idx != -1 else 0

    def LEFT(self, s, n): return str(s)[:int(n)]
    def RIGHT(self, s, n): 
        s, n = str(s), int(n)
        return s[-n:] if n > 0 else ""
    def MID(self, s, start, length=None):
        s, start = str(s), int(start) - 1
        if length is None: return s[start:]
        return s[start : start + int(length)]
    def CONCAT(self, *items):
        values = [self._val(item) for item in items]
        if any(isinstance(v, (PPLList, PPLMatrix, list, tuple)) for v in values):
            out = []
            for value in values:
                if isinstance(value, (PPLList, PPLMatrix, list, tuple)):
                    out.extend(list(value))
                else:
                    out.append(value)
            result = PPLList(out)
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.account_value(result, runtime=self, label="concatenated list")
            return result
        result = ''.join(str(v) for v in values)
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_value(result, runtime=self, label="concatenated string")
        return result
    def POS(self, target, pattern): return self.INSTRING(target, pattern)
    def UPPER(self, s): return str(s).upper()
    def LOWER(self, s): return str(s).lower()
    def TEXT_CLEAR(self):
        self._terminal_lines.clear()
        self._graphics_mode = False
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget._snapshot.output_chars = 0
            self._budget.recalculate(self)
        self.RECT()
        return 1
    def TEXT_AT(self, row, col, text):
        row_idx = max(1, int(float(self._val(row))))
        col_idx = max(1, int(float(self._val(col))))
        rendered = str(self._val(text))
        while len(self._terminal_lines) < row_idx:
            self._terminal_lines.append("")
        line = self._terminal_lines[row_idx - 1]
        if len(line) < col_idx - 1:
            line = line + (" " * ((col_idx - 1) - len(line)))
        prefix = line[:col_idx - 1]
        suffix_start = col_idx - 1 + len(rendered)
        suffix = line[suffix_start:] if len(line) > suffix_start else ""
        self._terminal_lines[row_idx - 1] = prefix + rendered + suffix
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.recalculate(self)
        self._render_terminal()
        return len(rendered)
    def STRING(self, x, precision=None):
        if precision is None: return str(x)
        try: return format(float(x), f".{int(precision)}g")
        except: return str(x)
    def NUM(self, s=_APP_MISSING):
        if s is _APP_MISSING:
            return self._set_current_view("Num")
        try:
            value = self._val(s)
            if isinstance(value, PPLString):
                value = str(value)
            return float(value)
        except Exception:
            return 0

    def BITAND(self, a, b): return int(a) & int(b)
    def BITOR(self, *args):
        if len(args) < 2:
            raise PPLError("BITOR() expects at least 2 arguments.")
        result = int(args[0])
        for arg in args[1:]:
            result |= int(arg)
        return result
    def BITXOR(self, a, b): return int(a) ^ int(b)
    def BITNOT(self, a): return ~int(a)

    def MOD(self, a, b): return int(a) % int(b)
    def DIV(self, a, b): return int(a) // int(b)
    def DET(self, m): return DET(m)
    def B_to_R(self, x):
        raw = str(self._val(x)).strip().lower()
        negative = raw.startswith('-')
        if negative:
            raw = raw[1:]
        if raw.startswith('#'):
            raw = raw[1:]
        if raw.endswith('b'):
            raw = raw[:-1]
        if not raw or any(ch not in '01' for ch in raw):
            raise PPLError("B→R() expects a binary value.")
        value = int(raw, 2)
        return -value if negative else value

    def R_to_B(self, x):
        value = int(round(float(self._val(x))))
        prefix = '-' if value < 0 else ''
        return PPLString(f"{prefix}#{abs(value):b}b")

    def REPLACE(self, obj, start_or_old, length_or_new, replacement=None):
        if replacement is None:
            return str(obj).replace(str(start_or_old), str(length_or_new))
        else:
            start, length = int(start_or_old) - 1, int(length_or_new)
            if isinstance(obj, list):
                res = PPLList(obj)
                res[start : start + length] = list(replacement)
                return res
            else:
                s = str(obj)
                return s[:start] + str(replacement) + s[start + length:]

    def EXPR(self, s): return ppl_expr(s)
    def MAKELIST(self, expr, var_name=None, start=1, end=1, step=1):
        """Create a PPL list by evaluating expr for var from start to end (inclusive).

        expr may be:
          - a no-arg lambda (transpiler wraps expressions so SET_VAR is called first)
          - a plain constant value (for MAKELIST(0, i, 1, n) style calls)
        var_name is the uppercased PPL variable name string used as the loop counter.
        """
        n_start = int(start) if start is not None else 1
        n_end   = int(end)   if end   is not None else 1
        n_step  = int(step)  if step  else 1
        indices = range(n_start, n_end + 1, n_step)
        if callable(expr):
            result = []
            for i in indices:
                if var_name:
                    self.SET_VAR(str(var_name), i)
                result.append(expr())
            out = PPLList(result)
            if getattr(self, "_budget", None) is not None and self._budget.active:
                self._budget.account_value(out, runtime=self, label="makelist result")
            return out
        # Constant expression — just repeat the value
        val = expr.value if isinstance(expr, PPLVar) else (expr if expr is not None else 0)
        out = PPLList([val] * max(0, len(indices)))
        if getattr(self, "_budget", None) is not None and self._budget.active:
            self._budget.account_value(out, runtime=self, label="makelist result")
        return out
    def EVAL(self, x): return self.EXPR(x)
    def sto(self, *args): return args[0] if args else 0

    def save(self, path='screen.png', force=None):
        if force is None:
            force = self._force_save_output
        if self.screen_is_dirty:
            if self._pg_enabled and pygame is not None and not force:
                return False
            if self._pg_enabled and pygame is not None:
                pygame.image.save(self._pg_screen, path)
            else:
                self.img.save(path)
            self._last_saved_path = os.path.abspath(path)
            return True
        return False

    def close(self):
        if getattr(self, "_budget", None) is not None:
            self._budget.deactivate()
        if self._pg_enabled and pygame is not None:
            try:
                pygame.display.quit()
                pygame.quit()
            finally:
                self._pg_enabled = False
                self._pg_should_close = False
                self._pg_window = None
                self._pg_screen = None
                self._pg_font = None
                self._pg_fonts.clear()

class _FinanceMock:
    """Stub for the HP Prime Finance object — all methods return 0.0."""

    def __getattr__(self, name):
        return lambda *a: 0.0
