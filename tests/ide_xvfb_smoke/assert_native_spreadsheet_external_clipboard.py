"""Spreadsheet collection clipboard input through the desktop and reopen."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    report = root / f"native_side_spreadsheet_external_clipboard{suffix}.json"
    dump = json.loads(report.read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def memory(state):
    return state["native_tools"]["memory"]


def sheet(state):
    return memory(state)["models"]["spreadsheet"]


def unpack(value):
    kind = value["type"]
    if kind == "list":
        return [unpack(item) for item in value["value"]]
    if kind == "matrix":
        data = [unpack(item) for item in value["value"]]
        cols = value["cols"]
        return [data[index:index + cols] for index in range(0, len(data), cols)]
    if kind == "real":
        return float(value["value"])
    return value["value"]


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return unpack(outcome["typed_result"])


states = snapshots("")
reopened = snapshots("_reopen")
for label, active, anchor in [
    ("matrix_grid", "A3", [2, 4]),
    ("vector_grid", "A5", [1, 7]),
    ("one_row_grid", "E1", [5, 2]),
]:
    calculator = memory(states[label])
    assert calculator["models"]["selected_cell"] == active
    assert calculator["spreadsheet_cursor"]["anchor"] == anchor
    assert calculator["spreadsheet_cursor"]["selecting"]
assert "H5" not in sheet(states["internal_mode_open"])["cells"]
assert not {"I5", "H6"}.intersection(sheet(states["internal_matrix"])["cells"])
expected = [
    [11, 12, 13, 14],
    [10, -9, 0.333333333333, 8, 2],
    [21, 22, 23, 0],
    [31, "beta", 33, 0],
    [71, 72, 0, 0],
    [[41, 42], [43, 44], 0],
    [[[9, 52], [53, 54]], 4, 0, [9, 99], 8],
    [[[81, 82], [83, 84]], 0],
    [[[101, 102], [103, 104]], 0, 0],
    [9, 77, 10, 8, [[9, 19]], 4],
]
assert result(states["before_source_change"]) == expected
expected[9][0] = 19
expected[6][3][0] = 19
expected[9][2] = 20
assert result(states["after_source_change"]) == expected
assert result(reopened["reopened_ppl"]) == expected
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
print("Spreadsheet external clipboard: collection shapes, effect on cells, internal modes, and reopen passed")
