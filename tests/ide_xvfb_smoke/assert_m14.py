#!/usr/bin/env python3
"""M14 assertions: the firmware's own CAS, Function app graphing, every
built-in app, and app computations, through the IDE (evidence of
run_m12.sh m14_cas, m14_function, m14_apps, m14_appwork).

Values are checked by the M4 glyph method (the CAS result glyph equals the
glyph of the expected expression typed in the same view; a control differs)
or by pixel-exact comparison, below the status bar, with result screens
recorded only after their values were checked by hand
(tests/runtime-m14/reference-ui/README.md). Local-firmware evidence only.

Usage: assert_m14.py <run directory>"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from assert_m12 import Report, changed_rows, console, png, same_result, snapshots  # noqa: E402

REF = HERE.parents[1] / "tests/runtime-m14/reference-ui"
KNOWN_UNSUPPORTED = {"_FindResourceW", "GetApplicationHeadInfoA"}


def newest_result(img, top=20, bottom=205, left=160, right=320):
    """The newest history result in a Home/CAS view, cropped to its ink.
    History entries are separated by uniform horizontal lines (the entry
    line's border is the lowest): the newest result lies between the two
    lowest separators, whatever its template's height."""
    def separator(y):
        row = img[y]
        return len(set(row)) == 1 and row[0] != (255, 255, 255)
    seps = [y for y in range(top, bottom) if separator(y)]
    if not seps:
        return None
    # The first entry has no separator above it: the history starts at top.
    upper, lower = (seps[-2] if len(seps) > 1 else top - 1), seps[-1]
    block = [img[k][left:right] for k in range(upper + 1, lower)]
    rows = [i for i, row in enumerate(block) if any(p != (255, 255, 255) for p in row)]
    cols = [x for x in range(right - left) if any(row[x] != (255, 255, 255) for row in block)]
    if not rows or not cols:
        return None
    return [tuple(block[i][min(cols):max(cols) + 1]) for i in range(min(rows), max(rows) + 1)]


def common(r, name, d, timeouts_ok=False):
    j = json.loads((d / "dump.json").read_text())
    if not timeouts_ok:
        r.check(f"{name}: no wait step timed out", not j["wait_timeouts"], str(j["wait_timeouts"]))
    left = (d / "leftover-pids.txt").read_text().strip()
    r.check(f"{name}: no firmware helper outlived the IDE", not left, left)
    new = {u["name"] for u in j["firmware_unsupported"]} - KNOWN_UNSUPPORTED
    r.check(f"{name}: no new unsupported OS service", not new, str(sorted(new)))
    return j


def reference(r, name, actual, ref):
    rows = changed_rows(png(actual), png(REF / ref), 0, 320)
    r.check(f"{name}: matches the verified reference {ref} below the status bar", not rows,
            f"changed rows {rows[:8]}{'…' if len(rows) > 8 else ''}")


def cas(r, d):
    common(r, "M14 CAS", d)
    cases = json.loads((HERE / "scripts/m14_cas_cases.json").read_text())
    for c in cases:
        n = c["name"]
        label = f"M14 CAS {c['subject']}"
        if n == "factor":
            # The CAS reorders a typed (x−1)·(x+1) to (x+1)·(x−1): only the
            # control is comparable; the result was read as (x−1)·(x+1).
            a, b = png(d / f"{n}_subject.png"), png(d / f"{n}_control.png")
            rows = [y for y in range(20, 200) if a[y][160:] != b[y][160:]]
            r.check(f"{label}: result differs from the unfactored control", bool(rows))
            r.note(label, "result (x−1)·(x+1), read on the CAS screen")
            continue
        subject, literal, control = (newest_result(png(d / f"{n}_{t}.png"))
                                     for t in ("subject", "literal", "control"))
        r.check(f"{label}: result glyph == the glyph of {c['literal']}", subject is not None and subject == literal,
                f"{len(subject or [])}×{len(subject[0]) if subject else 0} px block")
        r.check(f"{label}: result differs from the control {c['control']}", subject != control)


def function(r, d):
    common(r, "M14 Function", d, timeouts_ok=True)
    s = snapshots(json.loads((d / "dump.json").read_text()))
    r.check("M14 Function: F1 and F2 set and checked by the firmware", "symb" in s)
    for shot in ("plot", "zoom_in", "num"):
        reference(r, f"M14 Function {shot}", d / f"{shot}.png", f"function-{shot}.png")
    r.check("M14 Function: trace moves the cursor (Right)",
            bool(changed_rows(png(d / "plot.png"), png(d / "trace_right.png"), 0, 320)))
    r.check("M14 Function: zoom out differs from zoom in",
            bool(changed_rows(png(d / "zoom_in.png"), png(d / "zoom_out.png"), 0, 320)))


def apps(r, d):
    j = common(r, "M14 apps", d)
    names = json.loads((HERE / "scripts/m14_apps.json").read_text())
    text = console(j)
    started = sum(t == "G1 firmware returned 1" for t in text)
    r.check(f"M14 apps: STARTAPP returned for all {len(names)} apps", started == len(names), f"{started}/{len(names)}")
    home = None
    for i, app in enumerate(names):
        views = [png(d / f"{i:02d}_{v}.png") for v in ("app", "symb", "plot", "num")]
        if home is None:
            home = views[0]
        r.check(f"M14 apps: {app} opened its own screens",
                any(changed_rows(views[0], v, 0, 320) for v in views[1:]) or app in ("Spreadsheet", "Linear Solver", "Data Streamer", "Triangle Solver"),
                "static app (one view)" if app in ("Spreadsheet", "Linear Solver", "Data Streamer", "Triangle Solver") else "")


def appwork(r, d):
    common(r, "M14 app work", d)
    for before, after, what in (("s1v_num", "s1v_stats", "Statistics 1Var Stats"),
                                ("solve_num", "solve_solved", "Solve X²=4"),
                                ("s2v_num", "s2v_stats", "Statistics 2Var Stats"),
                                ("sheet_a1", "sheet_a2", "Spreadsheet A2=A1*2"),
                                ("lin_2x2", "lin_solved", "Linear Solver 2×2")):
        r.check(f"M14 {what}: the app computed (screen changed)",
                bool(changed_rows(png(d / f"{before}.png"), png(d / f"{after}.png"), 0, 320)))
        reference(r, f"M14 {what}", d / f"{after}.png", f"appwork-{after}.png")


def main():
    run = Path(sys.argv[1])
    r = Report()
    for name, check in (("m14_cas", cas), ("m14_function", function), ("m14_apps", apps), ("m14_appwork", appwork)):
        if (run / name / "dump.json").exists():
            check(r, run / name)
        else:
            r.check(f"{name}: dump exists", False, str(run / name))
    passed = sum(x["pass"] for x in r.results)
    print(f"{passed}/{len(r.results)} checks passed")
    (run / "assert_m14.json").write_text(json.dumps({"checks": r.results, "notes": r.notes}, indent=1))
    sys.exit(0 if passed == len(r.results) else 1)


if __name__ == "__main__":
    main()
