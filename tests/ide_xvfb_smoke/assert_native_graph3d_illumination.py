"""Check the rendered face and illumination against original virtual G1 planes and gestures."""
from collections import Counter
import json
from pathlib import Path
import sys

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
report = json.loads((root / "native_side_graph3d_illumination.json").read_text())
assert not report["wait_timeouts"], report["wait_timeouts"]
# The four fixed-camera controls compare exact RGB levels. The G1 swipes
# are delivered over 40 touch events, whereas the desktop uses its input
# frame timing, so allow two intensity levels on the moving controls.
expected = {
    "back": (True, 149, 0), "front": (False, 245, 0),
    "oblique": (False, 247, 0), "flat": (False, 215, 0),
    "horizontal": (False, 214, 2), "under": (True, 152, 2),
    "above": (False, 229, 2),
}
assert len(report["snapshots"]) == len(expected)
for snapshot in report["snapshots"]:
    label, state = snapshot["label"], snapshot["state"]["native_tools"]
    assert state["page"] == "Plot" and state["error"] is None, state["error"]
    bottom, target, tolerance = expected[label]
    with Image.open(root / f"graph3d_light_{label}.png") as image:
        raster = image.convert("RGB").crop((4, 45, 312, 206))
        colors = Counter({rgb: count for count, rgb in raster.getcolors(raster.width * raster.height)})
    surface = [(n, b) for (r, g, b), n in colors.items()
               if r == 0 and b > 0 and g == (b if bottom else 0)]
    count, value = max(surface)
    assert count > 150 and abs(value - target) <= tolerance, (label, count, value, target)
print("Graph 3D plane gradients and both faces after swiping match virtual G1 controls")
