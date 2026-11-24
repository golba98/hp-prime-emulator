#!/usr/bin/env python3
"""Verify radical fractions in Native History and Show from actual LCD pixels."""
import json
import sys
from itertools import groupby
from pathlib import Path

from PIL import Image

out = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
report = json.loads((out / "native_side_textbook_radicals.json").read_text())
assert not report["wait_timeouts"], report["wait_timeouts"]
states = {snap["label"]: snap["state"]["native_session"] for snap in report["snapshots"]}


def bars(label, rows):
    with Image.open(out / f"textbook_radical_{label}.png") as shot:
        frame = shot.convert("RGB")
    assert frame.size == (320, 240), frame.size
    segments = []
    for y in rows:
        start = 1
        dark = (max(frame.getpixel((x, y))) < 64 for x in range(start, 92))
        for ink, group in groupby(dark):
            width = sum(1 for _ in group)
            if ink and width >= 10:
                segments.append((y, start, width))
            start += width
    return segments


for name, source in (("sqrt", "SQRT(9/4)"), ("cubert", "NTHROOT(3,27/8)")):
    history = states[f"{name}_history"]
    entry = history["history"][-1]
    assert entry["expression"] == source and entry["result"] == "1.5", entry
    assert history["ans"]["value"] == "1.5", history["ans"]

    show = states[f"{name}_show"]
    assert show["history_position"] == len(history["history"]) - 1
    assert not show["history_result"], show["history_result"]
    assert show["history_viewer"] == {"x": 0, "y": 0}, show["history_viewer"]
    for field in ("ans", "run_id", "home_input", "history"):
        assert show[field] == history[field], field

    for view, rows in (("history", range(145, 202)), ("show", range(29, 96))):
        label = f"{name}_{view}"
        strokes = bars(label, rows)
        assert len(strokes) >= 2, (label, strokes)
        (bar_y, bar_x, bar_width), (rule_y, rule_x, rule_width) = strokes[-2:]
        assert rule_y - bar_y >= 12, (label, strokes)
        if name == "cubert":
            # The indexed sign leaves one white column beside the vertical
            # stem and overhangs the fraction rule by two pixels on the right.
            assert rule_x == bar_x + 2 and rule_width + 3 == bar_width, (label, strokes)
        else:
            # The square fraction also has a gap beside its vertical stem,
            # with its capped overbar extending farther to the right.
            assert rule_x == bar_x + 2 and rule_width + 4 == bar_width, (label, strokes)

closed = states["show_closed"]
assert closed["history_viewer"] is None, closed["history_viewer"]
assert closed["run_id"] == states["cubert_show"]["run_id"]
assert closed["ans"] == states["cubert_show"]["ans"]
print("Native radicals: separate overbars and fraction rules in History and Show, with both results still 1.5")
