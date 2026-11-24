#!/usr/bin/env python3
"""Assertions for the IDE Xvfb smoke-test dumps.

Usage: python3 assert.py dump_workflow.json dump_syntax.json ... [--gfx-png gfx_screen.png]

Exits non-zero on the first failed assertion.
"""
import argparse
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dumps", nargs="+", help="Ordered workflow through runconfig dumps")
    parser.add_argument("--gfx-png", help="Graphics screenshot from the same isolated run")
    options = parser.parse_args()
    args = options.dumps
    gfx_png = options.gfx_png
    if not 1 <= len(args) <= 7:
        parser.error("expected one to seven ordered scenario dumps")
    if gfx_png and len(args) < 4:
        parser.error("--gfx-png requires the graphics scenario dump")

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
            from PIL import Image

            with Image.open(gfx_png) as capture:
                image = capture.convert("RGB")
            check("gfx: png 320x240", image.size == (320, 240), str(image.size))
            check("gfx: red background", image.getpixel((5, 5)) == (255, 0, 0))
            check("gfx: blue box", image.getpixel((80, 70)) == (0, 0, 255))
            # A point inside a glyph's bounding box can legitimately be its
            # background. Require actual yellow glyph pixels in the declared
            # TEXTOUT_P region and forbid yellow outside it instead.
            yellow = [
                (x, y) for y in range(240) for x in range(320)
                if image.getpixel((x, y)) == (255, 255, 0)
            ]
            check("gfx: yellow text", len(yellow) >= 20, f"{len(yellow)} yellow pixels")
            check("gfx: text stays in its region", all(
                150 <= x < 185 and 150 <= y < 168 for x, y in yellow
            ))

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
            any("Stop requested by user" in m["text"] for m in d["console"])
            and any("Execution interrupted" in m["text"] for m in d["console"])
            and not d["worker_alive"] and d["vm_state"] == "Idle",
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
