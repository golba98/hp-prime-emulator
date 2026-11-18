"""Verify PPL and desktop Function selectors share the same saved definitions."""
import json
import sys
from pathlib import Path


def load(root, suffix=""):
    report = json.loads((root / f"native_side_function_checkbox{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {item["label"]: item["state"] for item in report["snapshots"]}


def values(state):
    assert state["native_tools"]["error"] is None, state["native_tools"]["error"]
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    items = outcome["typed_result"]["value"]
    assert all(item["type"] == "real" for item in items), items
    return [float(item["value"]) for item in items]


def curves(state):
    return state["native_tools"]["memory"]["graphs"]["Function"]["curves"]


def main(root):
    states = load(root)
    assert values(states["empty"]) == [1, 1, 0, 0, 0]
    assert values(states["definitions"]) == [0, 0, 0]
    assert values(states["selected"]) == [1, 1, 1, 0, 1]
    assert states["selected"]["native_tools"]["page"] == "Symbolic"
    assert curves(states["selected"])[0]["enabled"]
    assert not curves(states["pointer"])[0]["enabled"]
    assert curves(states["pointer"])[9]["enabled"]
    assert values(states["pointer_read"]) == [1, 1, 1, 1, 1, 0, 1]
    domain = states["numeric_domain"]
    assert domain["native_tools"]["page"] == "Numeric"
    assert domain["native_tools"]["error"] is None
    assert domain["native_session"]["last_outcome"]["typed_result"] == {
        "type": "real", "value": "1",
    }
    assert domain["native_tools"]["table"][:3] == [[0, None, 0], [1, 1, 3], [2, 0.5, 6]]
    for state in [states["numeric"], load(root, "_reopen")["reopened"]]:
        assert values(state) == [1, 0, 0, 1]
        assert state["native_tools"]["page"] == "Numeric"
        assert state["native_tools"]["table"][:3] == [[0, 0], [1, 3], [2, 6]]
        assert curves(state)[9]["expression"] == "3*X"
    print("Function CHECK, UNCHECK and ISCHECK share Symbolic, Numeric and saved F0 selections")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
