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

Three focused forensic tests pass. Relevant all-target Clippy with warnings
denied and formatting pass. Changed coordinator code:
ppl-programs/src/delete_store.rs, its journal_locking integration tests, and
ppl-compat/src/differential/firmware.rs.

## Parallel implementation currently unintegrated

- Runtime: post-CAS readiness, owner SVC progress and bounded deadlines; then
  snapshot safe-point/component prerequisites.
- Native: real bytecode external linkage and firmware-verified import timing;
  then typed-value layout capture/decoding.
- Full-system: pinned rootless Podman toolchain, actual ARM/GDB smoke, then
  source-demonstrated interrupt-controller fixes.

These are not completed merely by being listed. Each requires verified handoff,
integration and coordinator regression evidence.

## Findings carried into implementation

The retained debug profile remained Booting20s while servicing firmware and LCD
updates; fresh storage reached Ready3.897s with the same debug helper. Renderer
bounds appear valid. Exact joint parking/idle observations are required before
changing readiness; strict checkpoint quiescence remains preserved.

A genuine firmware control establishes initializer execution at Programs import:
shared A changes before the imported module is first called. Native's lazy
first-call initialization differs and is being corrected for installed modules.

The pinned Xenial toolchain image built, but its system libfdt1.4.0 is insufficient
for this QEMU fork. The research owner is populating the exact pinned open-source
DTC submodule in the exported build tree. No ARM execution is claimed yet.

## Continuation and verification

Baseline helper preserved at
/home/k9-vortex/.cache/prime-m17-baseline/prime-emulator, SHA256
73c7980c1af014f60551c6c3e8d76faee81a6db7123b12657542b6f718163b6d.

Final work must integrate all handoffs, verify workspace/GUI/firmware/recovery/
historical classifications, and run five alternating baseline/current benchmark
trials with builds and other firmware sessions idle. CPU live restoration and
kernel boot must remain unclaimed unless genuinely demonstrated.
