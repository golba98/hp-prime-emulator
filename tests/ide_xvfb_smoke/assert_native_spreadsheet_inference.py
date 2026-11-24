"""Read a live proportion report after editing its source cell and restarting."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ["", "_reopen"]:
    report = json.loads((root / f"native_side_spreadsheet_inference{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for item in report["snapshots"]:
        state, label = item["state"], item["label"]
        assert state["native_tools"]["error"] is None, label
        p = 0.42 if label == "report" else 0.4
        lower = p - 1.95996398454 * math.sqrt(p * (1 - p) / 50)
        if label == "ppl":
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            prop, value, bound, limit, kind = [v["value"] for v in outcome["typed_result"]["value"]]
            assert (prop, bound, kind) == ("\ue001₁", "Lower", "8")
            assert float(value) == p and math.isclose(float(limit), lower, abs_tol=1e-10)
        else:
            grid = state["native_model_results"]["Spreadsheet"]
            assert grid["C1"]["Text"] == "\ue001₁" and grid["C2"]["Text"] == "Lower", grid
            assert grid["D1"]["Number"] == p
            assert math.isclose(grid["D2"]["Number"], lower, abs_tol=1e-10), grid
print("Worksheet inference recomputed on edit and retained its formula and numeric results after restart")
