# IDE Xvfb Smoke Tests

Scripted end-to-end tests of the **real desktop IDE event loop** (eframe/egui
window, menus, editor, VM worker, framebuffer) on a private Xvfb display.
They never touch the user's real display: `WAYLAND_DISPLAY` is unset and
`DISPLAY` points at a freshly started Xvfb server.

The scripts drive the production `IdeAction` code paths via the opt-in
`PPL_IDE_AUTOMATION` environment hook (see
`crates/ppl-gui/src/ide/automation.rs`). On `exit` the IDE dumps its
observable state (console, problems, worker outcome, tabs, paused line) as
JSON, which the assertions below check.

## Prerequisites

- `Xvfb`, ImageMagick `import`/`magick`
- `cargo build --workspace --release` (the runner uses `release/ppl` in Cargo's
  target directory, as reported by `cargo metadata`; set `PPL_BIN` to override)

## Run

```bash
cd tests/ide_xvfb_smoke
./run_ide_auto.sh scripts/script_workflow.json 60
./run_ide_auto.sh scripts/script_syntax.json   60
./run_ide_auto.sh scripts/script_debug.json    60
./run_ide_auto.sh scripts/script_gfx.json      60
./run_ide_auto.sh scripts/script_input.json    60
./run_ide_auto.sh scripts/script_stop.json     60
./run_ide_auto.sh scripts/script_runconfig.json 60
python3 assert.py /tmp/ppl-ide-e2e/dump_workflow.json /tmp/ppl-ide-e2e/dump_syntax.json \
        /tmp/ppl-ide-e2e/dump_debug.json /tmp/ppl-ide-e2e/dump_gfx.json \
        /tmp/ppl-ide-e2e/dump_input.json /tmp/ppl-ide-e2e/dump_stop.json \
        /tmp/ppl-ide-e2e/dump_runconfig.json
```

(The dumps are written to the paths named in each script, by default under
`/tmp/ppl-ide-e2e/`; the runner prints the IDE exit code and elapsed time.)

## Scenarios

| Script | Workflow exercised | Pass condition |
|---|---|---|
| `script_workflow.json` | New Program → write PPL → Save → Close → Reopen → Run → output | console contains `30`; worker success value `30`; file on disk contains the source |
| `script_syntax.json` | Live diagnostics on a syntax error → fix → run | first buffer produces an error diagnostic; after the fix the run prints `10` |
| `script_debug.json` | Breakpoint → Debug (pause) → Step Over → Continue ×2 → finish | pauses on line 6; run finishes with `120` printed |
| `script_gfx.json` | RECT_P / TEXTOUT_P → framebuffer → PNG screenshot | PNG is 320×240, red background, blue box, yellow text |
| `script_input.json` | Interactive `INPUT` dialog with typed keys + Enter | user input 21 → program prints and returns `42` |
| `script_stop.json` | Run a `WAIT`-loop → Stop | worker interrupted (`--- Execution stopped by user ---`), prompt clean exit |
| `script_compile_gate.json` | Valid program runs → source broken (unsaved) → live squiggle/Problems/status → Run → fixed → Run; window screenshots `gate_*.png` | `assert_compile_gate.py`: broken source never runs (`Run blocked`, no `second` output, old outcome not re-reported); fixed program returns `3`; final state `Valid`, `Problems (0)` |
| `script_baseconv.json` | `tests/fixtures/baseconv/Baseconv_fixed.hpprgm`: typed hex string `FF`, invalid base `abc` (re-prompt with an error line), then a second run with `7B.8` | `assert_baseconv.py`: console has `Final Result: 255`; second run returns `01111011.1000`; no problems |
| `script_baseconv_cancel.json` | The same program, Esc in its third dialog | `assert_baseconv.py`: returns `0`, no result printed |
| `script_runconfig.json` | Run a parameterized exported function via run configuration | `SQUARE(5)` returns `25` |
| `script_editor.json` | Editor: valid PPL with unemulated commands, a broken program, typed code (auto-indent, pairing), window sweep 800×500 → 3840×2160 with real window screenshots, breakpoint, Problems panel | `assert_editor.py`: only Compatibility diagnostics on valid code; unclosed string is a Syntax error on its line; typed block indented; cursor and layout cache unchanged by every resize |
| `script_editor_hidpi.json` | The same at `WINIT_X11_SCALE_FACTOR=2` | screenshots are 2× the logical size; cursor and layout cache unchanged |

| `script_firmware_check.json` | Native backend, original-firmware Check (fresh checker): Unicode program rejected at 4:15 beside the native 4:11 → edit → passing Check → second tab → repeated Checks → Stop during a Check → Run, Restart; window screenshots `firmware_check_*.png` | `assert_firmware_check.py`: firmware items listed once and marked on the reported character; edits retire them; tabs never share results; only a firmware pass says so; Stop leaves nothing pending; after the 15 s warm-up, Checks use the direct firmware compiler call, each under 1 s; process samples |
| `script_firmware_session.json` | The same with the G1 Firmware backend (live session), plus a session Run of a rejected program | `assert_firmware_check.py`: as above, and the session's rejection reaches Problems at the firmware position for the current revision |

The firmware scenarios need the original firmware and must never use the
user's calculator profile. Run them with an isolated `HOME`,
`XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `PPL_FIRMWARE_PROFILE` (a copy of the
initialized data directory; factory A alone stops at first-run setup), the
`PPL_FIRMWARE*` variables from `docs/compatibility/diagnostic-parity.md`, and
optionally sample the IDE's runner children to pass as the process log:

```bash
python3 assert_firmware_check.py /tmp/ppl-ide-e2e/firmware_check.json \
        [samples.log pids-before.txt pids-after.txt]
```

### M12: the G1 Firmware calculator

`run_m12.sh <scenario>` runs `scripts/script_m12_<scenario>.json` (or any
firmware script given by path) with an isolated `HOME`/XDG, a fresh calculator
profile seeded from the configured initialized data, the asset paths of
`~/.config/hp-prime-emulator/firmware.toml` and the workspace's own
`prime-emulator`. Evidence (dump, LCD frames, window screenshots,
`session.log`, the profile's program list, helper processes before/after) goes
to `artifacts/m12/<UTC time>/<scenario>/` (`M12_RUN` overrides).

| Scenario | Exercises |
|---|---|
| `boot` | A: start, Ready, Home frame; typed `WAITING` vs the masked Home reference; B: `2+2`, `4`, `2+3` |
| `exec` | D: editor Run with LCD output and result; E: KEYECHO/TOUCHECHO via the on-screen keypad and an LCD mouse click, frames vs references; F: endless loop, Stop, next Run; H: Native run and back |
| `catalog` | C: Import to calculator, refusal of an existing name, Program Catalog, open by tap, firmware Check, run by name at Home |
| `resize` | G: six window sizes and layouts; a TOUCHECHO tap each, at a different pixel (needs `PPL_XVFB_SCREEN=3840x2160x24`) |
| `reliability` | I: five restart or power-off/on cycles of boot, Run, Stop, Run |
| `nav` | Ans, history, Home Settings, CAS `1+1`, Apps, an app, Help, short and long key presses |
| `perf` | FIREWORKS_G1 (10 KB) through the live loader; LCD frame delivery while a program animates; Stop |
| `onbreak` | The calculator's ON key (on-screen keypad) breaks an IDE run; the next Run works |

```bash
for s in boot exec catalog nav reliability perf onbreak; do M12_RUN=$PWD/../../artifacts/m12/run ./run_m12.sh $s 900; done
PPL_XVFB_SCREEN=3840x2160x24 M12_RUN=$PWD/../../artifacts/m12/run ./run_m12.sh resize 900
python3 assert_m12.py ../../artifacts/m12/run
```

### M13: storage and power state

`script_m13_*.json` (found by `run_m12.sh m13_<name>`). `M12_PROFILE` keeps
one calculator profile across runs, for closing and reopening the IDE:

```bash
R=$PWD/../../artifacts/m13/run; mkdir -p $R/profiles
for s in m13_persist m13_reopen; do M12_PROFILE=$R/profiles/calc M12_RUN=$R ./run_m12.sh $s 900; done
M12_RUN=$R ./run_m12.sh m13_crash 900
M12_PROFILE=$R/profiles/calc M12_RUN=$R ./run_m12.sh m13_reset 900
python3 assert_m13.py $R
```

| Scenario | Exercises |
|---|---|
| `m13_persist` | Variables, list, matrix, Home setting, Function F1, Home/CAS history, then Power off (sleep) and ON, then Shut down and Start; checks the four screens, Ans and the stored values |
| `m13_reopen` | A new IDE on the same profile: the same checks |
| `m13_crash` | The helper killed without OFF (a battery pull), then a reboot |
| `m13_reset` | Reset memory: backup kept, values and history cleared |

`run_ide_auto.sh` gives the IDE window keyboard focus with `x11_focus.py`:
Xvfb has no window manager, and the automation only advances while the
window is focused.

Editor scenarios need a large virtual screen:

```bash
PPL_XVFB_SCREEN=3840x2160x24 ./run_ide_auto.sh scripts/script_editor.json 120
PPL_XVFB_SCREEN=3840x2160x24 WINIT_X11_SCALE_FACTOR=2 \
  ./run_ide_auto.sh scripts/script_editor_hidpi.json 120
python3 assert_editor.py /tmp/ppl-ide-e2e/dump_editor.json /tmp/ppl-ide-e2e/dump_editor_hidpi.json
```

The screenshots (`/tmp/ppl-ide-e2e/editor_*.png`) are written by the IDE
itself (`window_screenshot` step, egui `ViewportCommand::Screenshot`).
Other automation steps: `focus_editor`, `type`, `key`, `goto`, `select`,
`select_tab`, `dock`, `font_size`, `resize`, `wait_ms`.

## Notes

- The scripts intentionally use only one X display number range (300–380)
  per run; concurrent runs pick free numbers.
- `script_visual.json` is a helper for manual window screenshots (capture
  with `import -display :N -window root shot.png` while it runs).
- The debugger scenario needs `continue` twice after a breakpoint: the
  breakpoint on the loop body fires again on the next iteration, which is
  correct breakpoint semantics.

Baseconv scenarios:

```bash
./run_ide_auto.sh scripts/script_baseconv.json 90
./run_ide_auto.sh scripts/script_baseconv_cancel.json 60
python3 assert_baseconv.py /tmp/ppl-ide-e2e/dump_baseconv.json /tmp/ppl-ide-e2e/dump_baseconv_cancel.json
```

Compile gate (the compiler decides what Run executes):

```bash
./run_ide_auto.sh scripts/script_compile_gate.json 60
python3 assert_compile_gate.py /tmp/ppl-ide-e2e/dump_compile_gate.json
```
