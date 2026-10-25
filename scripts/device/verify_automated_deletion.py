#!/usr/bin/env python3
"""Opt-in physical acceptance; deletes only fresh objects this run creates.

Uses the ordinary Rust CLI and its ownership/backup guards. No key injection,
helper execution, firmware operation or guessed USB packet is sent. Refactoring
and automated tests must use mocked subprocesses, never a connected calculator.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import secrets
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class AcceptanceRun:
    """Own one run's evidence and report; never retry a step."""

    binary: str
    directory: Path
    report: dict[str, Any]

    def record(self) -> None:
        (self.directory / "summary.json").write_text(
            json.dumps(self.report, indent=2) + "\n"
        )

    def run(
        self, label: str, command: list[str], typed_name: str | None = None
    ) -> subprocess.CompletedProcess[str]:
        step = {
            "label": label,
            "command": ["ppl", "device", *command],
            "state": "submitting",
        }
        self.report["steps"].append(step)
        self.record()
        result = subprocess.run(
            [self.binary, "device", *command],
            text=True,
            input=(typed_name + "\n") if typed_name else "",
            capture_output=True,
            check=False,
            timeout=600,
        )
        (self.directory / f"{label}.stdout").write_text(result.stdout)
        (self.directory / f"{label}.stderr").write_text(result.stderr)
        step.update(state="returned", exit_code=result.returncode)
        self.record()
        if result.returncode:
            raise RuntimeError(
                f"{label} failed; stopped without retrying. See "
                f"{self.directory}"
            )
        print(f"{label}: passed", flush=True)
        return result

    def delete_and_verify(self, label: str, kind: str, name: str) -> None:
        result = self.run(f"delete-{label}", ["delete", kind, name, "--json"])
        try:
            value = json.loads(result.stdout)
            confirmed = (
                value["deletion_confirmed"] is True
                and value["mechanism"] == "usb_delete"
                and value["helper"] is None
                and value["execution_confirmed"] is False
                and value["error"] is None
            )
            if not confirmed:
                raise ValueError("deletion confirmation fields do not match")
            backup = Path(value["backup_directory"]) / f"{kind}-{name}.bin"
            digest = value["results"][0]["backup_sha256"]
        except (KeyError, IndexError, TypeError, ValueError) as error:
            raise ValueError(
                f"Unexpected deletion result for {name}; evidence retained: "
                f"{error}"
            ) from error
        if hashlib.sha256(backup.read_bytes()).hexdigest() != digest:
            raise RuntimeError("Backup does not match recorded SHA-256")
        # A separate CLI process opens a new HID connection for this query.
        check = self.run(
            f"verify-{label}", ["delete", "verify", kind, name, "--json"]
        )
        value = json.loads(check.stdout)
        if (
            not isinstance(value, dict)
            or value.get("deletion_confirmed") is not True
        ):
            raise RuntimeError(f"Independent absence query failed for {name}")


def run_acceptance(binary: str, output_directory: Path) -> int:
    suffix = secrets.token_hex(3).upper()
    program, note, sentinel = [
        f"ZZT_{suffix}{kind}" for kind in ("P", "N", "S")
    ]
    directory = output_directory / suffix
    directory.mkdir(parents=True, exist_ok=False)
    source = directory / "test.hpprgm"
    source.write_text(f"EXPORT {program}()\nBEGIN\n  RETURN 42;\nEND;\n")
    note_source = directory / "note.txt"
    note_source.write_text(
        f"Disposable deletion acceptance fixture {suffix}.\n"
    )
    run = AcceptanceRun(
        binary,
        directory,
        {
            "status": "UNRESOLVED",
            "program": program,
            "note": note,
            "sentinel": sentinel,
            "steps": [],
            "execution_claimed": False,
        },
    )
    try:
        run.run("send-program", ["send", program, str(source)], program)
        run.run("send-note", ["send", note, str(note_source), "--note"], note)
        run.run(
            "send-sentinel",
            ["send", sentinel, str(note_source), "--note"],
            sentinel,
        )
        before = directory / "sentinel-before.bin"
        after = directory / "sentinel-after.bin"
        run.run(
            "read-sentinel-before",
            ["get", "note", sentinel, "--out", str(before)],
        )
        run.delete_and_verify("program", "program", program)
        run.delete_and_verify("note", "note", note)
        run.run(
            "read-sentinel-after",
            ["get", "note", sentinel, "--out", str(after)],
        )
        if before.read_bytes() != after.read_bytes():
            raise RuntimeError(
                "Sentinel changed; retain all evidence for investigation"
            )
        run.report["sentinel_unchanged"] = True
        run.delete_and_verify("sentinel", "note", sentinel)
        run.report["status"] = "VERIFIED"
        run.record()
        print(
            f"Unattended program/note deletion verified. Evidence: {directory}"
        )
        return 0
    except (
        RuntimeError,
        ValueError,
        OSError,
        subprocess.SubprocessError,
    ) as error:
        run.report["error"] = str(error)
        run.record()
        print(
            f"{error}\n"
            "No automatic retry or speculative cleanup was attempted.",
            file=sys.stderr,
        )
        return 1


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--hardware",
        action="store_true",
        required=True,
        help="Explicitly enable physical object creation/deletion.",
    )
    parser.add_argument("--ppl", default="target/debug/ppl")
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("artifacts/automatic-deletion-tests"),
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        return run_acceptance(args.ppl, args.output_dir)
    except OSError as error:
        print(
            f"error: cannot create acceptance evidence: {error}",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
