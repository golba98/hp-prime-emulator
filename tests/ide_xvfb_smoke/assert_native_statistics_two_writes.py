"""Check saved Statistics 2Var Symbolic edits through the VM and rendered views."""
import json
import sys
from pathlib import Path

from PIL import Image


def stages(root, suffix=""):
    report = json.loads((root / f"native_side_statistics_two_writes{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {entry["label"]: entry["state"] for entry in report["snapshots"]}


def result(state):
    tools = state["native_tools"]
    assert tools["error"] is None and not tools["calculating"], tools.get("error")
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    assert outcome["typed_result"]["type"] == "list", outcome
    return outcome["typed_result"]["value"]


def real(node):
    assert node["type"] == "real", node
    return float(node["value"])


def expression(node):
    assert node["type"] == "symbolic" and node["value"]["type"] == "symbolic", node
    return node["value"]["value"]


def word(node):
    assert node["type"] == "integer" and node["word"]["bits"] == 24, node
    return int(node["word"]["value"])


def row(state, index=0):
    return state["native_tools"]["memory"]["models"]["statistics2_book"]["analyses"][index]


def colored_pixels(root, name, colors):
    with Image.open(root / f"statistics_two_writes_{name}.png") as image:
        assert image.size == (320, 240), image.size
        histogram = image.convert("RGB").crop((0, 20, 320, 218)).getcolors(320 * 240)
        counts = {color: count for count, color in histogram}
        for rgb, minimum in colors.items():
            assert counts.get(rgb, 0) >= minimum, (name, rgb, counts.get(rgb, 0))


def plot_colors(root):
    def is_visible_blend(pixel, source):
        channel = source.index(min(source))
        opacity = (255 - pixel[channel]) / (255 - source[channel])
        return opacity >= 0.25 and all(
            abs(observed - (255 + opacity * (component - 255))) <= 2
            for observed, component in zip(pixel, source)
        )

    with Image.open(root / "statistics_two_writes_plot.png") as image:
        assert image.size == (320, 240), image.size
        plot = image.convert("RGB").crop((0, 0, 320, 220))
        bins = plot.getcolors(320 * 220)
        for rgb, minimum in [((0, 144, 240), 9), ((208, 16, 112), 75)]:
            count = sum(count for count, pixel in bins if is_visible_blend(pixel, rgb))
            assert count >= minimum, (rgb, count)


def main(root):
    states = stages(root)
    rejected, scalar, checked = result(states["scalar_rejected"])
    assert real(rejected) == real(checked) == 1
    assert expression(scalar) == "2.5"

    definition, evaluated, enabled = result(states["weighted_read"])
    fields = definition["value"]
    assert len(fields) == 8 and real(fields[3]) == 1
    assert [expression(field) for field in fields[:3]] == ["L1", "C2", "C3"]
    assert (word(fields[5]), real(fields[6]), word(fields[7])) == (0x0090F0, 9, 0xD01070)
    assert (real(evaluated), real(enabled)) == (25, 1)

    rejected, definition, enabled = result(states["invalid_prefix"])
    assert (real(rejected), real(enabled)) == (1, 0)
    assert [expression(field) for field in definition["value"][:2]] == ["C2", "C1"]
    assert real(definition["value"][3]) == 1

    edited = row(states["edit_axis"])
    assert edited["enabled"] and edited["x_source"] == "C1*2", edited
    assert edited["fit"] == "UserDefined"
    assert states["edit_axis"]["native_tools"]["page"] == "Symbolic"

    plottable = states["plot_skin"]
    reopened = stages(root, "_reopen")["reopened"]
    assert plottable["native_tools"]["page"] == "Plot"
    assert reopened["native_tools"]["page"] == "Symbolic"
    assert row(plottable) == edited == row(reopened)
    for state in (plottable, reopened):
        kind, definition, evaluated, enabled = result(state)
        assert (real(kind), real(evaluated), real(enabled)) == (8, 9, 1)
        assert expression(definition["value"][0]) == edited["x_source"]
        assert expression(definition["value"][4]) == edited["user_expression"]

    colors = {(0, 144, 240): 20, (208, 16, 112): 20}
    for name in ("weighted", "symbolic", "reopened"):
        colored_pixels(root, name, colors)
    observations, curve = plottable["native_tools"]["plot_points"]
    assert observations == [[2, 10], [4, 15], [6, 20]]
    assert len(curve) == 257 and all(abs(y - x * x) < 1e-9 for x, y in curve)
    assert plottable["native_tools"]["memory"]["statistics_fit_visible"][1]
    plot_colors(root)
    print("Statistics 2Var typed writes, live model, color rasters, keyboard editing and reopen passed")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
