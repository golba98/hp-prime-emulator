"""Verify that Plot Go To evaluates and marks a real surface point."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix=""):
    report = json.loads((root / f"native_side_graph3d_plot_goto{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {item["label"]: item["state"] for item in report["snapshots"]}


def trace(state, xy, z):
    tools = state["native_tools"]
    assert tools["page"] == "Plot" and tools["error"] is None, tools
    assert tools["graph3d_trace_point"] == {"function": "FZ1", "coordinates": xy, "height": z}


def returned(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    values = outcome["typed_result"]["value"]
    assert all(value["type"] == "real" for value in values), values
    return [value["value"] for value in values]


def marker(label):
    image = Image.open(root / f"graph3d_plot_goto_{label}.png").convert("RGB")
    graph = image.crop((162, 289, 1098, 807))
    pixels = graph.load()
    white = [(x, y) for y in range(graph.height) for x in range(graph.width)
             if min(pixels[x, y]) >= 245]
    assert 5 < len(white) < 160, (label, len(white))
    return (sum(x for x, _ in white) / len(white), sum(y for _, y in white) / len(white))


main = snapshots()
trace(main["initial"], ["0", "0"], "0")
trace(main["selected"], ["2", "3"], "2")
trace(main["canceled"], ["2", "3"], "2")
assert returned(main["typed"]) == ["0", "2", "-4", "4", "-4", "4", "0", "0"]
assert marker("selected") == marker("canceled")
assert marker("selected") != marker("initial")
restored = snapshots("_reopen")["restored"]
assert returned(restored) == ["1", "2", "1"]
trace(restored, ["0", "0"], "0")
print("Graph 3D Plot Go To evaluates and marks the selected point with unchanged typed settings")
