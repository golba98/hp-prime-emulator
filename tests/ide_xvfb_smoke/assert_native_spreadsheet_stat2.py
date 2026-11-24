"""Weighted STAT2 through the worksheet editor, PPL and process restart."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_spreadsheet_stat2{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        label, state = snapshot["label"], snapshot["state"]
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        if label == "ppl":
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            values = [v["value"] for v in outcome["typed_result"]["value"]]
            assert values[:2] == ["STAT2", "A:B:C"], values
            assert math.isclose(float(values[2]), 46 / 7, abs_tol=1e-10), values
            assert values[3:] == ["7", "224", "8"], values
        else:
            grid = state["native_model_results"]["Spreadsheet"]
            assert grid["E1"]["Text"] == "STAT2" and grid["F1"]["Text"] == "A:B:C", grid
            assert math.isclose(grid["F2"]["Number"], (27 if label == "report" else 46) / 7, abs_tol=1e-10), grid
print("Weighted STAT2 recalculated both variables and the cross product after an edit and restart")
