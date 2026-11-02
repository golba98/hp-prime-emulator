# Native EDITMAT implementation and firmware comparison

Implemented 2026-11-02. This is **firmware-only comparison**, not physical
HP Prime verification. The existing physical evidence and its 77 recorded
results are unchanged.

## Findings and implementation

EDITMAT previously compiled with an unsupported warning and always raised a
runtime error. The Host boundary supported only messages, choices and INPUT.
A matrix editor could be added without firmware delegation or a VM redesign.

The HP Prime User Guide (2018-01-12), printed p. 646, documents expression
arguments, matrix returns, M0–M9 writeback, optional titles/headers and read-only
mode. Local firmware probes resolved the otherwise undocumented Cancel behavior
and the difference between calculator matrix variables and local variables:

| Case | Observed firmware behavior | Native behavior |
|---|---|---|
| Change M1 cell (1,1) from 1 to 9, OK | M1 and return value contain 9 | Matches |
| Same change, Cancel after Enter | M1 and return value retain 1 | Matches |
| Edit a local matrix variable, OK | Original local unchanged; return contains 9 | Matches |
| Edit a matrix literal, OK | Returns edited matrix | Matches |
| Read-only with row/column headers | Edits ignored; returns original matrix | Matches |

Sources, key sequences, screenshot hashes and transcribed results are in
[evidence/native-editmat/observations.json](evidence/native-editmat/observations.json).
Each probe used a fresh copy of installed initial-data; no calculator, existing
profile or proprietary binary was modified. The firmware ELF SHA-256 was
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`.
The observations are explicitly labelled `firmware-only-observed`.

The compiler uses reference writeback only for bare M0–M9 arguments. Local
variables and matrix expressions use normal value calls. The VM builds an
edited copy, validates numeric cells, and only returns it after successful OK.
Cancel returns the original matrix; invalid cells reopen the editor at the
bad cell without partially updating a calculator matrix variable. Existing VM
value-limit checks run before writeback. Vectors retain their shape.

`Dialog::Matrix` carries dimensions, cell text, titles/headers, read-only state
and validation errors through the existing nonblocking GUI worker. The new
native grid supports paging through rows and columns, typed cell expressions,
Delete, arrows, Enter to set a cell, Menu/F7 to accept and Esc to cancel.
Esc during cell editing restores that cell first. Read-only Enter accepts.

## Validation

- `cargo test -p ppl-vm -p ppl-compiler -p ppl-gui`: **496 passed, 0 failed,
  2 existing ignored**. New tests cover value/reference semantics, cancellation,
  read-only operation, expressions/vectors, invalid arguments, re-prompting,
  navigation, rendering and real GUI Host worker delivery/restoration.
- `cargo clippy -p ppl-vm -p ppl-compiler -p ppl-gui --all-targets -- -D warnings`
  passed; `cargo fmt --all -- --check` passed.
- Actual standalone native GUI run under isolated Xvfb/X11 typed 9, Enter,
  Menu, printed `{[[9,2],[3,4]],[[9,2],[3,4]]}`, and returned `[[9,2],[3,4]]`.
  [Window capture](evidence/native-editmat/native-window.png) and
  [scrollable grid render](evidence/native-editmat/native-grid.png) were visually
  inspected. These are native captures, not firmware pixel-parity assertions.

For an equivalent GUI check, run a program containing
`M1:=[[1,2],[3,4]]; LOCAL a:=EDITMAT(M1); PRINT(STRING({M1,a})); RETURN a;`
with `ppl gui FILE --keys 'wait:1000,text:9,30,13' --exit-on-finish`.
The startup wait is a test input scheduling aid, not an implementation delay.
When using Xvfb on a Wayland desktop, unset `WAYLAND_DISPLAY` to ensure the
window is actually created on the isolated X display.

## Scope and remaining differences

The builtin is marked **Approximate**, not full firmware equivalence. The
native grid edits existing cells; firmware resizing, insert/delete row/column
menus, touch interaction and matrix-catalog services remain Firmware-mode
features. Blank cell expressions are rejected rather than silently deleting cells.
Native numeric expressions use the Native evaluator; unsupported CAS
expressions do not become supported through this editor. Validation occurs on
OK rather than at every firmware cell-commit boundary. Native keyboard/menu
presentation and cell-editor Escape transitions are not pixel/key-for-key
firmware emulation. The return/writeback/cancel contracts above were observed;
no physical G1 measurements were made.

Changed code: `ppl-compiler/{builtins,compile}.rs`, `ppl-vm/{host,builtins/io,
builtins/mod}.rs`, `ppl-gui/{dialog,dialog/matrix,host,app_native}.rs`, and
compiler/VM/GUI regression tests. Native EVAL and STARTAPP still report precise
Firmware-required errors; this implementation does not add CAS or application
services to Native mode.
