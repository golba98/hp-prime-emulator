"""Select the moving and driving points, enter a parametric curve, and reopen."""
import json
import math
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_geometry_locus{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        state, label = snapshot["state"], snapshot["label"]
        plot = state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]
        circle = plot[3]["value"]["Ok"]["Circle"]
        assert circle == {"center": {"x": 1, "y": 2}, "radius": 2}, circle
        if label == "locus":
            continue
        path = plot[4]["value"]["Ok"]["Parametric"]["paths"][0]
        assert len(path) >= 200 and all(math.isclose(p["x"]**2,p["y"],abs_tol=1e-9) for p in path)
        if label in ("plots", "ppl"):
            polar = plot[5]["value"]["Ok"]["Parametric"]["paths"][0]
            assert all(math.isclose(math.hypot(p["x"], p["y"]), 2) for p in polar)
            cobweb = plot[6]["value"]["Ok"]["Sequence"]["paths"]
            assert len(cobweb[1]) == 11 and cobweb[2][-1] == {"x": 0.59375, "y": -1}, cobweb
        if label == "ppl":
            out = state["native_session"]["last_outcome"]
            assert out["status"] == "success", out
            values = [float(v["value"]) for v in out["typed_result"]["value"]]
            assert values[:5] == [8, 2, 1, 2, 1] and math.isclose(values[5], -16/3), values
            assert values[6] == 8 and math.isclose(values[7], 4*math.pi), values
            assert values[8:] == [6, 6, -2], values
print("Geometry Locus selection, parametric interval editing, area and restart passed")
