from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class CommandSpec:
    min_args: int
    max_args: int | None
    bind_in_transpiler: bool = True
    zero_arg_auto_call: bool = False
    interactive: bool = False
    coordinate_space: str | None = None
    pixel_variant: str | None = None
    exact_mode: str | None = None


STRUCTURAL_KEYWORDS = frozenset(
    {
        "BEGIN",
        "END",
        "IF",
        "THEN",
        "ELSE",
        "IFERR",
        "FOR",
        "FROM",
        "TO",
        "STEP",
        "DOWNTO",
        "DO",
        "WHILE",
        "REPEAT",
        "UNTIL",
        "CASE",
        "DEFAULT",
        "RETURN",
        "BREAK",
        "CONTINUE",
        "LOCAL",
        "EXPORT",
        "PROCEDURE",
        "AND",
        "OR",
        "NOT",
        "MOD",
        "DIV",
        "XOR",
    }
)

CONTEXTUAL_ASSIGNMENT_KEYWORDS = frozenset({"FROM", "TO", "STEP", "DO"})
# Operator-like keywords are valid identifiers in real-world Prime programs and
# the transpiler/runtime already handle them as shadowed variables when they are
# declared locally.
NONRESERVED_OPERATOR_KEYWORDS = frozenset({"AND", "OR", "NOT", "MOD", "DIV", "XOR"})
ASSIGNMENT_RESERVED = STRUCTURAL_KEYWORDS - CONTEXTUAL_ASSIGNMENT_KEYWORDS - NONRESERVED_OPERATOR_KEYWORDS
SYSTEM_GLOBALS = frozenset({"ANS", "THETA", "VAR", "EXACT", "WINDOW", "MYLANGS"})

CRITICAL_SHADOW_BUILTINS = frozenset(
    {
        "PRINT",
        "DISP",
        "INPUT",
        "CHOOSE",
        "MSGBOX",
        "GETKEY",
        "WAIT",
        "RECT",
        "RECT_P",
        "LINE",
        "LINE_P",
        "PIXON",
        "PIXON_P",
        "TEXTOUT_P",
        "CIRCLE_P",
        "FILLCIRCLE_P",
        "RGB",
        "SIN",
        "COS",
        "TAN",
        "ASIN",
        "ACOS",
        "ATAN",
        "SQRT",
        "LOG",
        "LN",
        "EXP",
        "SIZE",
        "CONCAT",
        "SORT",
        "REVERSE",
        "MAKELIST",
    }
)

REJECTED_GRAPHICS = {
    "FILLRECT_P": "Error: Unknown command 'FILLRECT_P'. Use 'RECT_P' with a fill color argument for hardware compatibility.",
    "FILLRECT": "Error: Unknown command 'FILLRECT'. Use 'RECT_P' with a fill color argument for hardware compatibility.",
}

HARDWARE_SUGGESTIONS = {
    "FILLRECT": "RECT_P",
    "DRAWRECT": "RECT_P",
    "DRAWLINE": "LINE_P",
    "DRAWCIRCLE": "CIRCLE_P",
    "CLEARDISPLAY": "RECT_P",
    "CLEARSCREEN": "RECT_P",
    "CLRSCR": "RECT_P",
    "SETPIXEL": "PIXON_P",
    "PUTPIXEL": "PIXON_P",
    "DRAWPIXEL": "PIXON_P",
    "GETPIXEL": "GETPIX_P",
    "FILLSCREEN": "RECT_P",
    "DRAWTEXT": "TEXTOUT_P",
    "TEXTWRITE": "TEXTOUT_P",
    "PRINTF": "PRINT",
    "PRINTLN": "PRINT",
    "WRITELN": "PRINT",
    "WRITE": "PRINT",
    "DRAWIMAGE": "BLIT_P",
    "BLITIMAGE": "BLIT_P",
}


def _spec(
    min_args: int,
    max_args: int | None,
    *,
    bind_in_transpiler: bool = True,
    zero_arg_auto_call: bool = False,
    interactive: bool = False,
    coordinate_space: str | None = None,
    pixel_variant: str | None = None,
    exact_mode: str | None = None,
) -> CommandSpec:
    return CommandSpec(
        min_args=min_args,
        max_args=max_args,
        bind_in_transpiler=bind_in_transpiler,
        zero_arg_auto_call=zero_arg_auto_call,
        interactive=interactive,
        coordinate_space=coordinate_space,
        pixel_variant=pixel_variant,
        exact_mode=exact_mode,
    )


COMMAND_SPECS: dict[str, CommandSpec] = {
    # I/O
    "PRINT": _spec(0, None),
    "MSGBOX": _spec(1, 2),
    "INPUT": _spec(1, 6, interactive=True),
    "CHOOSE": _spec(2, None, interactive=True),
    "WAIT": _spec(0, 1, interactive=True),
    "GETKEY": _spec(0, 0, zero_arg_auto_call=True, interactive=True),
    "ISKEYDOWN": _spec(1, 1, interactive=True),
    "MOUSE": _spec(0, 1, zero_arg_auto_call=True, interactive=True),
    "DISP": _spec(1, 2),
    "DISP_FREEZE": _spec(0, 0, zero_arg_auto_call=True, interactive=True),
    "FREEZE": _spec(0, 0, zero_arg_auto_call=True, interactive=True),
    "DRAWMENU": _spec(0, 6, zero_arg_auto_call=True),
    "START": _spec(0, 1),
    "RESET": _spec(0, 1, zero_arg_auto_call=True),
    "VIEW": _spec(0, 2),
    "SYMB": _spec(0, 0, zero_arg_auto_call=True),
    "SYMBSETUP": _spec(0, 2, zero_arg_auto_call=True),
    "PLOT": _spec(0, 0, zero_arg_auto_call=True),
    "PLOTSETUP": _spec(0, 2, zero_arg_auto_call=True),
    "INFO": _spec(0, 0, zero_arg_auto_call=True),
    "NUMSETUP": _spec(0, 2, zero_arg_auto_call=True),
    "PROGRAMS": _spec(0, 1, zero_arg_auto_call=True),
    "HVARS": _spec(0, 1, zero_arg_auto_call=True),
    "NOTES": _spec(0, 1, zero_arg_auto_call=True),
    "AFILES": _spec(0, 1, zero_arg_auto_call=True),
    "DELAFILES": _spec(1, 1),
    "DELHVARS": _spec(1, 1),
    "TICKS": _spec(0, 0, zero_arg_auto_call=True),
    # Graphics
    "RECT": _spec(0, 7, zero_arg_auto_call=True, coordinate_space="screen", pixel_variant="RECT_P"),
    "RECT_P": _spec(0, 7, zero_arg_auto_call=True, coordinate_space="pixel"),
    "LINE": _spec(4, 7, coordinate_space="screen", pixel_variant="LINE_P"),
    "LINE_P": _spec(4, 7, coordinate_space="pixel"),
    "PIXON": _spec(2, 5, coordinate_space="screen", pixel_variant="PIXON_P"),
    "PIXON_P": _spec(2, 5, coordinate_space="pixel"),
    "PIXOFF": _spec(2, 4, coordinate_space="screen", pixel_variant="PIXOFF_P"),
    "PIXOFF_P": _spec(2, 4, coordinate_space="pixel"),
    "CIRCLE": _spec(3, 5, coordinate_space="screen", pixel_variant="CIRCLE_P"),
    "CIRCLE_P": _spec(3, 5, coordinate_space="pixel"),
    "FILLCIRCLE": _spec(3, 5, coordinate_space="screen", pixel_variant="FILLCIRCLE_P"),
    "FILLCIRCLE_P": _spec(3, 5, coordinate_space="pixel"),
    "ARC": _spec(3, 7, coordinate_space="screen", pixel_variant="ARC_P"),
    "ARC_P": _spec(3, 7, coordinate_space="pixel"),
    "TEXTOUT": _spec(3, 8, coordinate_space="screen", pixel_variant="TEXTOUT_P"),
    "TEXTOUT_P": _spec(3, 8, coordinate_space="pixel"),
    "BLIT": _spec(1, 11, coordinate_space="screen", pixel_variant="BLIT_P"),
    "BLIT_P": _spec(0, 11, coordinate_space="pixel"),
    "INVERT": _spec(0, 5, coordinate_space="screen", pixel_variant="INVERT_P"),
    "INVERT_P": _spec(0, 5, coordinate_space="pixel"),
    "RGB": _spec(3, 4, exact_mode="exact"),
    "FILLPOLY": _spec(1, 4, coordinate_space="screen", pixel_variant="FILLPOLY_P"),
    "FILLPOLY_P": _spec(1, 4, coordinate_space="pixel"),
    "TRIANGLE": _spec(2, 8, coordinate_space="screen", pixel_variant="TRIANGLE_P"),
    "TRIANGLE_P": _spec(2, 8, coordinate_space="pixel"),
    "SUBGROB": _spec(2, 6, coordinate_space="screen", pixel_variant="SUBGROB_P"),
    "SUBGROB_P": _spec(2, 6, coordinate_space="pixel"),
    "GROB": _spec(2, 4),
    "DIMGROB": _spec(2, 4, pixel_variant="DIMGROB_P"),
    "DIMGROB_P": _spec(2, 4),
    "GETPIX": _spec(2, 3, coordinate_space="screen", pixel_variant="GETPIX_P"),
    "GETPIX_P": _spec(2, 3, coordinate_space="pixel"),
    "GROBW": _spec(1, 1, pixel_variant="GROBW_P"),
    "GROBW_P": _spec(1, 1),
    "GROBH": _spec(1, 1, pixel_variant="GROBH_P"),
    "GROBH_P": _spec(1, 1),
    # Math
    "ABS": _spec(1, 1),
    "MAX": _spec(1, None),
    "MIN": _spec(1, None),
    "FLOOR": _spec(1, 1),
    "CEILING": _spec(1, 1),
    "ROUND": _spec(1, 2),
    "SQ": _spec(1, 1),
    "SQRT": _spec(1, 1),
    "LOG": _spec(1, 2),
    "LN": _spec(1, 1),
    "EXP": _spec(1, 1),
    "SIN": _spec(1, 1),
    "COS": _spec(1, 1),
    "TAN": _spec(1, 1),
    "ASIN": _spec(1, 1),
    "ACOS": _spec(1, 1),
    "ATAN": _spec(1, 2),
    "SINH": _spec(1, 1),
    "COSH": _spec(1, 1),
    "TANH": _spec(1, 1),
    "ASINH": _spec(1, 1),
    "ACOSH": _spec(1, 1),
    "ATANH": _spec(1, 1),
    "IP": _spec(1, 1),
    "FP": _spec(1, 1),
    "IFTE": _spec(3, 3),
    "RANDOM": _spec(0, 2),
    "RANDINT": _spec(1, 2),
    "INTEGER": _spec(1, 1),
    "REAL": _spec(1, 1),
    "SIGN": _spec(1, 1),
    "TRUNCATE": _spec(1, 2),
    "MANT": _spec(1, 1),
    "XPON": _spec(1, 1),
    "MOD": _spec(2, 2),
    "DIV": _spec(2, 2),
    "DET": _spec(1, 1),
    # Bit operations
    "BITAND": _spec(2, 2),
    "BITOR": _spec(2, None),
    "BITXOR": _spec(2, 2),
    "BITNOT": _spec(1, 1),
    "BITSHIFT": _spec(2, 2),
    "BITSL": _spec(2, 2),
    "BITSR": _spec(2, 2),
    "B_to_R": _spec(1, 1),
    "R_to_B": _spec(1, 1),
    # Strings
    "INSTRING": _spec(2, 3),
    "LEFT": _spec(2, 2),
    "RIGHT": _spec(2, 2),
    "MID": _spec(2, 3),
    "CONCAT": _spec(2, None),
    "POS": _spec(2, 2),
    "REPLACE": _spec(3, 4),
    "UPPER": _spec(1, 1),
    "LOWER": _spec(1, 1),
    "STRING": _spec(1, 2),
    "NUM": _spec(0, 1, zero_arg_auto_call=True),
    "ASC": _spec(1, 1),
    "CHR": _spec(1, 1),
    "CHAR": _spec(1, 1),
    "TRIM": _spec(1, 1),
    "STARTSWITH": _spec(2, 2),
    "ENDSWITH": _spec(2, 2),
    "CONTAINS": _spec(2, 2),
    # Lists / collections / types
    "SIZE": _spec(1, 1),
    "DIM": _spec(1, 1),
    "MAKELIST": _spec(4, 5),
    "SORT": _spec(1, 2),
    "REVERSE": _spec(1, 1),
    "ADDTAIL": _spec(2, 2),
    "APPEND": _spec(2, 2),
    "HEAD": _spec(1, 1),
    "TAIL": _spec(1, 1),
    "SIGMALIST": _spec(1, 1),
    "PILIST": _spec(1, 1),
    "TYPE": _spec(1, 1),
    "EXPR": _spec(1, 2),
    "EVAL": _spec(1, 1),
    "APPROX": _spec(1, 1, exact_mode="approx"),
    "EXACT": _spec(1, 1, exact_mode="exact"),
    "SUM": _spec(1, 4),
    "PRODUCT": _spec(1, 4),
    # CAS / statistics
    "CAS": _spec(1, 1),
    "FACTOR": _spec(1, 2),
    "EXPAND": _spec(1, 2),
    "PARTFRAC": _spec(1, 2),
    "SIMPLIFY": _spec(1, 2),
    "SOLVE": _spec(1, 3),
    "ZEROS": _spec(1, 2),
    "CZEROS": _spec(1, 2),
    "DIFF": _spec(1, 3),
    "INTEGRATE": _spec(1, 4),
    "LIMIT": _spec(1, 3),
    "SERIES": _spec(1, 4),
    "TAYLOR": _spec(1, 4),
    "LAPLACE": _spec(1, 2),
    "INVLAPLACE": _spec(1, 2),
    "FFT": _spec(1, 2),
    "IFFT": _spec(1, 2),
    "POLY": _spec(1, None),
    "DEGREE": _spec(1, 1),
    "COEFF": _spec(1, 2),
    "ROOTS": _spec(1, 1),
    "FACTORS": _spec(1, 1),
    "FSOLVE": _spec(1, 4),
    "NDERIV": _spec(1, 3),
    "NINT": _spec(1, 4),
    "MEAN": _spec(1, 2),
    "MEDIAN": _spec(1, 1),
    "STDDEV": _spec(1, 2),
    "VAR": _spec(1, 2),
    "CORR": _spec(2, 2),
    "COV": _spec(2, 2),
    "REG": _spec(1, 3),
    "PREDY": _spec(1, 2),
    "PREDX": _spec(1, 2),
    # Linter / validator awareness for commands not yet in the runtime
    "INSERT": _spec(3, 3, bind_in_transpiler=False),
    "TEXT_CLEAR": _spec(0, 0),
    "TEXT_AT": _spec(3, 3),
    "MAKEMAT": _spec(2, 3),
    "MAT2LIST": _spec(1, 1),
    "EDITMAT": _spec(1, 2, interactive=True),
    "MAKEMATRIX": _spec(2, 3, bind_in_transpiler=False),
    "ADDROW": _spec(3, 3, bind_in_transpiler=False),
    "DELROW": _spec(2, 2, bind_in_transpiler=False),
    "ADDCOL": _spec(3, 3, bind_in_transpiler=False),
    "DELCOL": _spec(2, 2, bind_in_transpiler=False),
    "QUO": _spec(2, 2, bind_in_transpiler=False),
    "REM": _spec(2, 2, bind_in_transpiler=False),
}

NON_CONTIGUOUS_ARITIES = {
    "RECT": frozenset({0, 1, 2, 4, 5, 6, 7}),
    "RECT_P": frozenset({0, 1, 2, 4, 5, 6, 7}),
}


COMMAND_ARITY = {
    name: (spec.min_args, spec.max_args)
    for name, spec in COMMAND_SPECS.items()
}

BUILTIN_NAMES = frozenset(
    name for name, spec in COMMAND_SPECS.items() if spec.bind_in_transpiler
)

BUILTINS_ZERO_ARGS = frozenset(
    name for name, spec in COMMAND_SPECS.items() if spec.zero_arg_auto_call
)

PIXEL_VARIANTS = {
    name: spec.pixel_variant
    for name, spec in COMMAND_SPECS.items()
    if spec.pixel_variant
}

INTERACTIVE_COMMANDS = frozenset(
    name for name, spec in COMMAND_SPECS.items() if spec.interactive
)


def command_accepts_arity(name: str, count: int) -> bool:
    name = name.upper()
    if name in NON_CONTIGUOUS_ARITIES:
        return count in NON_CONTIGUOUS_ARITIES[name]

    spec = COMMAND_SPECS.get(name)
    if spec is None:
        return False
    if count < spec.min_args:
        return False
    if spec.max_args is None:
        return True
    return count <= spec.max_args


def command_expected_arity(name: str) -> str:
    name = name.upper()
    allowed = NON_CONTIGUOUS_ARITIES.get(name)
    if allowed is not None:
        ordered = sorted(allowed)
        if len(ordered) == 1:
            return str(ordered[0])
        groups: list[str] = []
        start = ordered[0]
        prev = ordered[0]
        for value in ordered[1:]:
            if value == prev + 1:
                prev = value
                continue
            groups.append(str(start) if start == prev else f"{start}-{prev}")
            start = prev = value
        groups.append(str(start) if start == prev else f"{start}-{prev}")
        if len(groups) == 2:
            return f"{groups[0]} or {groups[1]}"
        return ", ".join(groups[:-1]) + f" or {groups[-1]}"

    spec = COMMAND_SPECS[name]
    if spec.max_args is None:
        return f"at least {spec.min_args}"
    if spec.min_args == spec.max_args:
        return str(spec.min_args)
    return f"{spec.min_args}-{spec.max_args}"
