"""Worksheet row sorting through the calculator layout and saved profile."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_sort{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def sheet(state):
    return state["native_tools"]["memory"]["models"]["spreadsheet"]


def unpack(value):
    if value["type"] == "list":
        return [unpack(item) for item in value["value"]]
    if value["type"] == "real":
        return float(value["value"])
    return value["value"]


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return unpack(outcome["typed_result"])


states = snapshots("")
reopened = snapshots("_reopen")
assert sheet(states["selected"]) == sheet(states["cancel_open"]) == sheet(states["cancelled"])
outside = [100, 200, 300, 400, 500, 600]
assert result(states["ascending_a"]) == [
    [0, 1, 2, 3, "Alpha", "beta"],
    [101, "two", "four", "one", "six", "five"],
    [-1, 1, 2, 3, "Alpha", "beta"],
    [-1, 3, -1, 2, -1, -1],
    [8, 2, 2, 2, 2, 2],
    outside,
    [-1, "KeyCell", "", "", "", ""],
    1,
]
assert sheet(states["ascending_grid"])["cells"]["B1"] == "=C1+1"

descending = [
    [1, "Alpha", 3, 2, "beta", 0],
    ["two", "six", "one", "four", "five", 601],
    [1, "Alpha", 3, 2, "beta", -1],
    [3, -1, 2, -1, -1, -1],
    [2, 2, 2, 2, 2, 8],
    outside,
    ["KeyCell", "", "", "", "", -1],
    1,
]
assert result(states["descending_b"]) == descending
assert sheet(states["descending_grid"])["cells"]["B6"] == "=C6+1"
assert result(states["live_formula"]) == [701, 8]
assert result(states["ties_ascending"]) == [
    ["", -4, -2, -2, 0, 0, 0, "A", "B", "a"],
    [10, 6, 2, 4, 1, 3, 5, 9, 8, 7],
    [-1, -1, -1, -1, -1, 2, -1, -1, -1, -1],
]
ties_descending = [
    [0, 0, 0, -2, -2, -4, "a", "B", "A", ""],
    [1, 3, 5, 2, 4, 6, 7, 8, 9, 10],
    [-1, 2, -1, -1, -1, -1, -1, -1, -1, -1],
]
assert result(states["ties_descending"]) == ties_descending
assert result(states["home_formatted_keys"]) == [1, 2, 0, "1"]
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
descending[1][-1] = 701
descending[5][-1] = 700
assert result(reopened["reopened_ppl"]) == descending
assert result(reopened["reopened_ties"]) == ties_descending
assert result(reopened["reopened_format"]) == [1, 2, 0, "1"]
print("Spreadsheet sort: pointer, keyboard, cancel, stable typed keys, formulas, formats and reopen passed")
