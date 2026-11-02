#!/usr/bin/env python3
"""Local firmware editing regressions; no physical parity claim.

Run run_m12.sh m16_input, then pass its scenario evidence directory here.
CAS letter case remains a documented failure, captured for the next milestone.
"""
import sys
from pathlib import Path
from assert_m12 import Report, common, png

d = Path(sys.argv[1])
r = Report()
common(r, "input", d)
before = png(d / "home_before_hold.png")
after = png(d / "home_after_hold.png")
# Ignore the blinking cursor at x=0 and inspect the input line only.
def line(image):
    return [row[5:200] for row in image[200:218]]

r.check("text was present before held Delete", any(len(set(row)) > 1 for row in line(before)))
r.check("held Delete cleared the input", all(len(set(row)) == 1 for row in line(after)))
typed = png(d / "cas_equals.png")
control = png(d / "equality_control.png")
# Physical Shift+decimal is the oracle; ignore status and cursor at the end.
r.check("desktop equality matches physical equality",
        [row[:22] for row in typed[200:218]] ==
        [row[:22] for row in control[200:218]])
sys.exit(0 if all(x["pass"] for x in r.results) else 1)
