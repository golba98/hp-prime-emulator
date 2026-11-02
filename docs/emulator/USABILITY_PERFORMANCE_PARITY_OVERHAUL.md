# Usability, performance and parity overhaul

## Current integration status — 2026-11-02

Working branch: `feat/g1-usability-overhaul`, based on `3399675` (the first
input increment). Local checkpoint commits are authorized and recorded; nothing
has been pushed or published. No new physical-device verification is claimed.

Implemented and integrated: consumption-paced Run, exact-source prepared cache,
validated reuse of firmware assignment compilation, native AFiles source
transfer, safe OFF storage recovery and profile locks, Native INPUT defaults /
layout / pages, Native EDITMAT, setup resource validation and pickers, allocation
failure handling and an experimental heap budget. Firmware-observed desktop text entry is also integrated, with atomic paste
validation and Home/CAS/editor case control.

Final integrated validation: **1663 Rust tests passed, 0 failed, 6 ignored**;
formatting and Clippy with warnings denied passed. All 14 actual GUI scenarios
completed without wait timeouts; 138 automated GUI assertions passed. Actual
firmware boot, import/execution, loader migrations, OFF and crash recovery
passed. All 77 historical physical-reference results were reproduced (68 exact,
4 partial, 5 transcription-based), without new physical measurements.

Controlled identical-source benchmark: successful 10 KB Run **34.511 → 2.982 s**;
unchanged rerun **34.542 → 1.427 s**; tiny rerun **11.839 → 0.751 s**;
edit → Check → Run **66.484 → 3.132 s**. Single quiet-machine samples;
methodology, hashes, CPU times and complete case table are below.

This report keeps the chronological engineering record below. Statements in
"First increment" describe that checkpoint; later sections supersede them.
Remaining limits are recorded explicitly rather than calling every milestone
complete.

## First increment (historical checkpoint)

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

## Remaining work at the first checkpoint

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

### Subsequent integrated implementation

- The dominant large-program delay was interpreted UTF-16 byte conversion in
  the PPL loader. `AFiles` native string decoding replaces that loop; genuine
  `Programs():=` compilation remains. Known V1 and V2 helper source texts are
  migrated exactly, and unknown user-modified helpers remain refused. Unicode
  source and legacy migrations pass actual firmware tests. Reproducible
  benchmark: `scripts/emulator/benchmark_run.py` (wall/CPU, hashes, verdicts,
  stage reports, 13 workflows including restart).
- Session output now joins its writer after atomically queueing Closed. This
  fixes an observed race where process exit could drop the last save/Closed
  notification. A no-assets regression exercises eight immediate failing
  startups and requires their terminal event before EOF.
- Both real recovery scenarios pass: normal OFF→restart→edit→kill→restore
  (29.40 s), and live OFF→ON→edit→kill→restore (21.24 s). Normal session import /
  Run / OFF test also passes (53.79 s), including checkpoint notifications.
- Native EDITMAT is implemented with firmware-observed cancellation, M0–M9
  writeback and expression/local-variable semantics. Existing cells are edited
  in a paged grid; headers/read-only mode and validation are supported. See
  `NATIVE_EDITMAT_REPORT.md` and committed firmware-only / Native captures.
  Component results: 496 passed, 2 ignored; actual Xvfb GUI interaction passed.
  Row/column insertion, resizing and touch editing remain unsupported.

AFiles/recovery integration recheck: three recovery tests passed (including
terminal event delivery), and both firmware session tests passed, covering
V1/V2 loader migration, Unicode, large import and globals retained on unchanged
rerun. The first attempt failed while copying disposable profiles because
parallel build caches exhausted the `/tmp` quota; rerun with TMPDIR in the
workspace cache passed. Logs: `/tmp/overhaul-afiles-recovery-session-retry.log`.
Pre-keyboard integrated Clippy with warnings denied is clean.

The new IDE status messages follow actual loader/check/import/invocation
transitions, with no artificial progress timer. Native/firmware pixel captures
and source/capture hashes are retained in `evidence/pixel-geometry/`.

### Desktop typing integrated

`993b6d3` integrates protocol v4 Text packets, observed firmware Alpha/Shift
state, context-correct mixed-case entry, transport batching, bounded paste,
paired delimiters and proper modifier release. Whole unsupported pastes are
rejected before delivery; newlines and pasted powers are rejected rather than
executed or reinterpreted. Source editing in the IDE remains the multiline
workflow. Details and limits: `DESKTOP_TYPING.md`.

Component results: 65 emulator / 302 GUI tests passed (2 ignored), clean fmt /
Clippy, actual 15751 GUI 10/10. Attempted 15157 live GUI was blocked before
input by the existing compiler observer's firmware-version restriction. This
does not invalidate the separate catalog-driven 77-case replay, but it means
desktop typing is not verified on 15157. Unknown indicator screens fail closed;
firmware selection-to-system clipboard and exhaustive catalog/dialog behavior
remain unsupported or untested.

Final baseline binary is rebuilt from pristine committed `3399675`, not the
intermediate handoff-only binary used for exploratory measurements. The same
13-case benchmark script is running against it and the integrated helper with
other builds and firmware sessions idle. Source/helper hashes and CPU time are
retained in each run's `results.json`.

## Controlled integrated performance result

Pristine baseline `3399675` versus integrated `993b6d3`, Rust 1.96.0 release,
Intel i7-13700 / Linux 7.2.13, same local firmware 15751 and initial storage.
One sample per case, with other builds and emulator sessions idle; this is not
a statistical latency distribution. All 13 expected results/verdicts passed
on both revisions, with identical source hashes. Times are helper request to
Result, excluding IDE dispatch/rendering. The 10,160-byte successful fixture
is comment-heavy and measures source-transfer/preparation, not complex runtime
computation. Original FIREWORKS is tracked separately because of its m_info error.

| Workload | Before wall (s) | After wall (s) | Before CPU (s) | After CPU (s) |
|---|---:|---:|---:|---:|
| tiny_cold | 16.872 | 7.147 | 4.880 | 2.620 |
| tiny_repeat | 11.839 | 0.751 | 3.040 | 0.290 |
| small_math | 14.774 | 2.280 | 3.910 | 1.090 |
| medium | 17.876 | 2.858 | 7.660 | 1.220 |
| large_cold | 34.511 | 2.982 | 24.830 | 1.390 |
| large_repeat | 34.542 | 1.427 | 25.260 | 0.670 |
| edited_check | 31.330 | 1.725 | 24.210 | 0.930 |
| edited_run | 35.154 | 1.407 | 25.330 | 0.710 |
| alternate_tiny | 12.684 | 2.809 | 3.900 | 1.420 |
| syntax_error | 10.287 | 6.894 | 3.240 | 2.660 |
| after_error_tiny | 13.117 | 2.657 | 4.080 | 1.270 |
| tiny_after_restart | 18.212 | 8.032 | 6.080 | 3.340 |
| repeat_after_restart | 12.828 | 1.027 | 3.890 | 0.450 |

Edit → Check → Run combined: 66.484 → 3.132 seconds. Initial boot was
1.049 → 1.256 seconds; post-OFF boot 1.079 → 1.420 seconds. Recovery adds
validated storage snapshots at startup/OFF, not hidden background Run work.

Committed evidence: `evidence/overhaul-benchmark/{before,after}.json` and
`baseline-provenance.json`, including source/helper hashes, CPU times and stage
reports. Full disposable profiles/reports remain under
`artifacts/overhaul/benchmark-{before-339,after-integrated}` (ignored; contain
user-supplied proprietary resources). Reproduction: `scripts/emulator/BENCHMARKS.md`.

## Final integrated verification and boundaries

All independent contributions were cherry-picked into the canonical feature
branch, with the session typing/loader export conflict resolved. The writer
join, first-OFF recovery counter fix, helper locks and cache invalidation hooks
remain integrated. No push, publication, deployment, physical-device write or
proprietary binary change was performed. Existing program fixtures and recorded
physical evidence have no diff against the starting checkpoint.

Validation (final production code, local firmware 15751 where applicable):

- Workspace: **1663 passed, 0 failed, 6 ignored**. `cargo fmt --all -- --check`
  and `cargo clippy --workspace --all-targets -- -D warnings` passed. One new
  test needed formatting; it was corrected. Logs:
  `/tmp/overhaul-final-{workspace,clippy,fmt}.log`.
- Explicit firmware integration: boot heartbeat, both loader migrations,
  Unicode source, large import, unchanged-source globals retained, normal OFF,
  two actual kill/recover scenarios and terminal event delivery all passed.
  `/tmp/overhaul-final-real-firmware.log`.
- Actual IDE/Xvfb: **14 scenarios, zero wait timeouts, 138/138 assertions**:
  boot/restart/Stop/performance 24, CAS/graph/apps 59, persistence/crash/reset 40,
  repeat/equality 5, context-aware typing/paste 10. All 18 built-in apps opened;
  computation captures matched existing local-firmware references. No helper
  outlived the tested IDE. Setup dialog was captured and visually inspected.
  Evidence: `artifacts/overhaul/gui-integrated`, assertion logs
  `/tmp/overhaul-gui-*-assert.log`.
- GUI performance on the original scenario: FIREWORKS Run→body **42.1 → 7.6 s**,
  following small animation **13.8 → 2.7 s**, boot **2.0 → 2.1 s**, approximately
  **30 changed LCD frames/s** while animating. The historical baseline and
  final samples use the same scenario but are not repeated statistical trials.
  FIREWORKS still reaches its pre-existing `m_info(1,1)` Invalid input error;
  only body-entry latency is claimed for that program. The separate successful
  10 KB benchmark above returns 42 and has no such error.
- New pixel geometry: Native and firmware match all 76,800 expected pixels;
  checker regression tests pass. Native EDITMAT captures include six observed
  firmware cases, including special writeback for `LOCAL M1`; the corresponding
  Native regression passes. No new physical-device parity claim.

### Remaining technical limits and exact next work

1. **Input:** verified Home/CAS/editor behavior relies on captured firmware
   status glyphs. Unknown/custom screens fail closed. Lowercase Alpha lock,
   exhaustive catalog/dialog entry, arbitrary selection-to-system clipboard,
   multiline firmware-editor paste and linear-to-template power conversion
   need further context/selection observations. Multiline PPL belongs in the
   IDE source editor; unsafe clipboard input is explicitly rejected. Do not
   enable these paths by guessing key sequences.
2. **Performance:** first-session loader preparation still costs about 7–8 s;
   syntax rejection can require the older catalog/direct diagnostic fallback.
   Investigate validated loader-state reuse across restart and invalid-source
   diagnostics after catalog growth, retaining exact source/context evidence.
   Rebenchmark computationally complex large programs separately from the
   comment-heavy transfer fixture, and collect repeated timing distributions.
3. **Recovery:** only startup storage baselines and completed OFF generations
   are recoverable. Unsaved activity since OFF can be lost; no periodic autosave
   or live CPU/RAM restoration is claimed. Guest threads use host thread,
   mutex/condition-variable, guest-call and file-handle state; restoring RAM and
   Unicorn registers alone would be inconsistent. Periodic forced OFF also
   needs a validated UI context to avoid changing an unfinished edit/dialog.
   Next: model those states or prove safe save contexts, then add controlled
   autosave and storage-retention policy. Current generations are retained
   without pruning, and recovery failure preserves existing data.
4. **Native:** EVAL's CAS evaluation and STARTAPP's calculator applications
   still require Firmware mode, with explicit compatibility warnings and
   runtime diagnostics. Native EDITMAT edits fixed-size cells; structural
   menus and touch are not implemented. INPUT checkbox/radio/choose controls
   and exact firmware layout rounding remain outside verified support.
5. **Parity/memory:** new graphics/input/app/persistence results are local
   firmware comparisons. Physical G1 captures are still needed for those
   areas, touch, resource limits and shutdown behavior. The 32 MiB option is an
   allocation budget, not total physical RAM: maps, stacks and host resources
   are separate. The live IDE compiler observer currently rejects firmware
   15157; the separate catalog-driven historical replay works. Extend that
   observer only with version-matched evidence; never bypass its guard.
6. **Setup/platforms:** resource pickers, validation, saved configuration and
   actionable unavailable/helper-version errors are implemented. Firmware mode
   remains Linux-only and requires user-supplied resources. Automatic packaging,
   OS dependency installation and other-platform firmware ports are not claimed.
   Build/install matching `ppl` and `prime-emulator` binaries: desktop Text uses
   session protocol **4**, and older helpers are rejected with an explanation.

These boundaries are not represented as full hardware equivalence or complete
implementation of every HP service. The completed changes deliver a practical
faster edit/check/run path while retaining explicit Native/Firmware distinctions.
