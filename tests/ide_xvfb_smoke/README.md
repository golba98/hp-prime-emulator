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
- `cargo build --workspace --release` (the runner uses `target/release/ppl`)

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
| `script_runconfig.json` | Run a parameterized exported function via run configuration | `SQUARE(5)` returns `25` |
| `script_editor.json` | Editor: valid PPL with unemulated commands, a broken program, typed code (auto-indent, pairing), window sweep 800×500 → 3840×2160 with real window screenshots, breakpoint, Problems panel | `assert_editor.py`: only Compatibility diagnostics on valid code; unclosed string is a Syntax error on its line; typed block indented; cursor and layout cache unchanged by every resize |
| `script_editor_hidpi.json` | The same at `WINIT_X11_SCALE_FACTOR=2` | screenshots are 2× the logical size; cursor and layout cache unchanged |

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
