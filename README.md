# HP Prime Emulator & Development Toolkit

[![Rust Version](https://img.shields.io/badge/rust-1.85%2B%20%282024%29-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Hardware Verified](https://img.shields.io/badge/hardware-HP%20Prime%20G1-success.svg)](#hardware-differential-verification)
[![Differential Testing](https://img.shields.io/badge/differential%20testing-passing-brightgreen.svg)](#differential-test-suites)
[![Workspace](https://img.shields.io/badge/crates-10%20crates-informational.svg)](#workspace-architecture)

An independent, native Rust emulator, compiler, interactive desktop IDE, and differential testing harness for **HP Prime PPL** (`.hpprgm`) programs — verified against physical HP Prime G1 hardware over USB.

The execution engine follows a strict pipeline: **Source Loader (UTF-8 / UTF-16LE) → Typed Pratt Parser → Bytecode Compiler with Spans → Isolated Stack VM**. It features genuine 12-digit BCD decimal arithmetic, Copy-on-Write value semantics, headless GROB graphics rasterization, an integrated desktop IDE, an interactive step debugger, and a guarded USB hardware bridge for differential testing.

---

<p align="center">
  <img src="assets/ide-display-output.png" alt="HP Prime PPL Emulator Display & IDE" width="680">
  <br>
  <em>Interactive Desktop IDE running a PPL graphics program with pixel-perfect 320×240 GROB rendering.</em>
</p>

---

## Table of Contents

- [Key Features](#key-features)
- [Pipeline Architecture](#pipeline-architecture)
- [Workspace Architecture](#workspace-architecture)
- [Installation & Requirements](#installation--requirements)
- [Quick Start Guide](#quick-start-guide)
  - [1. Interactive Desktop IDE](#1-interactive-desktop-ide)
  - [2. Headless Execution & Screen Capture](#2-headless-execution--screen-capture)
  - [3. Interactive CLI Step Debugger](#3-interactive-cli-step-debugger)
  - [4. Syntax Checking & Bytecode Disassembly](#4-syntax-checking--bytecode-disassembly)
  - [5. Example PPL Program](#5-example-ppl-program)
- [Hardware Differential Verification](#hardware-differential-verification)
  - [Hardware Safety Guardrails](#hardware-safety-guardrails)
  - [Device Workflows over USB](#device-workflows-over-usb)
  - [Differential Test Suites](#differential-test-suites)
- [Command Coverage & Emulation Status](#command-coverage--emulation-status)
- [Development & Quality Gates](#development--quality-gates)
- [License & Acknowledgments](#license--acknowledgments)

---

## Key Features

- **Typed Language Pipeline:**
  - Robust UTF-8 and UTF-16LE container loader handling official calculator export headers.
  - Typed Pratt parser handling expressions, control structures (`IF`, `CASE`, `FOR`, `WHILE`, `REPEAT`), and nested subroutines.
  - Bytecode compiler generating compact instructions accompanied by an exact line/column span table for diagnostics.

- **High-Fidelity Virtual Machine:**
  - Isolated stack machine implementing calculator-faithful semantics.
  - Strict **Copy-on-Write** (`Rc::make_mut`) value model: assigning or passing lists and matrices never creates unexpected reference aliases.
  - Authentic **1-based indexing** across all collections, strings, and matrix dimensions.
  - Configurable resource limits: execution step watchdog, wall-clock limits, and call-depth protection.

- **Exact Decimal Arithmetic (`ppl-math`):**
  - Custom 12-significant-digit BCD decimal `Real` representation with an exponent range of $\pm 499$, eliminating IEEE 754 binary floating-point rounding errors.
  - HP-style `#FFh` integers (`HpInt`), complex numbers, and full matrix linear algebra operations (`DET`, `TRN`, `INVERSE`, `DOT`, `CROSS`).

- **Headless & Hardware GROB Graphics (`ppl-graphics`):**
  - Complete G0–G9 graphic object buffers matching the calculator's native 320×240 display.
  - Bresenham rasterizer for `RECT_P`, `LINE_P`, `CIRCLE_P`, `ARC_P`, `BLIT_P`, and polygon fills.
  - Integrated typography using bundled DejaVu Sans font for `TEXTOUT_P`.
  - Zero-display-server headless mode capable of exporting direct PNG screenshots.

- **Dual Graphical Frontends (`ppl-gui`):**
  - **Integrated Desktop IDE (`egui` / `eframe`):** Multi-tab source editor, live syntax diagnostics, interactive on-screen HP keypad, console output, variable watch, and visual breakpoints.
  - **Standalone Virtual Calculator Window (`winit` / `softbuffer`):** Ultra-fast, lightweight pixel-perfect window for running programs and automated GUI test scripting.

- **Command-Line Step Debugger (`ppldb`):**
  - GDB-style interactive CLI debugger (`ppl debug`).
  - Line breakpoints, step over (`next`), step into (`step`), finish (`finish`), call stack inspection (`backtrace`), and local/global variable evaluation.

- **Hardware Differential Testing & USB Link (`ppl-device`, `ppl-compat`):**
  - Native USB HID communication with the physical HP Prime G1 (`03f0:1541`) using `hplp` v1 framing and CRC-16 integrity checks.
  - Automated differential testing engine that runs test suites simultaneously in the emulator and on real hardware, comparing outputs and screenshots.
  - Preserved empirical evidence logs validating language behaviors against physical firmware.

---

## Pipeline Architecture

```
PPL Source (.hpprgm / .ppl / text)
    │
    ▼
[ppl-syntax] ─────── Lexer & Pratt Parser ────────► Typed AST & Diagnostics
    │
    ▼
[ppl-compiler] ───── Semantic Analysis & Codegen ─► Bytecode & Span Table
    │
    ▼
[ppl-vm] ─────────── Stack VM & Host Trait ◄─────► [ppl-graphics] Framebuffers G0–G9
    │                                                     │
    ├─────────────────────────────┬───────────────────────┤
    ▼                             ▼                       ▼
[ppl-gui: IDE & Window]     [ppl-cli: Runner/Debug]  [ppl-device: USB Hardware]
(Native Desktop GUI)        (CLI & Headless PNG)     (HP Prime G1 USB Bridge)
```

---

## Workspace Architecture

The repository is structured as a modular Cargo workspace:

| Crate | Responsibility |
|---|---|
| [`ppl-math`](crates/ppl-math) | 12-digit BCD decimal `Real` ($\pm 499$ exp), `#FFh` integers (`HpInt`), `Complex`, formatting |
| [`ppl-syntax`](crates/ppl-syntax) | Source loader (UTF-8, UTF-16LE headers), lexer, Pratt parser, typed AST, diagnostics |
| [`ppl-compiler`](crates/ppl-compiler) | Built-in command specifications (163 commands), semantic analysis, scopes, bytecode compiler |
| [`ppl-vm`](crates/ppl-vm) | Stack VM, Copy-on-Write `Value` model, 1-based collections, built-ins, `Host` trait |
| [`ppl-graphics`](crates/ppl-graphics) | Headless GROB G0–G9 pixel buffers, Bresenham drawing, DejaVu Sans font, PNG export |
| [`ppl-device`](crates/ppl-device) | USB HID protocol (`03f0:1541`), `hplp` framing, CRC-16, screenshots, guarded file transfer |
| [`ppl-compat`](crates/ppl-compat) | Differential test harness (TOML), device program synthesis, note parser, evidence reports |
| [`ppl-gui`](crates/ppl-gui) | Native desktop IDE (`egui`) and standalone virtual calculator window (`winit` / `softbuffer`) |
| [`ppl-cli`](crates/ppl-cli) | Unified `ppl` command-line executable |
| [`ppl-bench`](crates/ppl-bench) | Criterion microbenchmarks and execution performance suites |

---

## Installation & Requirements

### System Requirements

- **Rust Toolchain:** Version ≥ 1.85 (Edition 2024). Install via [rustup](https://rustup.rs/):
  ```bash
  rustup update stable
  ```
- **C Compiler & Build Tools:** A standard C compiler (`gcc` or `clang`) for building the static `hidapi` backend (no external `libudev` shared library required).
- **Desktop Window System:** X11 or Wayland session for GUI operation.

### Building from Source

```bash
# Clone repository
git clone https://github.com/golba98/hp-prime-emulator
cd hp-prime-emulator

# Build all workspace crates in release mode
cargo build --workspace --release
```

The compiled binary will be located at `target/release/ppl`.

---

## Quick Start Guide

### 1. Interactive Desktop IDE

Launch the graphical IDE with an integrated code editor, visual debugger, interactive keypad, and emulated 320×240 display:

```bash
# Open an empty IDE workspace
cargo run --release --bin ppl -- gui

# Open a specific PPL program in the IDE
cargo run --release --bin ppl -- gui examples/FIREWORKS.hpprgm
```

### 2. Headless Execution & Screen Capture

Execute PPL programs headlessly in CI or terminal environments, capturing `PRINT` output to stdout and exporting the calculator screen buffer to a PNG:

```bash
# Execute headlessly with simulated keypresses and capture the final frame
cargo run --release --bin ppl -- run examples/FIREWORKS.hpprgm --keys "-1*5,4" --png fireworks.png

# Execute an inline PPL statement directly
cargo run --release --bin ppl -- run -c "EXPORT DEMO() BEGIN PRINT(2^10 + SQRT(144)); END;"
```

### 3. Interactive CLI Step Debugger

Launch the interactive command-line step debugger (`ppldb`) with line breakpoints and frame inspection:

```bash
cargo run --release --bin ppl -- debug examples/FIREWORKS.hpprgm
```

Debugger command summary:
- `s` / `step`: Step into next statement
- `n` / `next`: Step over subroutine calls
- `c` / `continue`: Resume execution until next breakpoint
- `b <line>` / `break <line>`: Set a breakpoint at `<line>`
- `locals`: Inspect local variables in the current stack frame
- `globals`: Inspect calculator global variables
- `bt` / `backtrace`: Display the call stack
- `q` / `quit`: Terminate the debug session

### 4. Syntax Checking & Bytecode Disassembly

Validate syntax and semantic validity without running the program, or inspect the compiled VM bytecode:

```bash
# Check syntax and semantics
cargo run --release --bin ppl -- check examples/FIREWORKS.hpprgm

# Disassemble compiled bytecode instructions
cargo run --release --bin ppl -- check -c "EXPORT ADD(a, b) BEGIN RETURN a + b; END;" --dump-bytecode
```

Output:
```text
fn <init> (params 0, locals 0):
     0  Zero
     1  Return
fn ADD (params 2, locals 0):
     0  Stmt
     1  LoadLocal(0)
     2  LoadLocal(1)
     3  Bin(Add)
     4  Return
     5  Zero
     6  Return
```

### 5. Example PPL Program

Here is an idiomatic PPL program drawing circles and displaying text:

```ppl
EXPORT DEMO()
BEGIN
    RECT_P(0, 0, 319, 239, 0, 0);               // Clear screen with black
    CIRCLE_P(160, 120, 50, #00FF00h);            // Draw green circle
    TEXTOUT_P("Hello from HP Prime PPL!", 50, 20); // Render anti-aliased text
    WAIT(2);                                    // Pause for 2 seconds
END;
```

---

## Hardware Differential Verification

A core design principle of this project is **empirical verification**: rather than guessing edge-case behaviors, emulator outputs are continuously compared against a genuine **HP Prime G1** (firmware `2.2.15157`, USB `03f0:1541`).

### Hardware Safety Guardrails

The test harness enforces non-negotiable safety rules when interacting with physical hardware:

1. **Strict Namespace Isolation:** Only objects starting with `ZZT_*` may ever be created or deleted. User programs and system variables are never touched.
2. **Double-Probe Preflight:** Before any program is uploaded, the target name is probed twice to guarantee absence, while a positive control object (`ZZT_CTRL`) is verified to respond.
3. **Durable Ownership Ledger:** Every upload is recorded in an append-only ledger (`tests/compat/runs/ledger.jsonl`). Deletion requires cryptographic or verified ledger authorization.
4. **No Destructive Overwrites:** Uploads refuse to overwrite existing objects. If a collision is detected, the operation aborts immediately.
5. **No Automatic Firmware Changes:** Date, time, and system settings are strictly read-only.

### Device Workflows over USB

With an HP Prime G1 connected via USB:

```bash
# Detect connected HP Prime calculators (read-only)
cargo run --release --bin ppl -- device detect

# Query calculator model, firmware version, and serial number (read-only)
cargo run --release --bin ppl -- device info

# Capture a real-time screenshot from the physical calculator screen
cargo run --release --bin ppl -- device screenshot calc_screen.png

# Run an uploaded test program (manual Run on the G1), then retrieve its note
cargo run --release --bin ppl -- device run ZZT_004752

# Delete a tool-created program or note without touching the calculator
# Physically verified: G1 build 15157, boot V0.050.640; backs up first.
cargo run --release --bin ppl -- device delete program ZZT_TEST --json
cargo run --release --bin ppl -- device delete note ZZT_NOTE --json

# Reconcile an interrupted deletion without resubmitting it
cargo run --release --bin ppl -- device delete verify --operation-id ZZT_Dxxxxx --json
```

Deletion uses Connectivity Kit's native USB `EB` command, independently
verified against actual calculator storage. It creates no helper or evidence
note. Matching creation receipts from `device send` authorize unattended
operation; unrelated objects are refused. `--manual-run` preserves the existing
helper workflow, and `--prepare-only` performs no deletion. See
[deletion and recovery](docs/device/deletion.md). Remote keypress execution
and arbitrary expression evaluation remain unverified on this G1 firmware.

### Differential Test Suites

Test suites are written in TOML (`tests/compat/cases/core.toml`). The differential harness executes each test on both the emulator and physical hardware, recording byte-for-byte comparisons and screenshots under `tests/compat/evidence/`:

```bash
# Run differential comparison against the core test suite
cargo run --release --bin ppl -- compare tests/compat/cases/core.toml --no-report

# Preview the generated calculator program without touching hardware
cargo run --release --bin ppl -- compare tests/compat/cases/core.toml --print-program
```

---

## Command Coverage & Emulation Status

The emulator implements **163 standard PPL built-in commands**:

| Category | Status | Highlights |
|---|---|---|
| **Math** | Implemented | `ABS`, `SQRT`, `ROUND`, `TRUNCATE`, `FLOOR`, `CEILING`, `IP`, `FP`, `SIGN`, `MOD`, `DIV`, `MAX`, `MIN`, `RANDOM`, `RANDINT`, `NTHROOT`, `FACTORIAL` |
| **Trigonometry** | Implemented | `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`, `ATAN`, `SINH`, `COSH`, `TANH`, `ASINH`, `ACOSH`, `ATANH` (respects `HAngle`: Radians, Degrees, Gradians) |
| **Bitwise** | Implemented | `BITAND`, `BITOR`, `BITXOR`, `BITNOT`, `BITSL`, `BITSR`, `R→B`, `B→R`, `SETBASE`, `GETBASE`, `SETBITS`, `GETBITS` |
| **Strings** | Implemented | `ASC`, `CHAR`, `DIM`, `INSTRING`, `LEFT`, `RIGHT`, `MID`, `ROTATE`, `UPPER`, `LOWER`, `REPLACE`, `STRING` |
| **Lists** | Implemented | `MAKELIST`, `SIZE`, `SORT`, `REVERSE`, `POS`, `ΣLIST`, `∏LIST`, `ΔLIST`, `HEAD`, `TAIL`, `APPEND`, `CONCAT` |
| **Matrices** | Implemented | `MAKEMAT`, `DET`, `TRN`, `IDENMAT`, `RANK`, `COLNORM`, `ROWNORM`, `INVERSE`, `ADDROW`, `ADDCOL`, `DELROW`, `DELCOL`, `DOT`, `CROSS` |
| **Graphics** | Implemented | `RECT_P`, `LINE_P`, `PIXON_P`, `PIXOFF_P`, `GETPIX_P`, `CIRCLE_P`, `ARC_P`, `BLIT_P`, `DIMGROB_P`, `SUBGROB_P`, `INVERT_P`, `FILLPOLY_P`, `RGB` |
| **I/O & Dialogs** | Implemented / Approximated | `PRINT`, `MSGBOX`, `INPUT`, `CHOOSE`, `GETKEY`, `ISKEYDOWN`, `WAIT`, `MOUSE`, `DRAWMENU` |
| **System & Time** | Implemented | `TICKS`, `TIME`, `DATE`, `TYPE`, `NOTES`, `PROGRAMS` |
| **CAS & App Scope** | *Out of Scope* | `CAS()`, `STARTAPP`, and Home app persistent settings are deliberately unsupported |

To inspect the real-time status of all commands in your terminal:

```bash
cargo run --release --bin ppl -- commands
```

For complete behavioral details and hardware discrepancies, see the [Compatibility Matrix](docs/compatibility/matrix.md).

---

## Development & Quality Gates

All contributions must pass the workspace quality gate:

```bash
# 1. Format check
cargo fmt --all -- --check

# 2. Workspace compilation check
cargo check --workspace --all-targets

# 3. Strict Clippy linter
cargo clippy --workspace --all-targets -- -D warnings

# 4. Workspace unit and integration tests
cargo test --workspace

# 5. Release build verification
cargo build --workspace --release
```

---

## License & Acknowledgments

### License

This project is licensed under the **MIT License**. See [LICENSE](LICENSE) for details.

### Acknowledgments & References

- **[hpcalc.org](https://www.hpcalc.org/prime/apps/):** The authoritative repository of authentic HP Prime community programs, used as reference corpora.
- **[hplp](https://github.com/debrouxl/hplp):** Reverse-engineered HP Prime USB protocol documentation by Lionel Debroux and contributors.
- **[DejaVu Fonts](https://dejavu-fonts.github.io/):** High-quality typography used for GROB text rendering.
- **HP Inc. Disclaimer:** This is an independent open-source project and is not affiliated with, endorsed by, or sponsored by HP Inc. "HP Prime" is a trademark of HP Inc.
