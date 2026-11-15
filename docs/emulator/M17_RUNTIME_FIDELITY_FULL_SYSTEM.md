# M17 — Runtime fidelity, full-system execution and debugging

> Feasible M17 implementation and final regression/performance verification are integrated. Live CPU/session restoration, calculator kernel boot and new physical captures remain explicitly unsupported.

M17 implements linked Native programs across the CLI and IDE, richer typed firmware observations, safer firmware input and OFF sequencing, request-bound live inspection, independently restorable snapshot components, and actual ARM926 execution on the pinned `hp-prime` QEMU model. The Rust HLE backend remains the production calculator backend.

This milestone does **not** implement resumable whole-calculator live snapshots, a booted PRIME_OS kernel, FullSystem PPL execution or new physical-device verification. Existing firmware-persistence recovery remains the supported recovery mechanism. A separately captured debug post-CAS checkpoint admission failure is recorded below; it must not be confused with either the corrected boot-readiness predicate or the corrected Shift-latched OFF sequence.

Integration branch: `feat/g1-usability-overhaul`, starting at `7e33a62` (M16). Final source checkpoint: `ebfe0f9`. Final tested helper SHA-256: `68744637ff033ec44a448d4616e7c7ff80ffe66966e86c2076efd94e4fe29e70`. No push, publication, hardware writes or proprietary firmware modification occurred.

## Implemented behavior and evidence

### Runtime readiness, deadlines and input acknowledgement

The retained post-CAS debug profile was making progress while remaining `Booting`: one diagnostic executed 82.8 million main-thread instructions in 15.015 seconds, delivered real LCD frames and continued handling OSWaitForEvent, OSSetEvent, GetEvent and GetSysTime. The sampled hot PC was a valid glyph alpha-blending loop. This evidence did not support calling it a host deadlock.

Boot readiness had reused the sustained idle share intended for safe interruption and persistence. Rendering-heavy but responsive firmware could therefore fail the boot predicate. Readiness now requires a validated LCD, two genuine normal timer/pump round trips, a fresh main wait and an empty input boundary. It adds no desktop input, arbitrary delay or synthetic timer burst. Strict checkpoint admission remains separate: its 80% idle threshold and 30-second deadline have not been relaxed.

The existing producer thread now also enforces finite run deadlines while an engine is blocked in a native wait. Cancellation wakes the host wait and returns a time-limit failure; it cannot turn a failed boot into a successful result. Deterministic readiness and blocked-wait regressions cover these paths. Unchanged debug normal-OFF/CAS/list/matrix/fresh-process recovery tests subsequently passed, with earlier recorded runs around 90–133 seconds; these are debug test durations, not interactive launch benchmarks.

A later actual GUI regression exposed a distinct synchronization bug: bracket paste stopped at `[1]` while an explicit keypad control produced `[1,2]`. Translation sampled a transient status region during redraw. A parked GetEvent pump and coherent RAM did not prove that the main UI had finished the preceding key.

Desktop translation now requires a fresh main-wait generation after the actual delivered keydown, with the main owner blocked in its exact unsignaled condition-variable wait. Signaled events, immediate-return callbacks and an owner awaiting Unicorn reentry cannot acknowledge UI completion. Admission is rechecked inside the memory gate. Raw releases, cleanup, Stop and managed calculator keys remain deliverable. No glyph-mask relaxation or timing delay was used. Deterministic regressions fail with the old predicate; real typing controls pass on firmware 15157 and 15751. GUI assertions now explicitly reject `Desktop typing stopped`, instead of treating quiet pixels as proof of success.

See [runtime readiness](M17_RUNTIME_READINESS.md). Relevant code: `prime-emulator/src/session/{readiness,typing}.rs`, `session.rs`, `runner.rs`, `services/{mod,sync}.rs` and the strengthened Xvfb typing assertion.

### Mode-aware firmware OFF and honest checkpoint failures

Controlled firmware tests established another independent bug. The old blind Shift→ON sequence toggled a pre-latched Shift off, so ON did not power down the calculator. Neutral mode saved in 0.914 seconds; Shift-latched mode timed out at 30.003 seconds, with Shift visibly cleared and no firmware sleep. Subsequent normal OFF from the now-neutral state saved and closed correctly.

The production OFF planner now observes the real firmware Shift glyph at a completed input boundary. With Shift already active it sends ON directly; otherwise it sends one Shift, waits for acknowledgement of that delivered press, confirms Shift and sends ON. It never retries Shift to manufacture acknowledgement. Alpha is not cleared to infer Shift. Unknown explicit-checkpoint screens are deferred before OFF input; legacy normal PowerDown retains its documented native-key fallback. An expired transaction clears queued power input so a delayed ON cannot arrive after failure.

After the fix, local controls for neutral, Shift, one-shot Alpha and locked Alpha all reported `Saved` in approximately 0.94–1.06 seconds. Waking cleared one-shot Alpha and retained Alpha lock. These controls verify firmware-native OFF persistence and modifier handling; they do not verify live CPU restoration.

The original debug post-CAS explicit-checkpoint timeout remains a separate finding. A retained failure showed strict idle share 72.73%, below the unchanged 80% requirement, and `off_sent=false`: no OFF sequence had been admitted. The earlier run had no retained modifier capture, so it cannot be assigned the Shift cause retrospectively. Failure reports now identify admission versus save stage, OFF/planner phase, main waits, actual UI/pump waits, queued and delivered input, sleep/save generation and idle measurements. An optional single-event trace records checkpoint admission without hot-loop logging.

The source-based native-backend experiment is complete. Cargo's default dev build selected unoptimized native Unicorn (`Debug`, logging enabled), while a package-only opt-level2 selects `RelWithDebInfo` (`-O2 -g -DNDEBUG`, logging disabled). Matched-source recovery trials measured O0 92.36/90.08 seconds versus O2 80.51/79.25/82.92 seconds: median 91.22→80.51 seconds, 11.74% lower. Rust debug settings, guest timers, the 80% gate and30-second deadline remain unchanged. Cargo.toml now applies this narrow dependency override; use `cargo --config 'profile.dev.package.unicorn-engine-sys.opt-level=0' ...` when native-engine assertions/unoptimized C debugging are specifically needed.

All five experimental trials passed; the independent final coordinator debug recovery passed 71.91 seconds. Admission timing records the first threshold crossing, not steady-state idle margin. Optimization does not guarantee that arbitrary busy firmware becomes checkpointable: strict requests still refuse unsafe admission and preserve prior generations. Raw build flags, commands, hashes and the separate72.73% refusal are retained in [backend comparison](../../evidence/m17/backend-build-comparison.json), [CMake configurations](../../evidence/m17/native-cmake-configs.json) and [strict-idle refusal](../../evidence/m17/strict-idle-refusal.json).

See [modifier controls](M17_MODIFIER_CHECKPOINT_CONTROLS.md). Relevant code and tests: `prime-emulator/src/{checkpoint,session}.rs`, `tests/firmware_recovery.rs`, `scripts/emulator/{checkpoint_controls,test_checkpoint_controls}.py` and retained local-firmware pixel fixtures.

### Native external-function linkage in retained sessions

Separately compiled bytecode units link into one VM. Calls use genuine exported names, one Host, one execution stack, shared resource/depth budgets and normal IFERR unwinding. Calculator globals/settings share live slots; private variables stay with their own module and source revision. Error frames retain the callee source identity and position. Missing, private or ambiguous exports fail explicitly, and edited dependencies invalidate their own linkage/state.

The linker follows the selected root's reachable dependency closure. An unrelated broken source, including an invalid builtin redefinition, cannot block a valid root. Normal cross-module recursion is bounded by the shared depth limit. Linked images containing Rc values stay local to the execution worker; retained session state and source catalogs remain Send.

A genuine firmware control established installation-time initialization: assigning a program in Programs changes its file initializer's shared variable before the first Run. Native Import/Edit now follows that observed behavior, while preserving unchanged module state. The SDK still initializes previously uninitialized linked units before their calls.

The CLI accepts repeatable `--module NAME=PATH` for Run and Check. Dependency inputs cannot alias output/report destinations. The IDE snapshots revision-bound open tabs for Native Run, Check and background diagnostics, invalidates callers after dependency edits and rejects stale compilation results. Callee errors point to the callee tab's original line. A real Xvfb IDE test displayed results 112→114→144 after a dependency edit, kept an unrelated invalid tab from blocking the caller, and showed a dependency division error at line 3.

Real retained-session Native/HLE comparisons pass installation initialization (12 assertions) and the three-program chain/edited dependency fixture (22 assertions). Unit tests additionally cover arguments, recursion, shared/private state, wrong arity, missing/private/ambiguous exports, edited revisions, IFERR and source-bound errors.

Limits remain explicit: replacing a catalog module dynamically during an active linked call returns Unsupported; cross-module exported variable references and initializer cycles need further firmware controls. Exact initializer failure/retry semantics are not claimed matched. Linked multi-module Debug is refused because the existing debugger owns one source buffer; single-module debugging remains available.

See [Native linkage](M17_NATIVE_LINKAGE.md). Principal files: `ppl-vm/src/{linker,machine,session,error}.rs`, compiler bytecode/compilation, `ppl-engine/src/{native,checking}.rs`, CLI unified commands and GUI compile/execution/diagnostic integration.

### Typed values from real firmware captures

Actual G1 firmware 15751 controls established complex, integer-variant, packed-decimal and real/complex matrix layouts. The decoder now handles vectors, rank-2 matrices, complex numbers, integer base/width variants and nested combinations with existing lists, strings and real values. Captures compare decoded components with genuine firmware RE/IM and indexed matrix results.

Decoding validates known allocation extents, dimensions/rank, cell counts, integer widths, decimal digits/exponents, tags, lengths and object identity. A shared 4,096-item / 1 MiB capture budget applies across nested structures. No guessed pointer is followed. Unknown objects remain explicitly undecoded; at most 4,096 bytes of their exact known host allocation can be retained for diagnosis. Representation/wire limits were tightened during integration, and oversized/malformed transport evidence cannot be accepted as an ordinary runtime error or a successful empty observation.

`tests/differential/m17/typed_values.toml` passes 17 Native/HLE assertions, including nested lists, strings, vectors, real/complex matrices, complex scalars, integer variants and a repeated retained run. Negative comparator controls identify changes in shape, vector identity, cell type, imaginary part, integer width/base and nested values at specific paths. No broad normalization was added.

Unsupported tags, special/nonfinite decimal encodings, rank > 2 and zero-dimensional matrices return explicit unsupported diagnostics. Rational results remain rational when no finite decimal is available. Firmware accepts ASCII `i` as an imaginary constant where Native currently requires the Unicode unit or pair syntax; the shared fixture deliberately uses pair syntax and documents the remaining input difference.

See [typed values](M17_TYPED_VALUES.md). Principal files: `prime-emulator/src/runtime_value.rs`, `runtime_value/representation.rs`, captured runtime-value fixtures, `ppl-compat/tests/typed_differential.rs` and the shared typed scenario.

### Live debugging and snapshot prerequisites

Protocol 7 exposes request/resource-bound session inspection. It reports real owner phases, service/caller/blocked-handle identity, timing, resource counts, main/input state and bounded diagnostics. Clients reject wrong identities, unsupported versions, oversized metadata and fabricated live-restore capability. `inspect_session.py` drains frames and queries only its owned helper using copied storage; a successful inspection after timeout is still reported as `session_ready:false`.

Safe-point requests have epochs and deadlines. Engines acknowledge only outside `emu_start` and native callbacks; registration is fenced during the pause lease, and drop/cancellation/expiry releases owners. The SVC accounting hot path remains allocation-free. Optional register copies are owner-thread observations, not restorable CPU contexts. Real idle inspection found thread 0 parked in OSWaitForEvent and thread 1 in GetEvent. Coherent RAM inspection was available, but those native continuations prevented resumable owner suspension. `live_restore_ready` remains false.

Two versioned component APIs now save/load/restore selected resources independently in fresh processes:

| Component | Implemented guarantee | Boundary |
| --- | --- | --- |
| Heap/allocator | Captures designated RAM plus allocation/free intervals under the memory gate and allocator lock; validates resource identity, partition/accounting and both digests before a fresh empty target changes. Atomic sibling-file replacement with fsync. | 256 MiB heap, 100,000 intervals, 8 MiB manifest. Other guest mappings, CPU/thread state and firmware globals outside the heap are absent. |
| Read-only VFS | Reconstructs original guest handles, positions, counters and cwd state by reopening jailed regular resources and verifying contents. Provisional descriptors drop on failure; installation happens only after validation. | 512 handles, 256 MiB referenced content, 4,096-byte paths, 2 MiB manifest. Private stable storage and exclusive owner required; writers/searches/durability errors refused. |

Twelve component tests passed before the final full regression. Fresh child processes verify RAM/allocator reuse and VFS offset/handle identity, including 32 reconstruction cycles and Linux descriptor counts. Negative tests reject corrupt/truncated/incompatible components, changed/replaced resources, occupied targets, writers, searches, symlinks, traversal and fabricated live-resume claims. A coordinator review found that opening a FIFO could precede the regular-file check; preflight rejection and a FIFO regression corrected that admission gap.

After unexpected termination, supported calculator recovery restores the last validated firmware OFF/checkpoint generation. Changes since that save can be lost. Heap/VFS component restoration does not extend that guarantee to arbitrary running sessions.

These are controlled component APIs, not an asynchronous live-snapshot feature. Capture requires callers to exclude writers outside the memory gate and to own stable private storage. Parent-directory fsync failure after an atomic rename represents durability uncertainty for a fully written replacement; it does not authorize installing a partial file.

See [live inspection](M17_LIVE_INSPECTION.md) and [snapshot components](M17_SNAPSHOT_COMPONENTS.md). Principal files: `prime-emulator/src/{safepoint,engine,services/threading}.rs`, `ppl-engine/src/{inspection,live}.rs`, memory and VFS component modules.

### Journal locking and differential forensics

A new concurrent subprocess/thread test reproduced false Busy after an owner closed its descriptor. `flock` belongs to the open file description, so a temporary descriptor inherited by a concurrently spawned child can prolong a lock until close-on-exec. `JournalLock` now explicitly unlocks its owned lease before closing, without retries, sleeps or serializing independent journals. A private duplicate-descriptor test proves release and that closing an old inherited description cannot unlock a later owner. Process contention, eight independent journals and all 18 parallel transaction tests pass.

The new integration test produced six owner-release failures before the change. Four earlier transaction stress runs had passed 72/72, so the older historical journal flake is not conclusively attributed to this reproduced mechanism. Busy diagnostics now include contender PID, exact path and Unix device/inode.

HLE differential reports retain bounded request/source-bound debug bundles before disposable profiles are removed. Async Await binds its original run request rather than a later control request. Malformed, oversized, missing or symlinked evidence produces explicit unavailable metadata. Transport warnings and missing semantic collections cannot become fake error/empty-value matches. The actual seven-report retained-session run preserved diagnostics after its profiles were removed.

Principal files: `ppl-programs/src/delete_store.rs`, `ppl-programs/tests/journal_locking.rs`, `ppl-compat/src/differential/firmware.rs` and differential regressions.

### Actual full-system research execution and INTC fixes

The pinned qemuPrime fork (`32e891a9fc568de3110343c3e35b9ec0d23f6f44`, QEMU 2.10.1) now builds inside a rootless, isolated Ubuntu 16.04 toolchain. GCC 5/Python 2.7 and dependencies stay in the container image; no system toolchain was changed. Its exact pinned DTC submodule is exported because Xenial's libfdt was insufficient. The scratch reference remains untouched. Reproduction scripts retain source/image/binary/package identities, logs and bounded runtime traces, and remove only their recorded owned containers.

The self-authored ELF **executes on the `hp-prime` S3C2416 machine model**, not a generic ARM fallback. Across fresh runs, GDB verifies initial PC `0x30000000`, one-step PC `0x30000004`, halt `0x3000005c`, mailbox `0x600d0001` and four RAM aliases returning `0x12345678`. Coordinator repetition independently confirmed these observations. Generic ARM results cannot satisfy the hp-prime alias acceptance predicate.

Unchanged qtest cases then reproduced two original model bugs: reset retained interrupt pending/offset state and an asserted CPU input; FIQ selection looked up enabled-but-not-pending sources and aborted. The isolated GPL overlay clears interrupt state/lines on reset and selects the actual single pending FIQ source with bounds checks. Reset and FIQ tests each pass three cycles, observing the modeled CPU IRQ/FIQ inputs, and UART writes produce `ABC`. Coordinator repetitions passed against the clean published-overlay build, whose independent ARM/GDB smoke also passed.

The evidence categories remain distinct: the mailbox test executes guest ARM instructions; qtest exercises device models with the CPU paused; neither executes a physical calculator kernel or proves physical INTC priority. Simultaneous FIQ priority remains explicitly unsupported. UART capture exists, but the ARM smoke itself does not transmit a UART payload.

The published overlay executable SHA-256 is `011b39176130685aba44be06f96b1e63c6b41d59efba4e48ce82bc26542a7b46`; evidence is in `research/full-system/evidence/m17-{smoke,intc}.json`. GPL changes stay in the separately licensed research overlay, not the Rust HLE implementation. No proprietary boot artifact was fetched or executed.

See [full-system research](FULL_SYSTEM_RESEARCH.md). Principal files: `research/full-system/{Containerfile.xenial,smoke.S,patches/0001-intc-reset-fiq.patch}` and `scripts/emulator/full_system_{build,run,intc}.py`.

## Regression and compatibility results

All final checks below use the integrated behavior. Resource-gated cases were explicitly configured separately; an ordinary offline workspace pass does not itself verify firmware.

| Check | Final result |
| --- | --- |
| Workspace tests, normal parallel execution | 1,853 passed; zero failures; six ignored |
| Formatting and workspace/all-target Clippy, warnings denied | Passed, including the optimized dev dependency profile |
| Actual GUI | 127 assertions passed: 101 firmware 15751 +11 calculator-only15157 +15 linked Native IDE |
| Configured release integration | 19 cases passed: boot, calculator-only capability refusal, inspection, sessions/imports, recovery, retained stateful scenarios, checkpoints and M17 drivers |
| Genuine Native/HLE comparisons | 51 assertions: initialization12, linkage22, typed values17 |
| Historical physical references | 77 classifications preserved on15157:68 exact, four partial, five transcription; zero mismatches; no new physical captures |
| Native historical leg | 49 exact, four transcription,24 missing; these remain separate from firmware matches |
| Component restoration | 12 synthetic tests including fresh child processes, corruption/resource rejection,32 reopen cycles with stable descriptor counts and FIFO preflight |
| Debug CAS/OFF/fresh-process recovery | Final coordinator run passed 71.91s; matched-source build experiment all five trials passed |
| Explicit modifier checkpoint controls | All four Saved, woke and shut down cleanly: neutral1.049s, Shift0.889s, Alpha1.111s, Alpha lock1.056s; lock preserved |
| Python tooling | All 25 tests passed |
| ARM/GDB and model checks | Actual hp-prime smoke plus three-cycle reset/FIQ qtests passed, independently repeated by coordinator |

Firmware Home, CAS, graphing,18 apps, PPL import/Check/Run, sustained typing, normal OFF and persistent recovery are exercised. Actual final CAS/graph/IDE screenshots were inspected. The retained paste failure, strict-idle refusal and unpatched QEMU failures remain available; successful later runs do not erase that evidence. Exact suite/log paths, identities and verification categories are in [verification summary](../../evidence/m17/verification-summary.json). Proprietary resources and raw profiles/screenshots remain external/ignored under artifacts/m17 or owned home-cache paths.

## Comparable performance measurements

Performance is measured with identical user-supplied firmware, ROM, copied initialized profile, workloads, script and host identity. The comparison rejects incomplete reports, wrong verdicts, changing helper identity, nonfinite/nonpositive durations and mismatched resource/source identities. The profile hash covers the validated copied tree, including directory and file kinds; a changed copy is refused. No benchmark opens USB or writes the original profile.

Five alternating before/after pairs provide 65 samples per helper over 13 workloads (130 workload measurements total). These are request-to-result helper wall times, with helper CPU time where available; they exclude IDE dispatch/rendering latency. `large_cold` means the first large import in a booted session, and boot is measured separately. Source check/compilation remains genuine firmware behavior.

The preserved M16 helper SHA-256 is `73c7980c1af014f60551c6c3e8d76faee81a6db7123b12657542b6f718163b6d`; final M17 is `68744637ff033ec44a448d4616e7c7ff80ffe66966e86c2076efd94e4fe29e70`. Both use firmware 15751, ELF SHA-256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`, identical ROM/profile/script/source hashes, Linux7.2.13/glibc2.43 on an Intel i7-13700 (24 logical CPUs), and Rust1.96.0. No concurrent project builds, GUI tests or other firmware sessions ran during measurement; normal desktop services remained running. All 130 expected results/verdicts passed.

| Workload | Paired M16 median (s) | M17 median (s) | Change |
| --- | ---: | ---: | ---: |
| tiny_cold | 7.033 | 7.032 | -0.01% |
| tiny_repeat | 0.752 | 0.752 | +0.06% |
| small_math | 2.381 | 2.408 | +1.13% |
| medium | 2.882 | 2.960 | +2.69% |
| large_cold | 3.034 | 3.008 | -0.88% |
| large_repeat | 1.352 | 1.354 | +0.14% |
| edited_check | 1.753 | 1.748 | -0.29% |
| edited_run | 1.336 | 1.357 | +1.60% |
| alternate_tiny | 2.712 | 2.759 | +1.74% |
| syntax_error | 1.769 | 1.792 | +1.29% |
| after_error_tiny | 2.721 | 2.718 | -0.12% |
| tiny_after_restart | 8.081 | 8.271 | +2.36% |
| repeat_after_restart | 1.025 | 1.028 | +0.25% |

All 13 medians are within2.7% of baseline; none crosses the 10% investigation threshold. Five samples do not establish a latency distribution or causality for these small movements. M17 preserves the existing fast production path rather than claiming another major speedup. The 10,160-byte large workload tests source launch/preparation, not arbitrary large-program computation. First tiny cold/restarted requests still include loader setup and take about7–8seconds. The native-Unicorn dev-build improvement is a separate measured result above.

[Complete paired samples and identities](../../evidence/m17/benchmark-summary.json) retain every sample. Raw source/stage/CPU/profile reports are in artifacts/m17/bench-final. The collector and comparison commands are documented in scripts/emulator/BENCHMARKS.md. Incomplete, changed-resource/source or failed-verdict trials cannot become a passing summary.

## Remaining limits and next engineering steps

1. Retain strict admission for busy firmware. The build experiment improves debug backend cost; it does not prove universal checkpoint availability. Use normal firmware OFF as the authoritative persistence path, inspect the phase/idle diagnostics on refusal and preserve failure evidence with PRIME_TEST_EVIDENCE.
2. Complete live snapshots only after resumable CPU contexts, stack-safe native SVC continuations, scheduler/handle/timer reconstruction, remaining mappings and coherent whole-machine commit are implemented and resumed in a fresh process. Current component restore is useful infrastructure, with no application-resume guarantee.
3. Extend Native only after controlled firmware observations for exported variables, initializer cycles/failures and dynamic replacement during calls. Add proper multi-source navigation before enabling linked Debug. Preserve the documented ASCII imaginary-unit difference until fixed and compared.
4. Expand typed capture controls for unsupported tags, special values and higher ranks without speculative pointer decoding or lossy normalization.
5. Continue full-system experiments with verified timer/interrupt delivery and authorized matching boot/NAND/OOB artifacts. Current update resources do not establish a bootable image. MMU/kernel progress, storage, LCD and input must be demonstrated before enabling FullSystem PPL.
6. Capture new physical-device scenarios only with actual authorized hardware interaction. Historical replay, local firmware evidence, Native comparisons and qtest model checks retain separate provenance.
7. Monitor parallel journal stress. The inherited-description failure is fixed with deterministic controls; attribution of the older intermittent flake remains unproven.

No physical G1 was used to create new M17 observations. No firmware binary, real calculator save or other project was changed. Research and recovery tests used isolated disposable storage and retained their evidence.

## Source and checkpoint record

The source-file groups above describe the implementation boundaries. The exact manifest is `evidence/m17/changed-files.txt`; recording the complete integrated change set, including final reports and evidence. Runtime code was tested at `ebfe0f9`; final reporting artifacts follow that checkpoint. Documentation is in this report, the focused M17 reports and the living overhaul/differential/architecture/full-system reports.

The branch retains its individual tested implementation and integration commits. For the complete checkpoint history, use `git log --reverse --oneline 7e33a62..ebfe0f9`; no history rewrite or squashing is required. Important causal checkpoints are:

| Commit | Change |
| --- | --- |
| `76f4364` | Explicit journal lease release |
| `89323d7`, `217099b` | Request/source-bound differential forensics |
| `9531938` | Actual pinned hp-prime ARM/GDB execution |
| `44d5b5e` | Readiness proof and blocked-wait deadlines |
| `2045b0c`, `99aa5ca`, `1b5f7f8` | Native VM linkage, engine/CLI catalogs and IDE integration |
| `45d5c5b` | Reproduced INTC reset/FIQ overlay fixes |
| `b92e505`, `8d482a1`, `25e2297`, `aeed448` | Complete workload/resource/profile/host benchmark identity |
| `4215c29`, `a56d95e` | Inspection, bounded safe points and allocation-free SVC accounting |
| `451aa26`, `131a9de` | Typed decoders and shared typed scenario |
| `8112dc5`, `d4c7a21` | Component restore and pre-open regular-resource admission |
| `9809579`, `b45f349` | Allocation/wire bounds and explicit observation gaps |
| `2351a62`, `e8c8588` | Detect stopped typing and acknowledge actual main-UI completion |
| `a43e080`, `d6f1826` | Retained checkpoint diagnostics and controlled modifier evidence |
| `d3fed72`, `f51f1a8` | Mode-aware OFF and optional single-event admission trace |
| `ebfe0f9` | Measured native-Unicorn dev profile and retained build/refusal evidence |
