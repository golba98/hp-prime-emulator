# ─────────────────────────────────────────────────────────────────
#  Constants and Configuration
# ─────────────────────────────────────────────────────────────────

from src.ppl_emulator.hpprime_specs import (
    BUILTIN_NAMES as _BUILTIN_NAMES,
    BUILTINS_ZERO_ARGS as _BUILTINS_ZERO_ARGS,
    STRUCTURAL_KEYWORDS as _STRUCTURAL_KEYWORDS,
    SYSTEM_GLOBALS as _SHARED_SYSTEM_GLOBALS,
)

_OPS = [
    (r'(?<![A-Za-z_])AND(?![A-Za-z_])',  ' and '),
    (r'(?<![A-Za-z_])OR(?![A-Za-z_])',   ' or '),
    (r'(?<![A-Za-z_])NOT(?![A-Za-z_])',  ' not '),
    (r'(?<![A-Za-z_])MOD(?![A-Za-z_(])',  '%'),
    (r'(?<![A-Za-z_])DIV(?![A-Za-z_(])',  '//'),
    (r'(?<![A-Za-z_])XOR(?![A-Za-z_])',  '^'),
    (r'≠',        '!='),
    (r'≤',        '<='),
    (r'≥',        '>='),
    (r'<>',       '!='),
    (r'(?<![<>!:=])=(?![=>])', '=='),  # PPL = is equality (not assignment)
]

_PYTHON_RESERVED = frozenset({
    # Built-in names that clash with common PPL variable/function names
    'set', 'list', 'map', 'filter', 'input', 'type',
    'dir', 'id', 'hex', 'oct', 'bin', 'str',
    # Python keywords that are not valid identifiers
    'yield', 'lambda', 'global', 'class', 'del', 'raise',
    'with', 'assert', 'async', 'await',
})

_SYSTEM_GLOBALS = _SHARED_SYSTEM_GLOBALS
BUILTINS = _BUILTIN_NAMES
BUILTINS_ZERO_ARGS = _BUILTINS_ZERO_ARGS
_STRUCTURAL = _STRUCTURAL_KEYWORDS

# Combined set used to decide whether a token is a keyword or a user identifier
_PPL_KEYWORDS = BUILTINS | _STRUCTURAL | _SYSTEM_GLOBALS
