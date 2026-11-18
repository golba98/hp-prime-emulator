"""Enter implicit, field, ODE and list definitions in the Curve menu, then reopen."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_geometry_plane{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for item in report["snapshots"]:
        state = item["state"]
        objects = state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]
        assert len(objects) == 5, objects
        circle, field, ode, listed = [obj["value"]["Ok"] for obj in objects[1:]]
        assert circle["Circle"]["radius"] == 1
        assert len(field["Objects"]) == 9
        for segment in field["Objects"]:
            a, b = segment["Polyline"]
            assert math.isclose(math.hypot(b["x"]-a["x"], b["y"]-a["y"]), 0.5)
        path = ode["Polyline"]
        assert (path[0]["x"], path[-1]["x"]) == (-2,2)
        assert all(math.isclose(p["y"],1+p["x"]**2/2) for p in path)
        assert listed["Polyline"] == [{"x":1,"y":4},{"x":2,"y":8},{"x":3,"y":12}]
        if item["label"] == "ppl":
            out = state["native_session"]["last_outcome"]
            assert out["status"] == "success", out
            assert [v["value"] for v in out["typed_result"]["value"]] == ["1","6","9","8","8"], out
print("Four Curve menu plot commands, typed PPL reads and cold reopen passed")
