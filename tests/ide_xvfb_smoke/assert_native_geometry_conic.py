"""Construct a dependent equation from the Curve menu and read it on cold reopen."""
from pathlib import Path
import json
import math
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    report = json.loads((root / f"native_side_geometry_conic{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    for snapshot in report["snapshots"]:
        state = snapshot["state"]
        if snapshot["label"] == "constructed":
            scene = state["native_model_results"]["Geometry:plot"]["geometry"]
            assert scene["objects"][1]["value"]["Ok"]["Circle"] == {"center": {"x": 0, "y": 0}, "radius": 9}, scene
        else:
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            kind, radius, area = (float(v["value"]) for v in outcome["typed_result"]["value"])
            assert kind == 8 and radius == 9 and math.isclose(area, 254.469004941), outcome
print("Geometry Curve menu, equation-defined circle and typed area passed across restart")
