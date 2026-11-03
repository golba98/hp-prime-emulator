# Incremental Rust architecture continuation

M16 starting point: feat/g1-usability-overhaul at 7fbe711. Retain its AFiles loader,
prepared-source checks, firmware-observed typing, OFF recovery and Native
INPUT/EDITMAT. Logical responsibilities already fit the existing crates; no
crate split or emulator rewrite is required.

## Current seams

- ppl-engine: source identity, cancellation, Native/Firmware Report and session
  transport. Backend availability and observation support are distinct.
- prime-emulator: ELF/Unicorn, mapped guest memory, SVC registry, host-backed
  threads/storage, display/input, trace and firmware execution jobs.
- ppl-compat: legacy evidence/provenance and the new differential foundation.
- ppl-gui: editor, nonblocking workers, capability presentation and diagnostics.

## Correctness before scheduler replacement

The audit identified aligned free-list padding counted twice; a four-byte C
string terminator; allocation before guest-I/O bounds checks; swallowed handler
faults; unparked synchronization waits; and worker ownership/lifetime gaps.
These require regression-backed fixes in their existing modules. Implementation
and validation status is recorded in the living overhaul report.

Each engine remains owned by its host thread and retains its mapped memory.
IRQs execute between slices on the owning engine. The snapshot gate prevents
captures racing active engines; it does not serialize all guest engines against
one another. Host thread priorities do not constitute a physical G1 scheduler.

Thread teardown must wake cancellable waits, preserve the first child failure,
join workers and release stacks after engine use. Stored spawners must not own
SharedState strongly. Closing a synchronization object must reject new users
while outstanding waiters retain a descriptor lease.

Add structured thread states, blocked-object/service identity and actual fault
registers through the existing trace/report boundary. Trace transitions must be
bounded and opt-in; do not add allocation-heavy disabled tracing.

## Public boundary evolution

Session protocol 5 adds managed-program capabilities independently of calculator
input/display/power. An unknown compiler profile must never become a validated
Check/Run verdict. Older helpers are rejected by the version handshake.

Differential backend identity is separate from the application's Engine enum.
Structured typed results and persistent Native scenario state should be shared
without importing GUI types into the emulator core. Unsupported observation
capabilities remain explicit. Any future typed session result must bind request
and exact source identity and preserve the existing raw reports.

## Promotion gates

An optional cooperative/serialized execution experiment may reuse admission
and parked guards only after every blocking SVC is covered. It stays outside
the default path until synchronization tests, real boot/Home/CAS/apps/import,
OFF/crash recovery and identical-source before/after benchmarks pass. This
continuation does not claim deterministic scheduling or live machine snapshots.

A complete snapshot requires CPU contexts, guest thread/wait state, timers,
IRQ queues, handles, in-flight calls and filesystem consistency. RAM/register
serialization alone is insufficient. OFF checkpoints remain the safe boundary.

Parallel implementation uses isolated worktrees and target directories.
Integrate tested commits, then run the whole workspace and GUI suite on the
canonical branch. Keep physical evidence immutable, proprietary resources
external, and full-system research as a separately gated process backend.

## M16 diagnostic observation boundary

With `--trace-svc`, each engine owner copies registers at SVC entry and slice
return into bounded latest-thread observations and the existing JSONL stream.
Observers consume copies; they never call Unicorn APIs on another thread.
Timestamps and boundary names expose stale observations during host callbacks.
Instruction counts remain lower bounds across interrupted slices. This enables
headless inspection without advertising pause/resume or restorable CPU state.
See the M16 report for implementation and validation status.

## Integrated M16 boundaries

Session protocol6 now carries source-bound named-program execution, typed
terminal evidence, scoped input and explicit persistent checkpoint receipts.
The reusable headless process client lives in ppl-engine; adapters never import
GUI types. Native retains its existing SessionContext. No crate split or guest
scheduler replacement was introduced.

Firmware OFF remains the persistence safe point. Explicit checkpoints verify
fresh idle admission and serialize durable files, not host continuations or
Unicorn internals. Full live restoration requires serializable pending SVCs,
allocator/handles/timers and coordinated owner-thread safe points.

Stateful capture bundles reuse the scenario source manifest and exact semantic
comparator. Physical qualification remains evidence-derived. Full-system
execution stays a separate unavailable backend until an independent boot is
demonstrated. The self-authored ARM926 smoke now compiles/links, but has not run.
