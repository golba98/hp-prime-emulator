"""Exercise both branches of a hyperbola through the plot, PPL and restart."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_geometry_hyperbola{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        state, label = snapshot["state"], snapshot["label"]
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        if label == "ppl":
            out = state["native_session"]["last_outcome"]
            assert out["status"] == "success", out
            values = [float(v["value"]) for v in out["typed_result"]["value"]]
            assert len(values) == 5 and all(math.isclose(v,e,abs_tol=1e-8) for v,e in zip(values,(6,2,3,0,338.985132275))), values
        else:
            plot = state["native_model_results"]["Geometry:plot"]
            curve = plot["geometry"]["objects"][-1]["value"]["Ok"]["Hyperbola"]
            assert curve["major"] == (1 if label == "constructed" else -1), curve
            assert math.isclose(curve["minor"], math.sqrt(3)), curve
            assert sum(series == "GD" for series in plot["labels"]) == 2, plot.keys()
print("Hyperbola drew both branches and recalculated its constrained point after a drag and restart")
