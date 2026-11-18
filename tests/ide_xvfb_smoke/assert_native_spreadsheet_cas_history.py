"""Check CAS worksheet recall through Native desktop input and restart."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_cas_history{suffix}.json").read_text())
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


def inputs(state):
    return state["native_session"]["home_input"]


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    assert outcome["typed_result"]["type"] == "list", outcome
    return [(item["type"], item["value"]) for item in outcome["typed_result"]["value"]]


def selection(state, cell, anchor):
    tools = memory(state)
    assert tools["models"]["selected_cell"] == cell
    assert tools["spreadsheet_cursor"]["anchor"] == anchor
    assert tools["spreadsheet_cursor"]["selecting"]


states = snapshots("")
reopened = snapshots("_reopen")
exact = states["exact_cas_history"]["native_session"]["history"]
assert len(exact) == 3 and all(not entry["error"] for entry in exact)
assert exact[0]["typed_result"] == {
    "type": "rational", "value": {"numerator": "2", "denominator": "3"}
}
assert exact[1]["typed_result"]["value"]["rows"] == 2
assert exact[1]["typed_result"]["value"]["cols"] == 2
assert [item["type"] for item in exact[2]["typed_result"]["value"]] == [
    "rational", "rational", "integer"
]
seeded = sheet(states["seeded_grid"])
assert not inputs(states["empty_cas"])
assert seeded == sheet(states["empty_cas"]) == sheet(states["cancelled_picker"])
assert seeded == sheet(states["cancelled_draft"])
assert memory(states["list_saved"])["cas_history"] == []
assert memory(states["matrix_saved"])["cas_history"] == exact
assert inputs(states["cancel_draft_source"]) == inputs(states["list_draft"]) == "{1/3,2/7,8}"
assert inputs(states["matrix_draft"]) == "[[1/3,2/7],[sqrt(2),4]]"
assert inputs(states["cas_input_draft"]) == "(2+4)/9"
assert inputs(states["edited_input_draft"]) == "(2+4)/9+1"
assert inputs(states["cas_result_draft"]) == "2/3"
selection(states["list_saved"], "B1", [2, 3])
selection(states["matrix_saved"], "D1", [5, 2])
expected = [
    ("real", "0"),
    ("string", "0.333333333333"),
    ("string", "0.285714285714"),
    ("string", "1.41421356237"),
    ("string", "4"),
    ("string", "outside"),
    ("string", "1.66666666667"),
    ("string", "0.666666666667"),
    ("real", "1"),
    ("real", "2"),
    ("real", "3"),
    ("real", "0"),
    ("string", "0.333333333333"),
    ("string", "0.285714285714"),
    ("real", "8"),
    ("real", "0"),
]
assert result(states["final_ppl"]) == expected
assert inputs(reopened["recall_after_restart"]) == "{1/3,2/7,8}"
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"]) == sheet(reopened["reopen_cancel"])
assert result(reopened["reopened_ppl"]) == expected
print("Spreadsheet Get from CAS: active and cached history, editable recall, typed blocks and restart passed")
