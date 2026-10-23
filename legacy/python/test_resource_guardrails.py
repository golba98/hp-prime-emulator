import contextlib
import io
import os
import sys

import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.ppl_emulator.runtime.engine import HPPrimeRuntime  # pyre-ignore
from src.ppl_emulator.runtime.resource_budget import ResourceLimitExceeded  # pyre-ignore
from src.ppl_emulator.runtime.types import PPLList  # pyre-ignore
from src.ppl_emulator.transpiler import transpile  # pyre-ignore


def _run_ppl(code: str):
    py_code = transpile(code, out_path=os.path.join(os.path.dirname(__file__), "_guardrail.png"))
    ns = {"__name__": "__main__", "__file__": "<guardrail>"}
    stdout = io.StringIO()
    with contextlib.redirect_stdout(stdout):
        exec(compile(py_code, "<ppl_transpiled>", "exec"), ns)
    return stdout.getvalue(), ns.get("_rt")


def test_print_buffer_budget_exceeded():
    rt = HPPrimeRuntime()
    try:
        rt._budget.max_output_chars = 16
        with pytest.raises(ResourceLimitExceeded) as excinfo:
            rt.PRINT("x" * 64)
        assert excinfo.value.kind == "output"
    finally:
        rt.close()


def test_list_allocation_budget_exceeded():
    rt = HPPrimeRuntime()
    try:
        rt._budget.max_single_object_bytes = 64
        with pytest.raises(ResourceLimitExceeded) as excinfo:
            rt.SET_VAR("A", PPLList([1] * 128))
        assert excinfo.value.kind == "memory"
    finally:
        rt.close()


def test_grob_allocation_budget_exceeded():
    rt = HPPrimeRuntime()
    try:
        rt._budget.max_single_object_bytes = 1
        with pytest.raises(ResourceLimitExceeded) as excinfo:
            rt.DIMGROB_P(100, 100)
        assert excinfo.value.kind == "memory"
    finally:
        rt.close()


def test_recursive_call_depth_budget_exceeded(monkeypatch):
    monkeypatch.setattr(HPPrimeRuntime, "_DEFAULT_CALL_DEPTH", 8)
    code = """
EXPORT T()
BEGIN
  LOCAL F(N)
  BEGIN
    IF N > 0 THEN
      RETURN F(N - 1);
    END;
    RETURN 0;
  END;
  PRINT(F(100));
END;
"""
    with pytest.raises(ResourceLimitExceeded) as excinfo:
        _run_ppl(code)
    assert excinfo.value.kind == "depth"


def test_loop_step_budget_exceeded(monkeypatch):
    monkeypatch.setattr(HPPrimeRuntime, "_DEFAULT_LINE_EVENTS", 30)
    code = """
EXPORT T()
BEGIN
  LOCAL I := 0;
  WHILE 1 DO
    I := I + 1;
  END;
END;
"""
    with pytest.raises(ResourceLimitExceeded) as excinfo:
        _run_ppl(code)
    assert excinfo.value.kind in {"steps", "time"}

