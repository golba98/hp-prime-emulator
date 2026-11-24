"""Recall Home expressions and typed results into an actual worksheet draft."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_home_history{suffix}.json").read_text())
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


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return outcome["typed_result"]["value"]


def inputs(state):
    return state["native_session"]["home_input"]


states = snapshots("")
reopened = snapshots("_reopen")
seeded = sheet(states["seeded_grid"])
assert seeded == sheet(states["cancelled_picker"]) == sheet(states["cancelled_draft"])
assert inputs(states["cancel_draft_source"]) == '{7,#10:64h,"typed"}'
assert inputs(states["older_expression_draft"]) == "2+3"
assert inputs(states["typed_list_draft"]) == '{7,#10:64h,"typed"}'
grid = memory(states["pasted_grid"])
assert grid["models"]["selected_cell"] == "D2"
assert grid["spreadsheet_cursor"]["anchor"] == [4, 4]
assert grid["spreadsheet_cursor"]["selecting"]
matrix_draft = states["matrix_draft"]
assert inputs(matrix_draft) == matrix_draft["native_session"]["last_outcome"]["typed_result"]["recall"]
matrix_grid = memory(states["pasted_matrix"])
assert matrix_grid["models"]["selected_cell"] == "B5"
assert matrix_grid["spreadsheet_cursor"]["anchor"] == [3, 6]
assert matrix_grid["spreadsheet_cursor"]["selecting"]
expected = result(states["final_ppl"])
assert expected[2]["type"] == "integer"
assert expected[2]["word"]["bits"] == 64
assert [
    float(value["value"]) if value["type"] == "real" else value["value"]
    for index, value in enumerate(expected[:-1])
    if index != 2
] == [5, 7, "typed", 1, "#10:64h", 1, 2, 3]
assert [int(value["value"]) for value in expected[-1]["value"]] == [1, 2, 3, 4]
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
assert result(reopened["reopened_ppl"]) == expected
print("Spreadsheet Get from Home: picker and draft cancel, older input, typed block, formats and reopen passed")
