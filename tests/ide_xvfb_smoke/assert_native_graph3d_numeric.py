"""Verify independent decimal Graph 3D samples, custom pairs and app isolation."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def states(suffix=""):
    report = json.loads((root / f"native_side_graph3d_numeric{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {item["label"]: item["state"] for item in report["snapshots"]}


def decode(value):
    if value["type"] == "real":
        return float(value["value"])
    if value["type"] == "list":
        return [decode(item) for item in value["value"]]
    assert value["type"] == "matrix" and value["vector"] and value["rows"] == 1
    return [decode(item) for item in value["value"]]


def outcome(state):
    result = state["native_session"]["last_outcome"]
    assert result["status"] == "success", result
    assert state["native_tools"]["error"] is None, state["native_tools"]["error"]
    return decode(result["typed_result"])


main = states()
reopened = states("_reopen")["reopened"]
assert outcome(main["defaults"]) == [0, 0, 0.1, 0.1, 2, 2, 0, []]
assert outcome(main["automatic"]) == [1, 2, -1, 0.5, 2, 2, 2, 0, []]
assert main["automatic"]["native_tools"]["page"] == "Numeric"
assert main["automatic"]["native_tools"]["table"][:4] == [
    [2, -1, 0], [2.5, 1, 4.5], [3, 3, 9], [3.5, 5, 13.5],
]
assert outcome(main["setup"]) == 1
assert main["setup"]["native_tools"]["page"] == "NumericSetup"
assert outcome(main["invalid"]) == [1, 1, 1, 1, 0.5, 2, 2, 0]
points = [[1, 3], [2, 4]]
assert outcome(main["custom"]) == [1, 1, 1, points]
assert outcome(main["isolated"]) == [-5, 3, 2, 2]
assert outcome(reopened) == [1, 2, -1, 0.5, 2, 2, 2, 1, points]
assert main["custom"]["native_tools"]["table"] == reopened["native_tools"]["table"] == [
    [1, 3, 7], [2, 4, 10],
]
print("Graph 3D Numeric Setup uses independent decimal axes and typed custom vectors through PPL and reopen")
