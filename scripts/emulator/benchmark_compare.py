#!/usr/bin/env python3
"""Validate identical firmware workloads and summarize paired benchmark trials."""
import argparse
import json
import math
from pathlib import Path
from statistics import median


def summarize(before, after):
    if not before or len(before) != len(after):
        raise ValueError("equal nonempty before/after trial groups are required")
    baseline = before[0]
    identity_keys = ["firmware_sha256", "initial_profile_sha256", "script_sha256", "platform"]
    for key in identity_keys:
        if not baseline.get(key):
            raise ValueError(f"missing comparison identity: {key}")
    expected = baseline.get("results", [])
    # The published benchmark includes restart cases; truncated/no-restart runs
    # cannot silently be reported as complete regression coverage.
    required = ["tiny_cold", "tiny_repeat", "small_math", "medium", "large_cold",
                "large_repeat", "edited_check", "edited_run", "alternate_tiny",
                "syntax_error", "after_error_tiny", "tiny_after_restart", "repeat_after_restart"]
    if [row.get("case") for row in expected] != required:
        raise ValueError("incomplete or unexpected workload sequence")
    signatures = [(row["case"], row.get("source_sha256"), row.get("source_bytes")) for row in expected]
    if any(not source or not size for _, source, size in signatures):
        raise ValueError("missing workload identity")
    for group in [before, after]:
        helper = group[0].get("helper_sha256")
        if not helper:
            raise ValueError("missing helper identity")
        for trial in group:
            if any(trial.get(key) != baseline[key] for key in identity_keys):
                raise ValueError("firmware, profile, script or host identity differs")
            if trial.get("helper_sha256") != helper:
                raise ValueError("helper changed within a trial group")
            rows = trial.get("results", [])
            if [(r.get("case"), r.get("source_sha256"), r.get("source_bytes")) for r in rows] != signatures:
                raise ValueError("workload sequence or source differs")
            if any(r.get("correct") is not True for r in rows):
                raise ValueError("a workload failed its expected firmware verdict")
            for row in rows:
                elapsed = row.get("elapsed_seconds")
                if isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed) or elapsed <= 0:
                    raise ValueError("invalid workload duration")
            boots = trial.get("boot_seconds", [])
            if len(boots) != 2 or any(isinstance(t, bool) or not isinstance(t, (int, float)) or not math.isfinite(t) or t <= 0 for t in boots):
                raise ValueError("fresh-process restart evidence is incomplete")
    result = {"schema_version": 1, "trials_per_helper": len(before),
              "operations": len(required) * (len(before) + len(after)),
              "all_expected_verdicts_passed": True,
              **{key: baseline[key] for key in identity_keys},
              "before_helper_sha256": before[0]["helper_sha256"],
              "after_helper_sha256": after[0]["helper_sha256"], "rows": []}
    for i, case in enumerate(required):
        a = [r["results"][i]["elapsed_seconds"] for r in before]
        b = [r["results"][i]["elapsed_seconds"] for r in after]
        result["rows"].append({"case": case, "before": a, "after": b,
                               "before_median": median(a), "after_median": median(b),
                               "change_percent": 100 * (median(b) / median(a) - 1)})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, nargs="+", required=True, help="ordered results.json paths")
    parser.add_argument("--after", type=Path, nargs="+", required=True, help="ordered results.json paths")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = summarize([json.loads(p.read_text()) for p in args.before],
                           [json.loads(p.read_text()) for p in args.after])
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
