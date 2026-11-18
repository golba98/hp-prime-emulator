"""Verify Statistics 2Var Reset restores only its own data and settings."""
import json
import sys
from pathlib import Path

from assert_native_statistics_one_reset import assert_dialog, result


def stages(root, suffix=""):
    report = json.loads((root / f"native_side_statistics_two_reset{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {snap["label"]: snap["state"] for snap in report["snapshots"]}


def tools(state):
    return state["native_tools"]


def book(state):
    return tools(state)["memory"]["models"]["statistics2_book"]


def main(root):
    current = stages(root)
    reopened = stages(root, "_reopen")
    assert result(current["summary"]) == [10, 9, 1, 1]
    before = result(current["before"])
    first, second = before[:2]
    assert first[:4] == ["C1", "C2", "C3", 1] and first[4]
    assert first[5:] == [0x0090F0, 9, 0xD01070]
    assert second[:4] == ["C1", "C2", "C3", 13] and second[4]
    assert before[2:] == [
        [1, 2, 3], [4, 7, 10], [1, 2, 3], [9],
        -4, 9, -5, 12, 2, 3, 1, 1, 1, 1,
        2, 2, 6, 1, 1, 1, 975310,
    ]
    assert result(current["after_cancel"]) == before
    assert book(current["after_cancel"]) == book(current["before"])
    assert_dialog(root, "cancel_prompt", "canceled", "statistics_two_reset")
    assert_dialog(root, "confirm_prompt", "confirmed", "statistics_two_reset")
    select = tools(current["inactive_highlight"])["memory"]
    assert select["gallery"]["highlight"] == "Statistics 2Var"
    assert select["selected_app"] == "Statistics 1Var"
    assert tools(current["confirmed"])["memory"]["selected_app"] == "Statistics 1Var"
    expected = [
        ["C1", "C2", "", 1, "", 0x0000FF, 1, 0x0000FF],
        ["", "", "", 1, "", 0xFF0000, 2, 0xFF0000], [], [], [], [],
        -1.8, 30, -1.8, 20, 1, 1, 0, 0, 0, 0,
        0, 0, 4, 0, 1, 0, 975310,
    ]
    assert result(current["after_reset"]) == result(reopened["reopened"]) == expected
    assert tools(current["symbolic_restored"])["page"] == "Symbolic"
    assert tools(current["numeric_cleared"])["page"] == "Numeric"
    for state in (current["numeric_cleared"], reopened["reopened"]):
        assert book(state)["columns"] == [[] for _ in range(10)]
        assert not book(state).get("fit_equations")
        assert not book(state)["analyses"][1]["user_expression"]
    other = [[2, 4, 8], [5, 9, 17], ["D1", "", 8, 0, 0x0000FF], 975310]
    assert result(current["other_app"]) == result(reopened["reopened_other"]) == other
    print("Statistics 2Var Reset preserved Statistics 1Var and Home, with scoped settings and fits cleared across reopen")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
