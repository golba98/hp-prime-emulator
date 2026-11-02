# M19 reconstructable synthetic SVC operations

M19 extends the M18 fresh-process experiment with real executed Unicorn SVC
traps and explicit portable wait/receive state. This is a prerequisite proof
for future firmware reconstruction. It does not restore a running calculator,
implement firmware OSWaitForEvent/GetEvent ABI semantics or change OFF recovery.
`live_restore_ready` remains false for firmware.

## Implemented boundary

`synthetic_snapshot::resumable` exposes `ResumableGuest`, `ResumableSnapshot`,
`SyntheticEvent`, `ServiceStop` and opaque `WaitTicket`. It uses the same fixed
ARM926 integer user-mode regions as M18. Only this separate owner enables the
self-authored ARM instructions SVC0x190001 and SVC0x190002. The existing CPU-only
owner still rejects SVC, and the HPUSYN01 version1 format remains unchanged.

The wait operation takes a bounded logical timeout in R0 and the sole logical
port identifier1 in R1. Receive takes an aligned eight-byte destination within
the data region in R0 and a timeout in R1. It returns an event kind/value pair
and R0=1, or R0=0 on logical timeout. Wait leaves one delivered event queued for
a later receive. These are intentionally synthetic service definitions, not
inferred interpretations of proprietary service arguments or event structures.

An actual Unicorn SVC interrupt records service number, arguments and the
post-SVC PC, then stops execution. The callback returns before the owner creates
or completes the pending operation. Checkpoint and completion require exclusive
mutable ownership; there is no host wait, native continuation, thread, callback
pointer or OS handle in the saved state. CPU instruction count includes the
executed SVC exactly once. Completion writes the service result and execution
continues at the captured post-SVC PC.

## Portable state and reconstruction

The separate bounded HPUSVC01 container binds the existing CPU/memory manifest
to the pending operation, arguments, post-SVC PC, request sequence, completion
count, optional queued event, logical time, wait start/deadline, logical resource
identity and binding epoch. Both manifests and memory payload are checksummed.
Unknown fields, unsupported operations, inconsistent CPU/timer/queue state,
changed resources/code/backend, invalid destination pointers and corrupt files
are refused before publishing a new owner. Save stages and synchronizes a single
complete file, atomically replaces the target and synchronizes its directory.
It requires private stable directories, as does the M18 store.

Restore constructs an entirely new Unicorn engine. The caller supplies the
matching logical resource identity, exactly reconstructed logical time and a
binding epoch greater than the saved epoch. A completion ticket binds request
sequence to that new epoch; tickets from the captured owner and repeated
completions are refused. The owner creates no OS resource and restores no host
queue. A serialized logical queued event is preserved intentionally. The epoch
contract prevents stale delivery within this owner; it is not a distributed
exactly-once protocol for multiple owners forked from the same checkpoint.

Logical timeout progression is explicit and independent of host wall time.
A saved pending deadline must agree exactly with its saved start and original
arguments. Advancing beyond that deadline completes the operation once; an old
event cannot complete it again.

## Actual execution and negative controls

The self-authored tests execute SVC, retain post-SVC PC0x10000004 and an executed
instruction count of1, save, destroy the original owner, and start a fresh child
process. The child loads the checkpoint, constructs a new engine and binding,
delivers an event and executes the guest suffix. Registers and once-only guest
memory writes match independent uninterrupted execution. Separate cases cover
blocked wait and blocked receive, and wait followed by receive of its queued
event. No proprietary firmware or hardware is involved.

Negative controls cover unsupported SVC, corrupt pending operation/PC/arguments,
invalid output bounds, expired/inconsistent timers, wrong executable identity,
wrong resource rebinding, reused binding epoch, old-owner ticket, wrong request,
duplicate completion, corrupt/truncated/wrong-format files and failed save paths.
A child is killed after the real staging header is written; the published
checkpoint remains byte-identical. Another child is killed after reconstruction
of its private CPU but before service rebinding/owner publication. The original
checkpoint remains intact and resumes successfully in a subsequent fresh child.

Run:

```sh
CARGO_TARGET_DIR="$HOME/.cache/prime-m19-hardware-target" cargo test -p prime-emulator --lib synthetic_snapshot -- --nocapture
CARGO_TARGET_DIR="$HOME/.cache/prime-m19-hardware-target" cargo test -p prime-emulator --lib
CARGO_TARGET_DIR="$HOME/.cache/prime-m19-hardware-target" cargo clippy -p prime-emulator --all-targets -- -D warnings
cargo fmt --all -- --check
```

The existing14 CPU-snapshot tests and8 new service tests pass. The coordinator
runs the integrated workspace regressions separately. These are synthetic
Unicorn execution results, distinct from guest QEMU hardware execution and
actual HP firmware observations.

## Remaining real firmware prerequisites

Production HostEvent/Condvar and guest call stacks, multiple guest thread/ISR
owners, scheduler coordination, host timers, filesystem resources, remaining
memory mappings and queued input still need their own reconstruction contracts.
No production scheduler, firmware SVC handler, recovery admission or persistence
policy changes in this increment. Actual live firmware restoration remains
unsupported; OFF-based recovery stays authoritative.

Final standalone library run:151 passed, including22 combined snapshot tests
and8 resumable-service tests. All-target emulator Clippy with warnings denied
and workspace formatting pass. Exact commands, source/program/test-binary/log
hashes, packed runtime version and negative controls are recorded in
`evidence/m19-resumable-svc.json`. The actual Unicorn version return is
33621247 (`0x020104ff`); the Rust binding package is2.1.5.

One initial full-library run failed the existing30ms safe-point scheduling test
while other builds were active (150 other tests passed). That unchanged test
passed in isolation, and the full151-test suite subsequently passed. No timeout
or production scheduling behavior was changed. Initial Clippy rejected my use
of an API newer than the repository\'s Rust1.85 MSRV; ordinary modulo now performs
that alignment check without a lint exception.
