"""Verify the Plot face colors, typed surfaces and separate-process restore."""

import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


def workflow(root, suffix=""):
    report = json.loads((root / f"native_side_graph3d_colors{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    states = {entry["label"]: entry["state"] for entry in report["snapshots"]}
    assert all(state["native_tools"]["error"] is None for state in states.values())
    return states


def book(state):
    return state["native_tools"]["memory"]["graph3d"]


def typed(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success" and outcome["typed_result"]["type"] == "list"
    values = outcome["typed_result"]["value"]
    assert all(part["type"] == "real" for part in values)
    return [int(part["value"]) for part in values]


def main(root):
    main_states = workflow(root)
    reopened = workflow(root, "_reopen")
    assert typed(main_states["typed"]) == typed(reopened["reopened_typed"]) == [0, 1, 0, 0, 1, 1]
    assert book(main_states["upper"])["colors"][0] == [0x800080, 0x00FFFF]
    for state in [main_states[key] for key in ("both", "lower", "symbolic")] + [
        reopened["reopened"], reopened["reopened_symbolic"]
    ]:
        assert book(state)["colors"][:2] == [[0x800080, 0xFF0000], [0xFF0000, 0xFF00FF]]
    assert book(main_states["initial"])["rotation"] == [-45, 25]
    assert book(main_states["lower"])["rotation"] == book(reopened["reopened"])["rotation"] == [315, -35]
    assert main_states["symbolic"]["native_tools"]["page"] == reopened["reopened_symbolic"]["native_tools"]["page"] == "Symbolic"

    def plot(label):
        file = root / f"graph3d_colors_{label}.png"
        with Image.open(file) as image:
            return image.convert("RGB").crop((0, 44, 312, 211))

    def pixels(label):
        image = plot(label)
        return {rgb: count for count, rgb in image.getcolors(image.width * image.height)}

    top_before = pixels("initial")
    top_after = pixels("both")
    bottom = pixels("lower")
    assert top_before[(0, 0, 215)] > 300 and top_before[(215, 0, 0)] > 300
    assert top_after[(108, 0, 108)] > 300 and (0, 0, 215) not in top_after
    assert bottom[(215, 0, 0)] > 300 and bottom[(215, 0, 215)] > 300
    assert ImageChops.difference(plot("lower"), plot("reopened")).getbbox() is None
    print("Graph 3D Plot recolors both filled faces and restores the typed surfaces and camera")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
