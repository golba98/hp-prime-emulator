# Diagnostic parity: editor diagnostics against the HP Prime

The editor and the Problems panel must not reject valid HP Prime PPL, miss
errors the calculator reports, or present the emulator's own analysis as
the calculator's verdict. This document describes how that is measured.

## Pipeline

```text
tests/diagnostics/cases/*.ppl ── cases.toml
        │
        ├──► original G1 firmware Check (local emulator)  ── ppl diag-parity capture
        │         └─► reference/local-firmware-g1-2.4.2.toml   (stored, reviewed)
        │
        ├──► physical HP Prime G1 Check (manual)
        │         └─► reference/physical-g1.toml               (stored, reviewed)
        │
        └──► native analyzer = the GUI's live analysis
                  compile_source(SourceFile::new(..), CompileOptions::default())
                          │
                          ▼
              ppl_compat::diag_parity::compare_case ──► parity report
                          │
                          ▼
              parity-baseline.toml (ratchet test)
```

References are captured separately and never derived from the native
analyzer, so the comparison is not circular. Each reference result carries
the hash of the exact source it checked; editing a case invalidates its
reference until it is re-captured.

## Provenance levels

| Level | Producer | Meaning |
|---|---|---|
| `native-compiler` | Rust parser + semantic analysis | Provisional editor analysis. Rules marked "(unverified rule)" have no calculator evidence. |
| `guest-firmware` / `local-firmware` | Original G1 firmware 2.4.2 (`armfir.elf`, SHA-256 `ad9779ff…fb39`) running its Program Editor Check under the local Unicorn emulator | Authentic HP compiler behaviour for that firmware version. Not a physical-calculator observation. |
| `physical-device` | A physical HP Prime G1, observed manually | The only level that can be tagged VERIFIED. None recorded yet. |

The physical G1 runs build 15157, not 2.4.2. A local-firmware result is
strong evidence about the HP compiler, not proof of what that calculator
accepts.

## Commands

```sh
# Compare the native analyzer with the stored firmware reference (offline).
ppl diag-parity report                 # Markdown table
ppl diag-parity report --json
ppl diag-parity report --check-baseline  # exit 1 on any baseline difference

# Compare two reference files (verdict, status, message, line, column,
# UTF-16 offset; provenance fields ignored). Exit 1 on any difference.
ppl diag-parity diff reference/a.toml reference/b.toml [--only id1,id2]

# Re-capture the reference from the original firmware (local emulator only;
# about 7 s per case with initialized data and the release runner).
export PPL_FIRMWARE="$PWD/scratch/research/firmware/extracted/fat/armfir.elf"
export PPL_OS_ROM="$PWD/scratch/research/firmware/extracted/PRIME_OS.ROM"
export PPL_FACTORY_A="$PWD/scratch/research/m7-private/factory/A"
export PPL_FIRMWARE_INIT_DATA="$PWD/scratch/research/m7-private/initial-ready/data"
export PPL_FIRMWARE_RUNNER="$PWD/../.cache/prime-emulator-target/release/prime-emulator"
ppl diag-parity capture --overwrite \
  --artifacts-dir artifacts/diagnostic-parity/capture-$(date -u +%Y%m%dT%H%M%SZ)

# The same cases through one long-lived firmware session, as the IDE's
# firmware backend runs them (new profile from PPL_FIRMWARE_INIT_DATA, never
# the user's). --reverse checks them in reverse order to expose leaked state.
# Writes latency.json (cold, warm, per UI stage) next to the case evidence.
ppl diag-parity capture --engine session|direct [--reverse | --shuffle SEED] [--repeat N] \
  --output artifacts/diagnostic-parity/session.toml \
  --artifacts-dir artifacts/diagnostic-parity/session-$(date -u +%Y%m%dT%H%M%SZ)
```

A re-capture must not change a stored result unnoticed: capture to a new
file, `diff` it against the stored reference, and replace the reference
only when every existing case is identical (new cases may be added).

`capture` refuses to run without firmware configuration; there is no
native fallback. A case for which the firmware reached no verdict (timeout,
infrastructure error) is recorded as `infrastructure_error`, never as
accepted or rejected, and the command exits 1. The per-case engine reports
(compiler entry/return, exact parser source, dialog text) are kept under
the artifacts directory, which is git-ignored because it can contain
proprietary firmware state; the reference file retains the result fields.

The ratchet test `cargo test -p ppl-compat --test diagnostic_parity` runs
offline. It fails when a case lacks a current reference, when a measured
outcome differs from `tests/diagnostics/parity-baseline.toml`, and when an
improvement is not yet recorded there. Independently of that file it also
fails on any verdict disagreement (false rejection or missed error), when
fewer than a fixed number of cases match verdict and position, when the
corpus shrinks, and when the reference was not captured from the known
`armfir.elf`; those floors live in the test source, so relaxing them is a
visible code change. `report --check-baseline` likewise exits 1 on any verdict
disagreement whatever the baseline records.

## How outcomes are judged

| Outcome | Meaning |
|---|---|
| `match` | Same verdict; when rejected, the native first error is at the firmware's line and column |
| `location-differs` | Both reject, at a different position |
| `false-rejection` | Native rejects a program the firmware accepts |
| `missed-error` | Native accepts a program the firmware rejects |
| `no-reference` / `reference-stale` | No usable reference |

Firmware facts that shape the comparison:

- The firmware reports **only the first error**. Further native errors are
  listed as "additional" and are neither confirmed nor contradicted.
- The firmware message is always the generic **"Error: Syntax Error"**,
  including for undefined names and wrong argument counts. Messages are
  therefore reported side by side as `not-comparable`, never as matching.
- Firmware positions are where its parser stopped, often the token after
  the mistake: end of file for a missing END, the `,` after the last
  accepted argument for too many arguments, the `)` for too few.
- Columns are Unicode-scalar columns on LF-normalized text, the same unit
  the editor uses.

## Physical procedure

See the header of `tests/diagnostics/reference/physical-g1.toml`. Uploads
need the user's explicit approval and use fresh `ZZT_` names; nothing is
automated.

## Baseline

Corpus: 75 cases (`tests/diagnostics/cases.toml`): the original 39 and 36
keyword-case probes (`k01`–`k36`, milestone 2). Every result in the stored
reference was captured from the original firmware. Every capture agreed on
every case it shared with another (verdict, message, line, column, UTF-16
offset): four milestone-1 captures, two full milestone-2 fresh captures (the
second with the rebuilt runner) and two session captures. Run `ppl diag-parity report` for the full table.

| | Milestone 1 start | Milestone 1 | Milestone 2 | Location parity |
|---|---|---|---|---|
| Verdict agreement, original 39 | 31/39 | 37/39 | **39/39** | **39/39** |
| Verdict agreement, all 75 | — | — | **75/75** | **75/75** |
| Location agreement, original 39 (rejected cases at the same line:column) | — | 15/23 | 15/23 | **23/23** |
| Location agreement, all 75 | — | — | 16/32 | **32/32** |
| `false-rejection` / `missed-error` | 2 / 6 | 2 / 0 | 0 / 0 | 0 / 0 |

The location-parity column is `docs/compatibility/native-location-parity.md`,
which also adds the `location-probes` and `location-arity-controls` corpora.

Milestone 1 corrections (native analyzer now rejects, as the firmware does):

| Case | Firmware | Rule change |
|---|---|---|
| `e06` `BLIT_P();` | rejected 3:9 | `BLIT_P` arity 1–11 (was 0–11) |
| `e18` `BLIT();` | rejected 3:7 | `BLIT` arity 1–11 (was 0–11) |
| `h03` `RETURN 1` + `END;` | rejected 4:1 | RETURN needs `;` before END (error at the END) |
| `h13` `RETURN 1` + IF's `END;` | rejected 5:2 | same rule |
| `h09` final `END` at end of file | rejected 5:1 | a function's END must be followed by `;` |
| `h10` `END` then next `EXPORT` | rejected 6:1 | same rule |

Firmware-accepted forms that stay accepted: no `;` before an IF block's END
(`h04`), before ELSE (`h05`), before UNTIL (`h06`), before the function's END
after an assignment (`h12`) or after a nested block (`h11`).

### Keyword letter case (milestone 2)

`h01` and `h07` were false rejections: the native lexer knew upper-case
keywords only. Probes `k01`–`k36` established the firmware's rules, which
are contextual rather than a blanket case fold:

| Firmware behaviour | Cases |
|---|---|
| Keywords in any ASCII case (`export`, `Begin`, `ExPoRt`, `downto`, `iferr`, …) are accepted where the grammar expects them | h01, h07, k01–k05, k24 accepted |
| The same words, other than upper case, are names in declarations and operands: `LOCAL begin:=4; RETURN begin+1;` | h08, k08–k20, k23 accepted |
| A non-upper-case keyword at the start of a statement followed by `:=` is a name | k36 accepted |
| A lower-case `end` before `;` closes its block, even as `RETURN end;` | k35 rejected; k30 rejected |
| `MOD`, `AND`, `OR`, `XOR` are operators in any case, also next to a `mod` variable | k06 (up to `div`), k21, k25–k29 accepted |
| `DIV` and `NOT` are operators only in upper case (`div`, `Div`, `not`, `Not` are names) | k06, k07, k32–k34 rejected; k31 `not(0)` rejected as an unknown function |
| Upper-case keywords are reserved: `RETURN MOD+1` | k22 rejected |

The native analyzer implements this as *soft keywords*: the lexer marks a
keyword spelt other than in upper case (`Token::soft`, keeping its spelling
and byte span; the source is never rewritten), and the parser reads it as a
name in name and operand positions. Highlighting and auto-indent reuse the
parser's resolution (`parser::resolve_soft_keywords`). A regression scan of
the 58 `.ppl`/`.hpprgm` files under `examples/`, `tests/` and crate fixtures
gave byte-identical `ppl check` output before and after the change.

Known disagreements after milestone 2: 16 `location-differs`, where the
native first error was at the start of the construct and the firmware's
where its parser stopped. Native location parity
(`native-location-parity.md`) since reports those positions as the firmware
does: the baseline now records no disagreement, 32/32 locations match.

## Firmware session (milestone 2)

The IDE's firmware backend runs programs in one long-lived firmware session
(`prime-emulator session`). Its Check stage now returns the guest's verdict
with the result (`Event::Result.check`): accepted or rejected, the exact
source hash the compiler parsed, and the guest diagnostic. The position is
mapped by the same function the fresh checker uses
(`checkprog::firmware_diagnostic`); an unmappable position is reported as
unavailable, never estimated, and a failure without a validated observation
carries no verdict. The IDE applies it to the tab revision that sent the
request, so a session Run's rejection appears in Problems and as a marker.

Fixed on the way: after its first request the session refused every Check
("catalog order could not be verified"), because the firmware's Program
Catalog re-reads only programs changed since it was last shown. The session
now infers the order from that partial read (`job::catalog_order`); a wrong
inference opens another program, which the source-hash validation rejects,
so it fails without a verdict instead of misattributing one.

| Comparison (75 cases) | Differences |
|---|---|
| Fresh checker vs stored reference (39 original cases) | 0 |
| Session, corpus order vs fresh checker | 0 |
| Session, reverse order vs fresh checker | 0 |
| Session, corpus order vs reverse order | 0 |

## Direct firmware Check (milestone 3)

`--engine direct` (and the IDE's Check) calls the original firmware compiler
directly inside the running emulated firmware, using the firmware's own
`Programs():=` set-and-compile sequence, without driving the calculator UI.
Details, contracts and evidence: `docs/emulator/M9_DIRECT_COMPILER_REPORT.md`.

| Capture (75 cases) | Differences from the stored reference |
|---|---|
| corpus order, reverse, shuffled | 0, 0, 0 |
| shuffled, every case 3× in one session (225 checks) | 0, and every repeat identical |

| Path | Cold | Warm p50 | Warm p95 |
|---|---|---|---|
| Fresh checker | 6.9 s per Check | — | — |
| Session, UI-driven | 15.5 s | 10.6 s | 11.1 s |
| Session, direct | 10.6 s | 12.3 ms | 16–23 ms |

## Check latency (milestone 2, measured)

| Path | Cold | Warm (median, p95) |
|---|---|---|
| Fresh checker, one firmware boot per Check | 6.9 s per Check (no warm state) | — |
| Live session | 15.5 s to the first verdict | 10.6 s, 11.1 s |

Fresh-checker phases (median of 63 reports): boot 0.61 s, program discovery
0.21 s, **compiler 1.8 ms**, whole guest session 6.66 s. Warm session stages
(median of 74): run the source loader 5.0 s (typing `ZZTIDELOAD`), open the
Program Catalog 1.9 s, open the editor and Check 3.6 s. Almost all the time
is calculator UI driving, which every Check needs because the verdict is the
Program Editor's own; boot is a small share, so reusing the running firmware
does not make Check faster. Milestone 3 removed the UI driving instead (see
above); the IDE's Check now uses the direct checker, with the fresh checker
as its fallback.

## Limitations

- One firmware version (2.4.2); the device's build 15157 may differ.
- First error only, generic message, no error code.
- A warm direct Check takes about 12 ms; the first Check of a GUI session
  uses the fresh checker (about 7 s) while the direct checker boots (about
  10.6 s).
- Firmware-backend Run of a valid program can still end with "Firmware
  runtime: Unsupported" while the calculator screen shows the right value
  (runtime-evidence capture, outside diagnostics).
- Linux only (the managed runner).
