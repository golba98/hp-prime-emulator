# M17 — Runtime fidelity and full-system execution

Starting point: feat/g1-usability-overhaul@7e33a62. Implementation is in progress.
No new physical captures, hardware writes or proprietary firmware modifications.

## Verified coordinator increments

### Journal lock lifetime

A new parallel process/thread test reproduced false Busy after the owner closed
its descriptor. flock belongs to the open file description, so a descriptor
temporarily inherited by a concurrently spawned child can prolong that lock
until close-on-exec.

Store::lock now returns an owning JournalLock whose Drop explicitly unlocks
before closing. Acquisition remains nonblocking; journal coverage, authorization
and submission semantics are unchanged. A private duplicate-descriptor regression
deterministically verifies owner release and that closing an old inherited
description cannot unlock a subsequent owner. Actual subprocess contention and
eight independent concurrent journals are also tested.

The new integration test failed before this fix (six owner-release failures) and
passes afterward. Three locking integration cases, one deterministic unit and all
18 parallel transaction tests pass. Before changes, four parallel transaction
runs passed72/72; the older specific historical failure is not conclusively
attributed to this reproduced descriptor mechanism.

Busy diagnostics now identify the contender PID, exact path and Unix device/
inode, without introducing retries, sleeps or blanket serialization.

### Retained differential forensics

Live HLE raw execution reports now retain the bounded job debug bundle before
their temporary calculator profile is removed. Request and source identities
must match; oversized, absent, malformed or symlinked reports produce explicit
unavailable metadata. Async Await uses its original run request, not the latest
input/control request. Semantic comparisons remain unchanged.

Four focused forensic tests pass. Relevant all-target Clippy with warnings
denied and formatting pass. Changed coordinator code:
ppl-programs/src/delete_store.rs, its journal_locking integration tests, and
ppl-compat/src/differential/firmware.rs.

## Integrated runtime, Native and ARM execution increments

Runtime readiness now proves event-pump responsiveness independently of strict
checkpoint idle admission. The preserved debug failure was executing firmware,
SVCs and valid rendering rather than deadlocked. A validated LCD, two genuine
timer/pump round trips and a fresh main wait establish Ready. Strict checkpoint
and interruption policies remain unchanged. Finite headless deadlines also wake
engines blocked inside native waits. Three readiness regressions and a bounded
wait regression pass. Dedicated owner validation passed100 emulator unit tests
and the unchanged debug normal-OFF post-CAS fresh-process test in133.50s.
See M17_RUNTIME_READINESS.md. Coordinator repetition of the unchanged debug
normal-OFF post-CAS fresh-process regression passed in128.12s.

Native exported calls now execute in one linked bytecode VM, sharing calculator
slots and budgets while preserving private per-module state and source-bound
call frames. Installed source revisions invalidate their own state/linkage.
Actual firmware established initializer execution at import, which Native now
matches. Nine VM linkage tests pass. Integrated CLI Native/HLE comparisons pass
12 initialization assertions and22 three-program/edited-dependency assertions.
The new automated m17_differential integration test repeats these same fixtures
with real firmware (2test cases passed,44.07s); unconfigured CI runs only Native.
Native engine and CLI now accept source catalogs (`--module NAME=PATH`), check
the same reachable closure they execute, protect dependency files from output
aliasing and identify foreign runtime source locations. Three actual CLI tests
pass after integration. See M17_NATIVE_LINKAGE.md for limits and IDE followup.

Pinned qemuPrime now builds under an isolated rootless Xenial toolchain and
executes the self-authored ELF on its hp-prime ARM926 model. GDB proves initial
PC0x30000000, single-step0x30000004, halt0x3000005c, mailbox0x600d0001,
and four RAM-alias values0x12345678. Owner repeated three fresh processes;
coordinator independently repeated a fourth execution. Eight Python regressions
pass. This is actual execution, not merely ELF linking, and uses no proprietary
boot artifacts. It is neither calculator kernel boot nor FullSystem PPL support.
The published GPL overlay fixes two reproduced INTC defects: reset retained
pending/offset state and asserted input; FIQ selection asserted when an enabled
source was not pending. Identical unpatched tests reproduce both failures.
Patched reset and FIQ tests each pass three cycles with actual modeled CPU
IRQ/FIQ input observations. Coordinator independently repeated both and the ARM
smoke on the final published-overlay build. Ten Python research tests pass.
Simultaneous FIQ priority and real interrupt handlers/kernel boot remain unclaimed.

Protocol7 adds request/resource-bound inspection, real engine-owner/SVC/handle
phases and bounded expiring safe-point acknowledgement. The integrated firmware
test inspected idle and active WAIT state, then delivered input and verified a
typed return7 (9.25s). An independently executed headless query observed thread0
blocked in OSWaitForEvent and thread1 in GetEvent. Coherent memory inspection was
available; native callbacks prevented owner pause. The response explicitly
reports live_restore_ready=false. Opt-in register copies are observations, not
resumable CPU contexts. See M17_LIVE_INSPECTION.md.

Pending independent increments: IDE catalog workflow, typed-value controls and
decoders, hot-path inspection improvements and component reconstruction. These
remain open until tested handoffs and combined regressions finish.

## Findings carried into implementation

The retained debug profile remained Booting20s while servicing firmware and LCD
updates; fresh storage reached Ready3.897s with the same debug helper. Renderer
bounds appear valid. Exact joint parking/idle observations are required before
changing readiness; strict checkpoint quiescence remains preserved.

A genuine firmware control establishes initializer execution at Programs import:
shared A changes before the imported module is first called. Native's lazy
first-call initialization differs and is being corrected for installed modules.

The pinned Xenial system libfdt1.4.0 is insufficient for this QEMU fork. The
isolated exported build tree now uses its exact pinned open-source DTC submodule;
the original research checkout and production HLE remain untouched.

## Continuation and verification

Baseline helper preserved at
/home/k9-vortex/.cache/prime-m17-baseline/prime-emulator, SHA256
73c7980c1af014f60551c6c3e8d76faee81a6db7123b12657542b6f718163b6d.

Final work must integrate all handoffs, verify workspace/GUI/firmware/recovery/
historical classifications, and run five alternating baseline/current benchmark
trials with builds and other firmware sessions idle. CPU live restoration and
kernel boot must remain unclaimed unless genuinely demonstrated.


## Coordinator verification checkpoint

Clean-cache coordinator workspace verification passed1,787 tests, zero failures,
six ignored in normal parallel mode. Source had no cross-checkout edits. An
earlier shared-target build artifact mismatch was corrected by assigning each
agent a distinct home-backed CARGO_TARGET_DIR and invalidating only compiler/VM
generated artifacts before this passing run.

Actual Native/HLE stateful-edit comparison passes22 assertions. All seven raw
execution reports retain captured request/source-bound diagnostic bundles after
the disposable profile has been removed (artifacts/m17/forensics-real.json).
This still uses the preserved M16 helper; later combined M17 tests remain required.

Coordinator verified an integrated parallel workspace run:1,803 passed, zero
failed, six ignored; formatting and full workspace/all-target Clippy passed.
This precedes inspection/frontend/component/decoder final integration.
One earlier workspace attempt exposed an outdated CLI diagnostic-text
assertion after the accurate FullSystem limitation message changed. The test now
asserts both unavailable PPL execution and the explicit no-kernel/no-PPL boundary;
all3 CLI differential tests pass. No assertion was
removed or weakened to suppress a semantic failure.

Actual version15157 historical replay preserves all77 classifications:68 exact,
four partial and five transcription, zero mismatches. The Native leg remains
49 exact/four transcription/24 missing. These categories retain their original
historical provenance; no new physical-device capture occurred.

Paired performance tooling now validates all13 workload verdicts, complete
fresh-restart observations, exact ELF/ROM/profile/script/source/host identity,
stable helper identity within groups, and positive finite durations. Initial
profile identity covers the validated copied tree, including empty directories;
changed copies, symlinks and special files are rejected. Five Python regressions
pass. Final five alternating trials per helper remain pending until agents,
builds, GUI and other firmware jobs are idle.
