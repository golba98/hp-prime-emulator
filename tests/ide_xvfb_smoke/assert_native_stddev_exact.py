#!/usr/bin/env python3
"""Check exact STDDEV types, spelling and approximation after a Native restart."""

import json
from pathlib import Path
import sys


def project(value):
    kind, payload = value["type"], value["value"]
    if kind == "list":
        return [project(item) for item in payload]
    if kind == "real":
        return json.loads(payload)
    assert kind == "string", value
    return payload


def main():
    artifacts = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
    group = sys.argv[2] if len(sys.argv) > 2 else "exact"
    fixture = Path(__file__).resolve().parents[1] / "fixtures" / f"native-stddev-{group}-controls.json"
    cases = {case["name"]: case for case in json.loads(fixture.read_text())["cases"]}
    for name, suffix in [("save", ""), ("reopen", "_reopen")]:
        report = json.loads((artifacts / f"native_side_stddev_{group}{suffix}.json").read_text())
        assert not report["wait_timeouts"], report["wait_timeouts"]
        label = f"{group}_stddev_{name}"
        snapshot, = (snap for snap in report["snapshots"] if snap["label"] == label)
        outcome = snapshot["state"]["native_session"]["last_outcome"]
        assert outcome["status"] == "success", outcome
        assert project(outcome["typed_result"]) == cases[name]["expected"], outcome
    print(f"STDDEV {group} types, expressions and approximations survive Native IDE restart")


if __name__ == "__main__":
    main()
