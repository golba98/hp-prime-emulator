# M12 — Genuine HP Prime G1 Full Calculator Mode

Date: 2026-10-29. Branch `feat/m12-full-calculator` (canonical checkout, from
`main` 8029e30; claim `agent-claims/m12`). No commit, push or merge was made.
No physical calculator was contacted: no USB command, key event, file
transfer or device read happened at any point.

Firmware: G1 2.4.2, `armfir.elf` SHA-256 `ad9779ff…2fb39`, run by the
existing Unicorn emulator (`prime-emulator`). Every result below is **local
firmware** evidence. **Physical-device parity: NOT VERIFIED** for all of it.

## 1. What was already there, and what M12 changed

Investigation found most of the pipeline in place since M10. The live session
(`prime-emulator session`, framed stdin/stdout), the IDE's `FirmwareClient`
worker, the frame streaming (changed frames only, XRGB, 16 ms floor), the
key/touch routing, the Full / Screen Only / Expand layouts, and the
session's Check/Run/Stop jobs were all reused. M12 adds no second firmware
system. It completes and hardens that one:

```
IDE (egui)                                     prime-emulator session (Unicorn)
 calculator panel ── session bar: state chip,   LiveSession::tick (device thread)
   Start / Power off / Restart / Import / SVC     Lifecycle: Booting ▸ Ready (UI idle meter)
 FirmwareClient (worker thread, one per          ▸ Stopping ▸ Ready (break settled)
   session_id; owns the helper's process group)  Unsupported: first hit of each call site
   ◄── Event::Lifecycle / Unsupported / ...      key queue: one transition per GetEvent
   ──► Command::Key / Touch / Run / Import ...    Jobs: CheckLoader ▸ Load ▸ [CheckImporter
 LCD texture reset per session (generation 1)        ▸ Import] | CheckTarget ▸ Invoke ▸ Execute
                                                  catalog order learned from user browsing too
```

Gaps found and fixed (pre-existing unless noted):

| # | Problem | Fix |
|---|---|---|
| G1 | No explicit session state, no separate session controls | `FirmwareSession` {Off, Booting, Ready, Running, Stopping, Failed, Unavailable} driven by new `Event::Lifecycle`; Start / Power off / Restart in the calculator panel and the Run menu |
| G2 | A restarted session's LCD froze: frames restart at generation 1, the texture accepted only higher generations | The texture is reset whenever the session id changes; the LCD is dark when off |
| G3 | Restart would have booted a *different* profile: the old helper still held the profile lock | The new session's worker first shuts the previous one down (bounded) and only then locks the same profile |
| G4 | Unsupported OS services silently returned 0 | Each first hit of a (service, call site) is sent as `Event::Unsupported`, shown in the console and as an `⚠ N SVC` badge with ids, names and callers. The substitution policy is unchanged, but now visible |
| G5 | Desktop key auto-repeat sent extra key-downs | Repeat events (and their text) are dropped; a held key stays down for the firmware |
| G6 | IDE shortcuts (Ctrl+S/N/W…, F-keys) fired while the calculator owned the keyboard (Ctrl is also its Alpha) | With the firmware calculator focused only Shift+F5 (Stop) stays an IDE shortcut; clicking the editor returns the keyboard |
| G7 | Closing the IDE did not wait for the helper | `on_exit` shuts every session down with a bounded wait, then kills the helper's process group |
| G8 | Key bursts were lost: typing `WAITING` at once reached the firmware as `WA` | The session gives the firmware one key transition at a time, after it took the previous one (as verified key scripts do), with a 50 ms fallback for programs that never ask for events |
| G9 | After the user opened the Program Catalog, every later IDE Run/Check/Import failed: "catalog order could not be verified (observed [])" | The session also learns the catalog order from catalog listings the user opens; when nothing was re-read, a complete earlier order stands. A wrong row is still rejected by the source validation (no verdict, never a wrong one) |
| G10 | A user Stop was shown as VM "Error" | Shown as stopped (`--- Firmware program stopped by user ---`), state idle |
| G11 | Missing assets, runner, profile lock or helper version looked like a crash | `Unavailable: <reason>` (version mismatch names both protocol versions) |
| G13 | **IDE Run/Import of larger programs failed** in the live session: the loader read the whole UTF-16 source as one byte list, and the firmware answered `Error: Insufficient memory` (FIREWORKS_G1, 10 KB). Appending character by character then took over 20 s | Chunked loader: 4 KB `AFilesB` reads, one `CHAR(list)` per chunk. A profile's M10 loader, recognised by its exact text, is upgraded before boot; any other `ZZT_LOADER` is still refused. The observed loader/importer invocations may run past the key-settle window, as Run invocations already could |
| G14 | Pressing the calculator's own ON key during an IDE run reported a firmware runtime error | An ON press while the IDE's program runs ends the run as stopped by the user; the firmware shows its own "Program interrupted" |
| G15 | A Run while the calculator itself was busy (e.g. ENTER on an empty Home line repeats the last entry, an endless loop) failed with only a settle timeout | The error now says that the calculator is busy and to press Stop or ON |
| G12 | **All Xvfb IDE scenarios hung (pre-existing since #21)**: the automation advances only while the window is focused, and a bare Xvfb never focuses a window | `run_ide_auto.sh` focuses the IDE window with `x11_focus.py` (libX11 via ctypes). Verified against pristine `main`, which hangs the same way |

New: **Import to calculator** stores the active tab as a new calculator program
named after it, through the firmware's own `Programs(name):=` (a reserved
`ZZT_IMPORT` helper, installed like `ZZT_LOADER`). It refuses an existing name
(case-insensitive) and reserved `ZZT_` names, and never replaces a program.

## 2. Files

Modified:

- `crates/ppl-engine/src/session.rs`: protocol v2: `Lifecycle`, `Unsupported`, `Import`
- `crates/ppl-engine/src/firmware.rs`, `firmware_unsupported.rs`: `kill_process_group`
- `crates/prime-emulator/src/session.rs`: lifecycle, unsupported reporting, key pacing, `ZZT_IMPORT` bootstrap
- `crates/prime-emulator/src/session/job.rs`: `IdleMeter`, Import stages, user catalog observation, `validate_import`
- `crates/prime-emulator/src/trace.rs`: first-hit unsupported call sites
- `crates/ppl-gui/src/ide/firmware.rs`: session ids, typed startup failures, bounded shutdown, restart hand-off
- `crates/ppl-gui/src/app_firmware.rs`: session state machine, start/restart/power off, import, input fixes
- `crates/ppl-gui/src/app.rs`: actions, LCD reset per session, exit shutdown, user-stop result
- `crates/ppl-gui/src/ide/mod.rs`: `FirmwareSession`, actions
- `crates/ppl-gui/src/ide/calculator.rs`: session bar
- `crates/ppl-gui/src/ide/display.rs`: LCD state text, pixel-perfect Screen Only, tests
- `crates/ppl-gui/src/ide/menu.rs`: shortcut ownership, Run-menu session items
- `crates/ppl-gui/src/ide/automation.rs`: M12 steps and dump fields
- `tests/ide_xvfb_smoke/run_ide_auto.sh`, `README.md`
- `docs/development/worktree-ownership.md`

New:

- `crates/prime-emulator/tests/firmware_session.rs`
- `tests/ide_xvfb_smoke/run_m12.sh`, `assert_m12.py`, `x11_focus.py`
- `tests/ide_xvfb_smoke/scripts/script_m12_{boot,exec,catalog,resize,reliability,nav,perf,onbreak}.json`

## 3. Test results

### 3.1 M12 IDE suite (real IDE event loop under Xvfb, live G1 firmware)

Three complete runs, each with all eight scenarios in a fresh isolated
profile: `artifacts/m12/20261029T185139Z-run{1,2,3}/` (dumps, LCD frames,
window captures, `session.log`, `ide-reports/`, helper process lists,
`assert.txt`, `assert_m12.json`).

| Scenario | Checks | Run 1 | Run 2 | Run 3 |
|---|---:|---:|---:|---:|
| A boot + B arithmetic (`boot`) | 9 | 9 | 9 | 9 |
| D execution, E input, F recovery, H engine switch (`exec`) | 20 | 20 | 20 | 20 |
| C Program Catalog and import (`catalog`) | 11 | 11 | 11 | 11 |
| G resizing (`resize`) | 16 | 16 | 16 | 16 |
| I reliability (`reliability`) | 9 | 9 | 9 | 9 |
| Navigation (`nav`) | 15 | 15 | 15 | 15 |
| Performance (`perf`) | 6 | 6 | 6 | 6 |
| ON break (`onbreak`) | 5 | 5 | 5 | 5 |
| **Total** | **91** | **91** | **91** | **91** |

**273/273 checks passed, 0 failed.** Across the 24 IDE sessions, no
firmware helper outlived its IDE.

What the checks establish (local firmware):

- **A Boot.** The session reaches Ready and the LCD shows a 320×240 firmware
  frame. With `WAITING` typed at Home, the frame is an **exact match** with
  the verified M10 reference `waiting-waiting.png` outside the declared
  status-bar mask (clock and battery, 279 px).
- **B Arithmetic** (M4 method, no hard-coded pixels). The result glyph of
  `2+2` is pixel-identical to that of `4`, and differs from that of `2+3`.
- **C Catalog.**
  - Import stored `M12HELLO` through `Programs():=`, with the exact source.
  - A second import of the same name was refused and nothing changed.
  - The Program Catalog gained the row. Tapping it opened the firmware
    editor, and the editor's Check soft key answered "No errors in the
    program".
  - `M12HELLO` typed at Home returned the same glyph as `77`, and a
    different one from `78`.
- **D Execution.**
  - The editor's Run executed in the firmware (state Running), with the
    program's own LCD output on screen.
  - `G1 firmware returned 42`, and the calculator was back at Home in state
    Ready.
- **E Input.**
  - KEYECHO's and TOUCHECHO's screens are **exact matches** with their
    verified references.
  - Enter clicked on the virtual keypad returned `30`.
  - A mouse click on the LCD at (100,50) returned `{0, 100, 50}`.
- **F Recovery.** An endless loop was Running. Stop ended it, reported as a
  user stop rather than an error, the state went back to Ready, and the next
  Run returned 42.
- **G Resizing.** Six window sizes from 800×600 to 3840×2160, in the
  Split/Calculator layouts and Full/Screen Only modes:
  - The LCD keeps 4:3 in every case.
  - Every tap reached the firmware at exactly the requested pixel, from the
  corners (1,1) and (318,238) to the centre.
  - Screen Only uses whole physical scales: 1× at 1920×1080, 8× at
    3840×2160.
- **H Engine switch.**
  - Native ran its own program (25).
  - Switching back found the same firmware session (one session id),
    Ready, with no held keys.
  - A unit test covers the key routing: firmware keys never reach the
    native VM, and native keys never reach the session.
- **I Reliability.** Five cycles per run (restart, power off/on, restart, …),
  each booting, running 42, being stopped and running again: 15/15
  cycles. Every cycle ran in a new session on the same profile.
- **Navigation.**
  - Home `Ans*3 → 12` (glyph method). History selection showed the
    firmware's Copy/Show soft keys, and Enter copied the entry.
  - Home Settings, CAS (`1+1 → 2`), the Application Library, the Function
    app's Symbolic View and Help are each a firmware screen.
  - Home is pixel-identical after leaving CAS, the app and Help.
- **ON break.** The calculator's own ON key, clicked on the virtual keypad,
  ended an IDE run as a user stop (the firmware shows "Program
  interrupted"), and the next Run returned 42.

### 3.2 Other suites

| Suite | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | **1589 passed, 0 failed**, 6 ignored. An earlier full run had 1 failure: the known `ppl-programs` journal-lock flake, which passed 18/18 serially twice |
| `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --check` | clean |
| Firmware-gated `prime-emulator` tests (incl. new `firmware_session`) | **24/24 passed**: app_suite 2, firmware_boot 1, firmware_compiler 5, firmware_display 4, firmware_programs 3, firmware_runtime 8, firmware_session 1. The last covers Hello v2, Booting, named unsupported services, Ready, frames, import, refusal, the v1 loader upgrade, a 10 KB import and shutdown |
| Existing firmware IDE suites (isolated): `firmware_runtime` / `firmware_check` / `firmware_session` | 9/9, 27/27, 31/31 PASS (after all M12 changes; `artifacts/m12/20261029T185139Z-regression/`) |
| Pristine `main` 8029e30 IDE Xvfb control (`script_workflow`) | hangs (G12), before M12; passes with the focus helper |

New unit tests:

- `ppl-engine`: protocol round trip of the new variants.
- `prime-emulator`: import name rules; catalog order when nothing was re-read.
- `ppl-gui`:
  - session state transitions
  - Unavailable vs Failed
  - LCD reset for a replacement session
  - restart/power-off hand-off
  - desktop auto-repeat filtering
  - engine-switch key isolation
  - shortcut ownership
  - pixel-perfect sizing
  - touch projection at fractional scales
  - program names

## 4. Screenshots (genuine firmware frames from the IDE)

The screenshots are kept locally and are not in the repository
(`docs/screenshots/` is git-ignored). Every run regenerates them:
`artifacts/m12/<run>/<scenario>/*.png`. The copies kept for this report, in
`docs/screenshots/m12/`, are:

- `01_firmware_home.png`, `02_home_arithmetic.png`
- `03_program_catalog.png`, `04_catalog_editor.png`, `05_firmware_check.png`, `06_imported_program_run.png`
- `07_ide_program_lcd.png`, `08_ide_program_result.png`, `09_cas.png`, `10_apps.png`
- `11`–`13`: IDE windows

## 5. Performance and responsiveness

Measured in the three final runs (Xvfb, software OpenGL; local firmware):

| Measure | Run 1 | Run 2 | Run 3 |
|---|---:|---:|---:|
| Boot to Ready (fresh IDE, after selecting G1 Firmware) | 2.0 s | 1.9 s | 2.0 s |
| Restart to Ready (5 cycles each) | 0.91–0.97 s | 0.93–0.96 s | 0.90–1.00 s |
| Changed LCD frames delivered while a program animates | 30.9/s | 32.0/s | 31.7/s |
| Small program: Run to body entry (after a large one) | 13.3 s | 13.9 s | 13.6 s |
| 10 KB program: Run to body entry | 43.2 s | 41.7 s | 41.8 s |

- The UI never blocks on the firmware. All session I/O runs on the client's
  worker and reader threads, and the GUI only polls events and swaps the
  newest frame.
- Only changed frames are sent (16 ms floor) and uploaded: a frame is
  uploaded only when its generation advances.
- Without automation the IDE repaints on demand only: about 30 frames in
  22 s while booting and checking. The automation pump repaints by itself
  (≈280 frames/s), so that figure is not a product measure.
- Frame capture uses the existing snapshot gate. It never stalls the guest:
  capture is skipped while busy, and the M10 fair request applies.
- Key pacing adds at most one firmware event-loop turn per queued key. A
  single key goes straight through.
- Run latency is dominated by the session's verified path: the loader, the
  catalog walk to the program, the firmware Check, then typing the call at
  Home. For a 10 KB source the loader alone takes about 25–30 s.

## 6. Firmware / OS-service limitations and open items

- Unsupported services hit on every boot: `0x10293 _FindResourceW`
  (caller `0x30604F90`) and `0x102CB GetApplicationHeadInfoA` (caller
  `0x30615910`). Both return 0. Every screen exercised here still worked.
  Nothing else was hit during Home, Catalog, editor, Check, CAS, Apps, the
  Function app, Help, Settings or program runs.
- The session's helper invocations (`ZZTIDELOAD`, `ZZTIDEIMPORT`) appear in
  the Home history, with their return values. They are real Home
  evaluations, and the Home history is part of the calculator state.
- `FIREWORKS_G1` (the project's published demo, with `BLIT_P(G0)`) now
  loads and starts from the IDE. It then stops on its first frame with the
  firmware's own `Error: Invalid input` at `mx:=m_info(1,1)`: with no finger
  down, G1 `MOUSE` returns a non-empty list without that element, so its
  `IF SIZE(m_info) > 0` guard passes. This is authentic program behaviour on
  the firmware, not an emulator fault.
- Loading a 10 KB source through the live loader takes about 25–30 s, and
  Run to body entry about 45 s. Small programs reach their body in a few
  seconds.
- Only the Function app's Symbolic view was opened. The other apps,
  graphing, Plot/Num views, CAS beyond `1+1`, and statistics/spreadsheet
  data entry were **not exercised** and are not claimed. CAS `1+1 → 2`
  rests on visual inspection plus a changed-screen check, not on a pixel
  reference.
- Import picks the catalog row from the observed listing order. If the
  order cannot be verified, the job fails with no verdict.
- Stopping is usually brief (≈ 70 ms): the firmware's break ends the
  invocation and the session reports Ready once its observer closes.
- Touch: one tap of six came back from `WAIT(-1)` as type 3 (MouseClick)
  instead of 0 (MouseDown). Both are firmware event types; the coordinates
  were exact. Not investigated further.
- While a Check or Run is compiling, the chip still says Ready; a second
  request is refused by the session as busy.
- **Key auto-repeat is not provided.** Holding Backspace or Left for 3 s acts
  once, and holding a digit for 2.5 s types one digit. On the calculator,
  held editing keys repeat; in the original OS that is keyboard-driver
  behaviour, which the host reimplements and whose delay and rate are not
  known from firmware evidence. It is reported as missing rather than
  invented. The desktop's own auto-repeat is deliberately not forwarded.
- Clock and timers are host wall-clock, not cycle-deterministic (as in M10).
- Status-bar clock/battery pixels are excluded by the declared M10 mask in the
  one masked comparison; all other reference comparisons are exact.
- Known unrelated flake: `ppl-programs` transactions "operation journal is
  locked" under parallel tests (passes serially).

## 7. Reproduction

```bash
cargo build --workspace --release
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
F=~/.local/share/hp-prime-emulator/firmware
PRIME_TEST_FIRMWARE=$F/armfir.elf PRIME_TEST_FACTORY_A=$F/factory-A PRIME_TEST_OS_ROM=$F/PRIME_OS.ROM \
PRIME_TEST_INIT_DATA=$F/initial-data PRIME_TEST_RUNNER=<target>/release/prime-emulator \
  cargo test --release -p prime-emulator --test firmware_session --test firmware_runtime \
    --test firmware_programs --test firmware_compiler --test firmware_boot --test firmware_display --test app_suite
cd tests/ide_xvfb_smoke
R=$PWD/../../artifacts/m12/run
for s in boot exec catalog nav reliability perf onbreak; do M12_RUN=$R ./run_m12.sh $s 900; done
PPL_XVFB_SCREEN=3840x2160x24 M12_RUN=$R ./run_m12.sh resize 900
python3 assert_m12.py $R
# Existing firmware IDE suites, isolated the same way:
for s in firmware_runtime firmware_session firmware_check; do M12_RUN=$R ./run_m12.sh scripts/script_$s.json 900; done
python3 assert_firmware_runtime.py /tmp/ppl-ide-e2e/firmware_runtime.json
python3 assert_firmware_check.py /tmp/ppl-ide-e2e/firmware_check.json
python3 assert_firmware_check.py /tmp/ppl-ide-e2e/firmware_session.json
# Interactive:
cargo run --release --bin ppl -- gui     # Run ▸ Engine ▸ G1 Firmware
```
