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
