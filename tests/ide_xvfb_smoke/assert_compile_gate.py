#!/usr/bin/env python3
"""Checks the compile-gate IDE scenario (script_compile_gate.json).

Usage: assert_compile_gate.py dump_compile_gate.json
"""
import json
import sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))
    failed += not ok


console = [m["text"] for m in d["console"]]
joined = "\n".join(console)


def index_of(needle):
    return next((i for i, t in enumerate(console) if needle in t), -1)


first, blocked, third = index_of("first"), index_of("Run blocked"), index_of("third")
check("gate: first program ran", first >= 0, joined)
check("gate: broken source was not run", blocked > first and "second" not in joined, joined)
check("gate: fixed source ran afterwards", third > blocked, joined)
check(
    "gate: the last run is the fixed program",
    d["worker_outcome"].get("kind") == "success" and d["worker_outcome"].get("value") == "3",
    str(d["worker_outcome"]),
)
check("gate: final source is valid", d["analysis"] == "Valid", d["analysis"])
check("gate: problems cleared", d["problems"] == [] and d["problems_title"] == "Problems (0)",
      f'{d["problems_title"]} {d["problems"]}')
sys.exit(1 if failed else 0)
