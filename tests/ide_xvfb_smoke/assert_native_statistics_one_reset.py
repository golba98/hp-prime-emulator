"""Verify the Statistics 1Var Reset dialog, scoped model changes and reopen."""
import json
import sys
from pathlib import Path

from PIL import Image, ImageChops


def stages(root, suffix=""):
    report = json.loads((root / f"native_side_statistics_one_reset{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {snap["label"]: snap["state"] for snap in report["snapshots"]}


def decode(node):
    kind = node["type"]
    if kind == "real":
        return float(node["value"])
    if kind == "list":
        return [decode(item) for item in node["value"]]
    if kind == "string":
        return node["value"]
    if kind == "integer":
        assert node["word"]["bits"] == 24, node
        return int(node["word"]["value"])
    assert kind == "symbolic" and node["value"]["type"] == "symbolic", node
    return node["value"]["value"]


def result(state):
    assert state["native_tools"]["error"] is None, state["native_tools"]["error"]
    outcome = state["native_session"]["last_outcome"]
    assert outcome["status"] == "success", outcome
    return decode(outcome["typed_result"])


def tool(state):
    return state["native_tools"]


def model(state):
    return tool(state)["memory"]["models"]["statistics1_book"]


def assert_dialog(root, prompt, dismissed, case="statistics_one_reset"):
    def image(label):
        return Image.open(root / f"{case}_{label}.png").convert("RGB")

    before = image(prompt)
    after = image(dismissed)
    assert before.size == after.size == (320, 240)
    area = (35, 75, 285, 165)
    difference = ImageChops.difference(before.crop(area), after.crop(area))
    assert sum(difference.convert("L").histogram()[1:]) > 1000


def main(root):
    main_states = stages(root)
    reopen_states = stages(root, "_reopen")
    expected_before = [
        ["D1", "D2", 8, 0, 0x20A0C0], ["D2", "", 1, 0, 0xFF0000],
        [1, 2, 3], [2, 1, 4], 1, 1, 16 / 7, 7,
        -3, 7, -4, 17, 2, 3, 1, 1, 1, 1, 2, 22, 2,
        2, 2, 6, 1, 123456789,
    ]
    before = result(main_states["before"])
    assert before[:6] == expected_before[:6] and before[7:] == expected_before[7:]
    assert abs(before[6] - expected_before[6]) < 1e-10
    assert result(main_states["after_cancel"]) == before
    assert model(main_states["before"]) == model(main_states["after_cancel"])
    assert_dialog(root, "cancel_prompt", "canceled")
    assert_dialog(root, "confirm_prompt", "confirmed")

    gallery = tool(main_states["inactive_highlight"])["memory"]
    assert gallery["gallery"]["highlight"] == "Statistics 1Var"
    assert gallery["selected_app"] == "Statistics 2Var"
    assert tool(main_states["confirmed"])["memory"]["selected_app"] == "Statistics 2Var"

    expected_after = [
        ["D1", "", 1, 0, 0x0000FF], ["", "", 1, 0, 0xFF0000],
        [], [], 1, 0, 0, 0, -1.8, 30, -1.8, 20, 1, 1, 0, 0, 0, 0, 0, 24, 1,
        0, 0, 4, 0, 123456789,
    ]
    assert result(main_states["after_reset"]) == expected_after
    assert result(reopen_states["reopened"]) == expected_after
    assert tool(main_states["symbolic_restored"])["page"] == "Symbolic"
    assert tool(main_states["numeric_cleared"])["page"] == "Numeric"
    for state in (main_states["numeric_cleared"], reopen_states["reopened"]):
        book = model(state)
        assert book["columns"] == [[] for _ in range(10)]
        assert not book.get("result_values") and book.get("result_analysis") is None
    expected_other = [
        [2, 4, 8], [5, 9, 17], ["C1", "C2", "", 8, "", 0x0000FF, 1, 0x0000FF],
        123456789,
    ]
    for state in (main_states["unrelated_preserved"], reopen_states["reopened_unrelated"]):
        assert result(state) == expected_other
    print("Statistics 1Var Reset preserved unrelated state and restored data, reports and settings across restart")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
