"""Formula-backed worksheet choice and companion index in Native desktop."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_choose{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def spreadsheet(state):
    return state["native_tools"]["memory"]["models"]["spreadsheet"]


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return [
        float(value["value"]) if value["type"] == "real" else value["value"]
        for value in outcome["typed_result"]["value"]
    ]


states = snapshots("")
reopened = snapshots("_reopen")
assert spreadsheet(states["unselected_menu"]) == spreadsheet(states["cancelled_grid"])
assert "B1" not in spreadsheet(states["cancelled_grid"])["cells"]
assert result(states["cancelled_ppl"])[:6] == [0, 0, 8, 0, -1, 0]
assert result(states["keyboard_picked"])[:6] == ["Blue", 2, 8, 3, 3, "Blue"]
assert result(states["external_index"])[:6] == ["Green", 2, 8, 2, 2, "Green"]
assert result(states["out_of_range_index"])[:6] == [0, 0, 8, 4, 4, 0]
assert result(states["before_list_change"]) == ["Blue", 2, 8, 3, 3, "Blue", 0, "Hi", 2, 30, 3, 20, 2]
expected = ["Blue", 2, 8, 3, 3, "Blue", 0, "third", 3, 30, 3, 20, 2]
assert result(states["final_ppl"]) == expected
saved = spreadsheet(states["saved"])
assert saved["cells"]["A1"] == '=CHOOSE($B$1,"Ink",{"Red","Green","Blue"})'
assert saved["cells"]["A3"] == "=CHOOSE(B3,F3,F4)"
assert saved["cells"]["A4"] == '=CHOOSE(TagCell,"Alias",{10,20,30})'
assert saved == spreadsheet(reopened["reopened_grid"])
assert result(reopened["reopened_ppl"]) == expected
assert result(reopened["reopened_selection"]) == ["Green", 2, 8, 2, 2, "Green", *expected[6:]]
print("Spreadsheet CHOOSE: cancel, keys, pointer, typed options, dependencies and reopen passed")
