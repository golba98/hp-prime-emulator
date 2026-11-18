"""Verify that the 3-D setup editor and PPL share persisted plot bounds."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def load(suffix):
    report = json.loads((root / f"native_side_graph3d_setup{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {snapshot["label"]: snapshot["state"] for snapshot in report["snapshots"]}


def values(state):
    result = state["native_session"]["last_outcome"]
    assert result["status"] == "success", result
    typed = result["typed_result"]
    assert typed["type"] == "list", typed
    assert all(entry["type"] == "real" for entry in typed["value"]), typed
    return [float(entry["value"]) for entry in typed["value"]]


def book(state):
    return state["native_tools"]["memory"]["graph3d"]


def image(label):
    return Image.open(root / f"graph3d_setup_{label}.png").convert("RGB").crop((0, 45, 315, 210))


states, reopened = load(""), load("_reopen")["reopened"]
assert values(states["defaults"]) == [-4, 4, -4, 4, -4, 4, 1, 1, 1]
ranges = [-3, 7, -2, 5, -6, 6, 1.5, 2, 2.5]
assert values(states["plot"]) == values(states["setup"]) == [1, *ranges]
assert book(states["plot"])["bounds"] == [[-3, 7], [-2, 5], [-6, 6]]
assert book(states["plot"])["ticks"] == [1.5, 2, 2.5]
assert states["setup"]["native_tools"]["page"] == "PlotSetup"

tick = book(states["setup_drag"])["ticks"][0]
assert tick > 1.5
assert values(states["function_before"]) == [-12, 18, -9, 11]
assert values(states["qualified"]) == [*values(states["function_before"]), -3, -6, 8, tick]
assert values(states["inverted"]) == [1, 8, 7, 1, 2.5, 2.5]
assert states["inverted"]["native_tools"]["page"] == "Plot"
assert states["inverted"]["native_tools"]["error"] == "X range minimum must be less than X range maximum"
assert values(states["repaired"]) == values(reopened) == [1, -3, 7, -2, 5, -6, 8, tick, 2, 2.5]
assert all(state["native_tools"]["error"] is None
           for name, state in states.items() if name != "inverted")
assert reopened["native_tools"]["error"] is None
assert book(states["repaired"]) == book(reopened)
assert ImageChops.difference(image("plot"), image("repaired")).getbbox()
assert ImageChops.difference(image("repaired"), image("reopened")).getbbox() is None
print("Graph 3D Plot Setup and PPL share typed XYZ ranges, validated tick spacing and restart state")
