"""Typed clipboard modes, cancel, formula liveness and worksheet reopen."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(name):
    dump = json.loads((root / f"native_side_spreadsheet_clipboard{name}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def sheet(state):
    return state["native_tools"]["memory"]["models"]["spreadsheet"]


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return [
        float(item["value"]) if item["type"] == "real" else item["value"]
        for item in outcome["typed_result"]["value"]
    ]


states = snapshots("")
reopened = snapshots("_reopen")
assert sheet(states["opened"]) == sheet(states["cancelled"])
assert sheet(states["cancelled"])["cells"]["D4"] == "101"
expected = {
    "values": [10, "alpha", 11, 0, 2, 20, 0, 3, -1],
    "formulas": [10, "alpha", 11, 0, 2, 20, 8, 3, -1],
    "format": [101, 102, 103, 104, 0, 106, 0, 2, -1],
    "values_format": [10, "alpha", 11, 0, 2, 20, 0, 2, -1],
    "formulas_format": [10, "alpha", 11, 0, 2, 20, 8, 2, -1],
}
for mode, values in expected.items():
    assert result(states[f"{mode}_read"]) == values, (mode, result(states[f"{mode}_read"]))
assert sheet(states["formulas"])["cells"]["D5"] == "=D4+1"
assert sheet(states["formulas_format"])["cells"]["D5"] == "=D4+1"
assert result(states["updated"]) == [41, 51]
assert result(states["extent_before"]) == [6, 5, -1, -1, -1, -1, 42, -1, -1, -1, -1, -1]
assert result(states["extent_grid_format"]) == [8, 6, 0, 2, -1, -1, 42, -1, -1, -1, -1, -1]
assert result(states["extent_paste_format"]) == [10, 8, 0, 2, 0, 2, 42, -1, -1, -1, -1, -1]
assert result(states["extent_paste_default_format"]) == [13, 8, 0, 2, 0, 2, 42, -1, 0, 42, 0, -1]
assert "H13" not in sheet(states["saved"])["cells"]
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
assert result(reopened["reopened_ppl"]) == [50, "alpha", 51, 0, 2, 20, 8, 2, -1]
assert result(reopened["reopened_extent"]) == result(states["extent_paste_default_format"])
print("Spreadsheet clipboard: five modes, inherited source, formatted extent, cancel and reopen passed")
