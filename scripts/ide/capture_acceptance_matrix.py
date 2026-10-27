#!/usr/bin/env python3
"""Automated visual acceptance screenshot matrix generator.

Captures real native IDE application states using the built-in PPL_IDE_AUTOMATION harness:
- docked / split layout
- expanded calculator
- screen-only calculator
- editor-focused layout
- debugging with active breakpoint
- small window (800x600)
- 1440p resolution (2560x1440)
- populated LCD (running graphics)
- firmware backend / Home
- firmware program catalog
- DPI scaling variations (1.0x, 1.5x, 2.0x)
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

REPO_ROOT = Path(__file__).resolve().parents[2]
PPL_BIN = REPO_ROOT / "../.cache/prime-emulator-target/release/ppl"
if not PPL_BIN.exists():
    PPL_BIN = REPO_ROOT / "target/release/ppl"

OUT_DIR = REPO_ROOT / "docs/screenshots"
OUT_DIR.mkdir(parents=True, exist_ok=True)

def run_scenario(name: str, steps: list, resolution="1920x1080x24", env_extra=None, example="examples/FIREWORKS.hpprgm"):
    shot_path = OUT_DIR / f"{name}.png"
    dump_path = OUT_DIR / f"{name}_dump.json"
    
    script = {
        "dump": str(dump_path),
        "steps": steps + [
            {"step": "window_screenshot", "path": str(shot_path)},
            {"step": "wait_ms", "ms": 100},
            {"step": "exit"}
        ]
    }
    
    script_file = Path(tempfile.gettempdir()) / f"auto_{name}.json"
    script_file.write_text(json.dumps(script, indent=2))
    
    env = os.environ.copy()
    env.update({
        "PPL_IDE_AUTOMATION": str(script_file),
        "LIBGL_ALWAYS_SOFTWARE": "1",
        "WINIT_UNIX_BACKEND": "x11",
    })
    if env_extra:
        env.update(env_extra)
        
    cmd = [
        "xvfb-run",
        "-s", f"-screen 0 {resolution}",
        str(PPL_BIN),
        "gui",
        str(REPO_ROOT / example)
    ]
    
    print(f"[*] Capturing scenario '{name}' -> {shot_path.name} ...", flush=True)
    start = time.perf_counter()
    res = subprocess.run(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=45)
    elapsed = time.perf_counter() - start
    
    if shot_path.exists():
        size_kb = shot_path.stat().st_size / 1024
        print(f"    [+] Success in {elapsed:.2f}s ({size_kb:.1f} KiB)", flush=True)
        return True
    else:
        print(f"    [-] Failed to capture {name} (rc={res.returncode}):\n{res.stderr.decode()}", flush=True)
        return False

def main():
    scenarios = [
        # 1. Docked / Split Layout (Default)
        ("01_docked_split_1080p", [
            {"step": "wait_ms", "ms": 200},
            {"step": "dock", "tab": "output"},
        ]),
        # 2. Expanded Calculator
        ("02_expanded_calculator", [
            {"step": "wait_ms", "ms": 100},
            {"step": "layout", "mode": "calculator"},
            {"step": "wait_ms", "ms": 200},
        ]),
        # 3. Screen-Only Calculator Mode
        ("03_screen_only_calculator", [
            {"step": "wait_ms", "ms": 100},
            {"step": "calculator_mode", "mode": "screen"},
            {"step": "wait_ms", "ms": 200},
        ]),
        # 4. Editor Focus Mode
        ("04_editor_focus_mode", [
            {"step": "wait_ms", "ms": 100},
            {"step": "layout", "mode": "editor"},
            {"step": "wait_ms", "ms": 200},
        ]),
        # 5. Debugging Paused at Breakpoint
        ("05_debugging_breakpoint", [
            {"step": "wait_ms", "ms": 100},
            {"step": "breakpoint", "line": 42},
            {"step": "debug"},
            {"step": "wait_paused", "timeout_ms": 5000},
            {"step": "dock", "tab": "variables"},
            {"step": "wait_ms", "ms": 200},
        ]),
        # 6. Small Window Constraints
        ("06_small_window_800x600", [
            {"step": "wait_ms", "ms": 100},
            {"step": "resize", "width": 800, "height": 600},
            {"step": "wait_ms", "ms": 300},
        ], "800x600x24"),
        # 7. 1440p High Resolution
        ("07_resolution_1440p", [
            {"step": "wait_ms", "ms": 100},
            {"step": "resize", "width": 2560, "height": 1440},
            {"step": "wait_ms", "ms": 300},
        ], "2560x1440x24"),
        # 8. Populated LCD with Running Graphics
        ("08_populated_lcd_fireworks", [
            {"step": "wait_ms", "ms": 100},
            {"step": "run"},
            {"step": "wait_ms", "ms": 800},
        ]),
        # 9. Firmware Backend Toggle (Home)
        ("09_firmware_backend_home", [
            {"step": "wait_ms", "ms": 100},
            {"step": "backend", "engine": "firmware"},
            {"step": "wait_ms", "ms": 600},
        ]),
        # 10. Problems Dock Tab Active
        ("10_dock_problems_view", [
            {"step": "wait_ms", "ms": 100},
            {"step": "dock", "tab": "problems"},
            {"step": "wait_ms", "ms": 200},
        ]),
        # 11. DPI Scaling 1.0x
        ("11_dpi_scale_1_0x", [
            {"step": "wait_ms", "ms": 200},
        ], "1920x1080x24", {"WINIT_X11_SCALE_FACTOR": "1.0"}),
        # 12. DPI Scaling 1.5x
        ("12_dpi_scale_1_5x", [
            {"step": "wait_ms", "ms": 200},
        ], "1920x1080x24", {"WINIT_X11_SCALE_FACTOR": "1.5"}),
        # 13. DPI Scaling 2.0x
        ("13_dpi_scale_2_0x", [
            {"step": "wait_ms", "ms": 200},
        ], "2560x1440x24", {"WINIT_X11_SCALE_FACTOR": "2.0"}),
    ]
    
    results = {}
    for item in scenarios:
        name = item[0]
        steps = item[1]
        res = item[2] if len(item) > 2 else "1920x1080x24"
        env_ex = item[3] if len(item) > 3 else None
        ok = run_scenario(name, steps, resolution=res, env_extra=env_ex)
        results[name] = ok
        
    print("\nCapture Summary:")
    for k, v in results.items():
        status = "PASSED" if v else "FAILED"
        print(f"  {k}: {status}")

if __name__ == "__main__":
    main()
