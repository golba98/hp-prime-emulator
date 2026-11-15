# Usability, performance and parity overhaul

## Side-emulator continuation — 2026-11-12

The 33-finding implementation and current verification record is maintained in
[side-remediation/REMEDIATION.md](side-remediation/REMEDIATION.md), with the original
audit, inventory and per-finding matrix beside it. This continuation implements
Native calculator workflows and concrete firmware fixes; unresolved firmware
clipboard, live restore, OS/device coverage and remaining native parity limits
are explicitly retained. No new physical-device verification is claimed.

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

Prior-overhaul validation (4adcc46): **1663 Rust tests passed, 0 failed, 6 ignored**;
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

Latest architecture continuation: **1717 tests passed**, strict differential
foundation and setup doctor integrated; see the final checkpoint below for
new fixes, three-trial timings and remaining adapter/snapshot work.

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

## Architecture continuation — 2026-11-02

Resumed from 4adcc46 after confirming clean main and switching back to the
preserved feature branch. The prior measured improvements remain the baseline;
43.9/13.6-second launch figures must not be reused as current performance.

Integrated so far:
- 1519f1d: fixed aligned heap-tail arithmetic, single-byte C terminators,
  prevalidated memory reads/writes, UTF-16 surrogate decoding. Seventeen memory
  regressions passed in isolation, including the three independently reproduced
  boundary failures.
- 42f1771: checked guest file-transfer ranges and SVC stack arithmetic; handler
  marshalling faults propagate with actual CPU/thread context instead of silently
  returning zero. Memory/SVC/loader tests and genuine boot passed before handoff.
- 845c9bf: Native INPUT preserves omitted descriptor metadata; malformed and
  unsupported control forms fail before Host dialogs/writeback. Component suite
  503 passed, 2 ignored, with additional focused regressions and clean Clippy.
- 0eb9b95: recovery IDs remain ordered after backward host-clock changes;
  staged/restored drive hashes are checked before further renames. Ten recovery
  tests pass. Session protocol 5 exposes managed-program availability separately
  from calculator operation. GUI and helper reject managed commands without a
  verified observer; they do not invent a syntax verdict.

Actual firmware 15157 calculator-only verification: Home arithmetic passed;
all ten same-build Home/CAS/editor/Alpha/paste GUI assertions passed. The old
15751 boot reference scored 8/9 because its header differs on 15157 (bbox confined
to rows1–16); this failed cross-version assertion is retained, not relabelled as
an exact match. The new explicit calculator-only integration test passed,
including capability-before-Ready and rejected managed Check with no verdict.
Evidence: /tmp/prime-next-15157 and /tmp/prime-next-calc-only-test.log.

Architecture and full-system source audit documents are now present. They
explicitly distinguish HLE, source-present device models, unverified boot-chain
research and implemented behavior. Differential framework, lifecycle cleanup
and setup preflight contributions are still being tested independently; final
integration results will supersede this checkpoint.

## Integrated architecture continuation checkpoint

All three independent implementations are integrated on the canonical feature
branch. No agent builds remain active. Local commits only; no push, hardware
write, firmware modification or change to the 77 recorded reference fixtures.

### Implemented changes

- Core memory/SVC: aligned allocation/free-list bounds, exact C termination,
  whole-range validation, UTF-16 pairs, checked stack/file-I/O arithmetic and
  real marshalling-fault propagation. Code: memory.rs, services/fsops.rs,
  services/mod.rs and engine.rs; regressions include neighboring allocations,
  enormous requests, invalid pointers and preserved stream state on rejection.
- Thread ownership: Weak spawner reference, worker joins on normal/early exit,
  stack RAII and first child-failure reporting with actual registers/thread ID.
  Synchronization operations hold descriptor leases; close wakes waiters and
  prevents recycled-address writes. Semaphore and critical waits park the
  snapshot gate. SVC JSON includes thread_id. Existing scheduler remains default.
- Recovery: monotonic IDs despite host clock rollback, checked overflow and
  verification of staged/already-installed drive contents before restore moves.
  Existing OFF saves, format compatibility and preserved archives remain intact.
- Firmware diagnostics: rejected source after catalog growth uses a unique,
  source-matched completed assignment context for direct checking. A direct
  syntax acceptance never proves executable preparation. Actual regression:
  import multiple programs → syntax error at line3 → successful valid Run.
- Capabilities: session protocol5 permits calculator-only firmware without
  compiler/runtime observers. Unsupported managed jobs receive no syntax
  verdict; GUI actions explain/disable unavailable execution. Protocol5 adds
  capability reporting, not a typed live-result notification API.
- Native INPUT: malformed or unsupported descriptors cannot silently become
  unrestricted fields; omitted metadata remains distinct; duplicate string-only
  codes retain literal string behavior. No dialog or writeback on rejection.
- Setup: shared read-only preflight, `ppl doctor`, async Settings validation,
  all missing-resource errors and stale-result rejection. Helper protocol and
  observer compatibility are explicitly NotChecked until actual startup.
- Differential foundation: new schema/Backend trait, strict typed comparisons,
  persistent Native SessionContext, structured one-shot HLE, qualified legacy
  physical replay, precise unavailable states and CLI. Source/setup/expectation
  binding, decimal-string arguments and nonempty images are validated. Existing
  legacy comparators/evidence remain untouched.

See ARCHITECTURE_ROADMAP.md, DIFFERENTIAL_TESTING.md and FULL_SYSTEM_RESEARCH.md
for responsibility boundaries, public interfaces, concrete usage and research
prerequisites. No QEMU fork build, kernel boot or complete live snapshot is
claimed. The local fork explicitly requires Python2 and predates modern QEMU APIs.

### Final validation

- Workspace: **1717 passed, 0 failed, 6 ignored** after integration and final
  differential hardening/CLI regressions. Logs:
  /tmp/prime-next-verified-workspace.log. Formatting and Clippy with warnings
  denied passed; logs /tmp/prime-next-verified-{clippy,fmt}.log.
- Earlier intermediate full run: 1678 passed, 1 known journal-lock failure,
  6 ignored. Serial transactions18/18 passed55.41s. The final full suite also
  passed that test, but its historical parallel flake is not claimed fixed.
- Real firmware: boot, both session/import/Unicode/migration/post-import-error
  tests, OFF and both kill/recover paths passed; final firmware log
  /tmp/prime-next-integrated-firmware.log. Calculator-only15157 boot and rejected
  managed Check passed, with capabilities arriving before Ready.
- GUI: seven final15751 workflows and100 assertions passed: boot9, typing10,
  CAS/graph/apps59, persistence22. All18 apps launched; computation images match
  existing references. Additional15157 input scenario10/10 passed. All selected
  GUI waits completed and helpers exited. Settings Validate was actually clicked
  in isolated Xvfb and its responsive result window was visually inspected.
- Physical-reference replay: unchanged **77 reproduced**, 68exact/4partial/
  5transcription; no new physical measurements. Artifacts:
  artifacts/architecture-continuation/parity-15157.
- Actual differential CLI: persistent Native state passed; Native/HLE arithmetic
  and full-frame graphics passed. The rational_difference fixture deliberately
  reports firmware15751 exact1/3 versus Native0.333333333333; this is retained as
  an observed typed difference, not normalized away or called a physical mismatch.

### Three-trial performance recheck

Same13 fixtures and source hashes; preserved previous-overhaul helper verified
against its committed benchmark hash, versus the current helper. All78 expected
operations passed. No simultaneous builds/other emulator sessions; PNG assertion
analysis overlapped the first baseline cold sample. Medians/ranges and helper
hashes are committed in evidence/architecture-continuation/benchmark-summary.json;
raw per-stage/CPU reports: artifacts/architecture-continuation/bench.

| Case | Prior median seconds | Current median seconds |
|---|---:|---:|
| Tiny cold | 7.049 | 7.361 |
| Tiny rerun | 0.750 | 0.751 |
| Small math | 2.356 | 2.382 |
| Medium | 2.858 | 2.935 |
| Large first submission | 3.084 | 3.083 |
| Large rerun | 1.330 | 1.403 |
| Edited Check | 1.750 | 1.694 |
| Edited Run | 1.332 | 1.312 |
| Alternate tiny | 2.805 | 2.734 |
| Syntax error | 6.969 | 1.742 |
| Valid after error | 2.755 | 2.715 |
| Tiny after restart | 7.989 | 7.947 |
| Rerun after restart | 1.102 | 1.053 |

These three samples are not a statistical tail-latency claim. Warm timings
remain practical; cold-loader preparation still needs fresh context evidence.

### Exact remaining engineering work

1. Add a source/request-bound typed live-session observation API and stateful HLE
   differential driver. Current HLE differential rejects multiple Runs, setup,
   variable probes, key/touch and power/restart steps rather than silently reset
   state. Native state is retained; physical replay remains legacy TYPE/STRING.
2. Extend physical scenario capture through the existing guarded manual workflow;
   never infer variables, graphics or persistence from old TYPE/STRING evidence.
   Full-system backend currently returns Unavailable, as designed.
3. Model remaining guest priority/suspend controls and general inter-engine memory
   ordering before promoting a cooperative scheduler or full live snapshots.
   Current admission gate protects captures, not deterministic guest scheduling.
4. Add opt-in snapshot retention/space reporting while preserving restore plans,
   interrupted archives and at least two compatible generations. KeepAll remains
   current behavior. Autosave still requires validated UI context or serialized
   complete runtime state; no unsafe forced OFF timer was added.
5. Capture remaining input contexts/lowercase lock and selection semantics;
   extend Native control widgets and structural EDITMAT only with observations.
   EVAL/STARTAPP remain precise Firmware-required operations.
6. Execute the documented isolated full-system build/synthetic device gates only
   with recorded toolchain/artifact provenance. Matching bootloader and NAND/OOB
   prerequisites remain missing from inspected supplied assets.

## M16 implementation checkpoint

Starting at `7fbe711`, the coordinator added opt-in owner-thread register
observations (`--trace-svc`) and isolated full-system prerequisite/smoke tooling.
The early workspace suite passes **1,720 tests, zero failures, six ignored**;
formatting and prime-emulator all-target Clippy pass. This result precedes the
stateful/recovery/capture agent integrations and must not be described as their
verification. Two research-preflight Python tests pass; the ARM926 smoke assembly
compiles, but has not linked or run. No new physical verification or performance
result is claimed. Detailed integration status lives in
`M16_STATEFUL_PARITY_AND_SNAPSHOTS.md`.


## M16 integrated completion

See M16_STATEFUL_PARITY_AND_SNAPSHOTS.md for current implementation,
limits, files and evidence. Genuine stateful HLE scenarios now cover named
modules, globals, edits, CAS, errors, graphics, key/touch waits and fresh restart
persistence. Native comparisons retain actual VM module state.

Explicit opt-in firmware OFF checkpoints have typed identity-bound receipts,
safe admission, atomic bounded publication and fresh-process recovery tests.
CPU/thread/framebuffer snapshots remain unsupported. Offline capture bundles
freeze every source revision and derive verification categories from evidence;
no new physical measurements or live USB collector are claimed.

Final verification: 1,780 workspace tests passed, zero failures, six ignored;
formatting and workspace Clippy passed; 110/110 GUI assertions; 15/15 explicit
configured release checks; all 77 historical classifications preserved
(68 exact, 4 partial, 5 transcription; zero mismatches). Known debug post-CAS
reboot stall also reproduces on unchanged normal PowerDown. Release recovery
passes and the checkpoint refuses unsafe idle admission.

Three fresh alternating trials per helper checked 78 expected benchmark results:
tiny rerun 0.751 to 0.776 s; large launch 3.033 to 3.131 s; syntax feedback
1.746 to 1.702 s. All 13 workloads remained within 10% of baseline.
Exact samples/hashes: evidence/m16/benchmark-summary.json. No major new speedup
is claimed. Nothing was pushed or published.

## M17 integrated implementation

See [M17 runtime fidelity and full-system execution](M17_RUNTIME_FIDELITY_FULL_SYSTEM.md)
for the final source, evidence and limits. Native external functions now execute
in one linked VM across the SDK, CLI and IDE, with revision-bound private state,
shared calculator slots, dependency invalidation and callee diagnostics. Three
real retained Native/HLE scenarios pass 51 assertions, including initialization,
edits, nested matrices/vectors, complex values and integer variants.

Boot responsiveness is separated from strict checkpoint quiescence. Desktop
translation now waits for actual main-UI acknowledgement, correcting an observed
partial bracket paste. Mode-aware OFF fixes the proven prelatched-Shift failure
without clearing Alpha or weakening safe persistence. Four modifier controls
save, wake and shut down successfully; Alpha lock persists on wake.

Protocol 7 provides bounded inspection and owner safe-point accounting. Heap /
allocator and read-only VFS components restore independently in fresh processes;
full running-calculator restoration remains unsupported. Integrity, resource,
corruption, FIFO preflight and descriptor-leak controls pass. Source-bound
forensics survive disposable profile cleanup; oversized semantic omissions are
explicitly unobserved. The inherited-description journal-lock failure is fixed;
attribution of every older intermittent flake remains unproven.

The pinned hp-prime QEMU model executes the self-authored ARM926 smoke under GDB.
The GPL INTC overlay fixes reproduced reset and FIQ-selection failures. Kernel
boot, FullSystem PPL and new physical-device verification remain unavailable.
All 77 historical classifications are preserved: 68 exact, four partial, five
transcription, zero mismatches.

Final regression evidence: 1,853 workspace tests passed, zero failures, six
ignored; formatting and warnings-denied workspace Clippy passed; 127 actual GUI
assertions; 19 configured release cases; 25 Python tooling tests. The final
debug CAS/OFF/fresh-process recovery passed in 71.91 seconds. Optimizing only the
native Unicorn dev dependency reduced matched-source recovery median from
91.22 to 80.51 seconds, without changing Rust debug settings, guest clocks or
the strict 80% checkpoint admission rule. Busy debug sessions can still refuse
that rule safely; retained diagnostics distinguish admission from power/save.

Five alternating production benchmark pairs verified all 130 expected results.
Median tiny rerun: 0.752 seconds; large cold launch: 3.008 seconds; large rerun:
1.354 seconds; edited Check: 1.748 seconds; syntax feedback: 1.792 seconds.
All 13 workload medians stayed within 2.7% of their fresh matched M16 baseline.
Production performance is preserved; no additional major speedup is claimed.
Exact samples, hashes, source and profile identities are in
evidence/m17/benchmark-summary.json and the final M17 report.
Nothing was pushed, published or written to physical hardware.

## M18 hardware/runtime implementation

Integrated from M17 reporting checkpoint `31cfa64`; tested source `11356a1`.
Actual self-authored ARM926 programs execute timer IRQ/FIQ handlers and return,
transmit UART, read/write GPIO and exercise MMU translation/permission/alignment/
prefetch exceptions. Dedicated timer and narrow ARMv5 alignment corrections remain
GPL research overlays. No proprietary boot chain or physical capture is claimed.

Native now supports owner-bound exported variables, ASCII imaginary-unit fallback
and verified retained-list subtype2. Linked debugging carries source revisions
through stepping, breakpoints, frames and immutable captured tabs. Synthetic
ARM/Thumb snapshots resume in fresh processes and explicitly recover a compatible
previous generation; whole-calculator live restoration remains disabled.

Workspace: 1,901 passing, 0 failures, 6 existing ignored; formatting/Clippy clean.
Five genuine Native/HLE comparisons: 94 assertions. Historical replay: 77 unchanged
classifications, 0 mismatches. Final GUI groups pass 169 assertions. See the [M18 report](M18_FULL_SYSTEM_HARDWARE_FOUNDATIONS.md) for
actual evidence, initial test failures and remaining limits.

Five paired benchmark trials pass 130 expected outcomes: tiny repeat 0.775s,
large cold 3.056s, large repeat 1.378s, syntax-error feedback 1.774s. All 13 workload
medians remain within 5% of the matched M17 baseline; no major new speedup claimed.
No push, proprietary-image modification or physical-device access occurred.

## M19 boot hardware and storage

M19 integrates NAND, clock/reset, LCD/input and synthetic boot diagnostics while preserving the selected PPL engine. Final verification passes1,919 workspace tests, formatting, Clippy,101 Native/HLE assertions,77 historical classifications and130 benchmark outcomes. Tiny repeat0.776s, large cold3.062s; largest paired median increase7.47%, with no>10% regression. No real kernel boot or physical parity is claimed. See the [M19 report](M19_BOOT_HARDWARE_AND_STORAGE.md) for evidence categories, limits and final verification.
