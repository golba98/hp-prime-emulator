"""Regression predictions through native worksheet entry, PPL and restart."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_prediction{suffix}.json").read_text())
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
expected = [13, 3, 57, -2, 8, 1, 2, [2, 5], 777, 13]
assert result(states["before_edit"]) == [11, 3, 34, -2, 8, 1, 2, [2, 5], 777, 11]
assert result(states["after_edit"]) == expected
assert result(reopened["reopened_ppl"]) == expected
saved = sheet(states["saved"])
assert saved == sheet(reopened["reopened_grid"])
assert saved["formats"] == sheet(states["formatted_grid"])["formats"]
for address, formula in [
    ("D1", "=PredY(1,A1,FitCoef)"),
    ("D2", "=PredX(1,C1,FitCoef)"),
    ("D3", "=PredY(9,A1,1,2,3)"),
    ("D4", "=PredX(4,-40,3,5)"),
]:
    assert saved["cells"][address] == formula, saved["cells"]
print("Spreadsheet predictions: worksheet entry, live dependencies, named coefficients, formats and reopen passed")
