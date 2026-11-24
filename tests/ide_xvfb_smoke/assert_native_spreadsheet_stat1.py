"""Weighted STAT1 through the worksheet editor, input edits, PPL and restart."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ["", "_reopen"]:
    report = json.loads((root / f"native_side_spreadsheet_stat1{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    states = {s["label"]: s["state"] for s in report["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        if label == "ppl":
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            names = [v["value"] for v in outcome["typed_result"]["value"]]
            assert names[:3] == ["n", "7", "σX"], names
            assert math.isclose(float(names[3]), 5.55124549364, abs_tol=1e-10)
            assert names[4] == "8"
        else:
            grid = state["native_model_results"]["Spreadsheet"]
            assert grid["C1"]["Text"] == "n" and grid["C2"]["Text"] == "σX", grid
            assert grid["D1"]["Number"] == 7
            expected = 1.45686271817 if label == "report" else 5.55124549364
            assert math.isclose(grid["D2"]["Number"], expected, abs_tol=1e-10), grid
print("Worksheet STAT1 updated its adjacent weighted report and reopened with its retained formula")
