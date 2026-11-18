#!/usr/bin/env python3
"""Compare Native IDE polygon results and LCD pixels with the original G1."""
import json
import sys
from pathlib import Path

from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
report = json.loads((out / "native_side_polygon_raster.json").read_text())
assert not report["wait_timeouts"], report["wait_timeouts"]
fixture = Path(__file__).resolve().parents[1] / "fixtures/native-polygon-controls.json"
cases = {case["name"]: case for case in json.loads(fixture.read_text())["cases"]}


def decode(value):
    if value["type"] == "list":
        return [decode(item) for item in value["value"]]
    if value["type"] == "real":
        return int(value["value"])
    assert value["type"] == "string", value
    return value["value"]


for label, case, image_name in [
    ("original_polygon", "desktop_polygon", "native_polygon_raster.png"),
    ("desktop_cartesian", "desktop_cartesian", "native_desktop_cartesian.png"),
]:
    snapshot, = (item for item in report["snapshots"] if item["label"] == label)
    outcome = snapshot["state"]["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    expected = cases[case]["expected"]
    assert decode(outcome["typed_result"]) == expected, label
    with Image.open(out / image_name) as image:
        frame = image.convert("RGB")
    assert frame.size == (320, 240), frame.size
    for index, color in enumerate(expected[-1]):
        point = (index % 7, index // 7)
        rgb = ((color >> 16) & 255, (color >> 8) & 255, color & 255)
        actual = frame.getpixel(point)
        assert actual == rgb, (label, point, actual, rgb)
print("Native FILLPOLY and FILLPOLY_P typed returns and all 98 LCD pixels match the original virtual G1")
