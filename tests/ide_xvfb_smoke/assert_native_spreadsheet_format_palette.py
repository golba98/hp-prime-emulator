"""Native worksheet palettes, local attributes, inheritance and restart."""

import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix):
    report = root / f"native_side_spreadsheet_format_palette{suffix}.json"
    dump = json.loads(report.read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    states = {event["label"]: event["state"] for event in dump["snapshots"]}
    for label, state in states.items():
        assert state["native_tools"]["error"] is None, (label, state["native_tools"]["error"])
        assert not state["native_tools"]["calculating"], label
    return states


def sheet(state):
    return state["native_tools"]["memory"]["models"]["spreadsheet"]


def unpack(value):
    if value["type"] == "list":
        return [unpack(item) for item in value["value"]]
    return float(value["value"])


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return unpack(outcome["typed_result"])


def background(label, x, y):
    state = states[label] if label in states else reopened[label]
    left, top = state["lcd_rect"]["min"]
    right, bottom = state["lcd_rect"]["max"]
    scale_x = lambda value: round(left + value * (right - left) / 320)
    scale_y = lambda value: round(top + value * (bottom - top) / 240)
    with Image.open(root / f"spreadsheet_palette_{label}.png") as screenshot:
        region = screenshot.convert("RGB").crop((scale_x(x), scale_y(y),
                                                scale_x(x + 14), scale_y(y + 8)))
    return max(region.getcolors(region.width * region.height))[1]


states = snapshots("")
reopened = snapshots("_reopen")
number = [1, -1, 3, 2, -1, 3]
color = [0xffd700, 0x555555, 0x00ffff, 0xff0000]
assert result(states["custom_cancelled"]) == [
    number, [0xffd700, 0x555555, 0xff00ff, -1], [-1, -1, -1, 13.3456789, 8],
]
assert result(states["cell_fill_ppl"]) == [number, color, [0xff00ff, -1, -1, 13.3456789, 8]]
inherited = [number, color, [-1, -1, -1, 13.3456789, 8]]
for label in ("column_fill_ppl", "row_fill_ppl"):
    assert result(states[label]) == inherited
assert result(reopened["reopened_ppl"]) == inherited
assert sheet(states["saved"]) == sheet(reopened["reopened_grid"])
for label, d2, d3 in [
    ("cell_fill", (255, 0, 255), (0, 0, 128)),
    ("column_fill", (0, 0, 128), (0, 0, 128)),
    ("row_fill", (255, 165, 0), (255, 255, 255)),
    ("reopened_grid", (255, 165, 0), (255, 255, 255)),
]:
    assert background(label, 219, 58) == d2, (label, background(label, 219, 58))
    assert background(label, 219, 75) == d3, (label, background(label, 219, 75))
    assert background(label, 151, 58) == (255, 165, 0), (label, background(label, 151, 58))
print("Spreadsheet palette: swatches, custom color, local/inherited fills, precision and reopen passed")
