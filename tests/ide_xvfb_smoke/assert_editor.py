#!/usr/bin/env python3
"""Assertions for the editor Xvfb scenarios (script_editor*.json).

Usage: python3 assert_editor.py dump_editor.json [dump_editor_hidpi.json]
Exits non-zero on the first failed assertion.
"""
import json
import os
import sys


def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f" {detail}"))
    if not cond:
        sys.exit(1)


def tab(d, title):
    return next(t for t in d["tabs"] if t["title"] == title)


def captures_preserve_state(d, label):
    import re
    # The resize sweep: the initial shot plus one per window size.
    caps = [
        c for c in d["window_captures"]
        if "initial" in c["path"] or re.search(r"_\d+x\d+\.png$", c["path"])
    ]
    check(f"{label}: screenshots written", all(os.path.getsize(c["path"]) > 10_000 for c in caps))
    for c in caps:
        w, h = c["screen_points"]
        pw, ph = c["pixels"]
        ppp = c["pixels_per_point"]
        check(
            f"{label}: {os.path.basename(c['path'])} pixels match points x scale",
            abs(pw - w * ppp) <= 1 and abs(ph - h * ppp) <= 1,
            str(c),
        )
    first = caps[0]
    for c in caps[1:]:
        check(
            f"{label}: cursor kept across resize ({os.path.basename(c['path'])})",
            c["cursor_char"] == first["cursor_char"],
            f"{c['cursor_char']} != {first['cursor_char']}",
        )
        check(
            f"{label}: no re-layout on resize ({os.path.basename(c['path'])})",
            c["layouts"] == first["layouts"] and c["analyses"] == first["analyses"],
            f"{c['layouts']}/{c['analyses']} vs {first['layouts']}/{first['analyses']}",
        )


def main():
    d = json.load(open(sys.argv[1]))
    demo = tab(d, "EDITDEMO.ppl")
    classes = {x["class"] for x in demo["diagnostics"]}
    check("valid PPL has no syntax/semantic errors", classes <= {"Compatibility"}, str(classes))
    codes = {x["code"] for x in demo["diagnostics"]}
    for code in ["unsupported-command", "lowercase-command",
                 "input-defaults-ignored", "symbolic-unsupported"]:
        check(f"compat diagnostic {code}", code in codes, str(codes))
    check(
        "typed block auto-indented and paired",
        demo["content"].endswith(
            'EXPORT TYPED()\nBEGIN\n  IF 1 THEN\n    PRINT("ok");\n  END;\n  END;'
        ),
        repr(demo["content"][-80:]),
    )
    check("breakpoint set from script", demo["breakpoints"] == [10], str(demo["breakpoints"]))
    broken = tab(d, "BROKEN.ppl")
    errs = [x for x in broken["diagnostics"] if x["is_error"]]
    check(
        "unclosed string is a syntax error on its own line",
        any(x["code"] == "unclosed-string" and x["class"] == "Syntax" and x["line"] == 3 for x in errs),
        str(broken["diagnostics"]),
    )
    check(
        "problems list every open file",
        {p["file"] for p in d["problems"]} >= {"EDITDEMO.ppl", "BROKEN.ppl"},
        str({p["file"] for p in d["problems"]}),
    )
    captures_preserve_state(d, "native")
    if len(sys.argv) > 2:
        h = json.load(open(sys.argv[2]))
        check("hidpi: scale factor 2", h["window"]["pixels_per_point"] == 2.0, str(h["window"]))
        captures_preserve_state(h, "hidpi")
    print("ALL EDITOR ASSERTIONS PASSED")


if __name__ == "__main__":
    main()
