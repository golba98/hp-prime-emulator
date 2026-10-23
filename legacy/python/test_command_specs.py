import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.ppl_emulator.hpprime_specs import (  # pyre-ignore
    BUILTIN_NAMES,
    BUILTINS_ZERO_ARGS,
    COMMAND_SPECS,
    command_accepts_arity,
    command_expected_arity,
)
from src.ppl_emulator.transpiler.constants import (  # pyre-ignore
    BUILTINS,
    BUILTINS_ZERO_ARGS as TRANSPILE_ZERO_ARGS,
)


def test_transpiler_constants_are_derived_from_shared_specs():
    assert BUILTINS == BUILTIN_NAMES
    assert TRANSPILE_ZERO_ARGS == BUILTINS_ZERO_ARGS
    assert "BEGIN" not in BUILTINS
    assert "RECT_P" in BUILTINS
    assert "VAR" in BUILTINS


def test_rect_p_uses_non_contiguous_arity_rule():
    assert command_accepts_arity("RECT_P", 0)
    assert command_accepts_arity("RECT_P", 1)
    assert command_accepts_arity("RECT_P", 2)
    assert command_accepts_arity("RECT_P", 4)
    assert command_accepts_arity("RECT_P", 7)
    assert not command_accepts_arity("RECT_P", 3)
    assert command_expected_arity("RECT_P") == "0-2 or 4-7"


def test_arc_p_supports_compact_circle_form():
    assert command_accepts_arity("ARC_P", 3)
    assert command_accepts_arity("ARC_P", 6)
    assert not command_accepts_arity("ARC_P", 2)
    assert command_expected_arity("ARC_P") == "3-7"


def test_blit_p_supports_extended_target_crop_signature():
    assert command_accepts_arity("BLIT_P", 11)
    assert not command_accepts_arity("BLIT_P", 12)
    assert command_expected_arity("BLIT_P") == "0-11"


def test_specs_capture_io_and_exactness_metadata():
    assert COMMAND_SPECS["INPUT"].interactive
    assert COMMAND_SPECS["GETKEY"].zero_arg_auto_call
    assert COMMAND_SPECS["APPROX"].exact_mode == "approx"
    assert COMMAND_SPECS["EXACT"].exact_mode == "exact"


def test_specs_cover_app_framework_commands():
    assert command_accepts_arity("START", 0)
    assert command_accepts_arity("START", 1)
    assert command_accepts_arity("VIEW", 2)
    assert COMMAND_SPECS["SYMB"].zero_arg_auto_call
    assert COMMAND_SPECS["NUM"].zero_arg_auto_call


def test_specs_cover_storage_and_grob_helpers():
    assert COMMAND_SPECS["NOTES"].zero_arg_auto_call
    assert COMMAND_SPECS["AFILES"].zero_arg_auto_call
    assert command_accepts_arity("DELAFILES", 1)
    assert command_accepts_arity("GROBW_P", 1)
    assert command_accepts_arity("GROBH_P", 1)
