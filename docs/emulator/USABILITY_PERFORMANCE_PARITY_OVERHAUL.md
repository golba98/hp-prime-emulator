# Usability, performance and parity overhaul

## Status — 2026-11-02

Implementation started on `feat/g1-usability-overhaul`, from main `7014e75`.
User confirmed release of canonical ownership. No commits or device access.
This is an implementation checkpoint, not completion of milestones A–F.

## Baseline

Intel i7-13700, Linux 7.2.13, Rust 1.96.0, release build, local G1 firmware
15751. Workspace: 1595 passed, 0 failed, 6 ignored; fmt and clippy passed.
Explicit firmware boot and session tests passed (large import and normal OFF
persistence included). GUI boot/performance: 15/15 assertions.

One measured baseline: boot 2.0 s; FIREWORKS_G1 (~10 KB) Run to body entry
42.1 s; following small animation program 13.8 s. FIREWORKS subsequently
fails at `m_info(1,1)` with Invalid input; execution success is not claimed.
Evidence: `/tmp/hp-prime-overhaul-gui-baseline`; baseline logs:
`/tmp/hp-prime-overhaul-baseline-*.log`.

## Milestone A — partial implementation

Root causes reproduced with genuine local firmware screenshots:
- Repeat events discarded by GUI, while firmware does not repeat held keys.
- Any repeat event suppressed all text in the same frame.
- Equality mapped to Enter.
- ReleaseAll appended releases behind stale queued presses.
- CAS aA becomes Aa because translation assumes Home (still unresolved).

Changes:
- `prime-emulator/src/session/repeat.rs`: monotonic 450 ms initial delay,
  50 ms repeat interval, only arrows/backspace; consumption-paced release/press
  pairs, no accumulated catch-up. Firmware ignored repeated down-only events
  in the first GUI experiment, which led to the paired implementation.
- Session cancels pending input on ReleaseAll and releases delivered keys;
  release cancels repetition immediately. Repetition pauses while guest is busy
  and is disabled during managed jobs/power-down.
- `ppl-gui/src/app_firmware.rs`: preserve text in mixed repeat batches;
  equality sends physical Shift + decimal; release is processed even when
  desktop command modifier is down.
- Unit regressions cover repeat timing, release, no catch-up, action-key
  exclusions, queue clearing, mixed repeat/text and equality translation.

Limitations: CAS/editor/search/dialog context detection, literal parentheses,
selection, clipboard and desktop-vs-physical modifier routing remain pending.
Typing with already latched physical modifiers is not yet normalized.

## Milestone B — initial optimization

Catalog observation cloned the entire filesystem trace every session tick,
including while idle. Added incremental trace reads with absolute cursors,
retaining overflow accounting. Unit test covers new events, empty tails and
visible overflow. No source/compilation cache or validation bypass introduced.
End-to-end latency improvement is not yet established.

## Remaining work, in order

1. Finish A: observe firmware context by version-validated evidence; implement
   Home/CAS/editor mappings, modifier ownership and literal punctuation. Expand
   raw GUI event automation for focus loss, held arrows, NumPad and paste.
2. B: preserve all eight stage timings, compare identical workloads repeatedly,
   then add source/context/namespace cache invalidation and validated direct
   preparation. Rebenchmark cold, repeated, edited and alternating programs.
3. C: implement idle firmware OFF/ON checkpoint transactions, immutable hashed
   generations, fsync/atomic publication and interrupted-shutdown recovery.
   RAM-only serialization is insufficient for host-backed guest threads.
4. D: INPUT reset/initial/layout and EDITMAT host dialog with evidence-backed
   semantics; actionable STARTAPP/EVAL firmware-required diagnostics.
5. E: replay the 77 historical results on 15157; new graphics/input/app/storage
   fixtures; opt-in experimental memory budget and correct guest OOM handling.
6. F: shared asset validation, setup wizard and progress/recovery UI.

No new physical parity evidence. Historical 77-case results were inspected,
not rerun in this implementation checkpoint. Recovery, Native command support,
memory profiles and setup changes are not implemented yet.

## Validation of the first increment

- Workspace after initial changes: 1600 passed, 0 failed, 6 ignored.
- Emulator unit tests after switching to release/press repeat: 44 passed.
- Clippy with warnings denied and formatting passed.
- Actual GUI probe: after a fully settled `abcdefghij`, holding Delete for
  two seconds now clears the line. Baseline removed only `j`.
- Actual GUI CAS capture: `=` stays in the editing line rather than evaluating.
  The separately captured case inversion is still present.
- Reproducible scenario: `tests/ide_xvfb_smoke/run_m12.sh m16_input 120`;
  then `python3 tests/ide_xvfb_smoke/assert_m16_input.py <run>/m16_input`.
  It compares typed equality with the firmware's physical Shift+decimal and
  verifies held deletion clears a nonempty input line.
- Evidence of inspected screenshots: `/tmp/hp-prime-overhaul-repeat-after`.

### Performance recheck (single sample, not a statistical claim)

Same release workload/scenario: large Run to body entry 43.9 s (baseline
42.1 s), following small Run 13.6 s (baseline 13.8 s), boot 2.1 s
(baseline 2.0 s). All six performance assertions passed; drawing delivered
30.4 changed frames/s. Large program retains its existing m_info error.
This does not establish an end-to-end improvement. A test/build overlapped
part of the after run, so these samples are not a controlled speed comparison.
Use repeated isolated measurements before making performance claims.
Evidence: `/tmp/hp-prime-overhaul-performance-after`.

### Final checkpoint verification

- Final workspace run: 1600 passed, 1 failed, 6 ignored. The failure was the
  previously recorded `ppl-programs` transactions journal-lock race
  (`verification_only_and_partial_write_errors_cannot_prove_no_submission`).
  That crate is unchanged; a serial rerun is recorded separately below.
- Final release GUI `m16_input`: 5/5 assertions passed, no wait timeout or
  orphan helper. Screenshot visually inspected: held deletion leaves an
  empty input line. The assertion excludes the blinking cursor and allows
  the horizontal input border (initial assertion incorrectly counted that
  border as residual text; corrected using the inspected screenshot).
- Final formatting, clippy with warnings denied, and diff whitespace checks
  passed. Logs: `/tmp/hp-prime-overhaul-final-{tests,clippy,build}.log`.
- GUI evidence: `/tmp/hp-prime-overhaul-gui-final/m16_input`.

The branch remains uncommitted. Start the next implementation pass with
firmware-context observation and case/literal punctuation routing; do not
mark A complete or start making fast-rerun claims from this checkpoint.

Serial rerun of unchanged transaction suite: **18/18 passed**, including the
previously failed test (55.31 s). Log:
`/tmp/hp-prime-overhaul-transactions-serial.log`. The parallel flake remains
unfixed and is not counted as a clean final workspace run.
