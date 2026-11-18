"""Check Do2VStats cache validity against actual program and Plot workflows."""
import json
import sys
from pathlib import Path

from assert_native_statistics_one_reset import result


def stages(root, suffix=""):
    report = json.loads((root / f"native_side_statistics_two_cache{suffix}.json").read_text())
    assert not report["wait_timeouts"], report["wait_timeouts"]
    return {snap["label"]: snap["state"] for snap in report["snapshots"]}


def main(root):
    current = stages(root)
    reopened = stages(root, "_reopen")
    expected = {
        "uncached_to_cached": [1, 1, 1, 9, 0],
        "home_and_style_keep_cache": [1, 1, 9, 0xE0F0],
        "same_column_write_invalidates": [0, 9, 0],
        "invalid_target_updates_other": [0, 13, 2, 1, 0],
        "uncached_board_column_invalidates": [0, 13, 0],
        "fresh_plot_cache": [1, 1, 17, 0],
    }
    for label, values in expected.items():
        assert result(current[label]) == values, label
    assert current["fresh_plot_cache"]["native_tools"]["page"] == "Plot"
    assert result(reopened["cold_reopen"]) == [0, 17, 0]
    print("Statistics 2Var cache passed same-value column invalidation, checked-fit side effects, live Plot and cold reopen")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e"))
