#!/usr/bin/env python3
"""Extract HP Burn/CAB/MSI bundles as data, never executable code.

Requires 7z. Use a fresh ignored output directory. Manifests contain hashes,
PE metadata and archive-validation results, not proprietary binary contents.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import subprocess
import sys
from pathlib import Path, PurePosixPath

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.research.ck_inspect import PE

LIMIT = 512 * 1024 * 1024


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cabinets(data: bytes):
    """Locate bounded embedded cabinets; 7z then checks their integrity."""
    cursor = 0
    while (cursor := data.find(b"MSCF\0\0\0\0", cursor)) >= 0:
        if cursor + 36 > len(data):
            raise ValueError("truncated cabinet header")
        size = struct.unpack_from("<I", data, cursor + 8)[0]
        if size < 36 or size > LIMIT or cursor + size > len(data):
            raise ValueError(
                "cabinet extends beyond installer or extraction limit"
            )
        yield cursor, data[cursor : cursor + size]
        cursor += size


def run7z(*args: str) -> str:
    result = subprocess.run(
        ["7z", *args], capture_output=True, text=True, timeout=120
    )
    if result.returncode:
        raise ValueError(
            f"7z {args[0]} failed: {result.stderr.strip()} "
            f"{result.stdout[-1000:]}"
        )
    return result.stdout


def check_listing(listing: str) -> None:
    entries = listing.split("----------\n", 1)
    if len(entries) != 2:
        raise ValueError("missing archive entry listing")
    total = 0
    count = 0
    for block in entries[1].split("\n\n"):
        fields = dict(re.findall(r"^([^=\n]+?) = (.*)$", block, re.M))
        if "Path" not in fields:
            continue
        name = fields["Path"].replace("\\", "/")
        path = PurePosixPath(name)
        if not name or path.is_absolute() or ".." in path.parts or ":" in name:
            raise ValueError("unsafe archive path")
        if fields.get("Symbolic Link") or fields.get("Hard Link"):
            raise ValueError("archive links are unsupported")
        total += int(fields.get("Size", "0"))
        count += 1
        if total > LIMIT or count > 10000:
            raise ValueError("archive exceeds extraction limit")


def extract_archive(archive: Path, output: Path) -> None:
    check_listing(run7z("l", "-slt", str(archive)))
    run7z("t", str(archive))
    output.mkdir()
    run7z("x", "-y", f"-o{output}", str(archive))


def pe_metadata(path: Path) -> dict:
    pe = PE(path)
    versions = []
    for section in pe.sections:
        if section["name"] != ".rsrc":
            continue
        raw = pe.data[section["offset"] : section["offset"] + section["size"]]
        for match in re.finditer(b"\xbd\x04\xef\xfe", raw):
            if match.start() + 52 <= len(raw):
                fields = struct.unpack_from("<13I", raw, match.start())
                if fields[1] == 0x10000:
                    versions.append(
                        ".".join(
                            str(v)
                            for v in (
                                fields[2] >> 16,
                                fields[2] & 65535,
                                fields[3] >> 16,
                                fields[3] & 65535,
                            )
                        )
                    )
    return dict(
        sha256=sha(pe.data),
        bytes=len(pe.data),
        bits=pe.bits,
        image_base=hex(pe.base),
        machine=hex(pe.machine),
        fixed_file_versions=versions,
        imports=sorted(set(pe.imports.values())),
    )


def extract_bundle(installer: Path, output: Path) -> dict:
    before = installer.stat()
    if before.st_size > LIMIT:
        raise ValueError("installer exceeds extraction limit")
    data = installer.read_bytes()
    after = installer.stat()
    if (before.st_size, before.st_mtime_ns) != (
        after.st_size,
        after.st_mtime_ns,
    ):
        raise ValueError("installer changed while reading")
    # Check the PE envelope first. Signature trust is separate.
    original = pe_metadata(installer)
    cabs = list(cabinets(data))
    if not cabs:
        raise ValueError("no embedded cabinets")
    output.mkdir(parents=True, exist_ok=False)
    manifest = dict(
        schema_version=1,
        installer=installer.name,
        **original,
        evidence_kind="offline PE/archive inspection; no vendor execution",
        signature_trust="not verified; check Authenticode in isolated Windows",
        cabinets=[],
        applications={},
    )
    for index, (offset, content) in enumerate(cabs):
        cab = output / f"bundle-{index}.cab"
        cab.write_bytes(content)
        unpacked = output / f"bundle-{index}"
        extract_archive(cab, unpacked)
        record = dict(
            offset=offset,
            bytes=len(content),
            sha256=sha(content),
            validation="7z t passed",
            msi=[],
        )
        for member in sorted(unpacked.iterdir()):
            if not member.is_file():
                continue
            with member.open("rb") as source:
                magic = source.read(8)
            if magic != bytes.fromhex("d0cf11e0a1b11ae1"):
                continue
            msi_output = output / f"bundle-{index}-{member.name}-msi"
            extract_archive(member, msi_output)
            msi_record = dict(
                member=member.name,
                sha256=sha(member.read_bytes()),
                cabinets=[],
            )
            for embedded in sorted(msi_output.glob("*.cab")):
                app = output / f"bundle-{index}-{member.name}-{embedded.stem}"
                extract_archive(embedded, app)
                msi_record["cabinets"].append(
                    dict(
                        member=embedded.name,
                        sha256=sha(embedded.read_bytes()),
                        validation="7z t passed",
                    )
                )
                for component in sorted(app.rglob("*")):
                    if component.is_file() and component.suffix.lower() in (
                        ".exe",
                        ".dll",
                    ):
                        manifest["applications"][
                            str(component.relative_to(output))
                        ] = pe_metadata(component)
            record["msi"].append(msi_record)
        manifest["cabinets"].append(record)
    (output / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("installer", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        report = extract_bundle(args.installer, args.output)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(
        f"Validated {report['installer']}: "
        f"{len(report['applications'])} PE components; "
        f"{args.output / 'manifest.json'}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
