# HP Prime Emulator

[![Rust CI](https://github.com/golba98/hp-prime-emulator/actions/workflows/rust.yml/badge.svg)](https://github.com/golba98/hp-prime-emulator/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Write, check, debug, and run HP Prime PPL programs on your desktop. The project includes a graphical editor, a native Rust execution engine, and a CLI for working with programs and a connected calculator.

The native engine runs without a calculator or firmware files. An optional Linux backend runs separately supplied HP Prime G1 firmware. This is an independent project and is not affiliated with HP.

![PPL editor and calculator display](assets/ide-display-output.png)

## Build and open the GUI

Install stable Rust with Cargo, your platform's C/C++ build tools, and HID development dependencies. Linux builds need `pkg-config` and `libudev` development files. The GUI needs a graphical desktop session.

From the repository directory:

```bash
cargo build --release --bin ppl
cargo run --release --bin ppl -- gui
```

This opens the desktop IDE. To open an existing program:

```bash
cargo run --release --bin ppl -- gui examples/FIREWORKS.hpprgm
```

The IDE uses the native engine. It is separate from the viewer for a physical calculator.

**Full Calculator** keeps the housing, LCD and keypad together as the panel shrinks; **Expand** makes the keys larger, and **Screen Only** remains an explicit display mode. The Native calculator has a session with Home expression evaluation, typed Ans, history and a program catalog populated from the current open editor tabs. Its settings and calculator variables survive successive runs within that IDE session. This is a limited Native environment; CAS, built-in calculator apps and full G1 navigation are not provided by it.

After a Native program finishes, **Esc** with calculator focus leaves its output and returns to the view it was launched from. Home evaluates expressions using the Native PPL engine; the catalog runs the current editor revision. **Return to editor** changes the layout separately. During execution, Esc keeps its program/dialog meaning, including cancellation of INPUT and delivery to GETKEY/WAIT. **Shift+F5** explicitly stops execution. An IDE dialog owns its own Esc press. The physical keyboard and on-screen calculator keys target the local selected engine; they do not send keys to a USB calculator.

## Use the CLI

Run these commands from the repository directory. Cargo reuses the built executable when the source has not changed.

```bash
# Check a program without running it
cargo run --release --bin ppl -- check examples/HELLO.ppl

# Run a program and print its output
cargo run --release --bin ppl -- run examples/HELLO.ppl

# Select a function and pass JSON arguments (returns 49)
cargo run --release --bin ppl -- run examples/SQUARE.ppl \
  --engine native --entry SQUARE --args '[7]' --json

# Save the program's final screen
cargo run --release --bin ppl -- run examples/FIREWORKS.hpprgm --png output.png

# Start the interactive debugger
cargo run --release --bin ppl -- debug examples/HELLO.ppl

# Browse commands and options
cargo run --release --bin ppl -- --help
cargo run --release --bin ppl -- run --help
```

Source loading supports PPL text and recognized HP Prime program containers. Use `commands` to inspect built-in command support and `compare --help` for compatibility testing options.

## Send a program to your calculator

Connect the HP Prime over USB. The status bar shows **Calculator ready** once it is detected. In the IDE, click **Send to calculator** (Ctrl+Shift+U):

1. The program is saved and checked. Errors stop the send and open the Problems panel.
2. The first time, you confirm the program's name on the calculator. The IDE remembers it for that file.
3. A new name is created on the calculator and verified by reading it back.
4. If the name already exists, the IDE backs up the calculator's copy and asks before replacing it.

Features the emulator can't test are listed before sending. After a send, **Compare screens** shows the calculator's screen next to the emulator's. Start the program on the calculator yourself first, because the IDE never presses calculator keys. The Explorer's **On Calculator** section lists the calculator's programs (the list may be incomplete) and can open a copy of one in a new tab. **File → Open Folder…** chooses the folder the Explorer shows.

The toolbar's **Compare screens** toggle (also **View → Compare Screens Panel**) keeps the comparison beside the editor and remembers its visibility after restarting. Click **Refresh both** to capture the selected emulator engine and read the physical screen. The panel labels the engine and capture times, warns when the emulator frame changes, and compares the full 320×240 images in RGB565 with optional magenta highlighting. Captures are sequential snapshots; refresh after changing either display. A restart clears the images and asks for a fresh capture.

## View a physical calculator

Connect the calculator over USB, then open its screen viewer:

```bash
cargo run --release --bin ppl -- device mirror
```

To save a screenshot instead:

```bash
cargo run --release --bin ppl -- device screenshot screen.png
```

These commands read the physical screen; they do not run the desktop emulator. USB access must be configured on the host. See the [live viewer guide](docs/device/live-screen.md) for permissions, scaling, controls, and capture options.

Full-screen capture speed is limited by the calculator's response. Normal incremental mirroring remains disabled pending changed-screen and sustained-stream hardware verification. See [viewer quality and performance](docs/device/LIVE_VIEWER_QUALITY_AND_PERFORMANCE.md) for measured results and current limitations.

## Compatibility

`ppl compare SUITE.toml` audits stored physical evidence before comparing it:
the current generated source, device profile, artifact hash, transcript and
recorded settings must agree. Conflicting observations remain visible. Legacy
TYPE/STRING matches describe those observations only; they do not establish full
value, side-effect, interaction or calculator-feature parity. Reports include an
`assessment-<suite>.json` file with audit decisions and per-reference results.

To freeze the broader declared suite and reproduce its offline assessment:

```bash
cargo build -p ppl-compat --example assess
python3 scripts/compat/assessment.py --freeze \
  --out artifacts/g1-assessment \
  --assessor ../.cache/prime-emulator-target/debug/examples/assess
```

This writes a versioned manifest, per-engine/category results and `report.html`.
Generated probes are reported separately from curated cases. Missing physical
references remain unmeasured even when local execution succeeds. The assessment
tools never upload programs or press physical calculator keys. Local firmware
reliability trials use `scripts/compat/firmware_reliability.py --help`; every
attempt retains its output and guest artifacts, including failures.

The native engine implements PPL in Rust; compatibility with the calculator is still under development. Check [built-in support](docs/compatibility/diagnostics.md) and test important programs against the behavior you need.

The optional original-firmware backend uses HP's compiler and runtime under ARM emulation. It currently runs through the headless CLI, requires separately supplied firmware and factory resources, and supports a documented G1 firmware version. See the [firmware setup and CLI guide](docs/emulator/M8_UNIFIED_CLI_REPORT.md) for dependencies, configuration, and supported values.

## Documentation and development

- [Developer guide](docs/development/SCRIPTS.md): workspace crates, build checks, tests, and research tools
- [Architecture](docs/architecture/overview.md) and [editor design](docs/architecture/editor.md)
- [Hardware testing](docs/device/hardware-testing.md)
- [Official HP software investigation](docs/reverse-engineering/HP_OFFICIAL_SOFTWARE_ARCHITECTURE.md)
- [Firmware research](docs/emulator/README.md)

## License

Project source is [MIT licensed](LICENSE). HP firmware is proprietary and is not distributed with this repository. Fonts and third-party dependencies retain their own licenses. See the [firmware research index](docs/emulator/README.md) for research references and licensing boundaries.
