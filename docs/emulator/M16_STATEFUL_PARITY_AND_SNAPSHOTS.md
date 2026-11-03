# M16 — Stateful parity and safe checkpoints

Starting point: `feat/g1-usability-overhaul@7fbe711`. The feasible M16 implementation and final integration verification are complete.
All independent agent contributions are integrated; limitations are explicit below.
No physical writes, remote hardware input or new physical captures occurred;
no firmware was modified or redistributed.

## Implemented stateful execution

Schema2 runs genuine retained firmware sessions through the existing Rust HLE.
Schema1 and historical evidence remain supported. Each scenario uses a copied,
disposable A/C profile. Named loading assigns source directly through firmware,
without a scratch alias that resets module globals. Stored runs do not silently
import or Check again. At function entry, actual firmware source/context binding
is verified before body entry, including after a fresh process starts.

All initial and edited sources have individual hashes in a sorted manifest.
Schema2's aggregate source_sha256 identifies that manifest. Compiler verdicts
remain distinct from source-bound execution evidence. Invalid identities fail
closed. Headless transport bounds pending requests, checks envelopes, preserves
mandatory replies and kills/reaps its owned helper on failure/cancellation.

| Scenario | Verified observations | Scope |
|---|---|---|
| stateful.toml | Alternating two named modules; retained counter; shared A; list/matrix projections | Native versus actual HLE |
| stateful_edit.toml | Same sequence, then source replacement; edited counter 41; other program returns 26 | Native versus actual HLE;22 assertions |
| firmware_lifecycle.toml | Cross-program calls; CAS; runtime error followed by success; pixel(12,34); OFF/fresh restart; persisted scalar/list/matrix/angle/counter | Actual HLE |
| async_input.toml | Key wait returns 7; subsequent touch wait returns{0,100,50} in the same session | Actual HLE |
| checkpoint.toml | Explicit typed persistent checkpoint and restart observations | Actual HLE |

Authored probes execute ordinary PPL and are not assumed pure. Matrix coverage
is explicitly dimensions/cell projections; direct matrix/complex/integer return
decoding remains unsupported. Native external-function linkage and firmware
power semantics remain unsupported. Stateful arbitrary in-place Check is
refused because it can reset globals; explicit import produces an exact-source
firmware compiler verdict. Legacy IDE Check/Run remains available and optimized.

### Input and lifecycle root causes

Start waits for actual body entry; Await collects its correlated terminal result.
Input acknowledgments explicitly report queue admission, not consumption.
Fixture return values prove actual key/touch behavior.

A real sequential-input test found that ReleaseAll discarded a key-up already
queued to firmware: desktop bookkeeping considered it delivered too early.
Cleanup now preserves releases across desktop and guest queues and provides a
correlated idle barrier. Late new input cannot silently edit Home. Coordinator
integration also bounds cleanup waits, fails promptly while asleep/shutting down,
and releases dragged touches at their latest position. Regression tests cover
these cases and saturated mandatory-response delivery.

## Persistent checkpoints and crash recovery

This is **firmware-persistence recovery, not live CPU restoration**. The explicit
checkpoint command is opt-in; no periodic OFF or live-RAM snapshot runs by default.

Admission rejects active work, input cleanup, held keys/touches and pending
input. A fresh main-wait generation and settled idle observation precede normal
firmware OFF. After completed sleep, the existing snapshot gate and durable VFS
boundary protect checksummed atomic publication. Optional ON occurs after
publication, and the typed client waits for Ready before returning.

Receipts identify the exact generation, manifest digest, bytes and composite
resource identity (ELF, optional ROM and profile runtime resource fingerprint).
Wrong resource identities are rejected. Existing manifest compatibility and
normal OFF behavior are preserved.

Explicit checkpoints bound payloads to 256 MiB and recovery storage to 1 GiB by
default, with bounded entries and streaming copies. Retention keeps at least two
validated compatible generations and protects restore references, archives,
pending and incompatible/corrupt evidence. Publication fails safely when those
protections cannot fit. Normal OFF retains its existing retention policy.

Verified persistent state includes variables, lists, matrices, settings and
program files that firmware itself saves. CPU registers, threads, blocked SVC
continuations, host handles, timers, transient framebuffer and general active
application restoration are not serialized or claimed.

Full live restoration needs serializable pending SVC operations plus coordinated
engine safe points and validated CPU/allocator/handle/timer reconstruction.
The existing memory snapshot gate alone does not supply this.

### Known debug limitation

Repeated post-CAS fresh boot can stall before Ready in debug builds. The same
sequence reproduces with unchanged normal PowerDown, without a second explicit
checkpoint. The new checkpoint instead refuses OFF when idle cannot be verified,
preserving its earlier generation. Release explicit and normal-OFF recovery
cycles pass. The underlying debug/firmware stall remains unresolved; it is not
hidden by weakening the admission gate. Failed and comparison evidence remains
in the recovery worktree under the owner's home cache.

The existing parallel transaction journal-lock test flake was not modified.
Final workspace verification ran serially, including all 18 transaction tests;
this is not a claim that the historical parallel flake is fixed.

## Physical capture infrastructure

The offline `ppl capture prepare/seal/validate/compare` commands freeze all
program revisions, actions, confirmations, observations and artifact hashes.
Raw identity replies, source/action-correlated transcripts and screenshots are
validated. Paths, sizes and PNG dimensions are bounded; existing output
directories are not overwritten. Relative-output atomic publication is tested.

Verification categories derive from checked provenance/comparisons: Native-only,
HLE-verified, Firmware-versus-Native, historical physical replay, newly
physical-verified and untested. Controlled mocks cannot gain physical status.
Manual attestations are not cryptographic hardware authentication. A live USB
collector remains unimplemented; existing guarded upload/manual-action workflows
remain authoritative. No new physical observation was created.

Historical replay on version-matched firmware15157 retains the original
77 classifications: 68 exact, 4 partial, 5 transcription, zero mismatches.
These are historical comparisons, not77 new captures or77 byte-exact matches.

## Headless debugging

With --trace-svc, engine owners copy registers/CPSR at SVC entry and slice return.
Latest observations are bounded to 128 threads. Timestamps and boundary names
expose stale observations during blocked callbacks. Instruction counts remain
lower bounds with explicit last-slice accuracy. Disabled tracing skips register
reads. Bounded diagnostic bundles (64 trace entries/PCs plus dropped counts) are
attached to managed job reports. These are forensic records, not atomic machine
snapshots or pause/resume state.

Actual firmware heartbeat verification produced 7,829 register observations across
two guest threads without faults. Synthetic ARM and bounded-recording tests pass.
JSONL output remains an opt-in stream; it is not a disk-budgeted snapshot.

## Full-system research

The production backend remains HLE; FullSystem remains explicitly unavailable.

scripts/emulator/full_system_preflight.py inventories the pinned qemuPrime
checkout/tools and hashes explicitly supplied artifacts without inferring
bootability. --build-smoke NEW_DIRECTORY builds only self-authored ARM926 code,
refuses an existing directory and validates ELF32/ARM/entry metadata.

The smoke ELF linked successfully using Clang and Rust's bundled rust-lld:
entry 0x30000000, SHA256
617bcd9865c392f24d145f96d5c0d7cfb97a4654c98adfc9e0a6b05032218d76.
It tests three RAM aliases through a GDB-readable mailbox; it has not executed.
There is no UART/timer/kernel-boot success claim. Python2 and a suitable QEMU
binary remain missing; real kernel boot additionally needs matching authorized
boot/NAND artifacts. No proprietary image was downloaded or changed.

## Integrated verification

- Workspace serial regression: 1,780 passed, 0 failed, 6 ignored.
- Formatting and workspace/all-target Clippy with -D warnings:passed.
- GUI: 100 assertions on firmware 15751 plus 10 calculator-only 15157 typing assertions,
  all passed; actual screenshots inspected. CAS, graphing,18 apps and persistence.
- Historical version15157 replay:77 preserved, zero mismatches (classifications above).
- Explicit configured release suites: 15/15 tests passed (boot/calculator-only,
  sessions/imports, recovery, stateful scenarios and typed checkpoints). Recovery
  5/5 took 136.15 s; stateful 3/3 took 48.94 s; checkpoint 3/3 took 27.09 s.
- Research preflight:3 Python tests passed; smoke compilation/link metadata verified.
- No new physical access, external publication or push.

Logs, screenshots, raw comparisons and benchmarks live under artifacts/m16
(ignored; proprietary resources remain external). Standalone reports:
M16_STATEFUL_EXECUTION.md, M16_PERSISTENCE_CHECKPOINTS.md and
PHYSICAL_SCENARIO_CAPTURE.md document APIs and command examples.

### Main changed code

- ppl-engine:session.rs,live.rs,checkpoint.rs.
- prime-emulator:session.rs,session/job.rs,checkpoint.rs,recovery.rs,
  runtime.rs,engine.rs,trace.rs,services/mod.rs; firmware recovery tests.
- ppl-compat:differential model/native/firmware adapters, capture.rs and tests;
  shared fixtures under tests/differential/m16.
- ppl-cli:capture commands and CLI tests; ppl-gui:event handling/keymap.
- scripts/emulator:full-system preflight/tests; research/full-system:smoke.S/smoke.ld.

## Remaining engineering work

Investigate the baseline debug post-CAS boot stall; retain strict idle admission.
Implement genuine Native external linkage and additional firmware value decoders
only with evidence. New physical CAS/graphics/input/persistence captures need an
authorized device workflow; mock success cannot replace them. Full live machine
snapshots need serializable guest waits/resources. Full-system work next builds
the pinned fork and executes the self-authored smoke before any kernel claim.

## Final performance recheck

Three alternating trials per helper on the same Intel i7-13700/Linux host,
firmware 15751, initialized storage and 13 source-identical workloads. No
concurrent builds or other firmware sessions. All 78 expected results/verdicts
passed. Times measure helper requests, excluding IDE rendering. Large cold
is the first large source in an already booted session: the existing successful
10,160-byte workload, not a claim about all large programs.

| Workload | Baseline median s | M16 median s | Change |
|---|---:|---:|---:|
| tiny_cold | 7.026 | 7.321 | +4.2% |
| tiny_repeat | 0.751 | 0.776 | +3.3% |
| small_math | 2.439 | 2.456 | +0.7% |
| medium | 2.836 | 2.882 | +1.6% |
| large_cold | 3.033 | 3.131 | +3.2% |
| large_repeat | 1.378 | 1.353 | -1.8% |
| edited_check | 1.746 | 1.719 | -1.5% |
| edited_run | 1.363 | 1.338 | -1.8% |
| alternate_tiny | 2.780 | 2.712 | -2.4% |
| syntax_error | 1.746 | 1.702 | -2.5% |
| after_error_tiny | 2.595 | 2.743 | +5.7% |
| tiny_after_restart | 8.122 | 7.994 | -1.6% |
| repeat_after_restart | 1.052 | 1.053 | +0.1% |

No workload crossed the 10% investigation threshold. Three samples do not
establish a latency distribution or causality for small movements. Existing
launch performance is preserved; M16 does not claim another major speedup.
Exact samples, hashes and environment: evidence/m16/benchmark-summary.json.
Raw stage/CPU/profile reports: artifacts/m16/bench.
