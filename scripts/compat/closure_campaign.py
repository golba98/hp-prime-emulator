#!/usr/bin/env python3
"""Prepare the next physical campaign locally. Never uploads or opens USB."""
import argparse
import hashlib
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PREVIOUS = ROOT / "artifacts/g1-resolution/20261028T200940Z"
BATCHES = [
    ("collections", "core", "list_size list_indexing list_append_zero mat_create_2x2 mat_indexing sem_list_copy_by_value scope_for_after_loop sem_for_limit_eval".split()),
    ("string_boundaries", "strings_baseconv", "bc_mid_past_end bc_mid_len_past_end bc_left_zero bc_left_past_end bc_instring_miss bc_instring_hit bc_instring_empty_needle bc_dim_empty".split()),
    ("remaining_core", "core", "scope_uninit_local scope_while_loop scope_repeat_loop sem_string_quoting sem_asc_multi sem_str_index_local sem_list_append_empty".split()),
    ("remaining_baseconv", "strings_baseconv", "bc_upper_mixed bc_upper_non_ascii bc_dim_unicode bc_string_real bc_string_large bc_mod_16 bc_ip_div bc_ip_negative bc_round_13_digits bc_large_mod bc_large_ip_div bc_frac_times_base".split()),
    ("extended", "g1_extended_v1", "numerical.complex_parts numerical.matrix_product language.list_copy language.precedence language.scope_return numerical.format_boundary".split()),
]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def selection():
    baseline = json.loads((PREVIOUS / "results.json").read_text())
    pending = {(r["suite"], r["id"]) for r in baseline["records"]
               if r["engine"] == "native" and r["verdict"] == "not_run"}
    seen = set()
    result = []
    for name, suite, ids in BATCHES:
        path = ROOT / "tests/compat/cases" / (suite + ".toml")
        raw = path.read_bytes()
        cases = {c["id"]: c for c in tomllib.loads(raw.decode())["case"]}
        selected = []
        for case_id in ids:
            key = (suite, case_id)
            if key not in pending or key in seen:
                raise ValueError(f"Not a distinct pending baseline case: {key}")
            case = cases[case_id]
            if case.get("isolate") or case.get("mutates"):
                raise ValueError(f"Isolated/mutating case cannot be batched: {key}")
            seen.add(key)
            source = case.get("expr", case.get("body"))
            selected.append({"baseline_case": list(key), "source_sha256": sha(source.encode()),
                             "descriptor": case})
        result.append({"id": name, "suite": suite, "suite_path": str(path),
                       "suite_sha256": sha(raw), "cases": selected})
    if len(seen) != 41:
        raise ValueError("Campaign must retain exactly 41 distinct pending cases")
    return result


def prepare(out, binary):
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / "campaign.json"
    if manifest_path.exists():
        raise ValueError("Campaign already prepared; use a new output directory")
    batches = selection()
    runs = out / "runs"
    sources = out / "prepared"
    sources.mkdir()
    for batch in batches:
        before = set(runs.glob("*.json"))
        command = [str(binary.resolve()), "compare", batch["suite_path"], "--cases",
                   ",".join(c["baseline_case"][1] for c in batch["cases"]),
                   "--channel", "note", "--device", "prepare", "--runs-dir", str(runs.resolve()),
                   "--evidence-dir", str((out / "evidence").resolve()), "--no-report"]
        run = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=30)
        (out / (batch["id"] + ".log")).write_text(run.stdout + run.stderr)
        if run.returncode:
            raise RuntimeError(f"Preparation failed: {batch['id']}; inspect retained log")
        added = set(runs.glob("*.json")) - before
        if len(added) != 1:
            raise ValueError("Expected exactly one new run manifest")
        path = added.pop()
        record = json.loads(path.read_text())
        source_path = sources / (record["program"] + ".hpprgm")
        source_path.write_text(record["source"])
        batch.update({"command": command, "run_manifest": str(path), "program": record["program"],
                      "note": record["note"], "source": str(source_path), "source_sha256": sha(source_path.read_bytes()),
                      "physical_verdict": "not_run", "requires": "Read-only collision preflight, then USER IDE Send and USER Run"})
    manifest_path.write_text(json.dumps({"schema_version": 1, "baseline": str(PREVIOUS),
        "baseline_results_sha256": sha((PREVIOUS / "results.json").read_bytes()),
        "original_cases": 154, "new_controls": 0, "pending_cases_selected": 41,
        "binary_sha256": sha(binary.read_bytes()), "batches": batches,
        "hardware_actions": [], "policy": "No automatic Send, Replace, Run, key injection or cleanup"}, indent=2))
    return manifest_path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    print(prepare(args.out, args.binary))
