#!/usr/bin/env python3
"""Check g1_pixel_geometry.ppl's full 320x240 image, including untouched pixels.

Accepts Native, local-firmware, or manually captured device PNGs. Passing this
checker alone never establishes physical provenance. No device I/O is performed.
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/ide_xvfb_smoke"))
from assert_m12 import png


def expected(x, y):
    if 10 <= x <= 29 and 20 <= y <= 39:
        return (255, 0, 0)
    if 50 <= x <= 70 and y == 60:
        return (0, 0, 255)
    if (x, y) == (100, 100):
        return (0, 255, 0)
    return (255, 255, 255)


def check(image):
    if len(image) != 240 or any(len(row) != 320 for row in image):
        raise ValueError("expected an unscaled 320x240 calculator capture")
    mismatches = [(x, y) for y, row in enumerate(image)
                  for x, actual in enumerate(row) if actual != expected(x, y)]
    return {"pixels_checked": 76800, "mismatches": len(mismatches),
            "first_mismatches": mismatches[:16], "passed": not mismatches}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    result = check(png(args.capture))
    result.update(capture_sha256=hashlib.sha256(args.capture.read_bytes()).hexdigest(),
                  program_sha256=hashlib.sha256((ROOT / "tests/compat/programs/g1_pixel_geometry.ppl").read_bytes()).hexdigest(),
                  evidence="image geometry only; device provenance must be recorded separately")
    print(json.dumps(result, indent=2))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
