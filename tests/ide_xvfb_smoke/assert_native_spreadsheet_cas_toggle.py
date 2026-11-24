"""Compare ordinary, CAS-formula and frozen CAS entries across a restart."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    dump = json.loads((root / f"native_side_spreadsheet_cas_toggle{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        tools = state["native_tools"]
        assert tools["error"] is None, (label, tools["error"])
        assert not tools["calculating"], label
    return states


def memory(state):
    return state["native_tools"]["memory"]


def sheet(state):
    return memory(state)["models"]["spreadsheet"]


def entry(state):
    return state["native_session"]["home_input"]


def typed(value):
    if value["type"] == "list":
        return [typed(item) for item in value["value"]]
    assert value["type"] in ("real", "string"), value
    return (value["type"], value["value"])


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return typed(outcome["typed_result"])


def cell(value_type, value, entered_type, entered):
    return [
        ("real", str(value_type)), ("string", value),
        ("real", str(entered_type)), ("string", entered),
    ]


states = snapshots("")
reopened = snapshots("_reopen")
assert not memory(states["ordinary_formula"]).get("spreadsheet_cas_mode", False)
assert memory(states["exact_draft"])["spreadsheet_cas_mode"]
assert entry(states["exact_draft"]) == "=sqrt(A1)"
assert sheet(states["exact_formula"])["cells"]["D2"] == '=CAS("sqrt(A1)")'
assert sheet(states["frozen_exact"])["cells"]["D3"] == "=2+sqrt(2)"
assert entry(states["cancel_draft"]) == "88+1"
assert sheet(states["cancelled"]) == sheet(states["frozen_exact"])
assert sheet(states["pasted_formula"])["cells"]["D6"] == '=CAS("sqrt(A5)")'
assert sheet(states["pasted_frozen_values"])["cells"]["D8"] == "3.41421356237"
assert sheet(states["pasted_frozen_formulas"])["cells"]["D9"] == "=2+sqrt(2)"
assert entry(states["recall_rational_draft"]) == "1/3"
assert sheet(states["frozen_rational"])["cells"]["D10"] == "=1/3"
before = [
    cell(0, "1.41421356237", 8, "sqrt(A1)"),
    cell(8, "sqrt(2)", 8, 'CAS("sqrt(A1)")'),
    cell(0, "3.41421356237", 8, "2+sqrt(2)"),
    cell(0, "88", 0, "88"),
    cell(0, "18", 8, 'CAS("zzsheetmode+A")'),
    cell(0, "0", 8, 'CAS("sqrt(A5)")'),
    cell(8, "x^2+2*x+1", 8, "x^2+2*x+1"),
    cell(0, "3.41421356237", 0, "3.41421356237"),
    cell(0, "3.41421356237", 8, "2+sqrt(2)"),
    cell(0, "0.333333333333", 8, "1/3"),
    ("real", "1"), ("real", "2"), ("real", "3"),
]
after = [
    cell(0, "2.2360679775", 8, "sqrt(A1)"),
    cell(8, "sqrt(5)", 8, 'CAS("sqrt(A1)")'),
    *before[2:],
]
assert result(states["before_ppl"]) == before
assert result(states["after_ppl"]) == after
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
assert entry(reopened["formula_edit_reopened"]) == "=sqrt(A1)"
assert entry(reopened["frozen_edit_reopened"]) == "=2+sqrt(2)"
assert result(reopened["reopened_ppl"]) == after
print("Spreadsheet CAS toggle: typed live and frozen results, cancellation, paste and reopen passed")
