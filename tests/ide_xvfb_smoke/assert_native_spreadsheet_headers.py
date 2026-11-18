"""Header editing, inherited values, PPL reads and profile reopen."""
import json
from pathlib import Path
import sys


def snapshots(name):
    dump = json.loads((root / f"native_side_spreadsheet_headers{name}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def sheet(state):
    return state["native_tools"]["memory"]["models"]["spreadsheet"]


def number(state, address):
    return state["native_model_results"]["Spreadsheet"][address]["Number"]


def results(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return [float(value["value"]) for value in outcome["typed_result"]["value"]]


root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
states = snapshots("")
reopened = snapshots("_reopen")

column = states["column"]
assert sheet(column)["column_entries"] == {"2": "=A1+Row"}
assert [number(column, address) for address in ("B1", "B2", "B3", "B4")] == [11, 777, 13, 14]
intersection = states["intersection"]
assert sheet(intersection)["row_entries"] == {"3": "=A2+Col"}
assert [number(intersection, address) for address in ("A3", "B3", "C3", "D3")] == [30, 13, 300, 24]
removed = states["removed_override"]
assert "B2" not in sheet(removed)["cells"]
assert number(removed, "B2") == 12

frozen = states["frozen"]
assert [number(frozen, address) for address in ("D1", "D3", "D4")] == [12, 12, 444]
removed_header = states["removed_header"]
assert sheet(removed_header)["column_entries"] == {"2": "=A1+Row"}
assert [number(removed_header, address) for address in ("D3", "D4")] == [24, 444]
assert results(states["ppl"]) == [30, 51, 52, 53, 150, 300, 24, 72, -1, -1]
assert results(reopened["reopened_ppl"]) == results(states["ppl"])
assert number(states["far"], "B100") == 150
assert len(sheet(states["far"])["cells"]) == 6
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
assert number(reopened["reopened_grid"], "B1") == 51
assert number(reopened["reopened_grid"], "D3") == 24
def named_values(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    def values(value):
        if value["type"] == "list":
            return [values(entry) for entry in value["value"]]
        assert value["type"] == "real", value
        return float(value["value"])
    return values(outcome["typed_result"])

names = sheet(states["named_headers"])["names"]
assert names["InputCol"] == {"left": 2, "right": 2, "top": 1, "bottom": 10000}, names
assert names["TopLine"] == {"left": 1, "right": 676, "top": 1, "bottom": 1}, names
column_values = list(range(51, 58))
assert named_values(states["named_values"]) == [column_values, column_values, 378, [50, 51, 100, 0]]
extended = list(range(51, 66))
expected = [extended, extended, 870, [50, 51, 100, 0, 0, 0]]
assert named_values(states["extended_named_values"]) == expected
assert named_values(reopened["reopened_named_values"]) == expected
print("Spreadsheet headers: editing, inheritance, dynamic named lists and aggregates, PPL and profile reopen passed")
