"""Verify the Graph 3D Zoom controls, shared typed window and separate restore."""

import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


def snapshots(root, suffix=""):
    report = json.loads((root / f"native_side_graph3d_plot_zoom{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {row["label"]: row["state"] for row in report["snapshots"]}


def verify_plot(state, bounds, coordinates):
    tools = state["native_tools"]
    assert not tools["calculating"] and tools["error"] is None, tools
    assert tools["page"] == "Plot"
    book = tools["memory"]["graph3d"]
    assert book["ticks"] == [1, 1, 1]
    assert book["bounds"] == bounds, (book["bounds"], bounds)
    point = tools["graph3d_trace_point"]
    assert point == {"function": "FZ1", "coordinates": coordinates[:2], "height": coordinates[2]}


def typed(state):
    result = state["native_session"]["last_outcome"]
    assert result["status"] == "success" and result["typed_result"]["type"] == "list"
    values = result["typed_result"]["value"]
    assert all(value["type"] == "real" for value in values)
    return [int(value["value"]) for value in values]


def main(root):
    main_states = snapshots(root)
    reopened = snapshots(root, "_reopen")
    for label, bounds in {
        "initial": [[-4, 4], [-4, 4], [-4, 4]],
        "all_in": [[-2, 2], [-2, 2], [-2, 2]],
        "restored_default": [[-4, 4], [-4, 4], [-4, 4]],
        "all_out": [[-8, 8], [-8, 8], [-8, 8]],
    }.items():
        verify_plot(main_states[label], bounds, ["0", "0", "0"])
    expected = {
        "selected": [[-8, 8], [-8, 8], [-8, 8]],
        "center": [[-6, 10], [-5, 11], [-6, 10]],
        "x_in": [[-2, 6], [-5, 11], [-6, 10]],
        "x_out": [[-6, 10], [-5, 11], [-6, 10]],
        "x_restored": [[-2, 6], [-5, 11], [-6, 10]],
        "y_in": [[-2, 6], [-1, 7], [-6, 10]],
        "y_out": [[-2, 6], [-5, 11], [-6, 10]],
        "z_out": [[-2, 6], [-5, 11], [-14, 18]],
        "z_restored": [[-2, 6], [-5, 11], [-6, 10]],
        "z_in": [[-2, 6], [-5, 11], [-2, 6]],
        "square": [[-2, 6], [-1, 7], [-2, 6]],
    }
    for label, bounds in expected.items():
        verify_plot(main_states[label], bounds, ["2", "3", "2"])
    verify_plot(reopened["reopened"], expected["square"], ["2", "3", "2"])
    assert typed(main_states["typed"]) == typed(reopened["reopened_typed"]) == [-2, 6, 1, -1, 7, 1, -2, 6, 1, 2]

    def graph(label):
        with Image.open(root / f"graph3d_plot_zoom_{label}.png") as image:
            return image.convert("RGB").crop((0, 44, 314, 212))

    def surface(label):
        view = graph(label)
        rgb = view.tobytes()
        mask = Image.frombytes(
            "L", view.size, bytes(255 if r == 0 and b >= 100 and g in (0, b) else 0
                                  for r, g, b in zip(rgb[::3], rgb[1::3], rgb[2::3]))
        )
        bounds = mask.getbbox()
        assert bounds is not None
        return bounds, mask.crop(bounds)

    selected_bounds, selected = surface("selected")
    zoom_bounds, zoomed = surface("square")
    reopened_bounds, restored = surface("reopened")
    assert selected.size != zoomed.size or ImageChops.difference(selected, zoomed).getbbox(), (
        selected_bounds, zoom_bounds
    )
    assert zoom_bounds[0] == reopened_bounds[0] and abs(zoom_bounds[1] - reopened_bounds[1]) <= 1
    assert ImageChops.difference(zoomed, restored).getbbox() is None
    print("Graph 3D Zoom controls share typed bounds, redraw the retained trace and restore the viewport")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
