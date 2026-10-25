#!/usr/bin/env python3
"""Isolated display/shortcut checks; input targets Xvfb, never a calculator."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import TYPE_CHECKING

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from scripts.lib.x11 import Display

if TYPE_CHECKING:
    from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BINARY = (
    ROOT.parent
    / ".cache/prime-emulator-target/release/examples/mirror_diagnostics"
)


def require(condition: object, message: object) -> None:
    """Keep acceptance checks active even when Python runs with -O."""
    if not condition:
        raise AssertionError(message)


def available_display() -> str:
    for number in range(190, 240):
        socket = Path(f"/tmp/.X11-unix/X{number}")
        lock = Path(f"/tmp/.X{number}-lock")
        if not socket.exists() and not lock.exists():
            return f":{number}"
    raise RuntimeError("no free isolated X11 display in range 190–239")


def wait_until(
    condition: Callable[[], object],
    process: subprocess.Popen,
    description: str,
    timeout: float = 5.0,
    interval: float = 0.05,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(
                f"{description}: process exited with {process.returncode}"
            )
        if condition():
            return
        time.sleep(interval)
    raise RuntimeError(f"timed out waiting for {description}")


def open_display(name: str, server: subprocess.Popen) -> Display:
    """The socket can exist before Xvfb accepts connections."""
    connection = None

    def connect() -> bool:
        nonlocal connection
        try:
            connection = Display(name)
        except RuntimeError:
            return False
        return True

    wait_until(connect, server, "X11 connection", timeout=2, interval=0.02)
    return connection


def stop_process(process: subprocess.Popen | None, timeout: float) -> None:
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def capture_window(display: str, window: int, output: Path) -> None:
    subprocess.run(
        ["import", "-display", display, "-window", str(window), str(output)],
        check=True,
        timeout=10,
    )


def read_rgb(path: Path) -> Image.Image:
    try:
        from PIL import Image
    except ImportError as error:
        raise RuntimeError(
            "Pillow is required for pixel checks; install Pillow"
        ) from error

    with Image.open(path) as image:
        return image.convert("RGB")


def check_displayed_surface(output: Path) -> None:
    with (
        read_rgb(output / "displayed.png") as displayed,
        read_rgb(output / "diagnostics/surface.png") as surface,
    ):
        require(displayed.size == surface.size, (displayed.size, surface.size))
        require(
            displayed.tobytes() == surface.tobytes(),
            "displayed X11 pixels differ from submitted surface",
        )


def press_key(display: Display, symbol: int, settle: float = 0.3) -> None:
    display.key(symbol)
    time.sleep(settle)


def check_shortcuts(
    display: Display, window: int, output: Path
) -> tuple[tuple[int, int], list[tuple[int, int]]]:
    initial = display.geometry(window)
    require(initial == (853, 640), initial)
    display.focus(window)
    press_key(display, 32)
    require("Paused" in display.title(window), display.title(window))
    captures = output / "diagnostics/captures.jsonl"
    count_before = len(captures.read_text().splitlines())
    time.sleep(0.35)
    count_after = len(captures.read_text().splitlines())
    require(count_before == count_after, (count_before, count_after))
    press_key(display, 32)
    require("Paused" not in display.title(window), display.title(window))
    scales = []
    for scale in range(1, 5):
        press_key(display, ord(str(scale)))
        geometry = display.geometry(window)
        expected = (round(320 * scale * 4 / 3), round(240 * scale * 4 / 3))
        require(geometry == expected, (geometry, expected))
        scales.append(geometry)
    press_key(display, 65481)  # F12
    require(
        list(output.glob("prime-screen-*.png")), "snapshot was not created"
    )
    return initial, scales


def check_resized_surface(output: Path) -> None:
    with (
        read_rgb(output / "resized.png") as displayed,
        read_rgb(output / "diagnostics/renderer-source.png") as source,
    ):
        require(displayed.size == (1000, 600), displayed.size)
        for y in range(600):
            for x in range(1000):
                expected = (
                    (16, 16, 20)
                    if x < 180 or x >= 820 or y < 60 or y >= 540
                    else source.getpixel(
                        ((x - 180) * source.width // 640, (y - 60) * source.height // 480)
                    )
                )
                require(displayed.getpixel((x, y)) == expected, (x, y))


def exercise_window(
    display: Display, name: str, app: subprocess.Popen, output: Path
) -> dict:
    def mirror_window() -> int | None:
        return next(
            (
                window
                for window in display.windows()
                if "Live Screen" in display.title(window)
            ),
            None,
        )

    wait_until(mirror_window, app, "mirror window")
    window = mirror_window()
    require(window is not None, "mirror window disappeared")
    wait_until(
        lambda: (output / "diagnostics/surface.png").exists(),
        app,
        "rendered surface",
    )
    time.sleep(0.2)
    capture_window(name, window, output / "displayed.png")
    check_displayed_surface(output)
    initial, scales = check_shortcuts(display, window, output)
    display.resize(window, 1000, 600)
    time.sleep(0.4)
    capture_window(name, window, output / "resized.png")
    check_resized_surface(output)
    rows = [json.loads(line) for line in (output / "diagnostics/captures.jsonl").read_text().splitlines()]
    require(sum(row.get("changed", False) for row in rows) == 1,
            "resume must not count an identical image as changed")
    presentations = [json.loads(line) for line in (output / "diagnostics/presentations.jsonl").read_text().splitlines()]
    require(sum(row.get("unique", False) for row in presentations) == 1,
            "resize/resume must not inflate unique presentations")
    start = time.monotonic()
    press_key(display, 65307, 0)
    app.wait(timeout=3)
    return {
        "backend": "Xvfb/X11",
        "dpi": 4 / 3,
        "initial_physical_size": initial,
        "scales_1_to_4": scales,
        "display_surface_mismatched_pixels": 0,
        "resized_mismatched_pixels": 0,
        "pause_resume": True,
        "snapshot": True,
        "escape_close_ms": (time.monotonic() - start) * 1000,
    }


def run_smoke(fixture: Path, output: Path, binary: Path) -> dict:
    for path in (fixture, binary):
        if not path.is_file():
            raise ValueError(f"input file does not exist: {path}")
    for command in ("Xvfb", "import"):
        if shutil.which(command) is None:
            raise ValueError(f"required command not found: {command}")
    name = available_display()
    output.mkdir(parents=True, exist_ok=False)
    app = None
    # Files prevent pipe-buffer deadlocks during GUI/pixel checks.
    with (
        tempfile.TemporaryFile() as server_errors,
        tempfile.TemporaryFile() as app_output,
        tempfile.TemporaryFile() as app_errors,
    ):
        xvfb = subprocess.Popen(
            ["Xvfb", name, "-screen", "0", "1920x1440x24", "-nolisten", "tcp"],
            stdout=subprocess.DEVNULL,
            stderr=server_errors,
        )
        try:
            socket = Path(f"/tmp/.X11-unix/X{name[1:]}")
            wait_until(
                socket.exists,
                xvfb,
                "isolated X server",
                timeout=2,
                interval=0.02,
            )
            with open_display(name, xvfb) as display:
                environment = os.environ.copy()
                environment.pop("WAYLAND_DISPLAY", None)
                environment.pop("WAYLAND_SOCKET", None)
                environment.update(
                    DISPLAY=name, WINIT_X11_SCALE_FACTOR="1.3333333333333333"
                )
                app = subprocess.Popen(
                    [
                        str(binary),
                        "--fixture",
                        str(fixture),
                        "--quality",
                        "full",
                        "--scale",
                        "2",
                        "--gui-seconds",
                        "30",
                        "--out-dir",
                        str(output / "diagnostics"),
                    ],
                    env=environment,
                    cwd=output,
                    stdout=app_output,
                    stderr=app_errors,
                )
                result = exercise_window(display, name, app, output)
                app_errors.seek(0)
                result["stderr"] = app_errors.read().decode()
                require(app.returncode == 0, result["stderr"])
                (output / "results.json").write_text(
                    json.dumps(result, indent=2)
                )
                return result
        except (
            AssertionError,
            OSError,
            RuntimeError,
            subprocess.SubprocessError,
        ) as error:
            app_errors.seek(0)
            server_errors.seek(0)
            details = (app_errors.read() + server_errors.read()).decode(
                errors="replace"
            )
            raise RuntimeError(f"{error}\n{details}".rstrip()) from error
        finally:
            try:
                stop_process(app, timeout=10)
            finally:
                stop_process(xvfb, timeout=5)


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Fixture-only X11 mirror smoke test; never opens USB. "
            "Requires Xvfb, ImageMagick, Pillow and XTest."
        )
    )
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        result = run_smoke(
            args.fixture.resolve(),
            args.out_dir.resolve(),
            (args.binary or DEFAULT_BINARY).resolve(),
        )
    except (
        OSError,
        RuntimeError,
        ValueError,
        subprocess.SubprocessError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
