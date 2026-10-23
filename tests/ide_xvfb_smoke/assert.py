#!/usr/bin/env python3
"""Assertions for the IDE Xvfb smoke-test dumps.

Usage: python3 assert.py dump_workflow.json dump_syntax.json ... [--gfx-png gfx_screen.png]

Exits non-zero on the first failed assertion.
"""
import json
import sys


def load(path):
    with open(path) as f:
        return json.load(f)


def check(name, cond, detail=""):
    if cond:
        print(f"PASS {name}")
    else:
        print(f"FAIL {name} {detail}")
        sys.exit(1)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    gfx_png = None
    if "--gfx-png" in sys.argv:
        gfx_png = sys.argv[sys.argv.index("--gfx-png") + 1]

    d = load(args[0])  # workflow
    check("workflow: vm finished", d["vm_state"] == "Done", d["vm_state"])
    check(
        "workflow: console shows 30",
        any(m["text"] == "30" for m in d["console"]),
        str(d["console"]),
    )
    check(
        "workflow: worker success 30",
        d["worker_outcome"].get("kind") == "success"
        and d["worker_outcome"].get("value") == "30",
        str(d["worker_outcome"]),
    )
    saved_ok = any(
        t.get("path") and "PRINT(x+y)" in t["content"] for t in d["tabs"]
    )
    check("workflow: reopened tab has the saved source", saved_ok)

    if len(args) > 1:  # syntax
        d = load(args[1])
        check(
            "syntax: fixed program ran",
            d["vm_state"] == "Done" and d["terminal"] == ["10"],
            f"{d['vm_state']} {d['terminal']}",
        )

    if len(args) > 2:  # debug
        d = load(args[2])
        check(
            "debug: finished with 120",
            d["worker_outcome"].get("kind") == "success"
            and d["worker_outcome"].get("value") == "120"
            and d["terminal"] == ["120"],
            str(d["worker_outcome"]),
        )
        check(
            "debug: breakpoint recorded",
            d["tabs"][d["active_tab"]]["breakpoints"] == [6],
        )

    if len(args) > 3:  # gfx
        d = load(args[3])
        check("gfx: run finished", d["vm_state"] == "Done")
        if gfx_png:
            import subprocess

            out = subprocess.run(
                ["magick", gfx_png, "-format",
                 "%w %h %[pixel:p{5,5}] %[pixel:p{80,70}] %[pixel:p{160,155}]",
                 "info:"],
                capture_output=True, text=True,
            ).stdout.strip()
            parts = out.split()
            check("gfx: png 320x240", parts[0] == "320" and parts[1] == "240", out)
            check("gfx: red background", parts[2] == "srgb(255,0,0)", out)
            check("gfx: blue box", parts[3] == "srgb(0,0,255)", out)
            check("gfx: yellow text", parts[4] == "srgb(255,255,0)", out)

    if len(args) > 4:  # input
        d = load(args[4])
        check(
            "input: typed 21 -> returned 42",
            d["worker_outcome"].get("kind") == "success"
            and d["worker_outcome"].get("value") == "42",
            str(d["worker_outcome"]),
        )

    if len(args) > 5:  # stop
        d = load(args[5])
        check(
            "stop: execution stopped by user",
            any("stopped by user" in m["text"] for m in d["console"])
            and not d["worker_alive"],
            str(d["console"]),
        )

    if len(args) > 6:  # run config (parameterized entry point)
        d = load(args[6])
        check(
            "runconfig: SQUARE(5) returned 25",
            d["worker_outcome"].get("kind") == "success"
            and d["worker_outcome"].get("value") == "25",
            str(d["worker_outcome"]),
        )

    print("ALL ASSERTIONS PASSED")


if __name__ == "__main__":
    main()
