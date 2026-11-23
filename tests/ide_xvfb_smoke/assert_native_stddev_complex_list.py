"""Check calculator complex-list statistics and saved Native types against G1."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
fixture = Path(__file__).resolve().parents[1] / "fixtures/native-stddev-complex-list-controls.json"
expected = json.loads(fixture.read_text())


def load(suffix):
    report = json.loads((root / f"native_side_stddev_complex_list{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {snapshot["label"]: snapshot["state"] for snapshot in report["snapshots"]}


def unpack(value):
    if value["type"] == "list":
        return [unpack(item) for item in value["value"]]
    if value["type"] == "real":
        return json.loads(value["value"])
    assert value["type"] == "string", value
    return value["value"]


def check(snapshot, record):
    outcome = snapshot["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    assert unpack(outcome["typed_result"]) == record["expected"], outcome


states = load("")
reopened = load("_reopen")
for case in expected["cases"]:
    check(states[case["name"]], case)
check(states["saved"], expected["persistence"]["saved"])
check(reopened["reopened"], expected["persistence"]["reopened"])
print("Complex STDDEV observations match G1 arithmetic, typed cancellation and saved-state reopen")
