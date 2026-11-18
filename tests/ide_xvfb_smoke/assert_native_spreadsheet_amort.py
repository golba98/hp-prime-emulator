"""A worksheet AMORT formula writes its loan schedule at a separate destination."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_spreadsheet_amort{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        label, state = snapshot["label"], snapshot["state"]
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        if label == "ppl":
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            values = [v["value"] for v in outcome["typed_result"]["value"]]
            assert values[:4] == ["Amortization", "987", "1", "4"], values
            for v, expected in zip(values[4:7], (19.3020552, -100.6979448, -3.3020552)):
                assert math.isclose(float(v), expected, abs_tol=1e-9), values
            assert values[7] == "8", values
        else:
            grid = state["native_model_results"]["Spreadsheet"]
            assert grid["E4"]["Text"] == "Amortization" and grid["B1"]["Number"] == 987, grid
            assert grid["B2"]["Text"] == "Period" and grid["D2"]["Number"] == 4, grid
            expected = -1.510025 if label == "report" else 19.3020552
            assert math.isclose(grid["E2"]["Number"], expected, abs_tol=1e-9), grid
print("AMORT updated the destination schedule after a principal edit and retained its formula on restart")
