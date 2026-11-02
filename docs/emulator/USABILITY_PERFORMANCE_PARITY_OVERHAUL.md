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

## Autonomous implementation continuation (2026-11-02)

The earlier "not implemented" and uncommitted statements above describe the
first increment only. The dedicated branch now contains reviewed local commits;
no changes were pushed. Integration and final validation are in progress.

### Implemented and integrated

- Firmware Run (`script.rs`, `session/job.rs`): consumption-paced key scripts,
  explicit loader handoff, and conservative reuse of unchanged prepared source.
  Cache eligibility checks exact source/container hashes, compiler observations
  and filesystem mutation evidence. External keyboard/touch/scripts and
  Stop/OFF invalidate it. Changed source still passes through firmware checking.
- Recovery (`recovery.rs`, `session.rs`, VFS): versioned, SHA-256 checked,
  fsynced generations after completed firmware OFF; atomic publication and a
  resumable restore journal; interrupted drives are archived, not overwritten.
  Startup baselines are labelled separately. No live CPU/RAM restoration or
  periodic OFF/ON autosave is claimed. Unsaved activity since the last OFF can
  still be lost. GUI reset/seeding now also respects the helper storage lock.
- Native INPUT: distinguish scalar list values from multiple field descriptors,
  validate initial/reset list cardinality, preserve variables on cancellation,
  expose Reset, and round-trip string/list values correctly. EVAL/STARTAPP
  explain which firmware service is required instead of claiming support.
- Allocation failure: guest malloc/calloc/realloc return null on exhaustion or
  overflow; failed realloc preserves its old allocation. Opt-in CLI
  `--heap-budget-mib` constrains allocations independently of the mapped arena.
  This is an experimental allocation budget, **not** a complete 32 MB G1 model.
- Setup: shared ARM ELF/resource validation; explicit resource paths for ROM
  and initialized data; file/directory pickers and a Validate action in Settings;
  malformed configuration no longer silently falls back to unrelated paths.

### Performance evidence so far

Single samples from isolated release firmware sessions, identical generated
requests, timing request receipt through Result (including program output).
These are not interchangeable with the earlier Run-to-body-entry measurements.

| Workload | Handoff only | Consumption pacing | Pacing + prepared cache |
|---|---:|---:|---:|
| ~10 KB FIREWORKS | 43.480 s | 36.893 s | 37.126 s |
| Following tiny program | 11.967 s | 8.161 s | 7.500 s |
| Unchanged tiny rerun | 12.032 s | 7.773 s | 0.976 s |

The large program still reports the authentic existing `m_info` runtime error;
these figures do not establish successful large-program execution. Loader
handoff alone does not save its entire timeout because loading overlaps it.
Evidence/methodology: `/tmp/prime-perf-{handoff,paced,cached}-evidence`.
Cold/edited/alternating workloads and final integrated GUI timings remain to be
completed. A post-import catalog ordering limitation fails closed rather than
risking checking or running the wrong source; investigation continues.

### Validation in progress

Native first increment: 475 tests passed, 2 ignored; performance increment:
47 library tests, Clippy, and real firmware session integration passed before
integration. These are component results, not the final integrated suite.
No new physical device measurements have been taken. The 77 historical captures
remain unchanged. Context-aware keyboard work, INPUT layout, post-import catalog
handling and broad integrated validation continue in isolated owned worktrees.

Further Native integration: `598105c` (canonical `b07df7c`) preserves INPUT
position expressions, supports screen-percentage placement, seven-line pages
and page titles. Component validation: 482 passed, 0 failed, 2 ignored; fmt and
Clippy clean. Captures `/tmp/native-positioned-input.png` and its `.seven.png`
variant were visually inspected by the implementation agent. This is guide-based
Native behavior, not a physical parity claim. Exact firmware rounding, overlap
and spatial navigation, checkbox/radio/choose controls remain outside coverage.

Integrated emulator library: 55 passed. GUI profile-lock tests: 5 passed.
Both supervisor and direct-helper locks now protect reset and seeding, and
locks are held through reset's rename. Real firmware boot heartbeat passed with
assets configured (1.03 seconds); full session and crash tests are still running.

### Integrated recovery regression and fix

Real firmware testing caught an integration bug: `SharedState::sleeps()` counts
completed sleeps (wakeups), while recovery initialized its last-saved counter
to zero. The first OFF was therefore skipped. The saved counter is now optional
(`None` until a checkpoint succeeds or fails), so the first sleep is processed.
The final failure status also includes the underlying cause. A separate VFS
regression test ensures expected `r+` probes of nonexistent files do not poison
later durability checks; actual failed writes still prevent checkpointing.

A real-firmware test now saves A=123 using OFF, restarts, changes A=456, kills
the helper, restarts again, and verifies A=123 and preserved interrupted-drive
archives. It passed (29.40 s); in-process OFF/ON crash coverage is being added.
These are disposable profiles, not user data or physical hardware. Initial
failed runs are retained in `/tmp/overhaul-{real-firmware-assets,crash-final}.log`;
passing evidence: `/tmp/overhaul-recovery-first-off.log`.

### Parity and regression checkpoint

- Integrated workspace: **1632 passed, 0 failed, 6 ignored** at this checkpoint
  (`/tmp/overhaul-workspace.log`). Includes the previously intermittent journal
  test; its earlier failure is not claimed to be diagnosed or fixed.
- Historical 15157 replay: **77/77 recorded references reproduced**, classified
  as 68 exact / 4 partial / 5 transcription, zero mismatches. Artifacts:
  `artifacts/overhaul/parity-15157/{matrix.json,matrix.md,raw}`. No new device I/O.
- New `g1_pixel_geometry.ppl` plus `scripts/compat/check_pixel_geometry.py`:
  all **76,800 pixels** match geometric expectations in both Native and local
  firmware 15751. Rectangle, line endpoints, isolated pixel and unchanged
  background checked; capture visually inspected. Checker tests reject shifted
  pixels and resized images. Physical comparison remains untested.
- Experimental 32 MiB allocation budget: local firmware boot and `2+2=4`
  captured and visually verified (`/tmp/prime-32m-budget`). No claim of full
  G1 memory equivalence or broad low-memory application compatibility.
