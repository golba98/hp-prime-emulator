"""Cancel and fill the Slopefield wizard, then choose an ODE initial point."""
import json
import math
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    dump = json.loads((root / f"native_side_geometry_field_wizard{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    for item in dump["snapshots"]:
        state = item["state"]
        objects = state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]
        arrows = objects[0]["value"]["Ok"]["Objects"]
        assert len(arrows) == 9
        for arrow in arrows:
            a,b = arrow["Polyline"]
            assert math.isclose((b["y"]-a["y"])/(b["x"]-a["x"]),a["x"]+a["y"],abs_tol=1e-11)
        if item["label"] != "field":
            path = objects[1]["value"]["Ok"]["Polyline"]
            assert any(abs(p["x"])+abs(p["y"]) < 0.11 for p in path), path[:3]
        if item["label"] == "ppl":
            result=state["native_session"]["last_outcome"]
            assert result["status"] == "success", result
            assert [v["value"] for v in result["typed_result"]["value"]] == ["6","9","8"]
print("Geometry Slopefield form, configurable variables, initial-point solution and restart passed")
