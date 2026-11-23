"""Exercise the Geometry Slider menu, keyboard adjustment and dependent PPL reads."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    dump = json.loads((root / f"native_side_geometry_slider{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    for item in dump["snapshots"]:
        state = item["state"]
        if item["label"] == "ppl":
            result = state["native_session"]["last_outcome"]
            assert result["status"] == "success", result
            assert [v["value"] for v in result["typed_result"]["value"]] == ["0","0.6","1.2"], result
        else:
            objects = state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]
            assert len(objects) == 1, objects
            slider = objects[0]["value"]["Ok"]["Parameter"]
            assert slider["limits"] == [0, 1], slider
            assert slider["value"] == (0.5 if item["label"] == "created" else 0.6), slider
print("Slider form, cancel, keyboard adjustment, dependent coordinates and cold PPL reads passed")
