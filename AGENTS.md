# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## What This Project Does

A high-fidelity native Rust emulator and differential testing harness for HP Prime PPL (`.hpprgm`) calculator programs.
The pipeline parses PPL source with a typed Pratt parser, compiles to native bytecode with line/column span tracking, and executes on an isolated stack VM with accurate 12-digit BCD decimal arithmetic, Copy-on-Write value semantics, headless GROB graphics, and hardware differential verification against a physical HP Prime G1.

## Workspace Crates

| Crate | Responsibility |
|---|---|
| `ppl-math` | 12-digit decimal `Real` ($\pm 499$ exp), `HpInt` (`#FFh`), `Complex`, formatting |
| `ppl-syntax` | Source loader (UTF-8, UTF-16, container headers), lexer, Pratt parser, typed AST, diagnostics |
| `ppl-compiler` | Builtin specifications, semantic analysis, scoping, bytecode compiler |
| `ppl-vm` | Stack VM, Copy-on-Write `Value`, collections (1-based), builtins, `Host` trait, debugger hooks |
| `ppl-graphics` | Headless GROB G0–G9 pixel buffers, Bresenham drawing, DejaVu Sans font, PNG read/write, RGB565 diff |
| `ppl-device` | USB HID communication (`03f0:1541`/`0441`/`2441`), `hplp` framing, CRC16, screenshots, guarded file transfer |
| `ppl-compat` | Differential test suites (TOML), emulator runner, device program synthesis, note parser, evidence reports |
| `ppl-gui` | Native window frontend (`winit 0.30`, `softbuffer 0.4`), 0–50 HP keycode mapping, non-blocking worker thread |
| `ppl-cli` | `ppl` binary CLI |

## Commands

```bash
# Build the workspace
cargo build --workspace

# Run all workspace unit & integration tests
cargo test --workspace

# Run formatting and linting
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# Execute a PPL program headlessly (PRINT to stdout, optional PNG)
cargo run --bin ppl -- run examples/FIREWORKS.hpprgm --png out.png

# Launch the native interactive GUI window
cargo run --bin ppl -- gui examples/FIREWORKS.hpprgm

# Run the interactive step debugger
cargo run --bin ppl -- debug examples/FIREWORKS.hpprgm

# Differential testing against test suites
cargo run --bin ppl -- compare tests/compat/cases/core.toml

# Print device program without calculator interaction
cargo run --bin ppl -- compare tests/compat/cases/core.toml --print-program

# Talk to a connected physical HP Prime over USB (read-only)
cargo run --bin ppl -- device detect
cargo run --bin ppl -- device info
cargo run --bin ppl -- device screenshot screen.png
```

## Non-Negotiable Rules

1. **Git Restrictions:**
   - Work strictly on branch `rust-rewrite`.
   - Never commit, push, merge, reset, or create pull requests without explicit user authorization.
2. **Physical Hardware Safety:**
   - Connected HP Prime G1 is a real physical device.
   - Never overwrite or delete existing user programs. `ppl device send` only creates new programs; ordinary names are allowed, but an existing name is always refused. Disposable test, cleanup and remote-execution objects must use `ZZT_*` names. Ordinary programs are never auto-cleaned or deleted.
   - **Narrow exception (user-approved):** the user may delete one specifically named ordinary program by running `ppl device delete program NAME --yes` (or by typing the exact name when `--yes` is omitted). Each invocation covers one program on one selected device, with identity check, double read, raw backup, a single `EB`, and independent absence verification. This never permits batch/automatic deletion, deleting other object types or system resources, skipping backups, or deleting during development, tests or agent-initiated work: agents and tests must not run `ppl device delete` against the physical calculator without separate explicit user authorization (read-only `device probe-anchor` / `device get` are fine when the user has allowed device access).
   - **Narrow exception (user-approved): IDE Send → Replace.** In the desktop IDE, *Send to calculator* creates a new program, or, when the name already exists, prepares a backed-up replacement (`ppl_programs::replacement`) that runs only after the user clicks *Replace* in a dialog bound to the exact source hash; editing the source cancels it. Each replacement covers one program, keeps the raw backup in the shared journal (`artifacts/delete`), and is never retried or automatic. Agents and tests must never trigger a send or replacement against the physical calculator without separate explicit user authorization; the IDE's device link refuses to open a calculator under `cfg(test)`.
   - Never perform firmware writes, date/time alterations, or undocumented memory writes.
   - Remote key injection (`EC`) is opt-in only (`--auto-run`) and disabled by default.
   - Hardware results must never be fabricated; only genuine calculator evidence may be tagged as `VERIFIED`.
