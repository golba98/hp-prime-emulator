"""Verify Graph 3D custom cells, stable decimal sort, calculation and reopen."""
import json
import sys
from pathlib import Path
from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")


def snapshots(suffix=""):
    report = json.loads((root / f"native_side_graph3d_numeric_custom{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {entry["label"]: entry["state"] for entry in report["snapshots"]}


def decode(value):
    kind = value["type"]
    if kind == "real":
        return float(value["value"])
    if kind != "list":
        assert kind == "matrix" and value["vector"] and value["rows"] == 1, value
    return [decode(item) for item in value["value"]]


def result(state):
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return decode(outcome["typed_result"])


def tools(state, cell):
    tool = state["native_tools"]
    assert tool["page"] == "Numeric" and tool["error"] is None, (tool["page"], tool["error"])
    assert tool["graph3d_numeric_cell"] == cell, tool["graph3d_numeric_cell"]
    return tool["table"]


main = snapshots()
assert tools(main["surface_selected"], [0, 2])[0] == [3, 1, 1]
with (Image.open(root / "graph3d_numeric_custom_definition.png") as displayed,
      Image.open(root / "graph3d_numeric_custom_hidden.png") as hidden):
    # The formula appears above the grid, occupying the first row below the
    # toolbar. Compare the right portion of that row, outside the X/Y headers.
    area = (230, 299, 497, 345)
    assert displayed.crop(area).tobytes() != hidden.crop(area).tobytes()

ascending = [[2, 5], [2, -2], [-1, 11], [5, -1], [3, 1]]
descending = [[2, 5], [2, -2], [3, 1], [5, -1], [-1, 11]]
assert result(main["ascending"]) == ascending
assert result(main["descending"]) == descending
for label, pairs in [("ascending_table", ascending), ("descending_table", descending)]:
    table = tools(main[label], [0, 2])
    assert [row[:2] for row in table] == pairs
    assert table[:2] == [[2, 5, None], [2, -2, None]]

assert tools(main["new_y_selected"], [5, 1])[-1][:2] == [-1, 11]
added = [*descending, [0, 5]]
assert result(main["added"]) == added
assert tools(main["added_table"], [5, 1])[-1] == [0, 5, -0.5]

edited = [[-3, 5], *added[1:]]
assert result(main["edited"]) == result(main["retained"]) == edited
assert tools(main["edited_table"], [0, 0])[0] == [-3, 5, -0.2]
assert tools(main["canceled"], [0, 0]) == tools(main["edited_table"], [0, 0])

restored = snapshots("_reopen")["restored"]
assert result(restored) == [edited, 1]
assert tools(restored, [0, 0]) == tools(main["edited_table"], [0, 0])
print("Graph 3D custom coordinate expressions and stable surface sorts survive restart")
