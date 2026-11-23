"""Construct a parabola, drag its focus and read a constrained tangent after restart."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_geometry_parabola{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snap in report["snapshots"]:
        state, label = snap["state"], snap["label"]
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        if label == "ppl":
            out = state["native_session"]["last_outcome"]
            assert out["status"] == "success", out
            values = [float(v["value"]) for v in out["typed_result"]["value"]]
            assert len(values) == 3 and all(math.isclose(v,e,abs_tol=1e-8) for v,e in zip(values,(2,1.5,0.5))), values
        else:
            objects = state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]
            curve = objects[-1]["value"]["Ok"]["Parabola"]
            assert math.isclose(curve["vertex"]["y"], 0 if label == "constructed" else 1, abs_tol=1e-8), curve
            book = state["native_tools"]["memory"]["models"]["geometry_book"]
            assert book["objects"][-1]["source"] == "parabola(GA,GD)", book
print("Parabola construction, focus drag, constrained point and tangent survive restart")
