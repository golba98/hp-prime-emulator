#!/usr/bin/env python3
"""Bounded local firmware trials. Never opens USB or invokes device commands."""
import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run(args):
    args.out.mkdir(parents=True, exist_ok=False)
    identity = {}
    for name in ("binary", "firmware", "os_rom"):
        path = getattr(args, name).resolve()
        identity[name] = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    (args.out / "identity.json").write_text(json.dumps(identity, indent=2))
    cases = [
        ("square", "crates/prime-emulator/tests/fixtures/m7/SQUARE.ppl", "SQUARE", "[7]", None),
        ("list", "crates/prime-emulator/tests/fixtures/m7/LISTRET.ppl", "LISTRET", "[]", None),
        ("graphics_input", "tests/compat/programs/g1_graphics_input.ppl", "G1GFX", "[]", "ENTER"),
    ]
    records = []
    for attempt in range(1, args.attempts + 1):
        for case, source, entry, values, keys in cases:
            dest = args.out / f"{case}-{attempt:02}"
            dest.mkdir()
            source_path = ROOT / source
            source_bytes = source_path.read_bytes()
            (dest / source_path.name).write_bytes(source_bytes)
            command = [str(args.binary.resolve()), "run", str(source_path),
                       "--engine", "firmware", "--entry", entry, "--args", values,
                       "--firmware", str(args.firmware.resolve()), "--os-rom", str(args.os_rom.resolve()),
                       "--factory-a", str(args.factory_a.resolve()), "--init-data", str(args.init_data.resolve()),
                       "--artifacts-dir", str((dest / "work").resolve()),
                       "--png", str((dest / "return.png").resolve()), "--max-time", "3",
                       "--timeout-s", "90", "--json"]
            if keys:
                command += ["--runtime-keys", keys]
            (dest / "command.json").write_text(json.dumps(command, indent=2))
            start = time.monotonic()
            try:
                result = subprocess.run(command, cwd=ROOT, capture_output=True, timeout=100)
                stdout, stderr, code = result.stdout, result.stderr, result.returncode
            except subprocess.TimeoutExpired as error:
                stdout, stderr, code = error.stdout or b"", error.stderr or b"", 124
            (dest / "stdout.json").write_bytes(stdout)
            (dest / "stderr.txt").write_bytes(stderr)
            try:
                report = json.loads(stdout)
            except (ValueError, UnicodeDecodeError):
                report = {}
            record = {"case": case, "attempt": attempt, "exit_code": code,
                      "status": report.get("status", "outer_timeout" if code == 124 else "undecodable"),
                      "source_sha256": hashlib.sha256(source_bytes).hexdigest(),
                      "return_value": report.get("return_value"), "error": report.get("error"),
                      "seconds": round(time.monotonic() - start, 3), "evidence": str(dest)}
            records.append(record)
            (args.out / "attempts.json").write_text(json.dumps(records, indent=2))
            print(json.dumps(record), flush=True)
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("binary", "firmware", "os-rom", "factory-a", "init-data", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--attempts", type=int, choices=range(1, 11), default=10)
    run(parser.parse_args())


if __name__ == "__main__":
    main()
