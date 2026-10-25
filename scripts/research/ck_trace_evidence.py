#!/usr/bin/env python3
"""Reproduce the focused Connectivity Kit trace without executing vendor code.

The output directory must not exist. The pinned input hash prevents mixing
addresses from another build. Generated evidence contains no physical USB data.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.research.ck_inspect import PE

SHA256 = "a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb"
RANGES: dict[str, list[tuple[int, int]]] = {
    "hid-enumeration": [
        (0x414300, 0x414620),
        (0x414680, 0x414820),
        (0x414B30, 0x414C15),
    ],
    "hid-os-io": [(0x414820, 0x414B22), (0x4A2CFE, 0x4A2D1C)],
    "hid-client-construction": [
        (0x4C3E40, 0x4C406C),
        (0x4C6440, 0x4C64C0),
        (0x4D1560, 0x4D15B0),
    ],
    "hid-worker": [(0x4C5270, 0x4C6440)],
    "hid-assembly": [(0x4C6D60, 0x4C73F0), (0x4CB930, 0x4CBC92)],
    "hid-receive": [(0x4C9B00, 0x4CA858)],
    "service-discovery": [(0x4D02C0, 0x4D0800)],
    "screen-scheduling": [
        (0x4C66A0, 0x4C6700),
        (0x4C7580, 0x4C7746),
        (0x4CCE00, 0x4CCEB1),
        (0x4CD140, 0x4CD200),
        (0x4CCF00, 0x4CD030),
    ],
    "message-queue": [
        (0x42C220, 0x42C328),
        (0x4648C0, 0x4648F0),
        (0x4CB220, 0x4CB69C),
        (0x4C7410, 0x4C7525),
    ],
    "capture-builder": [
        (0x450320, 0x4503A5),
        (0x431BD0, 0x431E33),
        (0x4289E0, 0x4289F6),
        (0x428B90, 0x428BD6),
        (0x4398B1, 0x4398E0),
    ],
    "project-ui": [
        (0x42A7C0, 0x42A980),
        (0x42AEE0, 0x42AFA0),
        (0x42CCF0, 0x42CECB),
        (0x42D090, 0x42D22A),
        (0x42D2B0, 0x42D4E0),
        (0x452410, 0x452431),
    ],
    "screen-ui-decode": [
        (0x44F7A0, 0x44F92A),
        (0x431230, 0x43132E),
        (0x42D660, 0x42D680),
        (0x42D8E0, 0x42DDAF),
        (0x42DDC0, 0x42E230),
    ],
    "png-decode": [(0x58AFC0, 0x58B147)],
    "screen-producer": [(0x5A12F0, 0x5A1B18), (0x59C290, 0x59C516)],
    "ui-dispatch": [
        (0x462000, 0x462045),
        (0x44EF70, 0x44EFF9),
        (0x44E7C0, 0x44E839),
        (0x44F110, 0x44F278),
    ],
    "ec-incoming": [
        (0x59E050, 0x59E3E0),
        (0x4BBAC0, 0x4BBAC9),
        (0x4B9F88, 0x4BA190),
    ],
    "ec-consumer": [
        (0x59C5B0, 0x59D2B0),
        (0x553A20, 0x553AB8),
        (0x474408, 0x4744CC),
        (0x41ABC0, 0x41ABD0),
    ],
    "ec-routing": [(0x59FB10, 0x59FC47), (0x59F960, 0x59F9C3)],
    "inventory": [
        (0x4315B0, 0x43161F),
        (0x4503B0, 0x4503C6),
        (0x450460, 0x4504D9),
        (0x42FF60, 0x42FF8B),
        (0x44FC07, 0x44FC50),
        (0x59DEC0, 0x59DF67),
    ],
    "event-filter": [(0x46B638, 0x46B672)],
    "other-os-io": [
        (0x4A2D60, 0x4A2DB0),
        (0x4A2DF0, 0x4A2FC3),
        (0x4A3070, 0x4A3221),
        (0x4A32A0, 0x4A32F9),
        (0x419080, 0x4191E0),
        (0x419300, 0x41941D),
    ],
}


def write_disassembly(pe: PE, output: Path, digest: str) -> dict[str, Any]:
    """Write pinned ranges in their original order, retaining provenance."""
    files = {}
    for name, ranges in RANGES.items():
        path = output / f"{name}.asm"
        with path.open("w") as out, contextlib.redirect_stdout(out):
            print(
                f"Binary SHA256 {digest}; PE32 base {pe.base:#x}; all "
                f"addresses preferred VAs."
            )
            print(
                "Ranges may include data/padding: disassembly alone does not "
                "establish reachability."
            )
            for start, end in ranges:
                print(
                    f"\nRANGE VA={start:#x}:{end:#x} RVA={start - pe.base:#x} "
                    f"file={pe.offset(start):#x}"
                )
                pe.disasm(start, end)
        files[path.name] = dict(
            sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
            ranges=[
                dict(
                    va=hex(start),
                    end_va=hex(end),
                    rva=hex(start - pe.base),
                    file_offset=hex(pe.offset(start)),
                )
                for start, end in ranges
            ],
        )
    return files


def write_tables(pe: PE, output: Path) -> dict[str, str]:
    path = output / "tables.txt"
    with path.open("w") as out, contextlib.redirect_stdout(out):
        pe.rtti(
            r"^\.\?AV(CIOObjectClientHID|CIOCIOObjectClientHID_G2|"
            r"CusbHid_win|CCalc|CConKitApp)@@"
        )
        for address in (0xEE9414, 0xEE92EC, 0xEE8F38):
            pe.words(address, 45)
        pe.words(0xEEF7BC, 28)
        for address in (
            0x111EBE4,
            0xF17908,
            0x111EF78,
            0x111C0B4,
            0x111C690,
            0x111C720,
        ):
            pe.qt(address)
        for address, length in (
            (0xF5E300, 20),
            (0x59D288, 15),
            (0x42DAC8, 20),
        ):
            offset = pe.offset(address)
            raw = pe.data[offset : offset + length].hex(" ")
            print(f"raw bytes at {address:#x}: {raw}")
        pe.words(0x59D26C, 7)
        pe.words(0x42DAA4, 9)
        pe.words(0x44F248, 12)
        if pe.imports:
            print("\nAPI IAT slots")
        for va, label in pe.imports.items():
            if any(
                name in label
                for name in (
                    "Hid",
                    "SetupDi",
                    "ReadFile",
                    "WriteFile",
                    "CreateFile",
                    "Overlapped",
                    "CancelIo",
                    "DeviceIoControl",
                )
            ):
                print(hex(va), label)
    return dict(sha256=hashlib.sha256(path.read_bytes()).hexdigest())


def write_evidence(pe: PE, output: Path) -> None:
    digest = hashlib.sha256(pe.data).hexdigest()
    if digest != SHA256:
        raise ValueError(
            "input hash differs from the investigated PE32 binary"
        )
    output.mkdir(parents=True, exist_ok=False)
    manifest = dict(
        binary=str(pe.path),
        sha256=digest,
        image_base=hex(pe.base),
        version="2.4.2 r15751 (PE UTF-16 resource value)",
        bits=pe.bits,
        machine=hex(pe.machine),
        sections=pe.sections,
        evidence_kind="static; vendor binary not executed",
        files={},
    )
    manifest["files"] = write_disassembly(pe, output, digest)
    manifest["files"]["tables.txt"] = write_tables(pe, output)
    (output / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    print(
        f"Wrote {len(manifest['files'])} focused evidence files and manifest "
        f"to {output}"
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        pe = PE(args.binary)
        if hashlib.sha256(pe.data).hexdigest() != SHA256:
            parser.error(
                "input hash differs from the investigated PE32 binary"
            )
        write_evidence(pe, args.output)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
