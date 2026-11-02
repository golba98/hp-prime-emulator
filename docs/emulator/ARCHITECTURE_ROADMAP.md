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
demonstrated. M17 executes the self-authored ARM926 smoke on qemuPrime's
hp-prime model through isolated QEMU/GDB tooling. This establishes actual ARM
and RAM-alias execution; it does not boot the proprietary kernel or provide a
FullSystem PPL backend.

## M17 integrated boundaries

Native external calls link independently compiled modules into one worker-local
execution image. Retained state contains source-bound values, not transferable
Rc bytecode. Per-module identity accompanies call frames; calculator globals
alias one live slot while private globals remain revision-owned. Installation
initialization is separately verified against firmware.

Boot responsiveness and persistent-save quiescence are distinct policies.
Readiness now requires a validated LCD and genuine event-pump round trips,
followed by a fresh main wait. The strict checkpoint/interrupt idle threshold
is unchanged. Wall-clock deadlines also wake a main engine blocked in a native
SVC, preventing bounded headless runs from waiting indefinitely.

Differential raw reports retain source/request-bound diagnostics after isolated
profile cleanup. Their register snapshots remain owner-thread observations,
not resumable Unicorn contexts. Snapshot admission and component reconstruction
now include independently reconstructed heap/allocator and read-only VFS
components. Their bounded, checksummed formats bind resource identity and require
fresh exclusive targets. Actual child-process tests verify allocator hole reuse,
handle/cursor identity and descriptor stability. They exclude CPU state,
blocked SVCs, writable handles and active directory searches. Arbitrary live
restoration stays unsupported; see M17_SNAPSHOT_COMPONENTS.md.

Session protocol7 exposes non-mutating request/resource-bound inspection. Real
engine owners acknowledge expiring pause epochs only outside Unicorn/native
callbacks. Names materialize when inspected, keeping the SVC accounting path
allocation-free. Expiry is reported explicitly. An acquired RAM gate does not
prove that native callback continuations can resume in a fresh process.

The Native IDE uses immutable revision-bound catalogs of open tabs for Check,
Run and diagnostics. Dependency changes invalidate compiled callers; runtime
frames map back to the callee's source. Multi-module debugger navigation remains
unsupported and is refused explicitly, preserving the single-module debugger.

Typed HLE evidence now includes controlled matrix/vector, complex and integer
representations. Bounds and exact structure remain part of comparison; semantic
type distinctions are not normalized away. Representation capture and transport
budgets are separate admission concerns. Unknown objects stay undecoded even
when their bounded allocation bytes are retained for diagnostics.

## M17 input, OFF and development build boundaries

Entering a native wait is distinct from the exact owner being blocked on an
unsignaled event. Desktop translation now requires that actual main-UI state
and a fresh generation after the preceding keydown, rechecked under the memory
gate. Raw releases and cancellation bypass character admission. These checks
prevent sampling a partial redraw without adding a machine-dependent delay.

Internal OFF sequencing observes firmware Shift state. It sends ON directly
when Shift is active, otherwise acknowledges one Shift before ON, preserving
Alpha lock. Unrecognized checkpoint contexts defer before input; normal shutdown
retains its native shortcut fallback. Strict idle admission remains 80% over
the existing sample window and retains its 30-second deadline. Failure reports
distinguish admission, input, sleep and durable-save stages; PRIME_TEST_EVIDENCE
can preserve isolated failed test profiles and bounded inspection/screenshots.

Cargo dev/test builds optimize only unicorn-engine-sys at level 2. This selects
the native backend's RelWithDebInfo configuration while retaining project Rust
debug settings. Matched-source recovery tests improved from median 91.22 to
80.51 seconds. Guest clocks, instruction limits and checkpoint policy did not
change. Busy firmware can still fail strict admission safely; optimization is
not proof of universal quiescence. Native-engine assertion/debug work can use
`cargo --config 'profile.dev.package.unicorn-engine-sys.opt-level=0' ...`.
