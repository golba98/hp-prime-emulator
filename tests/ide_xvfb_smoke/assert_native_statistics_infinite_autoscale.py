#!/usr/bin/env python3
"""Verify the separate interactive and PPL Statistics 1Var Autoscale paths."""
import json
import math
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/ppl-ide-e2e")
fixture = Path(__file__).resolve().parents[1] / "fixtures/native-statistics-one-autoscale-controls.json"
controls = json.loads(fixture.read_text())["cases"]
multi = next(case for case in controls if case["name"] == "autoscale_two_analyses_all_types")
histogram, bar = (next(row for row in multi["windows"] if row[0] == kind) for kind in (1, 5))


def snapshots(suffix=""):
    run = json.loads((root / f"native_side_statistics_infinite_autoscale{suffix}.json").read_text())
    assert not run["wait_timeouts"], run["wait_timeouts"]
    return {item["label"]: item["state"] for item in run["snapshots"]}


def bounds(state, row, ymin, ymax):
    outcome = state["worker_outcome"]
    assert outcome["kind"] == "success", outcome
    value = json.loads(outcome["value"].replace("−", "-").replace("{", "[").replace("}", "]"))
    assert len(value) == 9, value
    for result, expected in zip(value[:2], row[1:3]):
        assert math.isclose(result, expected, abs_tol=1e-9), (value, row)
    assert value[2:6] == [ymin, ymax, 0, 0], value
    assert value[6:] == histogram[5:], value


initial = snapshots()
checked = initial["both_checked"]["native_tools"]["memory"]["models"]["statistics1_book"]["analyses"]
assert [(r["enabled"], r["chart"], r["data_source"], r["frequency_source"]) for r in checked[:2]] == [
    (True, "histogram", "D1", "2"),
    (True, "histogram", "D2", "2"),
]
before = initial["initial_plot"]["native_tools"]
interactive = initial["interactive_autoscale"]["native_tools"]
assert interactive["error"] == "Insufficient numerical resolution", interactive["error"]
assert interactive["memory"]["model_windows"] == before["memory"]["model_windows"]
assert interactive["memory"]["models"]["histogram"] == before["memory"]["models"]["histogram"]

bounds(initial["programmatic_autoscale"], histogram, "-Inf", "+Inf")
assert initial["infinite_setup"]["native_tools"]["page"] == "PlotSetup"
assert initial["setup_applied"]["native_tools"]["page"] == "Plot"
bounds(initial["one_bound_edited"], histogram, "-3", "+Inf")

reopened = snapshots("_reopen")
bounds(reopened["reopened_one_infinite"], histogram, "-3", "+Inf")
assert reopened["partial_setup"]["native_tools"]["page"] == "PlotSetup"
bounds(reopened["both_bounds_edited"], histogram, "-3", "15")
bounds(reopened["finite_autoscale_recovery"], bar, str(bar[3]), str(bar[4]))
print("Statistics Autoscale: interactive rejection retains the finite setup. PPL returns typed signed bounds that survive reopening and recover to a finite plot.")
