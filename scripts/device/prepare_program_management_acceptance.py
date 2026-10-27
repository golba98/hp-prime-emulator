#!/usr/bin/env python3
"""Prepare LOCAL acceptance fixtures. Never opens USB or invokes calculator tools."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=Path("artifacts/program-management/acceptance"))
    args = parser.parse_args()
    sources = {
        "ZZT_PM_SM01.hpprgm": "EXPORT ZZT_PM_SM01()\nBEGIN\n RETURN 1;\nEND;\n",
        "ZZT_PM_SM01-new.hpprgm": "EXPORT ZZT_PM_SM01()\nBEGIN\n RETURN 2;\nEND;\n",
        "ZZT_PM_SN01.hpprgm": "EXPORT ZZT_PM_SN01()\nBEGIN\n RETURN 314159;\nEND;\n",
    }
    large = "EXPORT ZZT_PM_LG01()\nBEGIN\n RETURN 1;\nEND;\n//"
    sources["ZZT_PM_LG01.hpprgm"] = large + " " * (9938 - len(large))
    args.out_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    manifest = {"physical_operations_performed": False, "large_transfer_supported": False, "fixtures": []}
    for name, source in sources.items():
        path = args.out_dir / name
        with path.open("xb") as file:
            file.write(source.encode("utf-8"))
        manifest["fixtures"].append({"file": name, "file_sha256": hashlib.sha256(source.encode()).hexdigest(),
                                    "payload_bytes": len(source.encode("utf-16-le"))})
    with (args.out_dir / "manifest.json").open("x") as file:
        json.dump(manifest, file, indent=2)
        file.write("\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
