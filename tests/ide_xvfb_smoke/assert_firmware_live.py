"""Check real desktop Save/Restart/Resume evidence from script_firmware_live."""
import json
import sys
from pathlib import Path

from PIL import Image

root = Path(sys.argv[1])
report_dir = Path(sys.argv[2]) / "ide-reports" if len(sys.argv) > 2 else root / "ide-reports"
data = json.loads((root / "dump.json").read_text())
assert not data["wait_timeouts"], data["wait_timeouts"]
assert not data["firmware_program_running"]
states = {s["label"]: s["state"] for s in data["snapshots"]}
for name in ("waiting", "saved", "resumed"):
    assert states[name]["firmware_session"] == "Running", (name, states[name]["firmware_status"])
    assert states[name]["firmware_program_running"]
assert states["saved"]["firmware_live_save_result"] == {"Ok": None}
assert states["saved"]["firmware_live_save"] is None
assert states["restarted"]["firmware_session"] == "Ready"
assert not states["restarted"]["firmware_resumed"]
assert states["resumed"]["firmware_resumed"]
assert states["returned"]["firmware_session"] == "Ready"
assert states["resumed"]["firmware_environment"] == states["saved"]["firmware_environment"]
assert len({states[n]["firmware_session_id"] for n in ("saved", "restarted", "resumed")}) == 3
assert any(m["text"] == "G1 firmware returned {113, 37, 150}" for m in data["console"])
for name in ("waiting.png", "saved-window.png", "resumed.png", "returned.png"):
    assert (root / name).is_file(), name
# The clock/battery header can advance. The program's actual 320x220
# graphics must be identical across a complete helper-process replacement.
with Image.open(root / "waiting.png") as before, Image.open(root / "resumed.png") as after:
    assert before.size == after.size == (320, 240)
    assert before.convert("RGB").crop((0, 20, 320, 240)).tobytes() == after.convert("RGB").crop((0, 20, 320, 240)).tobytes()
report = json.loads((report_dir / "request-2.json").read_text())
assert report["success"], report["message"]
invocations = report["runtime"]["invocations"]
frames = [v for v in invocations if v["entry"] == "ZZT_GUI_FRAME"]
assert len(frames) == 1 and frames[0]["status"] == "completed"
assert frames[0]["result"]["value"] == {
    "type": "list",
    "value": [{"type": "real", "value": str(n)} for n in (113, 37, 150)],
}
print("Firmware desktop Save/Restart/Resume: preserved graphics, local 37 and single entry counter; returned {113,37,150}")
