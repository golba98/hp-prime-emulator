#!/usr/bin/env python3
"""Check matrix row geometry, scrolling and typed values in the Native desktop."""
from itertools import groupby
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


def snapshots(root, suffix=""):
    report = json.loads((root / f"native_side_textbook_matrix_spacing{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {
        snapshot["label"]: snapshot["state"]["native_session"]
        for snapshot in report["snapshots"]
    }


def matrix_font_installed(root):
    sources = set()
    for suffix in ("", "_reopen"):
        report = json.loads((root / f"native_side_textbook_matrix_spacing{suffix}.json").read_text())
        for snapshot in report["snapshots"]:
            state = snapshot["state"]
            font = state["calculator_font"]
            source = font["source"]
            assert font["error"] is None
            assert font["family"] == ("DejaVu Sans" if source is None else "Prime Sans")
            assert state["native_session"]["presentation"].get("font_size", "medium") == "medium"
            sources.add(source)
    assert len(sources) == 1, sources
    return None not in sources


def entry(state):
    result = state["history"][-1]
    assert not result["error"], result
    assert result["typed_result"]["type"] == "matrix", result
    return result


def matrix(state):
    typed = entry(state)["typed_result"]
    if "rows" in typed:
        return typed["rows"], typed["cols"], [cell["value"] for cell in typed["value"]]
    value = typed["value"]
    return value["rows"], value["cols"], value["values"]


def frame(root, name):
    with Image.open(root / f"textbook_matrix_spacing_{name}.png") as capture:
        assert capture.size == (320, 240), capture.size
        return capture.convert("L")


def runs(ys):
    return [
        (points[0], points[-1])
        for _, positions in groupby(enumerate(ys), key=lambda pair: pair[1] - pair[0])
        if (points := [y for _, y in positions])
    ]


def rail_runs(image, x, top, bottom):
    return runs([y for y in range(top, bottom) if image.getpixel((x, y)) < 95])


def rails(image):
    horiz = [
        x for x in range(320)
        if len(rail_runs(image, x, 24, 220)) == 1
        and sum(image.getpixel((x, y)) < 95 for y in range(24, 220)) >= 12
    ]
    assert horiz, "matrix rails absent"
    return min(horiz), max(horiz)


def height(run):
    return run[1] - run[0] + 1


def ink_bounds(image, x0, x1, y0, y1):
    return runs([
        y for y in range(y0, y1)
        if any(image.getpixel((x, y)) < 95 for x in range(x0, x1))
    ])


def main(root):
    states = snapshots(root)
    reopened = snapshots(root, "_reopen")
    installed = matrix_font_installed(root)
    ordinary = ("home_one", "home_two", "home_three", "home_five")
    cells = ("8", "6", "9", "5", "7", "4", "3", "2", "1", "0")
    for name, n in zip(ordinary, (1, 2, 3, 5)):
        assert matrix(states[name + "_history"]) == (n, 2, list(cells[:n * 2]))
    integer = lambda value: {"type": "integer", "value": value}
    rational = lambda numerator, denominator: {
        "type": "rational", "value": {"numerator": numerator, "denominator": denominator},
    }
    exact = [rational(str(n), str(n + 1)) for n in range(1, 5)]
    assert matrix(states["cas_two_history"]) == (2, 2, [integer(value) for value in cells[:4]])
    assert matrix(states["cas_fraction_history"]) == (2, 2, exact)
    assert matrix(states["cas_mixed_history"]) == (
        2, 2, [integer("8"), exact[0], integer("5"), integer("9")],
    )
    assert matrix(states["cas_radical_history"]) == (
        2, 2, [integer("8"), {"type": "symbolic", "value": "sqrt(x/(x+1))"},
               integer("5"), integer("9")],
    )

    # History and fitting Show use the original full rail heights.
    sizes = dict(zip(ordinary, (16, 36, 56, 96)))
    sizes.update(cas_two=36, cas_fraction=74, cas_mixed=55, cas_radical=57)
    for name, expected in sizes.items():
        history = states[name + "_history"]
        show = states[name + "_show"]
        assert history["history"] == show["history"], name
        assert show["history_viewer"] == {"x": 0, "y": 0} and show["history_result"], name
        view = frame(root, name + "_show")
        assert view.crop((0, 0, 320, 24)).getextrema() == (255, 255), name
        left, right = rails(view)
        assert abs(left + right - 318) <= 1, name
        segments = [rail_runs(view, x, 24, 220)[0] for x in (left, right)]
        assert [height(segment) for segment in segments] == [expected] * 2, name
        assert all(abs(top + bottom - 218) <= 1 for top, bottom in segments), name
        if installed and name in (*ordinary, "cas_two"):
            assert (left, right) == (142, 176), name
            assert [rail_runs(view, x, 24, 220)[0] for x in (left + 2, right - 2)] == segments, name
        view = frame(root, name + "_history")
        # The newest History pair ends one pixel above the y202 entry divider.
        segments = [rail_runs(view, x, 24, 202)[-1] for x in (1, 318)]
        assert segments == [(202 - expected, 201)] * 2, (name, segments)

    view = frame(root, "home_five_show")
    left, _ = rails(view)
    top = rail_runs(view, left, 24, 220)[0][0]
    digit_offsets = [(3, 12)] * 5 if installed else [(2, 11), (2, 11), (3, 11), (2, 11), (3, 11)]
    for row, (first, last) in enumerate(digit_offsets):
        row_top = top + row * 20
        digits = ink_bounds(
            view, left + 7, left + 19, row_top, min(row_top + 20, top + sizes["home_five"])
        )
        assert digits and (digits[0][0], digits[-1][1]) == (row_top + first, row_top + last)
    view = frame(root, "cas_mixed_show")
    left, _ = rails(view)
    top = rail_runs(view, left, 24, 220)[0][0]
    digit_top, digit_bottom = ink_bounds(view, left + 5, left + 14, top, top + 32)[0]
    fraction_rows = [
        y for y in range(top, top + 35)
        if sum(view.getpixel((x, y)) < 95 for x in range(left + 15, left + 34)) >= 10
    ]
    assert len(fraction_rows) == 1
    assert abs(fraction_rows[0] - (digit_top + digit_bottom) / 2) <= 3

    tall = states["cas_tall_history"]
    expected_cells = [integer(str(n)) for row in range(1, 17) for n in (row, row + 8)]
    assert matrix(tall) == (16, 2, expected_cells)
    labels = ["cas_tall_start", "cas_tall_down", "cas_tall_drag", "cas_tall_end"]
    assert all(entry(states[name]) == entry(tall) for name in labels)
    offsets = [states[name]["history_viewer"]["y"] for name in labels]
    assert offsets[:2] == [0, 16] and all(a < b for a, b in zip(offsets, offsets[1:])), offsets
    for name in labels[1:3]:
        view = frame(root, name)
        assert all(rail_runs(view, x, 24, 220) == [(24, 219)] for x in rails(view))
    end = frame(root, labels[-1])
    left, right = rails(end)
    assert abs(left + right - 319) <= 1
    for x in (left, right):
        assert rail_runs(end, x, 24, 220) == [(24, 213)]
    assert all(end.getpixel((x, 213)) < 95 for x in (left + 2, right - 2))

    for saved, previous, original in [
        ("home_reopened", "home_five", "home_five_show"),
        ("cas_reopened", "cas_tall", "cas_tall_start"),
    ]:
        assert reopened[saved + "_history"]["history"] == states[previous + "_history"]["history"]
        assert reopened[saved + "_show"]["history_viewer"] == {"x": 0, "y": 0}
        crop = (0, 24, 320, 220)
        old = frame(root, original).crop(crop)
        new = frame(root, saved + "_show").crop(crop)
        assert ImageChops.difference(old, new).getbbox() is None, saved
    print("Matrix History and Show match measured row spacing, preserve exact cells and scroll through restart")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
