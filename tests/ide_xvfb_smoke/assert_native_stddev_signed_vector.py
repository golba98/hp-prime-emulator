"""Compare signed collection STDDEV from actual Native IDE Run with original G1."""

import json
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
collection = sys.argv[2] if len(sys.argv) > 2 else "vector"
assert collection in ("vector", "matrix"), collection
report = json.loads((root / f"native_side_stddev_signed_{collection}.json").read_text())
assert not report["wait_timeouts"], report["wait_timeouts"]
snapshots = {item["label"]: item["state"] for item in report["snapshots"]}
fixture = Path(__file__).resolve().parents[1] / f"fixtures/native-stddev-signed-{collection}-controls.json"


def unpack(value):
    if value["type"] == "list":
        return [unpack(item) for item in value["value"]]
    if value["type"] == "real":
        return json.loads(value["value"])
    assert value["type"] == "string", value
    return value["value"]


for case in json.loads(fixture.read_text())["cases"]:
    outcome = snapshots[case["name"]]["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    actual = unpack(outcome["typed_result"])
    assert actual == case["expected"], (case["name"], actual, case["expected"])

print(f"Native STDDEV matches original signed {collection} types, numeric results and cancellation")
