#!/usr/bin/env python3
"""Independent conversion through system Qt5 QImage, never vendor DLLs.

Linux/Itanium C++ ABI only. Reads grayscale16 PNG bytes through Pillow and asks
system Qt to interpret little-endian RGB555, matching HP's viewer.
"""

import argparse
import ctypes as ct
import ctypes.util
import hashlib
import json
import sys
from pathlib import Path


def convert(raw: bytes, width: int, height: int) -> bytes:
    library = ctypes.util.find_library("Qt5Gui")
    if not library:
        raise ValueError("host system Qt5Gui is unavailable")
    qt = ct.CDLL(library)
    construct = qt._ZN6QImageC1EPhiiiNS_6FormatEPFvPvES2_
    construct.argtypes = [
        ct.c_void_p,
        ct.c_void_p,
        ct.c_int,
        ct.c_int,
        ct.c_int,
        ct.c_int,
        ct.c_void_p,
        ct.c_void_p,
    ]
    construct.restype = None
    pixel = qt._ZNK6QImage5pixelEii
    pixel.argtypes = [ct.c_void_p, ct.c_int, ct.c_int]
    pixel.restype = ct.c_uint32
    destroy = qt._ZN6QImageD1Ev
    destroy.argtypes = [ct.c_void_p]
    destroy.restype = None
    if width <= 0 or height <= 0 or len(raw) != width * height * 2:
        raise ValueError("invalid raw RGB555 extent")
    # QImage is 32 bytes in this ABI. Extra aligned storage avoids depending on
    # private internals; only public constructor/pixel/destructor APIs run.
    storage = ct.create_string_buffer(128)
    source = ct.create_string_buffer(raw)
    construct(storage, source, width, height, width * 2, 11, None, None)
    try:
        result = bytearray()
        for y in range(height):
            for x in range(width):
                value = pixel(storage, x, y)
                result.extend(
                    ((value >> 16) & 255, (value >> 8) & 255, value & 255)
                )
        return bytes(result)
    finally:
        destroy(storage)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("raw_png", type=Path)
    parser.add_argument("decoded_png", type=Path)
    args = parser.parse_args()
    try:
        from PIL import Image

        with Image.open(args.raw_png) as source:
            if source.mode not in ("I;16", "I;16B", "I"):
                raise ValueError("expected grayscale16 PNG")
            width, height = source.size
            raw = b"".join(
                v.to_bytes(2, "big") for v in source.get_flattened_data()
            )
        expected = convert(raw, width, height)
        with Image.open(args.decoded_png) as decoded:
            if decoded.size != (width, height):
                raise ValueError("decoded dimensions differ")
            actual = decoded.convert("RGB").tobytes()
        mismatches = sum(
            expected[i : i + 3] != actual[i : i + 3]
            for i in range(0, len(expected), 3)
        )
        print(
            json.dumps(
                dict(
                    oracle="system Qt5 QImage Format_RGB555",
                    pixels=width * height,
                    mismatches=mismatches,
                    rgb_sha256=hashlib.sha256(expected).hexdigest(),
                )
            )
        )
        return int(mismatches != 0)
    except (OSError, ValueError, AttributeError, ImportError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
