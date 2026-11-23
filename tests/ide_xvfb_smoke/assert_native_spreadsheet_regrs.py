"""Linear REGRS through the worksheet editor, PPL and process restart."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_spreadsheet_regrs{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        label, state = snapshot["label"], snapshot["state"]
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        if label == "ppl":
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            values = [v["value"] for v in outcome["typed_result"]["value"]]
            assert values[:2] == ["REGRS", "A:B"], values
            assert math.isclose(float(values[2]), -3, abs_tol=1e-10), values
            assert math.isclose(float(values[3]), 59 / 3, abs_tol=1e-10), values
            assert values[4:] == ["-8", "8"], values
        else:
            grid = state["native_model_results"]["Spreadsheet"]
            assert grid["E1"]["Text"] == "REGRS" and grid["F1"]["Text"] == "A:B", grid
            assert math.isclose(grid["F2"]["Number"], 1.5 if label == "report" else -3, abs_tol=1e-10), grid
print("Linear REGRS recalculated the slope, intercept and covariance after an edit and restart")
