#!/usr/bin/env python3
"""Compare actual Native IDE signed-list STDDEV output with the original G1."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
report = json.loads((root / "native_side_stddev_signed.json").read_text())
assert not report["wait_timeouts"], report["wait_timeouts"]
snapshot, = (snap for snap in report["snapshots"] if snap["label"] == "signed_deviation")
result = snapshot["state"]["native_session"]["last_outcome"]
assert result["status"] == "success", result
actual = [json.loads(item["value"]) if item["type"] == "real" else item["value"]
          for item in result["typed_result"]["value"]]
fixture = Path(__file__).resolve().parents[1] / "fixtures/native-stddev-signed-controls.json"
control, = (c for c in json.loads(fixture.read_text())["cases"] if c["name"] == "desktop_signed")
assert actual == control["expected"], (actual, control["expected"])
print("Native STDDEV matches the original typed real, complex and zero-total signed-list results")
