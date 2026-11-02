# M18 synthetic guest continuation

M18 adds actual fresh-process CPU continuation for a narrow, self-authored
synthetic guest. This is neither a live calculator snapshot nor FullSystem
machine restoration. Production firmware recovery and
`SessionInspection.readiness.live_restore_ready = false` remain unchanged.

## Implemented contract

`prime_emulator::synthetic_snapshot` exposes `SyntheticProgram`,
`SyntheticGuest`, `CpuState` and `SyntheticSnapshot`. A caller constructs a
program from original machine-code bytes, runs bounded instruction slices,
captures and saves a snapshot, and supplies the same program when loading and
restoring a new owner. Restore creates a new Unicorn engine; it never installs
partial state into a running calculator.

The synthetic machine explicitly selects Unicorn's ARM926 model, little endian,
user mode and one owner. Its complete supported architectural state consists
of R0–R15, CPSR flags/instruction-set state and these fixed regions:

| Region | Base | Size | Guest permission |
| --- | --- | --- | --- |
| Original code | `0x10000000` | 64 KiB | Read/execute |
| Data | `0x20000000` | 64 KiB | Read/write |
| Stack | `0x30000000` | 64 KiB | Read/write |

Code size and entry alignment are validated. The implementation whitelists a
small integer instruction subset before each guest instruction: selected
ordinary ARM data processing, word loads/stores, branches/BX, and Thumb
immediate arithmetic/comparisons, word loads/stores, ordinary branches/BX and
nonempty low-register push/pop. Instructions outside this subset are refused.
There are no native SVC handlers, firmware hooks, devices, asynchronous input,
interrupts, guest threads, MMU, coprocessors or floating-point state. Execution
faults, exceptions, unsupported instructions or short/timed-out slices poison
snapshot admission. Slices accept 1–100,000 instructions and a one-second native
execution limit. State validation rejects unsupported CPSR state and invalid
PC/SP locations.

The private Unicorn engine and synchronous `&mut self` API form the checkpoint
barrier. No engine or shared memory handle escapes this owner. Capture runs
after `emu_start` has returned, outside native callbacks. The opaque Unicorn
`Context` pointer is never serialized. Writing PC can clear Thumb state in
Unicorn; reconstruction reapplies the captured CPSR after PC and resumes at
`PC | CPSR.T`. Tests caught that real behavior during implementation.

## Format, validation and persistence

The version-1 binary container has an eight-byte kind header, a bounded
64-KiB JSON manifest and exactly 192 KiB of memory payload. The manifest records
fixed mapping addresses/permissions, CPU and actual runtime Unicorn version,
original program/mode identity, registers, CPSR, instruction count and payload
SHA-256. A second checksum binds the manifest fields. Original code and zero
padding must match the separately supplied `SyntheticProgram` exactly.
`calculator_live_resume` must be false.

Load checks regular-file type, length, bounds, schema, identity, architecture,
mapping, state and both digests before constructing a new engine. FIFO and
symlink inputs are refused before opening. Files/directories must be private
and stable during the operation; this is not a hostile-filesystem race defense.
Save uses a private sibling staging file, synchronizes its complete contents,
atomically replaces the destination and synchronizes the parent directory.
A directory-sync failure after rename denotes uncertain durability of a complete
replacement. No background autosave or rotating checkpoint generations are
introduced for this synthetic API.

## Executed tests

```sh
CARGO_TARGET_DIR="$HOME/.cache/prime-m18-snapshot-target" \
TMPDIR="$HOME/.cache/prime-m16-tmp" \
cargo test -p prime-emulator --lib synthetic_snapshot -- --nocapture
```

Create a private TMPDIR first if it does not exist. Ten snapshot tests pass;
the full emulator library suite passes 137 tests. All-target emulator Clippy
with warnings denied and formatting checks also pass.

- ARM and Thumb programs execute a prefix, store a marker, push a value and
  set comparison flags. The parent saves state and drops its owner. A fresh
  child process restores it and executes the suffix, taking the correct
  condition, recovering the stack and producing 24 with a once-only counter.
  Register/instruction state matches uninterrupted execution.
- Every intermediate boundary is saved/restored four times before continuation,
  preserving final register and memory observations.
- Thirty-two restore/run/drop cycles execute in a separate Linux child process
  with unchanged file-descriptor counts. Isolation avoids counting unrelated
  descriptors opened by parallel tests.
- A child enters the actual save routine and is killed after its staging
  header is written. The previous valid snapshot remains byte-for-byte intact
  and resumes in another fresh child. Partial staging files are never loaded
  implicitly.
- Negative tests reject corrupt/truncated/oversized length fields, trailing
  bytes, CPU/version/resource/layout changes, unsupported CPSR, bad PC/SP,
  changed original code, wrong permissions, FIFO/symlink inputs, exceptions,
  SVC, CP15 access, status-register instructions, unimplemented interworking
  instructions, code writes and unmapped memory faults.

These are **synthetic Unicorn guest** results, distinct from FullSystem QEMU
hardware tests, firmware recovery and physical-device observations. No firmware
artifacts or USB access are used by these tests.

## Remaining calculator-session blockers

The production `CpuThread` installs firmware hooks and owns rich `SharedState`;
its state cannot be reproduced by this synthetic contract. Real idle firmware
still has native `OSWaitForEvent`/`GetEvent` continuations on host stacks.
Events/semaphores/critical sections contain host synchronization primitives and
critical sections store host thread identity. CPU banked/system state, every
guest mapping, timers, ISR ownership, worker lifecycle, writable resources,
queued input, display and pending guest calls also require coordinated capture
and reconstruction.

The next safe extension is an explicitly reconstructable service/continuation
state machine tested independently before admission of a real calculator
session. The existing firmware OFF/checkpoint generation remains authoritative;
this synthetic feature does not extend its crash-recovery guarantees.
