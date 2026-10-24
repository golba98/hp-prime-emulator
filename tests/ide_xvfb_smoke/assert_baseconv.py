#!/usr/bin/env python3
"""Checks the Baseconv IDE scenarios.

Usage: assert_baseconv.py dump_baseconv.json dump_baseconv_cancel.json
"""
import json
import sys


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))
    failed += not ok


run = load(sys.argv[1])
console = [m["text"] for m in run["console"]]
check(
    "baseconv: typed FF (after an invalid base re-prompt) gives 255",
    "Final Result: 255" in console,
    str(console),
)
check(
    "baseconv: second run converts 7B.8 to 01111011.1000",
    run["worker_outcome"].get("kind") == "success"
    and run["worker_outcome"].get("value") == "01111011.1000",
    str(run["worker_outcome"]),
)
check("baseconv: no problems reported", run["problems"] == [], str(run["problems"]))

cancel = load(sys.argv[2])
check(
    "baseconv: Esc in the third dialog ends the program normally",
    cancel["worker_outcome"].get("kind") == "success"
    and cancel["worker_outcome"].get("value") == "0"
    and not any("Final Result" in m["text"] for m in cancel["console"]),
    str(cancel["worker_outcome"]),
)
sys.exit(1 if failed else 0)
