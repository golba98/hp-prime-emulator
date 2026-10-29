#!/usr/bin/env python3
"""Freeze the declared G1 suite, run offline audits and render an HTML report.

No USB commands, upload paths, physical key events or automatic retries.
"""
import argparse
import collections
import hashlib
import html
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CATEGORIES = ["compilation_loading", "language_runtime", "numerical", "cas_symbolic",
              "graphics", "interaction", "state_persistence", "errors_stability", "real_programs"]
VERDICTS = ["matched", "mismatched", "unsupported", "not_run", "blocked", "inconclusive", "unstable"]
STATUS = {"VERIFIED": "matched", "FAILED": "mismatched", "PARTIAL": "mismatched",
          "UNVERIFIED": "not_run", "UNSUPPORTED": "unsupported", "BLOCKED": "blocked",
          "INCONCLUSIVE": "inconclusive", "UNSTABLE": "unstable"}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def category(case):
    group = case.get("group", case["id"]).lower()
    if group.startswith(("num", "complex", "matrices")) or case["id"] in ["add", "multiply", "third", "round_half", "literal_half"]:
        return "numerical"
    if group.startswith("cas"):
        return "cas_symbolic"
    if group.startswith("errors"):
        return "errors_stability"
    if group.startswith("settings"):
        return "state_persistence"
    return "language_runtime"


def freeze(destination):
    suites = []
    for path in sorted((ROOT / "tests/compat/cases").glob("*.toml")) + sorted((ROOT / "tests/compat/generated").glob("*.toml")):
        data = tomllib.loads(path.read_text())
        cases = []
        for case in data["case"]:
            cases.append({"id": case["id"], "category": category(case),
                          "case_sha256": sha(json.dumps(case, sort_keys=True, ensure_ascii=False).encode()),
                          "comparison": "exact TYPE and STRING; formatting differences separate; no numerical epsilon"})
        suites.append({"path": str(path.relative_to(ROOT)), "suite": data["suite"],
                       "file_sha256": sha(path.read_bytes()),
                       "stratum": "generated" if "generated" in path.parts else "curated", "cases": cases})
    paths = ["examples/FIREWORKS.hpprgm", "examples/test_draw.hpprgm", "tests/fixtures/tester.hpprgm",
             "tests/fixtures/baseconv/Baseconv_original.hpprgm", "tests/fixtures/baseconv/Baseconv_fixed.hpprgm",
             "tests/fixtures/MultiPrecision.hpprgm", "tests/fixtures/tester_brackets.hpprgm"]
    workflows = ["title, start, animation, Esc", "deterministic full frame", "complete bounded animation",
                 "hex FF, invalid base abc, fractional 7B.8, cancel", "same inputs; derived program reported separately",
                 "read source, check original, exercise its declared exported workflow", "read source, check original, exercise its declared exported workflow"]
    programs = [{"path": p, "source_sha256": sha((ROOT / p).read_bytes()), "workflow": w,
                 "holdout": i >= 5, "transformations": [], "physical_verdict": "not_run"}
                for i, (p, w) in enumerate(zip(paths, workflows))]
    branch = "feat/g1-runtime-parity"
    branch_revision = subprocess.check_output(["git", "rev-parse", branch], cwd=ROOT, text=True).strip()
    branch_paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", branch, "tests/runtime/cases"], cwd=ROOT, text=True).splitlines()
    historical = []
    for p in branch_paths:
        raw = subprocess.check_output(["git", "show", f"{branch}:{p}"], cwd=ROOT)
        case = tomllib.loads(raw.decode())
        historical.append({"path": p, "descriptor_sha256": sha(raw), "id": case["id"],
                           "source_sha256": case.get("source_sha256"), "category": case.get("category"),
                           "physical_verdict": "not_run", "belongs_to_current_tree": (ROOT / p).exists(), "descriptor": case})
    manifest = {"schema_version": 1, "suite_version": "g1-assessment-1", "reference_firmware": 15157,
                "categories": CATEGORIES, "suites": suites, "real_programs": programs,
                "unmeasured_capabilities": [
                    {"id": "environment.home", "category": "language_runtime"},
                    {"id": "environment.cas_assumptions", "category": "cas_symbolic"},
                    {"id": "environment.builtin_apps", "category": "state_persistence"},
                    {"id": "state.globals_repeated_run", "category": "state_persistence"},
                    {"id": "state.save_reopen", "category": "state_persistence"},
                    {"id": "interaction.input_accept_cancel", "category": "interaction"},
                    {"id": "interaction.choose_menu", "category": "interaction"},
                    {"id": "interaction.keys_touch", "category": "interaction"},
                    {"id": "interaction.interrupt_recover", "category": "interaction"},
                    {"id": "graphics.font_clipping_buffers", "category": "graphics"}],
                "historical_runtime_branch": {"revision": branch_revision, "cases": historical},
                "policies": {"values": "TYPE/STRING legacy observations do not prove complete values or side effects",
                             "screens": "full 320x240 frame RGB565; retain raw RGB555 transport; no masks by default",
                             "settings": "read-only snapshot first; empty payload is unknown; match recorded reference settings",
                             "physical": "fresh ZZT program; explicit Send authorization required; no automatic write retry",
                             "cas": "exact, approximate, structure and formatting are separate; record assumptions",
                             "errors": "generic IFERR agreement alone is inconclusive"}}
    destination.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def run_offline(manifest, assessor, out):
    audited = out / "audited"
    audited.mkdir(exist_ok=True)
    for suite in manifest["suites"]:
        path = ROOT / suite["path"]
        if sha(path.read_bytes()) != suite["file_sha256"]:
            raise ValueError(f"suite changed since freeze: {path}")
        command = [str(assessor.resolve()), str(path), str(audited.resolve())]
        if list((out / "evidence" / suite["suite"]).glob("*.json")):
            command += [str((out / "runs").resolve()), str((out / "evidence").resolve())]
        result = subprocess.run(command, cwd=ROOT, capture_output=True, timeout=120)
        (out / (suite["suite"] + ".stdout")).write_bytes(result.stdout)
        (out / (suite["suite"] + ".stderr")).write_bytes(result.stderr)
        if result.returncode:
            raise RuntimeError((command, result.returncode))


def render(manifest, out):
    records = []
    audits = []
    for suite in manifest["suites"]:
        path = out / "audited" / f"assessment-{suite['suite']}.json"
        if not path.exists():
            continue
        data = json.loads(path.read_text())
        audits.extend(dict(suite=suite["suite"], **a) for a in data["audits"])
        descriptors = {c["id"]: c for c in suite["cases"]}
        for case in data["report"]["cases"]:
            obs = data["observations"].get(case["id"], [])
            records.append({"engine": "native", "suite": suite["suite"], "stratum": suite["stratum"],
                            **descriptors[case["id"]], "physical_reference": any(not o.get("disputed", False) for o in obs),
                            "native_compiled": case["emulator"]["kind"] != "compile_error",
                            "native_attempted": True, "verdict": STATUS[case["status"]],
                            "comparison_scope": data["comparison_scope"], "observations": obs,
                            "native": case["emulator"]})
    physical_path = out / "red-comparison.json"
    # Both engines have the same declared probe denominator. An unexecuted
    # firmware case stays not_run even if a physical reference is available.
    records.extend({"id": r["id"], "suite": r["suite"], "stratum": r["stratum"],
                    "category": r["category"], "engine": "firmware",
                    "physical_reference": r["physical_reference"], "engine_attempted": False,
                    "verdict": "not_run"} for r in list(records))
    if physical_path.exists():
        red = json.loads(physical_path.read_text())
        records.append({"id": red["case_id"], "engine": "native", "suite": "red-fixture", "stratum": "interactive",
                        "category": "graphics", "physical_reference": True, "native_attempted": True,
                        "verdict": red["verdict"], "evidence": str(physical_path), "program_completed": False})
        records.append({"id": red["case_id"], "engine": "firmware", "suite": "red-fixture", "stratum": "interactive",
                        "category": "graphics", "physical_reference": True, "engine_attempted": False,
                        "verdict": "not_run", "evidence": str(physical_path)})
    trials_path = out / "reliability/attempts.json"
    real_path = out / "real-programs/results.json"
    real_programs = json.loads(real_path.read_text()) if real_path.exists() else manifest["real_programs"]
    for program in real_programs:
        for engine in ["native", "firmware"]:
            for cat in ["real_programs", "compilation_loading"]:
                records.append({"id": cat + ":" + program["path"], "suite": "real-program-workflows",
                                "stratum": "curated", "category": cat, "engine": engine,
                                "physical_reference": False, "verdict": "not_run",
                                "local_observations": program if engine == "native" else None})
    for capability in manifest.get("unmeasured_capabilities", []):
        for engine in ["native", "firmware"]:
            records.append({**capability, "suite": "environment-and-interaction", "stratum": "curated",
                            "engine": engine, "physical_reference": False, "verdict": "not_run"})
    trials = json.loads(trials_path.read_text()) if trials_path.exists() else []
    for case in ["square", "list", "graphics_input"]:
        attempts = [a for a in trials if a["case"] == case]
        successes = sum(a["status"] == "success" for a in attempts)
        records.append({"id": "reliability." + case, "suite": "fresh-session-reliability", "stratum": "reliability",
                        "engine": "firmware", "category": "errors_stability", "physical_reference": False,
                        "verdict": "unstable" if 0 < successes < len(attempts) else "blocked" if attempts and not successes else "not_run",
                        "attempts": len(attempts), "successful_sessions": successes,
                        "failed_sessions": len(attempts) - successes, "expected_attempts": 10,
                        "functional_physical_verdict": "not_run"})
    table = []
    for engine in ["native", "firmware"]:
        for stratum in ["curated", "generated", "interactive", "reliability"]:
            for cat in CATEGORIES:
                selected = [r for r in records if r["engine"] == engine and r["category"] == cat and r["stratum"] == stratum]
                counts = collections.Counter(r["verdict"] for r in selected)
                table.append({"engine": engine, "stratum": stratum, "category": cat, "declared_and_recorded": len(selected),
                              "valid_physical_reference": sum(r["physical_reference"] for r in selected),
                              **{v: counts[v] for v in VERDICTS}})
    result = {"schema_version": 1, "suite_version": manifest["suite_version"], "records": records,
              "category_counts": table, "audits": audits, "firmware_attempts": trials,
              "real_programs": real_programs,
              "limitations": ["TYPE/STRING agreement is a limited observable, not full semantics",
                              "Generated counts are a separate stratum; no whole-calculator percentage",
                              "Zero category cases means unmeasured, not supported",
                              "Firmware reliability records have no physical reference and do not prove functional parity",
                              "Historical runtime and diagnostic branches are not the current implementation"]}
    pending_path = out / "bootstrap-pending-reference.json"
    result["pending_references"] = [json.loads(pending_path.read_text())] if pending_path.exists() else []
    (out / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    rows = []
    for item in table:
        if item["declared_and_recorded"] or item["stratum"] == "curated":
            rows.append("<tr>" + "".join(f"<td>{html.escape(str(item[k]))}</td>" for k in ["engine", "stratum", "category", "declared_and_recorded", "valid_physical_reference", *VERDICTS]) + "</tr>")
    header = "".join(f"<th>{k}</th>" for k in ["Engine", "Stratum", "Category", "Cases", "Physical reference", *VERDICTS])
    body = """<!doctype html><html lang="en"><meta charset="utf-8"><title>HP Prime G1 compatibility assessment</title>
<style>body{font:16px system-ui;max-width:1500px;margin:2rem auto;padding:1rem;color:#18212b}td,th{border:1px solid #bbb;padding:.4rem;text-align:left}table{border-collapse:collapse;font-size:14px}pre{white-space:pre-wrap}a{color:#005da6}</style>
<h1>HP Prime G1 compatibility assessment</h1>
<p>Reference: physical G1 firmware 15157, boot V0.050.640. Current-tree and historical evidence are kept separate. This is an interim measured assessment; pending cases remain visible.</p>
<p>The red drawing checkpoint matches Native at all 76,800 RGB565 pixels. It does not establish the program's return value or general graphics parity. Legacy arithmetic and string probes expose rounding and formatting disagreements. Original-firmware runs are currently blocked by retained Check-navigation failures before program execution.</p>
<p><a href="results.json">Machine-readable results and per-case observations</a> · <a href="manifest.json">Frozen manifest and program workflows</a> · <a href="discrepancies.json">Discrepancy ledger</a> · <a href="baseline.json">Baseline source identity</a> · <a href="resources.json">Resource hashes</a></p>
<h2>Per engine and category</h2><p>Generated probes and curated probes are identifiable separately in every result record. Empty categories are unmeasured. Not run means no eligible physical comparison, even when the native engine was exercised. Formatting-only PARTIAL results are counted as mismatches and identified in their observations.</p>
<table>""" + "<tr>" + header + "</tr>" + "".join(rows) + "</table>"
    body += "<h2>Limits and practical verdict</h2><ul>" + "".join("<li>" + html.escape(x) + "</li>" for x in result["limitations"]) + "</ul>"
    if result["pending_references"]:
        body += '<h2>Fresh eight-probe capture awaiting transcription confirmation</h2><p>The physical terminal shows run tag 7076ef76 and @@END at settings 0|0|0. Seven observations agree with Native; the halfway literal is 1.23456789012 on the G1 and 1.23456789013 in Native. These remain provisional until the requested user confirmation; they are not added to matched counts yet. Source retrieval found an added newline and space before the opening comment, with the entire executable source unchanged; both hashes are retained in <a href="observed-source-transformation.json">the transformation record</a>.</p><img width="640" height="480" src="evidence/g1_assessment_v1/ZZT_E109E3.screen.decoded.png" alt="Fresh physical G1 terminal capture"><p><a href="evidence/g1_assessment_v1/ZZT_E109E3.screen.raw.png">Raw RGB555 capture</a> · <a href="bootstrap-transcript.txt">Provisional transcript</a> · <a href="first-transfer-verification.json">Independent transfer verification</a></p>'
    body += '<h2>Measured red drawing checkpoint</h2><p>Full native frame versus physical G1: 0 of 76,800 RGB565 pixels differ. No crop or mask. <a href="red-comparison.json">Comparison details</a>.</p><img width="320" height="240" src="red-physical.png" alt="Physical red fixture"><img width="320" height="240" src="red-rgb565-diff.png" alt="Black difference image with zero mismatches">'
    body += "<p>Native is usable for measured deterministic PPL probes and the demonstrated drawing checkpoint; validate real-program workflows individually. CAS is explicitly unsupported by the Native builtin catalogue. Full Home, CAS, app, persistence, interaction and timing parity have not been established. Use the physical G1 when those behaviours matter. The local-firmware engine needs reliable startup/Check evidence before it can be recommended for unattended workflows on this build.</p>"
    body += "<h2>Real programs</h2><p>The native fixed Baseconv fixture completed hex, fractional conversion, invalid-input recovery and cancellation checks. It is a derived source, not proof that the original Baseconv program is compatible. BounceTest and the held-out bracket matrix fixture were also exercised locally. No complete real-program workflow has a physical reference yet.</p><pre>" + html.escape(json.dumps(real_programs, indent=2)) + "</pre>"
    body += "<h2>Local firmware attempts</h2><pre>" + html.escape(json.dumps(trials, indent=2)) + "</pre>"
    (out / "report.html").write_text(body + "</html>\n")
    print("Report", out / "report.html", "records", len(records), "firmware attempts", len(trials), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--assessor", type=Path)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    path = args.out / "manifest.json"
    manifest = freeze(path) if args.freeze else json.loads(path.read_text())
    if args.assessor:
        run_offline(manifest, args.assessor, args.out)
    render(manifest, args.out)


if __name__ == "__main__":
    main()
