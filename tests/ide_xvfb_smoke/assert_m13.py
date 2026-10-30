#!/usr/bin/env python3
"""M13 assertions: storage, memory and power state of the G1 Firmware
calculator (evidence of run_m12.sh m13_persist, m13_reopen on the same
profile, m13_crash, m13_reset after them). Local-firmware evidence only:
what a physical G1 preserves is verified in M15.

Usage: assert_m13.py <run directory>"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from assert_m12 import Report, changed_rows, console, png, same_result, snapshots  # noqa: E402

EXPECT = "G1 firmware returned {5, 99, 6, 3, 1}"


def views_equal(r, name, d, a, b, screens=("home", "settings", "symb", "cas")):
    for screen in screens:
        x, y = d / f"{screen}_{a}.png", d / f"{screen}_{b}.png"
        rows = changed_rows(png(x), png(y), 0, 320)
        r.check(f"{name}: {screen} after {b} equals {a}", not rows, f"changed rows {rows[:6]}")


def common(r, name, d):
    j = json.loads((d / "dump.json").read_text())
    r.check(f"{name}: no wait step timed out", not j["wait_timeouts"], str(j["wait_timeouts"]))
    left = (d / "leftover-pids.txt").read_text().strip()
    r.check(f"{name}: no firmware helper outlived the IDE", not left, left)
    return j


def persist(r, d):
    j = common(r, "M13 persist", d)
    s, text = snapshots(j), console(j)
    r.check("M13 persist: the setup program ran in the firmware", "G1 firmware returned 1" in text)
    r.check("M13 persist: the setup changed Home Settings (Radians → Degrees)",
            bool(changed_rows(png(d / "settings_default.png"), png(d / "settings_before.png"), 0, 320)))
    r.check("M13 persist: the setup changed F1(X) in the Function app",
            bool(changed_rows(png(d / "symb_default.png"), png(d / "symb_before.png"), 0, 320)))
    r.check("M13 persist: Power off is the firmware's own sleep", s["asleep"]["firmware_session"] == "Sleeping"
            and s["asleep"]["firmware_alive"] and s["asleep"]["firmware_session_id"] == s["set_done"]["firmware_session_id"])
    r.check("M13 persist: a non-ON key leaves it asleep", s["asleep_key"]["firmware_session"] == "Sleeping")
    r.check("M13 persist: ON wakes the same session",
            s["woken"]["firmware_session"] == "Ready" and s["woken"]["firmware_session_id"] == s["asleep"]["firmware_session_id"])
    views_equal(r, "M13 sleep/wake", d, "before", "woken")
    r.check("M13 persist: Shut down ends the helper", s["shut_down"]["firmware_session"] == "Off" and not s["shut_down"]["firmware_alive"])
    r.check("M13 persist: Start is a new session",
            s["restarted"]["firmware_session_id"] != s["woken"]["firmware_session_id"])
    views_equal(r, "M13 shut down/start", d, "before", "restarted")
    same_result(r, "M13 shut down/start: Ans", d, ("ans_restarted", "Ans"), ("lit42_restarted", "42"), ("lit43_restarted", "43"))
    r.check("M13 shut down/start: A, Z, L2, M1, HAngle kept (firmware returned {5, 99, 6, 3, 1})", EXPECT in text,
            str([t for t in text if t.startswith("G1 firmware returned")]))


def reopen(r, d, persist_dir):
    j = common(r, "M13 reopen", d)
    text = console(j)
    for screen in ("settings", "symb", "cas"):
        rows = changed_rows(png(persist_dir / f"{screen}_before.png"), png(d / f"{screen}_reopened.png"), 0, 320)
        r.check(f"M13 IDE close/reopen: {screen} equals the state before closing", not rows, f"changed rows {rows[:6]}")
    same_result(r, "M13 IDE close/reopen: Ans", d, ("ans_reopened", "Ans"), ("lit42_reopened", "42"), ("lit43_reopened", "43"))
    r.check("M13 IDE close/reopen: A, Z, L2, M1, HAngle kept", EXPECT in text,
            str([t for t in text if t.startswith("G1 firmware returned")]))


def crash(r, d):
    j = common(r, "M13 crash", d)
    s = snapshots(j)
    r.check("M13 crash: the killed helper is reported as failed", s["killed"]["firmware_session"].startswith("Failed"))
    lost = bool(changed_rows(png(d / "settings_before.png"), png(d / "settings_after_crash.png"), 0, 320))
    r.note("Battery pull (helper killed, no OFF)",
           "changes since the last OFF were lost (Home Settings back to its earlier value)" if lost
           else "Home Settings survived")
    r.check("M13 crash: the calculator boots again after the crash", "home_after_crash" in s)


def reset(r, d):
    j = common(r, "M13 reset", d)
    s, text = snapshots(j), console(j)
    msg = next((t for t in text if t.startswith("Calculator memory reset")), "")
    backup = Path(msg.rsplit(" at ", 1)[-1]) if msg else None
    r.check("M13 reset: memory reset with a backup", bool(backup) and (backup / "C").is_dir(), msg)
    r.check("M13 reset: the stored values are cleared (firmware returned {0, 0, 0})",
            "G1 firmware returned {0, 0, 0}" in text, str([t for t in text if t.startswith("G1 firmware returned")]))
    r.check("M13 reset: Home history cleared", bool(changed_rows(png(d / "home_before_reset.png"), png(d / "home_after_reset.png"), 0, 320)))


def main():
    run = Path(sys.argv[1])
    r = Report()
    persist(r, run / "m13_persist")
    reopen(r, run / "m13_reopen", run / "m13_persist")
    crash(r, run / "m13_crash")
    if (run / "m13_reset" / "dump.json").exists():
        reset(r, run / "m13_reset")
    passed = sum(x["pass"] for x in r.results)
    print(f"{passed}/{len(r.results)} checks passed")
    (run / "assert_m13.json").write_text(json.dumps({"checks": r.results, "notes": r.notes}, indent=1))
    sys.exit(0 if passed == len(r.results) else 1)


if __name__ == "__main__":
    main()
