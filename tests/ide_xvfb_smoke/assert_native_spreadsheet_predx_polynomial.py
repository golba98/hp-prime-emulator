"""Quadratic inverse prediction through worksheet entry, PPL and restart."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_predx_polynomial{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def sheet(state):
    return state["native_tools"]["memory"]["models"]["spreadsheet"]


def unpack(item):
    if item["type"] == "list":
        return [unpack(value) for value in item["value"]]
    assert item["type"] == "real", item
    return float(item["value"])


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return unpack(outcome["typed_result"])


states = snapshots("")
reopened = snapshots("_reopen")
expected = [4, 4, 4, 0.9, 8, 1, 2, [-6, -1, 1], 777, 4, 2.20506763921, -2]
assert result(states["before_edit"]) == [3, 3, 3, 0.9, 8, 1, 2, [-6, -1, 1], 777, 3, 4, -2]
assert result(states["after_edit"]) == expected
assert result(reopened["reopened_ppl"]) == expected
saved = sheet(states["saved"])
assert saved == sheet(reopened["reopened_grid"])
assert saved["formats"] == sheet(states["formatted_grid"])["formats"]
for address, formula in [
    ("D1", "=PredX(9,A1,PolyCoef)"),
    ("D2", "=PredX(10,A1,-6,-1,1,0)"),
    ("D3", "=PredX(11,A1,-6,-1,1,0,0)"),
    ("D4", "=PredX(9,0,0.27,-1.2,1)"),
]:
    assert saved["cells"][address] == formula, saved["cells"]
print("Spreadsheet PredX: selected roots, degree reduction, named coefficients, formatting and reopen passed")
