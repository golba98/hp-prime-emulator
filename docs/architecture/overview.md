# HP Prime Emulator — Architecture Overview

The HP Prime PPL Emulator is written in 100% native Rust, organized as a multi-crate Cargo workspace.

```
Cargo.toml (workspace, resolver 3, edition 2024)
crates/
  ppl-math      HP Prime numeric model (12-digit BCD Reals, HpInt #FFh, Complex, formatting)
  ppl-syntax    Source loader (UTF-8, UTF-16LE, container headers), lexer, Pratt parser, AST, diagnostics
  ppl-compiler  Builtin signature tables, semantic analysis, scopes, and bytecode generator
  ppl-vm        Stack machine, CoW Value model, runtime built-ins, Host abstraction, debugger hooks
  ppl-graphics  Headless Grob framebuffers (G0–G9, DIMGROB), Bresenham rasterizer, font rendering, PNG I/O
  ppl-device    USB HID communications (hplp framing, CRC-16, device detection, screenshots, file transfers)
  ppl-compat    Differential compatibility runner, TOML test suites, device program generation, evidence recording
  ppl-gui       Desktop IDE (eframe/egui 0.32: editor, debugger, calculator, problems) and the plain winit/softbuffer calculator window
  ppl-cli       `ppl` executable (run, check, commands, device, compare, debug, gui)
```

---

## Data Flow Pipeline

```
PPL Source (.hpprgm / .ppl / text)
    │
    ▼
[ppl-syntax] ── Lexer & Pratt Parser ──► AST & Diagnostics
    │
    ▼
[ppl-compiler] ── Semantic Analysis & Codegen ──► Bytecode & Span Table
    │
    ▼
[ppl-vm] ── Stack VM & Host Trait ◄──► [ppl-graphics] Framebuffer G0–G9
    │                                          │
    ▼                                          ▼
[ppl-cli / ppl-gui] (Headless CLI / Debugger / Native Window / Hardware Differential Testing)
```

---

## Key Design Principles

1. **Exact Precision Semantics:** Reals use 12 significant decimal digits with an exponent range of $\pm 499$. Operations preserve genuine calculator roundoff and formatting rules rather than IEEE binary floating-point artifacts.
2. **Copy-on-Write Memory Model:** Values use `Rc` and `Rc::make_mut` to implement strict pass-by-value semantics. Assigning or passing lists and matrices never creates unexpected reference aliases.
3. **Decoupled Architecture:** `ppl-vm` and `ppl-graphics` have zero window system dependencies. They execute headlessly across all environments and unit tests without requiring a display server or physical GPU.
4. **Hardware Differential Loop:** Calculator compatibility is measured empirically against a real HP Prime G1 using differential test suites (`ppl compare`), verifying that emulator behavior faithfully converges to physical calculator hardware.

---

## IDE Editor and Diagnostics

The IDE editor never tokenizes PPL itself. Highlighting, bracket
matching, auto-indent, the outline and completion all come from the real
lexer through `ppl_compiler::highlight`.

Diagnostics fall into four classes: Syntax, Semantic, Compatibility and
Runtime. A command the emulator lacks is a Compatibility warning, never an
error.

- Editor architecture, caching, performance and tests:
  [`editor.md`](editor.md).
- Diagnostic classes, every code, and the evidence behind each language
  rule: [`../compatibility/diagnostics.md`](../compatibility/diagnostics.md).
