#!/usr/bin/env python3
"""Checks script_firmware_runtime.json: programs run in the live G1 firmware
session through the GUI, with completion reported from firmware evidence.

A returned list completes (formerly "Firmware runtime: Unsupported"), a key
and a tap sent the way the GUI sends real keyboard/mouse input reach WAIT(-1),
Stop interrupts a running program, and the session runs again afterwards.

Usage: assert_firmware_runtime.py /tmp/ppl-ide-e2e/firmware_runtime.json
"""
import json
import re
import sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
snaps = {s["label"]: s["state"] for s in d["snapshots"]}
console = [m["text"] for m in d["console"]]
failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))
    failed += not ok


def results():
    return [l for l in console if l.startswith(("G1 firmware", "Firmware runtime", "Firmware operation"))]


print("INFO results: " + " | ".join(results()))
check("no completed run is reported as Unsupported",
      not any("Unsupported" in l for l in console), [l for l in console if "Unsupported" in l])
check("the returned list completes with its value",
      "G1 firmware returned {1, 2}" in console, results())
check("a WAIT(-1) program is reported running while it waits",
      "Running" in str(snaps["key-running"]["vm_state"]), snaps["key-running"]["vm_state"])
key = [l for l in results() if l.startswith("G1 firmware returned") and l != "G1 firmware returned {1, 2}"]
check("the GUI Enter key reached WAIT(-1), which returned its key code",
      len(key) >= 1 and re.fullmatch(r"G1 firmware returned \d+", key[0]) is not None, key)
check("a WAIT(-1) program is running before the tap",
      "Running" in str(snaps["touch-running"]["vm_state"]), snaps["touch-running"]["vm_state"])
check("the GUI tap reached WAIT(-1) at the tapped pixel: {type 0, x 100, y 50}",
      "G1 firmware returned {0, 100, 50}" in console, results())
check("an endless loop is running before Stop",
      "Running" in str(snaps["loop-running"]["vm_state"]), snaps["loop-running"]["vm_state"])
check("Stop interrupts it as a cancellation, not a completion",
      "Firmware operation cancelled" in console
      and "Running" not in str(snaps["loop-stopped"]["vm_state"]), snaps["loop-stopped"]["vm_state"])
check("the session runs a new program after the interruption",
      "G1 firmware returned 42" in console, results())
sys.exit(1 if failed else 0)
