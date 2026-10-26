#!/usr/bin/env python3
"""Inspect/replay preserved FC PNG responses offline; never opens a USB device.

Input is the existing human-readable HID exchange log. Output is image metadata
and hashes, never calculator pixels. Optional replay uses Pillow in memory.
Coordinates follow the recovered CK viewer; they are not a completeness marker.
"""

from __future__ import annotations

import argparse
import binascii
import hashlib
import io
import json
import re
import struct
import sys
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from PIL import Image

SCALES = {
    4: 1,
    5: 1,
    6: 2,
    7: 2,
    8: 1,
    9: 1,
    10: 2,
    11: 2,
    12: 3,
    13: 3,
    16: 3,
    17: 3,
    20: 4,
    21: 4,
    22: 4,
    23: 4,
}
LIMIT = 16 * 1024 * 1024


def assemble(text: str) -> tuple[int, bytes, int]:
    """Require one request and complete ordered response; ignore padding."""
    if len(text) > 4 * LIMIT:
        raise ValueError("log too large")
    request = None
    message = bytearray()
    count = 0
    expected = None
    for line in text.splitlines():
        match = re.fullmatch(
            r"\s*[0-9.]+s (host→calc|calc→host)\s+(\d+) B\s+([0-9A-Fa-f ]+)",
            line,
        )
        if not match:
            if line.strip():
                raise ValueError("unrecognized HID log line")
            continue
        direction, declared, hexbytes = match.groups()
        report = bytes.fromhex(hexbytes)
        if len(report) != int(declared) or not report:
            raise ValueError("invalid report length")
        if direction == "host→calc":
            if (
                request is not None
                or len(report) != 4
                or report[:3] != b"\0\0\xfc"
            ):
                raise ValueError("expected exactly one FC request")
            request = report[3]
            continue
        if request is None:
            raise ValueError("response before request")
        if report[0] == 255:
            continue
        if expected is not None and len(message) >= expected:
            raise ValueError("additional report after completed response")
        if report[0] != count % 255:
            raise ValueError("out-of-sequence report")
        count += 1
        message.extend(report[1:])
        if len(message) > LIMIT:
            raise ValueError("response too large")
        if expected is None and len(message) >= 6:
            expected = 6 + int.from_bytes(message[2:6], "big")
            if expected > LIMIT:
                raise ValueError("declared response too large")
    if expected is None or len(message) < expected:
        raise ValueError("incomplete response")
    return request, bytes(message[:expected]), count


def inspect(request: int, message: bytes) -> tuple[dict[str, Any], bytes]:
    """Validate the FC envelope and describe PNG metadata, without pixels."""
    if len(message) < 46 or message[:2] != b"\xfc\x01":
        raise ValueError("expected a version-1 FC PNG response")
    if len(message) != 6 + int.from_bytes(message[2:6], "big"):
        raise ValueError("message length mismatch")
    if message[8] != request or request not in SCALES:
        raise ValueError("unexpected screenshot format")
    # CK computes CRC over format, coordinates and PNG; the existing Rust
    # zero-CRC-field formulation is equivalent because the initial CRC is zero.
    if binascii.crc_hqx(message[8:], 0) != int.from_bytes(message[6:8], "big"):
        raise ValueError("screenshot CRC mismatch")
    png = message[13:]
    if png[:8] != b"\x89PNG\r\n\x1a\n" or png[8:16] != b"\0\0\0\rIHDR":
        raise ValueError("missing PNG IHDR")
    width, height, depth, color = struct.unpack_from(">IIBB", png, 16)
    if not 0 < width <= 320 or not 0 < height <= 240:
        raise ValueError("invalid source dimensions")
    x, y = struct.unpack_from("<HH", message, 9)
    full_origin = (x, y) == (65535, 65535)
    if full_origin:
        x = y = 0
    factor = SCALES[request]
    if (x + width) * factor > 320 or (y + height) * factor > 240:
        raise ValueError("patch outside LCD")
    return dict(
        format=f"{request:02X}",
        origin=[x, y],
        full_origin_sentinel=full_origin,
        scale=factor,
        png_dimensions=[width, height],
        bit_depth=depth,
        color_type=color,
        png_bytes=len(png),
        crc_valid=True,
        response_sha256=hashlib.sha256(message).hexdigest(),
    ), png


def read_response(path: Path) -> tuple[int, bytes, int]:
    """Attach the input path to log-reading and assembly errors."""
    try:
        return assemble(path.read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"{path}: {error}") from error


def decode_patch(metadata: dict[str, Any], png: bytes) -> Image.Image:
    """Match HP's little-endian RGB555 QImage consumer, not PNG luminance."""
    from PIL import Image

    with Image.open(io.BytesIO(png)) as raw:
        raw.load()
        if raw.size != tuple(metadata["png_dimensions"]):
            raise ValueError("decoded dimensions differ from IHDR")
        if metadata["color_type"] == 0 and metadata["bit_depth"] == 16:
            pixels = []
            for value in raw.get_flattened_data():
                # Pillow returns the PNG's BE numeric sample. HP instead copies
                # the two decoded bytes into a little-endian RGB555 word.
                value = ((value & 255) << 8) | (value >> 8)
                red, green, blue = (
                    (value >> 10) & 31,
                    (value >> 5) & 31,
                    value & 31,
                )
                pixels.append(
                    (
                        (red << 3) | (red >> 2),
                        (green << 3) | (green >> 2),
                        (blue << 3) | (blue >> 2),
                    )
                )
            patch = Image.new("RGB", raw.size)
            patch.putdata(pixels)
        else:
            patch = raw.convert("RGB")
    return patch


def replay(paths: Sequence[Path]) -> Iterator[dict[str, Any]]:
    """Compose in argument order; a format switch needs a new baseline."""
    from PIL import Image

    canvas = None
    last_format = None
    for path in paths:
        request, message, reports = read_response(path)
        metadata, png = inspect(request, message)
        factor = metadata["scale"]
        patch = decode_patch(metadata, png)
        if metadata["full_origin_sentinel"]:
            canvas = Image.new("RGB", (320, 240))
            last_format = request
        if canvas is None or request != last_format:
            raise ValueError(
                "patch without a full baseline in the same format"
            )
        patch = patch.resize(
            (patch.width * factor, patch.height * factor),
            Image.Resampling.NEAREST,
        )
        x, y = metadata["origin"]
        before = canvas.tobytes()
        canvas.paste(patch, (x * factor, y * factor))
        after = canvas.tobytes()
        metadata.update(
            log=path.name,
            reports=reports,
            canvas_changed=before != after,
            changed_pixels=sum(
                before[i : i + 3] != after[i : i + 3]
                for i in range(0, len(after), 3)
            ),
            canvas_sha256=hashlib.sha256(after).hexdigest(),
            unfilled_right_columns=320 % factor,
        )
        yield metadata


def inspect_logs(paths: Sequence[Path]) -> list[dict[str, Any]]:
    rows = []
    for path in paths:
        request, message, count = read_response(path)
        row, _ = inspect(request, message)
        row.update(log=path.name, reports=count)
        rows.append(row)
    return rows


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", type=Path)
    parser.add_argument(
        "--replay", action="store_true", help="compose in memory using Pillow"
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        rows = (
            list(replay(args.logs)) if args.replay else inspect_logs(args.logs)
        )
    except (OSError, ValueError, ImportError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(rows, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
