#!/usr/bin/env python3
"""Reproduce RGB555, scheduling, virtual-screen and Windows raster evidence.

Input is the ignored root produced by official_archives.py (ck32/ck64/vc64).
All four hashes are checked before creating output. Never executes vendor code.
"""

import argparse
import contextlib
import hashlib
import json
import sys
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.research.ck_inspect import PE

MODULES = {
    "ck32": (
        "ck32",
        "ConnectivityKit.exe",
        "a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb",
        [
            (0x42D530, 0x42D620),
            (0x42CA60, 0x42CAD0),
            (0x42D8E0, 0x42DD08),
            (0x42DDC0, 0x42E230),
            (0x58AFC0, 0x58B147),
            (0x5A1640, 0x5A1686),
            (0x4628F0, 0x462930),
            (0x45C9A2, 0x45C9F0),
        ],
    ),
    "ck64": (
        "ck64",
        "ConnectivityKit.exe",
        "477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1",
        [(0x140032490, 0x140032590)],
    ),
    "vc64": (
        "vc64",
        "HPPrime.exe",
        "17b26c0eda9dba07a35383e786c0c9717e196e41f60b0dfefa8f52c7e87742a4",
        [
            (0x140012620, 0x140012820),
            (0x1400136F0, 0x1400137D8),
            (0x140015250, 0x1400152D0),
            (0x140015370, 0x140015490),
            (0x140015DAB, 0x140015E4E),
            (0x140016470, 0x1400165CC),
            (0x140029AB0, 0x140029B40),
        ],
    ),
    "qwindows": (
        "ck64",
        "qwindows.dll",
        "221713c6be9d31a8c6cc06b79240423f6492dace0620554296d976ba3ff38101",
        [
            (0x180062100, 0x180062340),
            (0x180062340, 0x180062910),
            (0x18000D090, 0x18000D110),
        ],
    ),
}


def write(root: Path, output: Path):
    images = {}
    for key, (bundle, filename, expected, _) in MODULES.items():
        manifest = json.loads((root / bundle / "manifest.json").read_text())
        matches = [
            p for p in manifest["applications"] if Path(p).name == filename
        ]
        if len(matches) != 1:
            raise ValueError(f"expected exactly one {filename}")
        image = PE(root / bundle / matches[0])
        if hashlib.sha256(image.data).hexdigest() != expected:
            raise ValueError(f"{key} hash differs from investigated binary")
        images[key] = image
    output.mkdir(parents=True, exist_ok=False)
    record = {
        "evidence_kind": "static; vendor binaries never executed",
        "modules": {},
    }
    for key, image in images.items():
        ranges = MODULES[key][3]
        file = output / f"{key}.asm"
        with file.open("w") as stream, contextlib.redirect_stdout(stream):
            print(
                f"SHA256 {MODULES[key][2]}; "
                f"image base {image.base:#x}; preferred VAs."
            )
            print(
                "Bounded instruction evidence; embedded data/padding is not "
                "reachable merely because disassembled."
            )
            for start, end in ranges:
                print(
                    f"\nRANGE VA={start:#x}:{end:#x} "
                    f"RVA={start - image.base:#x} "
                    f"file={image.offset(start):#x}"
                )
                image.disasm(start, end)
        record["modules"][key] = dict(
            sha256=MODULES[key][2],
            image_base=hex(image.base),
            bits=image.bits,
            evidence_file=file.name,
            evidence_sha256=hashlib.sha256(file.read_bytes()).hexdigest(),
            ranges=[
                dict(
                    va=hex(a),
                    end_va=hex(b),
                    rva=hex(a - image.base),
                    file_offset=hex(image.offset(a)),
                )
                for a, b in ranges
            ],
        )
    file = output / "virtual-metaobjects.txt"
    with file.open("w") as stream, contextlib.redirect_stdout(stream):
        for address in (0x141039AC8, 0x14103A770, 0x14103A7A0, 0x14103A7D0):
            images["vc64"].qt(address)
    record["virtual_metadata_sha256"] = hashlib.sha256(
        file.read_bytes()
    ).hexdigest()
    (output / "manifest.json").write_text(json.dumps(record, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("extracted_root", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        write(args.extracted_root, args.output)
    except (OSError, ValueError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"Wrote hash-pinned evidence to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
