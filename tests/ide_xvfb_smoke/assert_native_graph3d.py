"""Check Graph 3D source, calculation, input, raster geometry and persistence."""
import json
from pathlib import Path
import sys
from PIL import Image, ImageChops


def load(root, suffix=""):
    report = json.loads((root / f"native_side_graph3d{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {item["label"]: item["state"] for item in report["snapshots"]}


def result(state):
    assert state["native_tools"]["error"] is None, state["native_tools"]["error"]
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    value = outcome["typed_result"]
    parts = value["value"] if value["type"] == "list" else [value]
    assert all(part["type"] == "real" for part in parts), parts
    return [float(part["value"]) for part in parts]


def book(state):
    return state["native_tools"]["memory"]["graph3d"]


def check_table(state):
    tool = state["native_tools"]
    assert tool["page"] == "Numeric"
    assert tool["table"][:4] == [[0, 0, 0], [0.1, 0.1, 0], [0.2, 0.2, 0], [0.3, 0.3, 0]]


def check_images(root):
    def load_image(name):
        return Image.open(root / f"graph3d_{name}.png").convert("RGB").crop((0, 40, 320, 204))
    plot, rotated, dragged, reopened = (
        load_image(name) for name in ("plot", "rotated", "dragged", "reopened")
    )
    for image in (plot, rotated, dragged, reopened):
        pixels = {rgb: count for count, rgb in image.getcolors(image.width * image.height)}
        # The saddle has changing normals on both sides. Check coverage and
        # shading separately so a flat-colored mesh cannot satisfy this test.
        distinct = 0
        for lower in (False, True):
            shades = {b: count for (r, g, b), count in pixels.items()
                      if r == 0 and b > 32 and g == (b if lower else 0)}
            assert sum(shades.values()) > 100 and len(shades) > 1, shades
            distinct += len(shades)
        assert distinct > 20, distinct
    assert ImageChops.difference(plot, rotated).getbbox()
    assert ImageChops.difference(rotated, dragged).getbbox()
    assert ImageChops.difference(dragged, reopened).getbbox() is None


def main(root):
    states, reopened = load(root), load(root, "_reopen")
    expected = {
        "symbolic": [1, 8, 0], "plot": [1, 1.6, 0, 1],
        "numeric": [1, 1.6, 8], "qualified": [1.6, 8, 1.6],
    }
    for name, values in expected.items():
        assert result(states[name]) == values, name
    assert states["symbolic"]["native_tools"]["page"] == "Symbolic"
    gallery = states["gallery_started"]["native_tools"]
    assert gallery["page"] == "Symbolic"
    assert gallery["memory"]["selected_app"] == "Graph 3D"
    assert gallery["memory"]["gallery"]["recent"][0] == "Graph 3D"
    assert states["plot"]["native_tools"]["page"] == "Plot"
    assert not book(states["symbolic"])["definitions"][0]["enabled"]
    assert book(states["plot"])["definitions"][0]["enabled"]
    assert book(states["plot"])["bounds"] == [[-4, 4]] * 3
    assert book(states["plot"])["rotation"] != book(states["rotated"])["rotation"]
    assert book(states["rotated"])["rotation"] != book(states["dragged"])["rotation"]
    assert book(states["dragged"]) == book(reopened["reopened"])
    assert result(reopened["reopened"]) == [1, 1, 1.6, 1]
    assert result(reopened["reopened_table"]) == [1]
    check_table(states["numeric"])
    check_table(reopened["reopened_table"])
    check_images(root)
    print("Graph 3D used live PPL definitions, rendered and rotated surfaces, and restored the saved table and camera")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
