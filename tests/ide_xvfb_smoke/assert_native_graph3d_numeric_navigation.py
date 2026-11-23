"""Check selection, zoom, Go To and the persisted Graph 3D coordinate grid."""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix=""):
    report = json.loads((root / f"native_side_graph3d_numeric_navigation{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {item["label"]: item["state"] for item in report["snapshots"]}


def configuration(state):
    result = state["native_session"]["last_outcome"]
    assert result["status"] == "success", result
    assert result["typed_result"]["type"] == "list", result
    numbers = result["typed_result"]["value"]
    assert all(number["type"] == "real" for number in numbers), numbers
    return [float(number["value"]) for number in numbers]


def table(state, row):
    tools = state["native_tools"]
    assert tools["page"] == "Numeric" and tools["error"] is None, (tools["page"], tools["error"])
    assert tools["graph3d_numeric_row"] == row, tools["graph3d_numeric_row"]
    return tools["table"]


main = snapshots()
assert table(main["third_row"], 2)[2] == [3, 7, 17]
x_zoom = table(main["x_zoom_table"], 2)
assert x_zoom[0] == [2.75, 3, 8.75] and x_zoom[2] == [3, 7, 17]
assert configuration(main["x_zoom"]) == [2.75, 3, 0.125, 2, 4, 3, 0]

assert table(main["pointer_row"], 2)[2] == [3, 7, 17]
y_zoom = table(main["y_zoom_table"], 2)
assert y_zoom[0] == [2, -5, -8] and y_zoom[2] == [3, 7, 17]
assert configuration(main["y_zoom"]) == [2, -5, 0.5, 6, 4, 3, 0]

assert table(main["trig_table"], 0)[:2] == [[0, 0, 0], [7.5, 7.5, 22.5]]
assert configuration(main["trig"]) == [0, 0, 7.5, 7.5, 4, 3, 0]

go_to = table(main["go_table"], 5)
assert go_to[0] == [3.5, -15, -26.5] and go_to[5] == [6, -5, -4]
assert table(main["canceled"], 5) == go_to
assert configuration(main["go"]) == [3.5, -15, 0.5, 2, 4, 3, 0]

restored = snapshots("_reopen")["restored"]
assert configuration(restored) == [3.5, -15, 0.5, 2, 4, 3, 1]
assert table(restored, 0) == go_to
print("Graph 3D Numeric zoom preserved the selected pair and Go To restored its typed grid")
