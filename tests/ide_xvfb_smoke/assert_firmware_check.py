#!/usr/bin/env python3
"""Checks the firmware-diagnostic IDE scenarios (script_firmware_check.json
with the native backend, script_firmware_session.json with the firmware
session). Needs the original firmware configured (see README).

Usage: assert_firmware_check.py dump.json [samples.log pids-before pids-after]
"""
import json
import sys

d = json.load(open(sys.argv[1], encoding="utf-8"))
snaps = {s["label"]: s["state"] for s in d["snapshots"]}
failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))
    failed += not ok


def firmware_problems(st, file=None):
    return [p for p in st["problems"] if p["provenance"] == "guest-firmware"
            and (file is None or p["file"] == file)]


def tab(st, title):
    return next(t for t in st["tabs"] if t["title"] == title)


def marked(st, title):
    """Text under each firmware marker of `title` (byte spans of UTF-8)."""
    t = tab(st, title)
    if not t["firmware"]:
        return []
    raw = t["content"].encode("utf-8")
    return [raw[i["span"][0]:i["span"][1]].decode("utf-8") for i in t["firmware"]["items"]]


st = snaps["native-only"]
check("native analysis alone is labelled native", firmware_problems(st) == []
      and all(p["provenance"] == "native-compiler" for p in st["problems"])
      and st["status_analysis"].startswith("Native:"), st["problems"])
check("no firmware verdict before Check", tab(st, "FWCHK.ppl")["firmware"] is None
      and st["status_firmware"] is None)

st = snaps["checked-rejected"]
fw = firmware_problems(st, "FWCHK.ppl")
check("firmware error listed once at 4:15 (after θ and π)",
      [(p["line"], p["col"]) for p in fw] == [(4, 15)], fw)
check("firmware marker sits on the `;` the firmware reported",
      marked(st, "FWCHK.ppl") == [";"], marked(st, "FWCHK.ppl"))
# Native location parity reports the unclosed `(` where the firmware stops,
# the `;` at 4:15; the native item stays listed beside the firmware one.
check("native error kept beside it, at the firmware position",
      any(p["provenance"] == "native-compiler"
          and (p["line"], p["col"]) == (4, 15) for p in st["problems"]), st["problems"])
check("status bar names the firmware verdict", st["status_firmware"] == "Firmware: 1 error(s)",
      st["status_firmware"])

st = snaps["edited"]
check("an edit retires the firmware result", firmware_problems(st) == []
      and tab(st, "FWCHK.ppl")["firmware"] is None and st["status_firmware"] is None)
check("native success is not called firmware-verified",
      "not firmware-verified" in st["problems_empty_message"], st["problems_empty_message"])

st = snaps["checked-passed"]
check("a firmware pass is reported as such", st["problems"] == []
      and st["firmware_passed"] == ["FWCHK.ppl"]
      and "Original-firmware Check passed" in st["problems_empty_message"]
      and st["status_firmware"] == "Firmware: Check passed", st["problems_empty_message"])

for label in ("second-tab-checked", "first-tab-again", "repeated"):
    st = snaps[label]
    fw = firmware_problems(st)
    check(f"{label}: only FWTWO has a firmware error, at 3:9",
          [(p["file"], p["line"], p["col"]) for p in fw] == [("FWTWO.ppl", 3, 9)], fw)
    check(f"{label}: FWCHK keeps its own pass", "FWCHK.ppl" in st["firmware_passed"]
          and tab(st, "FWCHK.ppl")["firmware"]["items"] == [], st["firmware_passed"])
    check(f"{label}: FWTWO marker on the `)`", marked(st, "FWTWO.ppl") == [")"],
          marked(st, "FWTWO.ppl"))

st = snaps["stopped-during-check"]
check("Stop leaves no Check pending", st["check_pending"] is False)
check("Stop keeps the held verdict for the same text",
      [(p["file"], p["line"]) for p in firmware_problems(st)] == [("FWTWO.ppl", 3)],
      firmware_problems(st))

for label in ("ran", "restarted"):
    st = snaps[label]
    check(f"{label}: nothing pending afterwards", st["check_pending"] is False)
    check(f"{label}: FWCHK still passed, FWTWO still rejected",
          "FWCHK.ppl" in st["firmware_passed"]
          and [p["file"] for p in firmware_problems(st)] == ["FWTWO.ppl"], st["firmware_passed"])

if "session-run-rejected" in snaps:
    st = snaps["session-run-edited"]
    check("session run: the edit retired FWTWO's earlier verdict",
          firmware_problems(st, "FWTWO.ppl") == [], firmware_problems(st))
    st = snaps["session-run-rejected"]
    fw = firmware_problems(st, "FWTWO.ppl")
    check("session run: the live session's rejection is listed at 3:9",
          [(p["line"], p["col"]) for p in fw] == [(3, 9)], fw)
    t = tab(st, "FWTWO.ppl")
    check("session run: the verdict is for the current revision",
          t["firmware"] is not None and t["firmware"]["revision"] == t["revision"], t["firmware"])
    check("session run: marker on the `)`", marked(st, "FWTWO.ppl") == [")"], marked(st, "FWTWO.ppl"))

console = "\n".join(m["text"] for m in d["console"])
session_backend = "Firmware runtime:" in console or "G1 firmware" in console
if session_backend:
    # Firmware backend: the Run compiled the exact source (FWCHK stays
    # passed above); its runtime result is reported, not asserted here.
    runs = [l for l in console.splitlines() if l.startswith(("Firmware runtime", "G1 firmware returned"))]
    print("INFO firmware Run results: " + "; ".join(runs))
    check("a firmware Check is not shown as a VM error",
          all("Syntax Error" not in str(s["state"]["vm_state"]) for s in d["snapshots"]
              if not s["label"].startswith("session-run")),
          [s["state"]["vm_state"] for s in d["snapshots"]])
else:
    check("the valid program ran and returned θ+2π", "9.42477796077" in console, console[-600:])

direct = [l for l in console.splitlines() if "direct compiler call" in l]
print(f"INFO direct firmware Checks: {len(direct)}; " + "; ".join(direct[:3]))
check("later Checks used the direct firmware compiler call", len(direct) >= 3, len(direct))
import re
times = [float(m.group(1)) for l in direct for m in [re.search(r"call, (\d+) ms", l)] if m]
check("every direct Check took under 1 s", times and max(times) < 1000, times)

if len(sys.argv) > 2:
    # Samples of how many firmware runners the IDE process had as children.
    counts = [int(line.split()[1]) for line in open(sys.argv[2]) if len(line.split()) == 2]
    # The warm direct checker runs beside at most one fresh checker; the
    # firmware backend adds its live calculator session.
    limit = 3 if session_backend else 2
    check(f"at most {limit} firmware process(es) at a time", max(counts, default=0) <= limit,
          max(counts, default=0))
    check("firmware processes were observed", max(counts, default=0) >= 1, "none sampled")
if len(sys.argv) > 4:
    before = set(open(sys.argv[3]).read().split())
    after = set(open(sys.argv[4]).read().split())
    check("no firmware process left behind after exit", after <= before, after - before)

sys.exit(1 if failed else 0)
