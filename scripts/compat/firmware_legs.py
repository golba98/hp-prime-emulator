#!/usr/bin/env python3
"""M15: three-way parity of recorded physical-G1 evidence.

For each recorded hardware run (tests/compat/evidence/<suite>/ZZT_*.json,
print channel), the exact program the G1 ran (its run manifest's `source`,
SHA-256 checked) is executed again on:

  * local firmware builds given with --build NAME=ASSET_DIR (an asset directory
    holds armfir.elf, PRIME_OS.ROM, factory-A/ and initial-data/), and
  * the native Rust engine,

and each case's `TYPE|STRING(v)` record is compared with the physical one.
Nothing is sent to a calculator. The physical column is only ever the
recorded, user-confirmed evidence.

Attribution, per case, when a local build with the device's build number is
given (--device-build):
  * that build matches the device and another build does not → firmware
    version difference;
  * that build differs from the device → emulation (or host) difference.

Usage:
  firmware_legs.py --build 15751=~/.local/share/hp-prime-emulator/firmware \\
                   --build 15157=~/.local/share/hp-prime-emulator/firmware-15157 \\
                   --device-build 15157 --out artifacts/m15/legs
"""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / "tests/compat/evidence"
RUNS = ROOT / "tests/compat/runs"


def ppl_binary():
    meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"],
                          cwd=ROOT, capture_output=True, text=True, check=True)
    return Path(json.loads(meta.stdout)["target_directory"]) / "release/ppl"


def parse_records(lines, short_ids):
    """`cN|TYPE|TEXT` / `cN|E|` lines between the @@ZZT header and @@END."""
    out, header, inside = {}, None, False
    for line in lines:
        if line.startswith("@@ZZT|"):
            header, inside = line, True
            continue
        if line.startswith("@@END"):
            break
        if not inside or "|" not in line:
            continue
        short, rest = line.split("|", 1)
        if short not in short_ids or rest.startswith("SETCHG"):
            continue
        kind, _, text = rest.partition("|")
        out[short_ids[short]] = {"kind": "error"} if kind == "E" else \
            {"kind": "value", "ty": int(kind), "text": text}
    return header, out


def run_leg(ppl, source_path, program, engine, assets=None, timeout=300):
    cmd = [str(ppl), "run", "--engine", engine, "--json", "--print-mode", "terminal", "--quiet",
           "--entry", program]
    if assets:
        a = Path(assets).expanduser()
        cmd += ["--firmware", str(a / "armfir.elf"), "--os-rom", str(a / "PRIME_OS.ROM"),
                "--factory-a", str(a / "factory-A"), "--init-data", str(a / "initial-data"),
                "--program-name", program, "--timeout-s", str(timeout)]
    else:
        cmd += ["--max-time", "60"]
    cmd.append(str(source_path))
    done = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 120)
    try:
        report = json.loads(done.stdout)
    except json.JSONDecodeError:
        return {"status": "no-report", "stderr": done.stderr[-800:]}, []
    lines = [o["text"] for o in report.get("output", []) if o.get("kind") == "print" and o.get("text") is not None]
    return report, lines


def note_program(source, note):
    """The print-channel program with each `PRINT(x);` appending `x` and a
    newline to a string that is stored in the note `note` before the final
    RETURN; `PRINT();` (clear) is dropped. Case functions and records are
    unchanged. Balanced parentheses, ignoring string contents."""
    out, i = [], 0
    while True:
        j = source.find("PRINT(", i)
        if j < 0:
            out.append(source[i:])
            break
        out.append(source[i:j])
        k, depth, in_str = j + len("PRINT("), 1, False
        while depth:
            ch = source[k]
            if ch == '"':
                in_str = not in_str
            elif not in_str:
                depth += ch == "("
                depth -= ch == ")"
            k += 1
        arg = source[j + len("PRINT("):k - 1]
        assert source[k] == ";", source[j:k + 1]
        out.append(f"zr:=zr+{arg}+CHAR(10);" if arg.strip() else "")
        i = k + 1
    text = "".join(out)
    head = "LOCAL v, ok, h0, f0, d0;"
    assert head in text
    text = text.replace(head, head + '\n  LOCAL zr:="";', 1)
    tail = text.rindex("RETURN 0;")
    return text[:tail] + f'Notes("{note}"):=zr;\n  ' + text[tail:]


# Device key codes (prime-emulator key-script names): letters are Alpha +
# their ASCII code; lowercase at Home is Alpha Shift (M7); `_` is Shift Space.
def typed(name):
    keys = []
    for ch in name:
        if ch.isdigit():
            keys.append(ch)
        elif ch == "_":
            keys += ["SHIFT", "0x20"]
        elif ch.isupper():
            keys += ["ALPHA", f"0x{ord(ch):02X}"]
        else:
            keys += ["ALPHA", "SHIFT", f"0x{ord(ch.upper()):02X}"]
    return keys


def run_catalog_leg(work, program, source, assets, hdigits, timeout=600, note=None):
    """Runs the program the way a user would, with no version-pinned hooks:
    imported before boot, compiled by the Program Catalog editor's Check,
    run by name at Home, then OFF (which stores the note). The firmware's own
    note file is the result. Works on any firmware build."""
    import shutil
    a = Path(assets).expanduser()
    pe = ppl_binary().with_name("prime-emulator")
    data = work / "data"
    shutil.rmtree(work, ignore_errors=True)
    data.mkdir(parents=True)
    for drive in ("A", "C"):
        shutil.copytree(a / "initial-data" / drive, data / drive)
    # A note-channel program runs unchanged and is read from its own note.
    if note is None:
        note = "ZZTR" + program[4:]
        source = note_program(source, note)
    src = work / f"{program}.ppl"
    src.write_text(source, encoding="utf-8")
    subprocess.run([str(pe), "import", "--program", str(src), "--data-dir", str(data)],
                   check=True, capture_output=True)
    keys = []
    if hdigits is not None:  # the device's recorded HDigits (Sto ▸ HDigits)
        keys += [str(hdigits), "TAP:25:229"] + typed("HDigits") + ["ENTER"]
    keys += ["SHIFT", "1", "DELAY:1500", "TAP:100:51", "DELAY:800", "TAP:293:229", "DELAY:1500",
             "ENTER", "HOME"] + typed(program) + ["ENTER", "DELAY:8000", "SHIFT", "ON", "DELAY:3000"]
    subprocess.run([str(pe), "run", "--firmware", str(a / "armfir.elf"), "--os-rom", str(a / "PRIME_OS.ROM"),
                    "--data-dir", str(data), "--on-unsupported", "zero", "--max-instructions", "100000000000",
                    "--max-seconds", str(timeout), "--keys", ",".join(keys), "--report", str(work / "run.json"),
                    "--screenshot-dir", str(work / "shots"), "-v", "0"],
                   capture_output=True, timeout=timeout + 60)
    path = data / "C" / "DATA" / f"{note}.hpnote"
    if not path.exists():
        return {"status": "no-note"}, []
    text = path.read_bytes().decode("utf-16-le").rstrip("\0")
    return {"status": "note"}, text.split("\n")


def normalise_display(t):
    """ppl-compat's documented display differences (evidence.rs): a match
    needing these is PARTIAL, never VERIFIED."""
    return t.strip().replace("\u1d07", "E").replace("\u2212", "-")


def transcribed(t, ty):
    """Conventions of the screenshot transcriptions in the evidence: the
    firmware's private-use imaginary unit U+E003 written as `i`, and a string
    result's surrounding quotes left out. Never VERIFIED or PARTIAL."""
    t = normalise_display(t).replace("\ue003", "i")
    if ty == 2 and len(t) >= 2 and t[0] == t[-1] == '"':
        t = t[1:-1]
    return t


def status(local, physical):
    if local is None:
        return "MISSING"
    if same(local, physical):
        return "VERIFIED"
    if local["kind"] == physical["kind"] == "value" and local["ty"] == physical["ty"]:
        if normalise_display(local["text"]) == normalise_display(physical["text"]):
            return "PARTIAL"
        if transcribed(local["text"], local["ty"]) == transcribed(physical["text"], physical["ty"]):
            return "TRANSCRIPTION"
    return "MISMATCH"


def same(a, b):
    if a is None or b is None:
        return False
    if a["kind"] != b["kind"]:
        return False
    return a["kind"] == "error" or (a["ty"] == b["ty"] and a["text"] == b["text"])


def show(o):
    if o is None:
        return "—"
    return "error" if o["kind"] == "error" else f'{o["text"]} (type {o["ty"]})'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", action="append", default=[], help="NAME=ASSET_DIR (repeatable)")
    ap.add_argument("--device-build", default=None, help="the local build matching the device")
    ap.add_argument("--out", required=True)
    ap.add_argument("--runner", choices=["catalog", "observer"], default="catalog",
                    help="catalog: as a user runs it, any build (default); observer: `ppl run`, 15751 only")
    ap.add_argument("--suites", default=None, help="comma-separated suites (default: all print-channel evidence)")
    args = ap.parse_args()
    builds = dict(b.split("=", 1) for b in args.build)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    ppl = ppl_binary()
    rows, runs = [], []
    for ev_path in sorted(EVIDENCE.glob("*/ZZT_*.json")):
        ev = json.loads(ev_path.read_text())
        if args.suites and ev["suite"] not in args.suites.split(","):
            continue
        manifest = json.loads((RUNS / f'{ev["program"]}.json').read_text())
        note = manifest.get("note") if manifest["channel"] == "note" else None
        if manifest["channel"] not in ("print", "note"):
            continue
        source = manifest["source"]
        sha = hashlib.sha256(source.encode()).hexdigest()
        assert sha == manifest["sha256"] == ev["program_sha256"], ev_path
        program = ev["program"]
        src = out / "programs" / f"{program}.ppl"
        src.parent.mkdir(exist_ok=True)
        src.write_text(source, encoding="utf-8")
        short_ids = dict(manifest["short_ids"])
        legs = {}
        hdigits = int(ev["settings"]["HDigits"]) if "HDigits" in ev.get("settings", {}) else None
        for name, assets in builds.items():
            if args.runner == "catalog":
                report, lines = run_catalog_leg(out / "work" / f"{program}.{name}", program, source,
                                                assets, hdigits, note=note)
            else:
                report, lines = run_leg(ppl, src, program, "firmware", assets)
            header, results = parse_records(lines, short_ids)
            legs[name] = {"status": report.get("status"), "header": header, "results": results}
            (out / "raw").mkdir(exist_ok=True)
            (out / "raw" / f"{program}.{name}.json").write_text(json.dumps({"report": report, "lines": lines}, indent=1))
        if note is None:
            report, lines = run_leg(ppl, src, program, "native")
        else:
            report, lines = {"status": "n/a: note channel (the native leg reads PRINT output)"}, []
        header, results = parse_records(lines, short_ids)
        legs["native"] = {"status": report.get("status"), "header": header, "results": results}
        (out / "raw" / f"{program}.native.json").write_text(json.dumps(report, indent=1))
        runs.append({"program": program, "suite": ev["suite"], "sha256": sha,
                     "device_header": ev["raw_text"].splitlines()[0],
                     "legs": {k: {"status": v["status"], "header": v["header"]} for k, v in legs.items()}})
        for case_id, rec in sorted(ev["results"].items()):
            physical = rec["outcome"]
            row = {"suite": ev["suite"], "program": program, "case": case_id, "physical": physical}
            for name, leg in legs.items():
                row[name] = leg["results"].get(case_id)
            verdicts = {name: status(row[name], physical) for name in legs}
            row["verdicts"] = verdicts
            dev = args.device_build
            if dev and dev in verdicts:
                others = [n for n in builds if n != dev]
                if verdicts[dev] in ("MISMATCH", "MISSING"):
                    row["attribution"] = "EMULATION: the device's own build differs locally"
                elif any(verdicts[o] in ("MISMATCH", "MISSING") for o in others):
                    row["attribution"] = "FIRMWARE VERSION: the device's build matches, another build differs"
                else:
                    row["attribution"] = ""
            rows.append(row)
    (out / "matrix.json").write_text(json.dumps({"runs": runs, "cases": rows}, indent=1))
    names = list(builds) + ["native"]
    md = ["| suite | case | physical G1 | " + " | ".join(names) + " | attribution |",
          "|---|---|---|" + "---|" * len(names) + "---|"]
    for r in rows:
        md.append(f'| {r["suite"]} | `{r["case"]}` | {show(r["physical"])} | '
                  + " | ".join(f'{r["verdicts"][n]}: {show(r[n])}' for n in names)
                  + f' | {r.get("attribution", "")} |')
    import collections
    md.append("")
    for n in names:
        c = collections.Counter(r["verdicts"][n] for r in rows)
        md.append(f"{n}: " + ", ".join(f"{k} {c[k]}" for k in
                                         ("VERIFIED", "PARTIAL", "TRANSCRIPTION", "MISMATCH", "MISSING") if c[k])
                  + f" (of {len(rows)})")
    (out / "matrix.md").write_text("\n".join(md) + "\n")
    print("\n".join(md[-len(names):]))


if __name__ == "__main__":
    sys.exit(main())
