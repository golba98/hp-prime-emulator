"""Typed worksheet sort through the calculator layout and a saved profile."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_typed_sort{suffix}.json").read_text())
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
    assert outcome["typed_result"]["type"] == "list", outcome
    return outcome["typed_result"]["value"]


def numbers(portable):
    assert portable["type"] == "list", portable
    assert all(item["type"] == "real" for item in portable["value"]), portable
    return [int(item["value"]) for item in portable["value"]]


states = snapshots("")
reopened = snapshots("_reopen")
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
entered = sheet(states["seeded"])["entered_values"]

# The witness number follows its specific input row. Equal keys retain their
# relative positions in both directions even when their typed values differ.
for case, column, input_types, ascending, descending in [
    ("wide_words", "A", [1, 1, 1, 1, 0, 0, 1, 1, 1],
     [7, 8, 5, 1, 2, 9, 6, 3, 4], [3, 4, 6, 1, 2, 9, 5, 8, 7]),
    ("mixed_lists", "D", [6] * 10,
     [9, 5, 4, 7, 2, 10, 6, 3, 1, 8], [1, 3, 2, 6, 4, 7, 10, 5, 8, 9]),
    ("text_objects", "H", [2, 4, 2, 2, 6, 2, 3, 2, 1, 2],
     [8, 3, 1, 2, 4, 5, 7, 9, 10, 6], [6, 4, 2, 5, 1, 3, 7, 8, 10, 9]),
    ("negative_words", "L", [1, 1, 0, 1, 1, 1, 1, 1, 0, 0],
     [4, 1, 3, 6, 8, 10, 9, 2, 5, 7], [7, 5, 2, 9, 10, 6, 8, 3, 1, 4]),
]:
    for suffix, expected in [("ascending", ascending), ("descending", descending)]:
        label = f"{case}_{suffix}"
        state = states[label]
        ids, types, keys, outside = result(state)
        assert numbers(ids) == expected, (label, numbers(ids), expected)
        assert numbers(types) == [input_types[i - 1] for i in expected], (label, types)
        assert len(keys["value"]) == len(expected), (label, keys)
        assert int(outside["value"]) == 777, (label, outside)
        sorted_values = sheet(state)["entered_values"]
        assert [sorted_values[f"{column}{i}"] for i in range(1, len(expected) + 1)] == [
            entered[f"{column}{i}"] for i in expected
        ], label
    assert result(states[f"{case}_descending"]) == result(reopened[f"{case}_reopened"]), case

print("Spreadsheet typed sort: pointer, keyboard, stable word/list/compound keys, PPL readback and reopen passed")
