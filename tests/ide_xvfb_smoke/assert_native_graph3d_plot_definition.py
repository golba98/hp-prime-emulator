"""Use the Plot definition picker and editor on two live Graph 3D surfaces."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def states(suffix=""):
    report = json.loads((root / f"native_side_graph3d_plot_definition{suffix}.json").read_text())
    assert not report["wait_timeouts"]
    return {snapshot["label"]: snapshot["state"] for snapshot in report["snapshots"]}


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    values = outcome["typed_result"]["value"]
    assert all(value["type"] == "real" for value in values), values
    return [value["value"] for value in values]


def surface(state, function, height):
    tools = state["native_tools"]
    assert tools["page"] == "Plot" and tools["error"] is None, tools["error"]
    assert tools["graph3d_trace_point"] == {
        "function": function, "coordinates": ["0", "0"], "height": height,
    }
    return tools["memory"]["graph3d"]["definitions"]


main = states()
original = surface(main["initial"], "FZ1", "0")
assert [item["source"] for item in original[:2]] == ["0", "1"]
assert surface(main["selected"], "FZ2", "1") == original
updated = surface(main["updated"], "FZ2", "2")
assert updated[0] == original[0] and updated[1]["source"] == "2" and updated[1]["enabled"]

invalid = main["invalid"]["native_tools"]
assert invalid["error"] and invalid["memory"]["graph3d"]["definitions"] == updated
assert surface(main["canceled"], "FZ2", "2") == updated
goto = main["goto_edited"]["native_tools"]
assert goto["page"] == "Plot" and goto["error"] is None
assert goto["graph3d_trace_point"] == {
    "function": "FZ2", "coordinates": ["2", "3"], "height": "2",
}
assert goto["memory"]["graph3d"]["definitions"] == updated
assert result(main["typed"]) == result(main["retained"]) == ["0", "2", "0", "1"]

with (Image.open(root / "graph3d_plot_definition_selected.png") as before,
      Image.open(root / "graph3d_plot_definition_updated.png") as after):
    graph = (450, 325, 800, 620)
    assert ImageChops.difference(before.convert("RGB").crop(graph),
                                after.convert("RGB").crop(graph)).getbbox()

reopened = states("_reopen")["reopened"]
assert result(reopened) == ["0", "2", "0", "1", "1"]
assert surface(reopened, "FZ1", "0") == updated
print("Graph 3D Plot selects, edits and redraws FZ2 with exact PPL and restart readback")
