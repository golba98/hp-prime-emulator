#!/usr/bin/env python3
"""Check a negative fractional INVERT_P boundary through IDE Run."""
import json
import sys
from pathlib import Path

from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
report = json.loads((out / "native_side_invert_raster.json").read_text())
assert not report["wait_timeouts"], report["wait_timeouts"]
snapshot, = (
    entry for entry in report["snapshots"] if entry["label"] == "invert_fractional_edge"
)
result = snapshot["state"]["native_session"]["last_outcome"]
assert result["status"] == "success", result

fixture = Path(__file__).resolve().parents[1] / "fixtures/native-invert-controls.json"
cases = json.loads(fixture.read_text())["cases"]
control, = (case for case in cases if case["name"] == "desktop_fractional_edge")
reported = result["typed_result"]["value"]
assert [int(value["value"]) for value in reported[:2]] == control["expected"][:2]
pixels = control["expected"][2]
assert [int(value["value"]) for value in reported[2]["value"]] == pixels

frame = Image.open(out / "native_invert_fractional_edge.png").convert("RGB")
assert frame.size == (320, 240), frame.size
for index, color in enumerate(pixels):
    point = (index % 7, index // 7)
    expected = (color >> 16, (color >> 8) & 255, color & 255)
    actual = frame.getpixel(point)
    assert actual == expected, (point, actual, expected)
print("Native INVERT_P returns and all 49 LCD pixels match the original virtual G1")
