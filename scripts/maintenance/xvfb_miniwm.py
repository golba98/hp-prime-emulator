#!/usr/bin/env python3
"""Minimal X11 window manager for Xvfb screenshots.

Usage: xvfb_miniwm.py <display> <width> <height> [duration_secs]

Mapped windows are placed at (0, 0). Small override-redirect windows are also
resized by a periodic sweep because they do not generate MapRequest events.
This is an offline smoke-test utility, not a general-purpose window manager.
"""

from __future__ import annotations

import argparse
import ctypes
import math
import sys
import time
from collections.abc import Sequence
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.lib.x11 import Display, XEvent

SUBSTRUCTURE_REDIRECT_MASK = 1 << 20
SUBSTRUCTURE_NOTIFY_MASK = 1 << 19
MAP_REQUEST = 20
CONFIGURE_REQUEST = 23


def force_size(display: Display, width: int, height: int) -> None:
    for window in display.windows():
        try:
            old_width, old_height = display.geometry(window)
        except RuntimeError as error:
            # Clients may disappear between enumeration and geometry queries.
            print(f"wm: ignored {error!r}", flush=True)
            continue
        if old_width < width or old_height < height:
            display.move_resize(window, width, height)
            print(
                f"forced window 0x{window:x} from {old_width}x{old_height} "
                f"to {width}x{height}",
                flush=True,
            )


def handle_event(
    display: Display, event: XEvent, width: int, height: int
) -> None:
    if event.type == MAP_REQUEST:
        window = event.map_request.window
        if window:
            display.move_resize(window, width, height)
            display.library.XMapWindow(display.handle, window)
            print(
                f"mapped window 0x{window:x} at {width}x{height}", flush=True
            )
    elif event.type == CONFIGURE_REQUEST:
        request = event.configure_request
        if request.window:
            requested_width = min(request.width or width, 2 * width)
            requested_height = min(request.height or height, 2 * height)
            display.move_resize(
                request.window, requested_width, requested_height
            )


def manage_display(
    name: str, width: int, height: int, duration: float
) -> None:
    with Display(name) as display:
        display.handle_errors(
            lambda code: print(f"wm: X error code={code}", flush=True)
        )
        display.library.XSelectInput(
            display.handle,
            display.root,
            SUBSTRUCTURE_REDIRECT_MASK | SUBSTRUCTURE_NOTIFY_MASK,
        )
        display.library.XSync(display.handle, False)
        print(
            f"wm: managing root {display.root} of {name.encode()!r}",
            flush=True,
        )
        deadline = time.monotonic() + duration
        last_sweep = 0.0
        event = XEvent()
        while time.monotonic() < deadline:
            now = time.monotonic()
            if now - last_sweep > 0.5:
                force_size(display, width, height)
                last_sweep = now
            if display.library.XPending(display.handle) == 0:
                time.sleep(0.1)
                continue
            display.library.XNextEvent(display.handle, ctypes.byref(event))
            handle_event(display, event, width, height)


def positive_dimension(value: str) -> int:
    dimension = int(value)
    if dimension <= 0:
        raise argparse.ArgumentTypeError("dimensions must be positive")
    return dimension


def finite_duration(value: str) -> float:
    duration = float(value)
    if not math.isfinite(duration):
        raise argparse.ArgumentTypeError("duration must be finite")
    return duration


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("display")
    parser.add_argument("width", type=positive_dimension)
    parser.add_argument("height", type=positive_dimension)
    parser.add_argument(
        "duration_secs", type=finite_duration, nargs="?", default=60.0
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        manage_display(
            args.display, args.width, args.height, args.duration_secs
        )
    except (OSError, RuntimeError) as error:
        print(str(error), flush=True)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
