# M18 — Full-system hardware foundations and runtime fidelity

M18 is integrated on `feat/g1-usability-overhaul`, from M17 reporting checkpoint
`31cfa64`. Tested source checkpoint: `11356a1`. Regression, GUI and performance verification are complete. No kernel boot, physical
capture or whole-calculator live restoration is claimed.

## Actual hardware execution

The pinned hp-prime QEMU model now uses a dedicated five-channel S3C2416 timer,
with 16-bit counters, shared prescalers, internal divisors, manual load,
one-shot/autoreload, counter observation and reset/stop cancellation. It replaces
the incompatible borrowed Exynos timer without changing shared Exynos models.
The default 66 MHz PCLK is an explicit research input, not verified clock-tree
emulation. External clocks, DMA and output waveforms remain unsupported.

Four self-authored ARM926 guest cases each passed three fresh processes in the
hardware worktree. The coordinator independently executed all four and the
original smoke through the integrated Rust CLI:

| Case | Actual executed observations |
| --- | --- |
| Timer IRQ | Nine handler entries and architectural returns; periodic delivery, controller/CPU masks, pending/clear and two pending sources under a fixed arbitration setting. UART `IIIIIIIIIE` and a mainline marker prove continuation. |
| Timer FIQ | Three handler entries/returns, zero IRQ entries, banked FIQ state and UART `FFFE`. Simultaneous-FIQ priority remains unverified. |
| UART/GPIO | Guest instructions configure UART and emit exact `M18 UART\n`; non-keyboard GPIO configuration/data readbacks match. Electrical pins, baud timing and key scanning are not verified. |
| MMU | Translated RAM alias; data translation, alignment and user permission aborts; prefetch abort; real vector entry and guest recovery. The same LDR block executes with alignment checks clear/set/clear. Caches stay disabled. |

The MMU control reproduced an old-QEMU ARMv5 bug: ordinary A32 integer accesses
ignored SCTLR.A. A narrow GPL research overlay adds alignment to translated-block
identity and the applicable memory operations. The original failing assertion
is preserved. Newer ARMv6+ behavior is unchanged; multiword alignment edge cases
remain unverified. Five-channel virtual-clock timer tests passed three fresh
processes; original M17 smoke/reset/FIQ regressions also pass.

Final clean published-overlay QEMU SHA-256:
`6312c95a5e628798913cbe40cfd8a053801d6637ae44fb76c9e27ed8f0c71c5a`.
The original reference checkout is unchanged. GPL model patches remain outside
Rust application code. The timer explicitly blocks migration rather than
silently omitting its active state.

See [hardware evidence, device coverage and boot-readiness matrices](../../research/full-system/M18_HARDWARE_EXECUTION.md)
and [retained captures](../../research/full-system/evidence/m18-hardware.json).
Coordinator raw results are under `artifacts/m18/full-system-*/run`.

## FullSystem integration and ownership

`ppl full-system info --build-root PATH --json` validates the pinned build
manifest and actual executable digest without booting or opening USB.
`ppl full-system run --build-root PATH --case timer_irq --output NEW_DIR --json`
executes a synthetic hardware case. Cases are `smoke`, `timer_irq`, `timer_fiq`,
`uart_gpio` and `mmu`. Normal PPL Engine remains Native/Firmware; FullSystem
PPL, CAS, graphing and calculator UI remain unavailable.

The IDE has a separate FullSystem diagnostics dialog, a persistent research path
and asynchronous revision-bound validation. It leaves the selected PPL engine
unchanged. Its rendered screenshot was inspected. Source-aware Native debugging
remains separate from guest GDB capability.

The launcher atomically reserves a private output directory; raw runner artifacts
are under `OUTPUT/run`. RAII guards terminate/reap owned children on all error
paths. Cleanup can read only a bounded regular CID inside that owned directory.
Deterministic race tests prove a competing caller cannot remove the winner's
container. Actual SIGINT testing returned 130, reported cancellation, preserved
0700 output ownership and verified the recorded container ID no longer existed.

Results must bind machine, source revision, executable, image and case. A
successful result requires actual guest entry/step/terminal observations and the
specific mailbox/UART contract. Empty observations or unsupported kernel/physical
claims are rejected. This validates trusted local tooling, not arbitrary
third-party attestation. Runtime preflight is distinct from resource discovery.
Isolated-build preflight no longer requires host Python2 or a host QEMU binary.

## Synthetic CPU continuation and safe fallback

The new synthetic snapshot owner supports a fixed 192 KiB code/data/stack layout,
ARM926 integer user-mode ARM/Thumb instructions, architectural registers, flags
and bounded execution. Unsupported instructions, SVCs, coprocessors and CPU
modes fail closed. It serializes no opaque Unicorn Context, host pointer,
descriptor or synchronization primitive.

Versioned manifests bind code/runtime identity, mappings, CPU state and memory
checksums. Restore validates before constructing a fresh owner. Actual child
processes execute the saved suffix with preserved flags, stack and counters,
without replaying the prefix. Execution tests established that Unicorn PC writes
can clear Thumb state, so the authoritative saved CPSR is applied afterward.

A bounded store retains current and previous valid generations. Corrupt,
incompatible or missing primary state cannot overwrite a valid backup. Recovery
explicitly reports which compatible generation was selected. Tests cover
corruption, interrupted writes/rotation, incompatible resources, actual fresh
child recovery and 32 reconstruction cycles with stable descriptor counts.
Fourteen synthetic tests pass.

This is executable synthetic continuation, not a resumed calculator application.
Native OSWait/GetEvent continuations, scheduler state, timers and remaining host
resources still block coherent real-firmware restoration. `live_restore_ready`
remains false. Existing OFF/persistence checkpoints remain authoritative.
See [snapshot contract](M18_SNAPSHOT_CONTINUATION.md).

## Native execution, values and linked debugging

Native now links exported variables to their owning module slots, including
scalar/list/matrix reads, writes and indexed mutation. Private state, revisions
and calculator globals retain their boundaries. Fresh SDK reads initialize the
owner; variable-initializer cycles return explicit Unsupported instead of zero.

ASCII `i` falls back to the imaginary constant after ordinary symbol resolution,
matching controlled firmware observations while retaining local/private names
and capital I behavior. Header/file declaration and assignment validation still
has documented differences. Duplicate-export rebinding, initializer retry and
function cycles, and active-call source replacement remain unverified or refused.

Repeated retained calls exposed list tag 0x16/subtype 2, previously unobserved.
Controlled raw captures and independent scalar/list/matrix projections verified
its layout; the bounded decoder now supports that exact form. Unknown tags,
special values and unsupported matrix ranks still produce explicit diagnostics.
The frozen-helper unobserved failure is preserved rather than relabelled success.

Linked debugging now carries module/revision identity through breakpoints,
statement stepping, stack frames, errors and private-global inspection. CLI Debug
accepts `--module NAME=PATH` and qualified breakpoints. The IDE activates the
correct callee source; edited/closed sources open as read-only captured revisions.
A corrected publication race prevents an old pause from satisfying a new step.
Actual Xvfb tests pass 40 assertions. See [debugging and fidelity](M18_NATIVE_DEBUGGING.md).

Five genuine Native/HLE comparisons pass 94 assertions: M17 initialization 12,
linkage 22 and typed values 17; M18 imaginary-unit 14 and exported variables 29.
No broad value/error normalization was added.

## Checkpoint admission and regression evidence

The strict 80% idle policy, existing window and deadline are unchanged. Diagnostics
now expose observed interval span, maximum sampling gap and a time-weighted
sample estimate. It never extrapolates after the final sample and cannot prove
intervening owner state; it is explicitly excluded from admission. No broader
safety predicate was promoted. Busy checkpoints may still refuse safely.

| Verification | Result |
| --- | --- |
| Workspace | 1,901 passed, 0 failed, 6 existing ignored |
| Formatting / all-target Clippy | Passed; warnings denied |
| Firmware GUI | 59 CAS/graph/apps + 31 boot/persistence + 11 typing (15751) + 11 typing (15157) |
| Native GUI | 15 retained linkage + 40 linked debugger assertions |
| Research dialog | 2 state assertions; screenshot inspected |
| Genuine Native/HLE | 94 assertions across five scenarios, 0 mismatches |
| Configured release tests | 11 firmware + 3 stateful + 3 checkpoint cases; checkpoint group includes one Native-only case |
| Debug post-CAS fresh-process recovery | Passed, 71.54s; one run, not a new performance median |
| Synthetic snapshots | 14 tests including actual fresh child continuation/fallback |
| FullSystem Python tooling | 20 tests |
| Journal stress |Four parallel runs: 72 transaction + 12 journal assertions, 0 failures |
| Historical version15157 replay | 77 classifications: 68 exact, 4 partial, 5 transcription, 0 mismatch |
| Native historical leg | 49 exact, 4 transcription, 24 missing; unchanged |

The six initial workspace failures came from one test-fixture race: the PID
readiness file existed before its content was written. Five subsequent failures
were mutex-poison fallout. Atomic publication of the complete PID fixed the
fixture; the full workspace then passed. No production locking was weakened.
The first app GUI run used 120s despite 149.4s of scripted waits; the standard 300s
budget passed the unchanged assertions. Both failed attempts remain retained.

The frozen-M17 direct scratch Run control also refused a catalog/source binding;
the supported named Store/RunStored path supplied the verified new scenarios.
That refusal is preserved as infrastructure evidence, not a PPL language result.

Raw integrated logs, screenshots, comparison JSON and failed experiments are
under `artifacts/m18`. Historical parity references and proprietary resources
were not changed. GUI tests disabled calculator USB monitoring/jobs.

## Boot resources and next executable work

The inspected local assets contain PRIME_OS.ROM (1 MiB), PRIME_APP.DAT (32 MiB),
PRIME_MASTER.DAT (4 MiB) and configured HLE application ELFs. Metadata/hashes are
retained in [resource inventory](../../evidence/m18/authorized-resource-inventory.json).
No matching BXCBOOT/BESTAARM or complete NAND/OOB image was found in that scope.
Update components were not modified or combined into a purported boot image.
No proprietary bootloader/kernel instruction was executed.

Next hardware work is a self-authored NAND page/OOB/ECC control, then clock/reset,
storage and display/input validation before attempting a matching authorized boot
chain. Further CPU snapshots require resumable SVC state machines and complete
resource/scheduler reconstruction. Native followups require controlled duplicate
export, initializer failure/cycle and active replacement semantics. New physical
verification requires separately authorized hardware interaction.

## Performance

Five alternating M17/M18 pairs completed all 13 identity-matched workloads:
130 expected outcomes passed. Firmware, ROM, initial storage, script, host and
workload hashes match; each helper remains fixed within its group. No competing
project builds/tests ran during measurement. Host: Intel i7-13700, 24 logical CPUs,
Linux 7.2.13/glibc 2.43. Request-to-correlated-result wall time excludes IDE dispatch
and rendering; raw results retain CPU time where available.

| Workload | M17 median(s) | M18 median(s) | Change |
| --- | ---: | ---: | ---: |
| `tiny_cold` | 7.0981 | 6.9462 | -2.14% |
| `tiny_repeat` | 0.7775 | 0.7746 | -0.37% |
| `small_math` | 2.5074 | 2.6319 | +4.97% |
| `medium` | 2.9072 | 2.9587 | +1.77% |
| `large_cold` | 3.0078 | 3.0559 | +1.60% |
| `large_repeat` | 1.4280 | 1.3776 | -3.53% |
| `edited_check` | 1.7222 | 1.6946 | -1.60% |
| `edited_run` | 1.3622 | 1.3374 | -1.82% |
| `alternate_tiny` | 2.7564 | 2.7313 | -0.91% |
| `syntax_error` | 1.7196 | 1.7735 | +3.14% |
| `after_error_tiny` | 2.7023 | 2.7131 | +0.40% |
| `tiny_after_restart` | 7.9686 | 8.0532 | +1.06% |
| `repeat_after_restart` | 1.0532 | 1.0999 | +4.44% |

All median changes remain within 5%; no >10% regression was observed. This
preserves interactive speed; M18 does not claim a major new production speedup.
The 10,160-byte large case measures managed launch, not arbitrary compute speed.
Tiny cold includes first-use firmware loader preparation; restart is separate.

Baseline helper SHA-256:
`68744637ff033ec44a448d4616e7c7ff80ffe66966e86c2076efd94e4fe29e70`.
M18 helper SHA-256:
`20bd0ba769256b941bb311e07417739026d1692df722ad783a2a54a097809165`.
See [paired results and full identity checks](../../evidence/m18/benchmark-summary.json).
Raw trials are `artifacts/m18/bench-final/{before,after}-{0..4}`.
