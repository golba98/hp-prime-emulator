# M16 — Stateful parity and safe checkpoints

Starting point: `feat/g1-usability-overhaul` at `7fbe711`. Implementation in progress.
No new physical calculator observations have been captured. Existing 77 historical
references remain unchanged. HLE execution remains distinct from kernel emulation.

## Integrated coordinator changes

Opt-in `--trace-svc` diagnostics now publish copied guest registers, CPSR,
thread identity, boundary, timestamp and sequence to the existing JSONL stream.
Latest observations are bounded to 128 thread identities. Register reads happen
only on the Unicorn engine's owning host thread. An SVC-entry observation may
remain stale while the callback waits; it is not a current CPU context or a
whole-calculator pause. Instruction counts are labelled lower bounds, with the
last slice's counting accuracy preserved. Disabled tracing skips register reads.

Validation: three new Rust tests cover disabled recording, bounded latest
observations and actual ARM register copying without resuming the guest. The
filtered observation suite passed 4/4, including an existing worker-fault test.
Prime emulator all-target Clippy passed.

## Full-system executable research preparation

`scripts/emulator/full_system_preflight.py` produces a versioned, read-only JSON
inventory of the pinned qemuPrime checkout, available toolchain and explicitly
supplied artifact hashes. It never infers bootability from an artifact hash.
Missing prerequisites produce exit 1 with individual blockers. Two Python tests
pass, including immutable artifact hashing and fail-closed missing tools.

`research/full-system/smoke.S` and `smoke.ld` provide self-authored ARM926 RAM
alias smoke code with a GDB-readable result mailbox. Clang successfully compiled
the assembly to `/tmp/prime-m16-smoke.o`; linking and execution are unverified.
The pinned board wires UART to a null chardev by default, so this smoke does not
promise visible UART output. The mailbox reports success only after checking
all three additional 32 MiB aliases.

Local preflight finds pinned revision
`32e891a9fc568de3110343c3e35b9ec0d23f6f44`, but no Python 2, ARM linker or QEMU
binary. Docker/Podman and Clang exist. Kernel boot still additionally requires
matching authorized boot/NAND artifacts. No proprietary firmware was modified,
redistributed or downloaded.

## Active isolated implementation

- Stateful execution: schema evolution, typed live-session observations and
  retained named firmware programs in `/tmp/prime-m16-stateful`.
- Recovery: explicit firmware OFF checkpoints and typed recovery receipts in
  `/tmp/prime-m16-recovery`.
- Capture: offline immutable scenario bundles and provenance validation in
  `/tmp/prime-m16-capture`.

These contributions are not integrated or verified merely by being listed here.
The canonical coordinator owns integration and final regression evidence.

## Snapshot safety boundary

The existing snapshot gate excludes active engines for coherent memory reads;
it does not serialize resumable host callback continuations. Blocked SVCs,
host mutex ownership, allocator bookkeeping, guest-call plans, open files and
relative timer deadlines need explicit reconstruction before full live restore
can be safe. M16's viable recovery boundary is completed firmware OFF with
validated persistent storage. CPU/thread/framebuffer restoration must remain
unsupported until independently implemented and validated in a fresh process.

Early coordinator-wide regression: `cargo test --workspace` passed **1,720 tests,
zero failures, six ignored** across 93 reported test suites. Log:
`/tmp/prime-m16-workspace-early.log`. This predates agent integration.
Baseline release helper preserved at `/tmp/prime-m16-baseline/prime-emulator`,
SHA-256 `9eceeb68b81daec82650cb19bb72374786c7d983ae6d9b3019d68bcd53f8a6e2`.

M16 follow-up: the preflight now discovers Rust's bundled `rust-lld` as an ARM
linker. `--build-smoke NEW_DIRECTORY` compiles and links the self-authored smoke,
validating ELF32/ARM/entry metadata and refusing an existing output directory.
With TMPDIR on home storage, the local build succeeded with ELF entry
`0x30000000`, SHA-256
`617bcd9865c392f24d145f96d5c0d7cfb97a4654c98adfc9e0a6b05032218d76`.
Execution remains unverified; Python 2 and QEMU are still missing. Three Python
tests pass, including preservation of existing output directories. This
supersedes the earlier missing-linker/assembly-only observation above.

Actual firmware15751 debug run: heartbeat verified (three delivered key events, no faults), 7829 owner-thread register observations across 2 threads. JSONL and report are retained under artifacts/m16/debug-real*. This is HLE-only evidence.
