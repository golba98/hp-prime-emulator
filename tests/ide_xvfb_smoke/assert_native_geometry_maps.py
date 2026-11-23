"""Construct an inversion from the Transform menu and read dependent objects after restart."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_geometry_maps{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        state = snapshot["state"]
        objects = state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]
        assert objects[7]["value"]["Ok"]["Point"] == {"x": 2, "y": 1}, objects[7]
        if snapshot["label"] == "ppl":
            out = state["native_session"]["last_outcome"]
            assert out["status"] == "success", out
            vals = [float(v["value"]) for v in out["typed_result"]["value"]]
            assert len(vals) == 6 and all(math.isclose(a,b) for a,b in zip(vals,[2,1,-2,2,2,2])), vals
print("Transform menu inversion and projected points, pole, polar line and cold reads passed")
