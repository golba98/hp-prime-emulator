#!/usr/bin/env python3
"""M12 assertions over the evidence of run_m12.sh scenarios.

Usage: assert_m12.py <run directory> [scenario ...]
Every check reads genuine firmware frames saved by the IDE (its live LCD) and
the IDE's state dump. Results are local-firmware evidence only: physical
calculator parity is NOT VERIFIED by any of them. Standard library only.
"""
import json
import os
import struct
import subprocess
import sys
import zlib
from pathlib import Path

STATUS_BAR = 20  # firmware status bar rows (host clock and battery)
ROOT = Path(__file__).resolve().parents[2]
REFERENCE = ROOT / "tests/runtime/reference"


def frame_diff(actual, reference, mask=None):
    """Exact comparison by the workspace's `prime-emulator frame-diff`."""
    tool = os.environ.get("PPL_FIRMWARE_RUNNER")
    if not tool:
        meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"],
                              cwd=ROOT, capture_output=True, text=True, check=True)
        tool = str(Path(json.loads(meta.stdout)["target_directory"]) / "release/prime-emulator")
    cmd = [tool, "frame-diff", str(actual), str(reference)]
    if mask:
        cmd += ["--mask", str(mask)]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode not in (0, 1):
        return {"exact": False, "error": out.stderr.strip()}
    return json.loads(out.stdout)


def png(path):
    """Decodes an 8-bit RGB/RGBA non-interlaced PNG into rows of RGB tuples."""
    data = Path(path).read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    pos, idat, w = 8, b"", 0
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
            assert depth == 8 and color in (2, 6) and interlace == 0, (path, depth, color)
            bpp = 3 if color == 2 else 4
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw, stride, rows, prev = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append([tuple(line[x * bpp:x * bpp + 3]) for x in range(w)])
        prev = line
    return rows


def region(img, x0, x1, y0, y1):
    return [row[x0:x1] for row in img[y0:y1]]


def changed_rows(a, b, x0, x1):
    return [y for y in range(STATUS_BAR, len(a)) if a[y][x0:x1] != b[y][x0:x1]]


def bands(rows):
    out = []
    for y in rows:
        if out and y == out[-1][1] + 1:
            out[-1][1] = y
        else:
            out.append([y, y])
    return out


class Report:
    def __init__(self):
        self.results = []
        self.notes = []

    def note(self, name, detail):
        """A measured observation, not a pass/fail claim."""
        self.notes.append({"note": name, "detail": detail})
        print(f"NOTE {name}: {detail}")

    def check(self, name, ok, detail=""):
        self.results.append({"check": name, "pass": bool(ok), "detail": detail})
        print(f"{'PASS' if ok else 'FAIL'} {name}{': ' + detail if detail else ''}")


def dump(d):
    return json.loads((d / "dump.json").read_text())


def common(r, name, d):
    j = dump(d)
    r.check(f"{name}: no wait step timed out", not j["wait_timeouts"], str(j["wait_timeouts"]))
    leftover = (d / "leftover-pids.txt").read_text().strip()
    r.check(f"{name}: no firmware helper outlived the IDE", not leftover, leftover)
    return j


def boot(r, d):
    j = common(r, "A boot", d)
    snaps = {s["label"]: s["state"] for s in j["snapshots"]}
    r.check("A boot: session reached Ready", snaps.get("ready", {}).get("firmware_session") == "Ready")
    home = png(d / "home.png")
    r.check("A boot: firmware LCD frame is 320x240", len(home) == 240 and len(home[0]) == 320)
    r.check("A boot: frame is not blank", len({p for row in home[STATUS_BAR:] for p in row}) > 2)
    # B (M4 method): the result glyph of 2+2 is pixel-identical to that of 4,
    # and differs from that of 2+3 (negative control).
    same_result(r, "B arithmetic", d, ("after_2p2", "2+2"), ("after_4", "4"), ("after_2p3", "2+3"))


def same_result(r, name, d, subject, literal, control):
    """M4 method on Home history frames. `subject` and `literal` were each
    entered last; their newest rows differ on the left (the entries) and must
    be identical on the right (the results). `control`'s result must differ."""
    a, b, c = (png(d / f"{f}.png") for f, _ in (subject, literal, control))
    left = bands(changed_rows(a, b, 0, 160))
    r.check(f"{name}: the newest history rows differ ({subject[1]} vs {literal[1]})", bool(left), str(left))
    if left:
        y0, y1 = left[-1][0], left[-1][1] + 1
        same = region(a, 160, 320, y0, y1) == region(b, 160, 320, y0, y1)
        ink = len({p for row in region(b, 160, 320, y0, y1) for p in row}) > 1
        r.check(f"{name}: {subject[1]} result glyph == {literal[1]} glyph", same and ink, f"rows {y0}-{y1 - 1}")
        differs = region(c, 160, 320, y0, y1) != region(b, 160, 320, y0, y1)
        r.check(f"{name}: {control[1]} result glyph differs (control)", differs)


def reference(r, name, actual, ref, mask=None):
    c = frame_diff(actual, REFERENCE / ref, mask and REFERENCE / mask)
    r.check(f"{name}: exact match with {ref}" + (" outside the declared mask" if mask else ""),
            c.get("exact"), f"changed={c.get('changed')} masked={c.get('masked')} bbox={c.get('bbox')}"
            + (f" {c['error']}" if "error" in c else ""))


def boot_reference(r, d):
    reference(r, "A boot: typed WAITING at Home", d / "typed_waiting.png",
              "local-firmware-g1-2.4.2/waiting-waiting.png", "masks/statusbar-clock-battery.png")


def console(j):
    return [m["text"] for m in j["console"]]


def snapshots(j):
    return {s["label"]: s["state"] for s in j["snapshots"]}


def execution(r, d):
    j = common(r, "D-H exec", d)
    s, text = snapshots(j), console(j)
    returned = [t for t in text if t.startswith("G1 firmware returned")]
    r.check("D execution: program ran in the firmware (Running state)",
            s["exec_running"]["firmware_session"] == "Running")
    r.check("D execution: LCD shows the program's own output while running",
            png(d / "exec_running.png")[110][160] != png(d / "exec_done.png")[110][160])
    r.check("D execution: returned 42", returned[:1] == ["G1 firmware returned 42"], str(returned))
    r.check("D execution: back at the firmware UI, session Ready",
            s["exec_done"]["firmware_session"] == "Ready")
    reference(r, "E input: KEYECHO screen", d / "keyecho.png", "local-firmware-g1-2.4.2/keyecho-delay-1500.png")
    r.check("E input: virtual keypad Enter reached the firmware (WAIT returned 30)",
            "G1 firmware returned 30" in returned, str(returned))
    reference(r, "E input: TOUCHECHO screen", d / "touchecho.png", "local-firmware-g1-2.4.2/touchecho-delay-1500.png")
    r.check("E input: LCD mouse tap at (100,50) reached the firmware as a touch",
            "G1 firmware returned {0, 100, 50}" in returned, str(returned))
    r.check("F recovery: the loop was running", s["loop_running"]["firmware_session"] == "Running")
    r.check("F recovery: Stop cancelled it", any("stopped by user" in t for t in text))
    r.check("F recovery: a user Stop is not an error", "Error" not in s["loop_stopped"]["vm_state"],
            s["loop_stopped"]["vm_state"])
    r.check("F recovery: session Ready after Stop", s["loop_stopped"]["firmware_session"] == "Ready")
    r.check("F recovery: next program returned 42", returned[-1:] == ["G1 firmware returned 42"], str(returned))
    r.check("H switch: Native engine ran its program (25)",
            s["native_done"]["worker_outcome"].get("value") == "25")
    sid = {s[k]["firmware_session_id"] for k in ("ready", "native_done", "back_to_firmware")}
    r.check("H switch: the firmware session survived the switch (one session id)", len(sid) == 1, str(sid))
    r.check("H switch: firmware Ready on return", s["back_to_firmware"]["firmware_session"] == "Ready")
    r.check("H switch: no held calculator keys leaked across engines",
            not s["back_to_firmware"]["keys_down"], str(s["back_to_firmware"]["keys_down"]))
    r.check("Unsupported OS services are reported, not hidden",
            all("name" in u for u in j["firmware_unsupported"]), str(j["firmware_unsupported"]))


def catalog(r, d):
    j = common(r, "C catalog", d)
    text = console(j)
    r.check("C catalog: the firmware stored M12HELLO",
            "G1 firmware stored the current source as program M12HELLO (Program Catalog)" in text)
    r.check("C catalog: a second import of the same name is refused, nothing changed",
            "The calculator already has a program named M12HELLO; it was not changed" in text)
    programs = (d / "profile-programs.txt").read_text().split()
    r.check("C catalog: M12HELLO.hpprgm exists in the calculator profile", "M12HELLO.hpprgm" in programs)
    before, after = png(d / "catalog_before.png"), png(d / "catalog_after.png")
    rows = bands(changed_rows(before, after, 0, 320))
    r.check("C catalog: the Program Catalog listing gained a row after the import", bool(rows), str(rows))
    editor, checked = png(d / "editor.png"), png(d / "checked.png")
    r.check("C catalog: tapping the row opened another screen (the program editor)",
            bool(changed_rows(after, editor, 0, 320)))
    r.check("C catalog: the firmware's Check soft key answered on screen",
            bool(changed_rows(editor, checked, 0, 320)))
    same_result(r, "C catalog: M12HELLO run at Home", d, ("after_name", "M12HELLO"), ("after_77", "77"), ("after_78", "78"))


def resize(r, d):
    j = common(r, "G resize", d)
    returned = [t[len("G1 firmware returned "):] for t in console(j) if t.startswith("G1 firmware returned")]
    taps = [s for s in j["snapshots"] if s["label"].startswith("tap_") and not s["label"].endswith("before")]
    r.check("G resize: one firmware result per tap", len(returned) == len(taps), f"{len(returned)}/{len(taps)}")
    for snap, value in zip(taps, returned):
        _, i, x, y = snap["label"].split("_")
        st = snap["state"]
        lo, hi = st["lcd_rect"]["min"], st["lcd_rect"]["max"]
        w, h = hi[0] - lo[0], hi[1] - lo[1]
        kind, rx, ry = (v.strip() for v in value.strip("{}").split(","))
        size = "x".join(str(int(v)) for v in st["window"]["screen_points"])
        r.check(f"G resize: {size} {st['layout']}/{st['calculator_mode']}: LCD keeps 4:3",
                abs(w / h - 4 / 3) < 0.005, f"{w:.1f}x{h:.1f}")
        # 0 MouseDown or 3 MouseClick: both are firmware touch event types.
        r.check(f"G resize: {size}: tap at ({x},{y}) reached the firmware at ({rx},{ry})",
                (rx, ry) == (x, y) and kind in ("0", "3"), f"type {kind}")
    sizes = [tuple(c["pixels"]) for c in j["window_captures"]]
    r.check("G resize: window resized to every requested size", len(set(sizes)) == len(sizes), str(sizes))


def reliability(r, d):
    j = common(r, "I reliability", d)
    s, text = snapshots(j), console(j)
    returned = [t for t in text if t.startswith("G1 firmware returned")]
    cycles = sorted({k.split("_")[0] for k in s})
    ids = []
    for c in cycles:
        n = int(c[1:])
        ok = (s[f"{c}_booting"]["firmware_session"] == "Booting"
              and s[f"{c}_ready"]["firmware_session"] == "Ready"
              and s[f"{c}_stopped"]["firmware_session"] == "Ready"
              and "Error" not in s[f"{c}_stopped"]["vm_state"]
              and returned[2 * n:2 * n + 2] == ["G1 firmware returned 42", f"G1 firmware returned {100 + n}"])
        if f"{c}_off" in s:
            ok = ok and s[f"{c}_off"]["firmware_session"] == "Off" and not s[f"{c}_off"]["firmware_alive"]
        r.check(f"I reliability: cycle {n} ({'power off/on' if f'{c}_off' in s else 'restart'}): "
                "boot, Run, Stop, Run", ok, str(returned[2 * n:2 * n + 2]))
        ids.append(s[f"{c}_ready"]["firmware_session_id"])
    r.check("I reliability: every cycle ran in a new session", len(set(ids)) == len(ids), str(ids))
    r.check("I reliability: stops reported as user stops",
            sum("stopped by user" in t for t in text) == len(cycles))


def navigation(r, d):
    common(r, "Navigation", d)
    same_result(r, "Navigation: Ans", d, ("after_ans", "Ans*3"), ("after_12", "12"), ("after_13", "13"))
    home = png(d / "home_from_settings.png")
    for screen in ("settings", "cas", "apps", "app_open", "help"):
        r.check(f"Navigation: {screen} is a firmware screen other than Home",
                bool(changed_rows(home, png(d / f"{screen}.png"), 0, 320)))
    # Home is the same after CAS, an app and Help (status bar excluded).
    for back in ("home_from_cas", "home_from_app", "home_final"):
        r.check(f"Navigation: {back} restores the same Home screen",
                not changed_rows(home, png(d / f"{back}.png"), 0, 320))
    r.check("Navigation: CAS answered 1+1 on its own screen",
            bool(changed_rows(png(d / "cas.png"), png(d / "cas_1p1.png"), 0, 320)))
    if (d / "held_1.png").exists():
        ink = lambda img: sum(p != (255, 255, 255) for row in img[200:218] for p in row[:300])
        tap, held = ink(png(d / "tap_1.png")), ink(png(d / "held_1.png"))
        r.check("Navigation: a short key press typed into the entry line", tap > 0, f"ink {tap}")
        r.note("Long press (2.5 s on '1')",
               f"entry-line ink {held} vs {tap} for a 0.2 s press: "
               + ("the firmware auto-repeated the held key" if held > 2 * tap else "no auto-repeat observed"))


def perf(r, d):
    j = common(r, "Performance", d)
    s = {x["label"]: x for x in j["snapshots"]}
    text = console(j)
    r.check("Performance: FIREWORKS_G1 (10 KB) loaded and entered its body",
            s["fw_running"]["state"]["firmware_session"] == "Running")
    r.note("FIREWORKS_G1 outcome (authentic firmware)",
           next((t.replace("\n", " | ") for t in text if "m_info" in t), "no runtime error reported"))
    r.note("Large program: Run to body entry",
           f"{(s['fw_running']['t_ms'] - s['fw_requested']['t_ms']) / 1000:.1f} s (10 KB source: loader, catalog, Check, invocation)")
    r.check("Performance: animation entered its body", s["running"]["state"]["firmware_session"] == "Running")
    t0, t1 = s["anim_t0"], s["anim_t1"]
    dt = (t1["t_ms"] - t0["t_ms"]) / 1000
    frames = t1["state"]["firmware_frame_generation"] - t0["state"]["firmware_frame_generation"]
    r.check("Performance: changed LCD frames kept arriving while it animated", frames > 0, f"{frames} in {dt:.1f} s")
    r.note("LCD frame delivery while animating", f"{frames / dt:.1f} changed frames/s over {dt:.1f} s")
    r.note("Boot to Ready", f"{s['ready']['t_ms'] / 1000:.1f} s after the script started")
    r.note("Small program: Run to body entry",
           f"{(s['running']['t_ms'] - s['run_requested']['t_ms']) / 1000:.1f} s")
    r.check("Performance: Stop ended the animation, session Ready",
            s["stopped"]["state"]["firmware_session"] == "Ready")


def onbreak(r, d):
    j = common(r, "ON break", d)
    s, text = snapshots(j), console(j)
    r.check("ON break: the program was running", s["running"]["firmware_session"] == "Running")
    r.check("ON break: the calculator's ON key ended it as a user stop",
            "Error" not in s["after_on"]["vm_state"] and any("stopped by user" in t for t in text),
            s["after_on"]["vm_state"])
    r.check("ON break: the next Run returned 42", "G1 firmware returned 42" in text)


CHECKS = {"boot": lambda r, d: (boot(r, d), boot_reference(r, d)), "exec": execution, "catalog": catalog, "resize": resize, "reliability": reliability, "nav": navigation, "perf": perf, "onbreak": onbreak}


def main():
    run = Path(sys.argv[1])
    names = sys.argv[2:] or [n for n in CHECKS if (run / n / "dump.json").exists()]
    r = Report()
    for name in names:
        d = run / name
        if not (d / "dump.json").exists():
            r.check(f"{name}: dump exists", False, str(d))
            continue
        CHECKS[name](r, d)
    passed = sum(x["pass"] for x in r.results)
    print(f"{passed}/{len(r.results)} checks passed")
    (run / "assert_m12.json").write_text(json.dumps({"checks": r.results, "notes": r.notes}, indent=1))
    sys.exit(0 if passed == len(r.results) else 1)


if __name__ == "__main__":
    main()
