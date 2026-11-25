#!/usr/bin/env python3
"""Check Notes formatting in the LCD and its plain-text PPL readback."""
import json
from pathlib import Path
import sys

from PIL import Image


root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
text = "Plain control\nRound item\nNext item"


def snapshots(suffix):
    result = json.loads((root / f"native_side_note_bullets{suffix}.json").read_text())
    return {entry["label"]: entry["state"] for entry in result["snapshots"]}


def marker_rows(label):
    with Image.open(root / f"note_bullets_{label}.png") as capture:
        assert capture.size == (320, 240), label
        image = capture.convert("RGB")
        # Sample the marker gutter within the editor. The Bullets toolbar
        # starts below it, at a different height when the saved note is listed.
        body_top, body_bottom = (
            (97, 154) if label in ("saved", "restored", "cleared") else (72, 129)
        )
        active = [
            y for y in range(body_top, body_bottom)
            if sum(max(image.getpixel((x, y))) < 150 for x in range(2, 9)) >= 2
        ]
    groups = []
    for y in active:
        if not groups or y > groups[-1][-1] + 1:
            groups.append([])
        groups[-1].append(y)
    return [sum(group) // len(group) for group in groups]


def assert_readback(state):
    result = state["worker_outcome"]
    assert result["kind"] == "success" and result["value"] == text, result


created = snapshots("")
reopened = snapshots("_reopen")
for state in (created["saved"], reopened["restored"]):
    tool = state["native_tools"]
    assert tool["page"] == "Notes" and tool["error"] is None, tool
    assert tool["memory"]["name"] == "BULLETNOTE", tool
    assert tool["memory"]["content"] == text, tool
assert_readback(created["ppl"])
assert_readback(reopened["ppl"])

plain = marker_rows("plain")
formatted = marker_rows("formatted")
continued = marker_rows("continued")
saved = marker_rows("saved")
restored = marker_rows("restored")
cleared = marker_rows("cleared")
assert not plain, plain
assert len(formatted) == 1 and 87 <= formatted[0] <= 99, formatted
assert len(continued) == 2 and continued[0] == formatted[0], continued
assert 13 <= continued[1] - continued[0] <= 20, continued
assert len(saved) == 2 and saved == restored, (saved, restored)
assert cleared == restored[:-1], (cleared, restored)
print("Note bullets mark the active paragraph, continue with Enter, and survive saving and restart.")
