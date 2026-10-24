# Baseconv regression fixtures

A user's base converter (bases 2–16, with fractions) used as the
end-to-end regression for typed INPUT, dialogs, string commands and
loop control in the **native** Rust pipeline (`ppl-syntax` →
`ppl-compiler` → `ppl-vm`, CLI and IDE). This is not the firmware
backend (`crates/prime-emulator`): passing here does not make the native
compiler equivalent to the HP Prime's own compiler.

## E. Fixtures

| File | What it is |
|---|---|
| `Baseconv_original.hpprgm` | The user's first version, verbatim |
| `Baseconv_corrected.hpprgm` | The user's corrected version, verbatim |
| `Baseconv_fixed.hpprgm` | The corrected version plus one source fix (see below); the expected-results fixture |

## A. Root causes

### Emulator defects (fixed)

1. **Typed INPUT fields were ignored.** The compiler kept only the
   variable of `{{inStr,[2]}}` and discarded `[2]`, and the VM ran every
   entered text through the expression evaluator. Typing `FF` failed with
   "undeclared variable FF", `7B.8` with a syntax error, and `255` came
   back as a real, so `UPPER(inStr)` then failed with a bad argument type.
   HP staff (Cyrille de Brébisson, Tim Wessman) are quoted on the HP
   Museum forum describing a `[2]`-only field as showing the entry without
   quotes, storing it directly as a string and never evaluating it (found
   through threads such as
   [11057](https://hpmuseum.org/forum/thread-11057.html) and
   [7387](https://hpmuseum.org/forum/printthread.php?tid=7387); the
   individual posts were not re-read for this report).
   Fix: the compiler passes each field's allowed types to INPUT as a
   hidden argument (`compile.rs`, `ref_builtin`), and `io::input` stores
   `[2]`-only entries verbatim.
2. **An invalid entry aborted the program.** A syntax error, or a value
   whose type the field does not allow, raised `Invalid input` and ended
   the run. The calculator keeps its form open instead. Fix: the dialog
   reopens with the reason on its status line (`Dialog::Input.error`),
   the entered texts kept and focus on the offending field.
3. **String fields did not round-trip.** A field holding a string started
   with the bare text, so pressing Enter without editing evaluated `abc`
   as an identifier. Fix: untyped fields start from `STRING`'s quoted
   form; `[2]` fields show the bare text.
4. **Headless runs could not cancel INPUT.** `HeadlessHost` now takes
   `ScriptedInput::{Text, Cancel}`; on the CLI, `--input '<ESC>'` cancels.
5. **(Test infrastructure)** `tests/ide_xvfb_smoke/run_ide_auto.sh` ran
   `./target/release/ppl`, which `.cargo/config.toml` (target-dir
   `../.cache/prime-emulator-target`) left stale. It now asks
   `cargo metadata` for the target directory (`PPL_BIN` overrides).

`ppl check` already accepted both user versions with 0 errors and 0
warnings: LOCAL after statements, `LOCAL d` re-declared in sibling loop
bodies, `{{inStr,[2]}}`, `IFTE`, `REPEAT … UNTIL`, `BREAK`, early
`RETURN`. CLI and IDE share `compile_source` with default options. No
parser or semantic rule was relaxed.

### Source defects (not emulator bugs)

*Corrected version.* Step 12 (decimal fraction → target base) runs even
when the number has no fractional part. Its first pass appends `"0"` and
then breaks, so `1010₂→10` prints `10.0`, `255→16` prints `FF.0` and `0`
prints `0.0`. `Baseconv_fixed.hpprgm` wraps step 12 in
`IF DIM(fracPart) > 0 THEN … END;`, the same guard the program already
uses for its "Fraction result" line. Nothing else changes.

*Original version.*
- `INPUT(temp, …)` on the list `{""}` is an untyped field: typing `FF`
  is an undeclared name, and typing `"FF"` stores a string, after which
  `temp(1)` returns the first character's code (70), which `UPPER`
  rejects. Only typing `{"FF"}` works.
- Digits are not validated (`19` in base 8 and `G1` in base 16 give
  nonsense).
- Precision is not truncated to an integer.
- It has the same trailing `.0` defect.
- It ends with a busy loop on `ISKEYDOWN(4)` followed by `WAIT(-1)`, so
  two key presses are needed to finish.

### Precision (program limitation)

The converter computes with 12-significant-digit reals.
- Integers above 12 digits are rounded before conversion:
  `9999999999999`₁₀ is converted as `1E13` (`9184E72A000`₁₆), and
  `FFFFFFFFFFFFFFFF`₁₆ gives `18446744073800000000`.
- Fractions that terminate in decimal stay exact when multiplied by the
  base (`0.1`₁₀ → `0.000110011001…`₂ to any precision). `0.1`₃ = 1/3
  ends after 12 digits.

## B. Changed code

| Area | File |
|---|---|
| Field types to runtime, `input-unknown-type`, `input-field-position-ignored` | `crates/ppl-compiler/src/compile.rs` |
| Typed INPUT, re-prompt loop, round-trip texts | `crates/ppl-vm/src/builtins/io.rs` |
| `InputField.string_only`, `Dialog::Input { error, focus }`, `ScriptedInput` | `crates/ppl-vm/src/host.rs` |
| Error line and initial focus in the IDE dialog | `crates/ppl-gui/src/dialog.rs` |
| `--input '<ESC>'` | `crates/ppl-cli/src/cmd/{run,debug}.rs` |

## C. Regression tests

| Test | Covers |
|---|---|
| `crates/ppl-vm/tests/input.rs` | typed / scalar / list INPUT, write-back, cancel, re-prompt, round trip, script exhaustion, error location |
| `crates/ppl-vm/tests/baseconv.rs` | the three fixtures end to end: conversion table, invalid numbers and bases, cancel at each dialog, repeated runs, pinned source defects |
| `crates/ppl-compiler/tests/diagnostics.rs` | LOCAL placement, duplicate LOCAL, typed INPUT forms and errors with positions, genuine syntax errors still rejected |
| `crates/ppl-vm/tests/semantics.rs` | `DIM`, `MID`, `LEFT`, `INSTRING`, `UPPER`, `STRING`, `IP`, `MOD` edge cases |
| `crates/ppl-gui/src/dialog.rs`, `host.rs` | error line and focus, typed string and Esc through `GuiHost` |
| `crates/ppl-gui/src/ide/diagnostics.rs` | Problems panel == `compile_source` diagnostics for the fixtures and broken variants |
| `crates/ppl-cli/tests/baseconv_cli.rs` | `ppl check` / `ppl run` with scripted answers and `<ESC>` |
| `tests/ide_xvfb_smoke/scripts/script_baseconv*.json` + `assert_baseconv.py` | real IDE event loop: typed hex, invalid base re-prompt, second run, Esc |
| `tests/compat/cases/strings_baseconv.toml` | 20 non-interactive probes for G1 differential runs |

## D. Results (2026-10-24, Fedora, native backend)

`Baseconv_fixed.hpprgm`, precision 10 unless noted:

| Input | From | To | Result |
|---|---|---|---|
| `1010` | 2 | 10 | `10` |
| `255` | 10 | 16 | `FF` |
| `FF` | 16 | 10 | `255` |
| `A3` | 16 | 2 | `10100011` |
| `7B.8` | 16 | 2 | `01111011.1000` |
| `10.5` | 10 | 2 | `1010.1` |
| `0` | 10 | 2 | `0` |
| `19` | 8 | 10 | MSGBOX "Invalid digit: 9", returns 0 |
| `G1` | 16 | 10 | MSGBOX "Invalid digit: G", returns 0 |
| `-ff` / `0010` / `1.` / `.5` | 16→10 / 2→10 / 10→2 / 10→2 | | `-255` / `2` / `1` / `0.1` |
| empty, `.`, `-`, `1.2.3` | | | MSGBOX, returns 0 |
| bases 1, 17, 2.5, 0, -3 | | | MSGBOX and prompt again; `abc` reopens the dialog with "Syntax error" |
| `0.1`, precision 0 / 2.7 / 99 | 10 | 2 | `0.0` / `0.00` / 30 digits |
| Esc in dialog 1–4 | | | returns 0, no result printed |

Commands run (all exit 0):

```text
cargo fmt --all -- --check
scripts/emulator/run.sh check  --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings   # 0 warnings
scripts/emulator/run.sh test   --workspace --all-targets   # 33 binaries: 992 passed, 0 failed, 1 ignored
tests/ide_xvfb_smoke: script_{workflow,syntax,debug,gfx,input,stop,runconfig} → assert.py ALL ASSERTIONS PASSED
tests/ide_xvfb_smoke: script_baseconv{,_cancel} → assert_baseconv.py 4/4 PASS
ppl compare tests/compat/cases/strings_baseconv.toml   # emulator side: 20 UNVERIFIED (no G1 evidence yet)
```

Hardware: no G1 evidence was collected for this suite yet. Typed-INPUT
behaviour is backed by the forum statements above, not by this project's
own captures, and stays **UNVERIFIED**.

## F. Usage

```bash
# Check (same diagnostics as the IDE Problems panel)
cargo run --release --bin ppl -- check tests/fixtures/baseconv/Baseconv_fixed.hpprgm

# Run headlessly; one --input per field, '<ESC>' cancels a dialog
cargo run --release --bin ppl -- run tests/fixtures/baseconv/Baseconv_fixed.hpprgm \
  --input FF --input 16 --input 10 --input 10

# IDE: open the file, Run, type into each dialog, Enter = OK, Esc = cancel
cargo run --release --bin ppl -- gui tests/fixtures/baseconv/Baseconv_fixed.hpprgm
```

The release binary is built into Cargo's target directory
(`../.cache/prime-emulator-target/release/ppl` with this repository's
`.cargo/config.toml`). Any `target/release/ppl` in the repository is stale.

## G. Remaining limitations

- Entries in untyped fields are evaluated in a fresh machine: they cannot
  refer to the program's variables or to globals.
- INPUT's reset and initial values (arguments 5–6) and field positions
  (`{x,width,line}`) are accepted but not applied.
- Mixed type lists (for example `[0,2]`) use simple membership. HP staff
  reported firmware quirks with lists containing 0 (UNVERIFIED).
- An empty `[2]` field yields `""`. `INSTRING(s,"")` returns 0, and
  `MID`/`LEFT` clamp out-of-range lengths. All are UNVERIFIED on G1.
- Re-prompt messages are the emulator's wording, not the firmware's.
- The program's arithmetic is 12-digit decimal; the conversions are not
  exact for larger values.
