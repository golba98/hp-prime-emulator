# M19 — NAND, boot hardware and storage

M19 is integrated and regression/performance verification is complete. Starting checkpoint is `e0d1e6e`, the M18 reporting successor to `11356a1`, on `feat/g1-usability-overhaul`.

**Actual HP Prime kernel boot: not attempted — missing matching authorized artifacts.** The executed boot chain is self-authored. Production Rust HLE remains the calculator/PPL backend; FullSystem diagnostics remain separate.

## Executed hardware foundation

The pinned qemuPrime source remains `32e891a9fc568de3110343c3e35b9ec0d23f6f44`, VERSION 2.10.1. Six ordered GPL overlays build in an owned export using the existing pinned Xenial/DTC toolchain. No shared reference checkout, original firmware resource or physical calculator was modified.

The clean combined build is `/home/k9-vortex/.cache/prime-m19-integrated-build-04`; QEMU SHA-256 is `e474ec9f608ea44aaf4934d3dafaf8546a78c8985c2d4bf148f667c262e777b3`. Its `build.json` records all six overlay digests and immutable container identity. Earlier failed DTC-path and malformed patch attempts remain under `artifacts/m19` and owned cache directories.

| Component | Implemented | Host tests | Guest execution | Physical parity |
|---|---|---|---|---|
| CPU/MMU | Existing ARM926 plus M18 alignment overlay | Existing reset/controller controls | Original smoke, IRQ/FIQ, UART/GPIO and MMU fault/recovery cases retained | Not established |
| Timer/INTC | Five-channel timer; clock-change integration; existing interrupt subset | Five-channel virtual-time and INTC reset/FIQ controls | Existing IRQ/FIQ handlers and new timer rate/gate/reset observations | Not established |
| NAND/OOB/ECC | Bounded EC/DA synthetic geometry, command/address/data cycles, page/OOB program/read/erase, busy/status/reset/locks; MAIN 512-byte 1-bit ECC | 15 NAND controls plus two bootstrap/reset controls | NAND round trips, bad-block-marker skip, ECC classification/software repair | Not established |
| Clocks/reset | Opt-in S3C2416 clock tree; fixed-clock compatibility default; documented software reset | Clock/divider/gate/reserved configuration and reset controls | Deterministic guest rate changes, UART gating and software reset | Not established |
| UART/GPIO | Existing UART transport with clock inputs; resettable matrix state | Register/reset/input checks | Exact UART bytes, matrix scanning and held-key aliases | Not established |
| LCD | Repaired S3C two-window register subset and bounded mappings | OSD stride, buffer selection, invalid mappings and repeated reset | Window-0 RGB565, padded stride, buffer flip/update and exact captures | Not established |
| Keyboard/touch | Inherited matrix and bounded NT11002 polling contract | Malformed touch transactions and reset | Scripted key/touch transitions, coordinates, held-input reset and release | Not established |
| Boot chain | Explicit synthetic main-data bootstrap policy | OOB stripping and reset recopy | Reset address zero → stage 1 → NAND-loaded, CRC-checked stage 2 | Not established |

[NAND implementation and exact evidence](../../research/full-system/M19_NAND_EXECUTION.md), [clock execution](../../research/full-system/M19_CLOCK_EXECUTION.md), and [LCD/input execution](../../research/full-system/M19_LCD_INPUT.md) record source contracts, commands, input identities, observations and remaining limits.

NAND writes use a complete erased synthetic 276,824,064-byte interleaved base, mounted read-only, and disposable qcow2 v2 overlays. Reopen tests establish main/OOB persistence. Bad blocks are physical OOB markers; guest software applies the test-owned skipping policy. No proprietary image is a NAND test input. MAIN 512-byte ECC has independent vectors and guest corruption tests. The 2,048-byte extension is an unverified research derivation; spare and 4-/8-bit ECC, NAND IRQ and DMA remain unsupported.

Clock-tree mode is explicitly selected with `s3c2416-sysc.clock-tree=on` and supplied oscillator inputs. Default research timer PCLK remains 66 MHz. Instruction-count virtual time is deterministic, not physical CPU cycle accuracy. UART transport remains immediate; rate calculations and gates do not establish serial wire timing. LCD verification covers RGB565 window 0 and window-1 registers, not complete compositing, palette, RGB888 execution or accurate vblank/IRQ timing. Touch raw extents, packet format and conversion delay remain inherited model assumptions.

## Reproduction and evidence categories

Build a new directory using `full_system_build.py`, the pinned reference/DTC checkout and overlays `0001` through `0006` in order. Guest cases run through:

```sh
ppl full-system info --build-root /home/k9-vortex/.cache/prime-m19-integrated-build-04 --json
ppl full-system run --build-root /home/k9-vortex/.cache/prime-m19-integrated-build-04 --case nand --output NEW_DIRECTORY --timeout 60 --json
```

Cases are `smoke`, `timer_irq`, `timer_fiq`, `uart_gpio`, `mmu`, `nand`, `nand_ecc`, `boot_chain`, `clock_reset`, `lcd` and `input`. The coordinator executed every case three times against the final combined binary: 33 passes. Exact command arrays and return codes are in `artifacts/m19/integrated-guests-summary.json`; every run retains guest source/ELF hashes, GDB entry/step/halt/register/mailbox observations and actual UART. All five separate integrated host-model suites pass; these are qtest device access, not ARM execution.

The boot experiment has no `-kernel`: stage 1 starts at zero, with code deliberately crossing a NAND main-page boundary. It reads a 96-byte stage 2 from NAND page 4 into `0x30010000`, checks CRC `0xab6fb861` and transfers control through guest instructions. Stage 2 reaches `0x30010040`, reports `0x5a190002` and emits `M19 BOOT2\n`. GDB never sets PC or writes payload/results. The host bootstrap copy is a declared model policy; it does not emulate Samsung iROM, its ECC format, boot straps or physical DRAM training.

The Rust adapter validates case-specific execution and source identities, mailbox semantics, UART artifacts and ELF hashes. For boot it checks both ELF identities; for NAND it rehashes the immutable base; for LCD it independently checks all captured pixels. Capability discovery binds new cases to exact published overlay hashes. Cancellation cleanup recognizes only bounded CID paths inside the privately owned run directory, including NAND's sequential stages. Actual NAND CLI SIGINT returned 130 and the captured owned container was independently absent.

The GUI runs diagnostics asynchronously with cancellation and revision-bound results, displays captured LCD output and retains UART/register details. An actual Xvfb LCD diagnostic and its preview were inspected. Native debugging remains separate. FullSystem PPL, CAS and calculator UI remain unavailable.

## Boot prerequisites

| Prerequisite | Available/tested | Remaining requirement |
|---|---|---|
| CPU reset/entry | Synthetic reset zero and ELF-entry controls | Matching actual boot-mode/reset contract |
| Initial memory | Modeled boot RAM and existing DRAM aliases | Physical boot mapping/DRAM initialization evidence |
| Clocks | Explicit selectable research inputs/tree | Actual G1 crystal/PLL setup and required clock paths |
| NAND geometry/OOB | Synthetic EC/DA profile and raw OOB tests | Actual chip identity, complete authorized image and verified layout |
| Boot ROM/first stage | Self-authored bootstrap and executed handoff | Matching authorized BXCBOOT/BESTAARM and ROM/strap behavior |
| Kernel entry | Synthetic stage 2 executes | Verified PRIME_OS load layout/entry and dependencies |
| Application stage | Existing HLE armfir resources | Real kernel execution and services sufficient to launch armfir |

Authorized local update components still hash to the M18 inventory: PRIME_OS.ROM 1 MiB, PRIME_APP.DAT 32 MiB, PRIME_MASTER.DAT 4 MiB, plus configured application ELFs. No matching BXCBOOT/BESTAARM or complete NAND/OOB image was found in the inspected scope. These components were never combined into a claimed boot image, downloaded or modified.

## Runtime and firmware observations

Production OFF/persistence recovery remains authoritative and `live_restore_ready=false`. The separate `synthetic_snapshot::resumable` owner executes two self-authored SVC instructions, captures explicit pending wait/receive data after Unicorn returns from its callback, and restores a fresh engine/process. Logical time, deadlines, queue state, request sequences and binding epochs prevent stale or duplicate completion. It serializes no native stack, callback or host handle. All23 snapshot tests pass, including9 service tests and fresh-child continuation/terminated-save/restore controls. This is a synthetic prerequisite proof, not the proprietary OSWaitForEvent/GetEvent ABI. See [service contract and remaining firmware blockers](M19_RESUMABLE_SVC_FOUNDATIONS.md). Coordinator review also added checksummed-Thumb-state rejection for this ARM-only owner.

Retained firmware controls captured duplicate exported globals in both import orders: a newly compiled consumer observed the first imported definition, while each producer retained its own slot. Complete replacement/rebinding semantics remain unverified; Native ambiguity remains explicit Unsupported. The remaining M18 initializer-cycle and declaration/validation-stage limitations are retained, without broad error/value normalization. Initializer failure changed the independent probe once, but exact source installation was refused; later RunStored attempts were infrastructure refusals. Those attempts establish no initializer retry equivalence and remain preserved.

New genuine Native/HLE boundary comparisons cover nested complex lists and a full-width 64-bit integer: seven assertions pass, alongside all 94 retained assertions. CAS `1/3` and `x+1` produced unverified tag `0x18`/subtype `2`; their bounded 32-byte captures remain Unsupported, with no speculative pointer traversal. New replay/corruption tests preserve those limits. Compact control evidence is under `evidence/m19`; full captures remain in `artifacts/m19`.

## Final verification

Tested implementation checkpoint: `7cd5d98`. Final workspace: **1,919 passed, zero failed, six existing ignored**. Formatting and all-target Clippy with warnings denied pass. Emulator Python tooling53 and general Python tooling41 pass. Configured firmware11, stateful22 and checkpoint3 tests pass without skips, with exact identities in `evidence/m19/runtime-regressions-display.json`.

Actual GUI assertions pass: boot9, CAS/graph/apps59, persistence/crash recovery35, typing11 on15751 and11 on15157, and linked Native debugging40. Two separate actual FullSystem GUI diagnostic runs complete; the captured LCD preview was inspected. Physical USB monitoring/jobs were disabled throughout.

All33 combined guest runs and five host-model suites pass. Synthetic snapshots23 pass, including9 service tests. Historical77 classifications and101 Native/HLE assertions are preserved. All five identity-matched M18/M19 benchmark pairs pass all130 expected outcomes. Final Native linkage GUI also passes15 unchanged assertions; combined scripted GUI groups total180 assertions, plus two executed research GUI cases.

Initial integrated workspace validation exposed `ETXTBSY` when a test directly executed its newly written cleanup mock. The fixture now invokes the stable `/bin/sh` interpreter with that same mock script and retains all cleanup assertions. Production cleanup behavior is unchanged. The first post-handoff evidence replay also correctly noticed an additional strengthened input capture; its expected fixture count was updated without removing any validation. Failed logs remain retained.

## Performance preservation

Five alternating before/after pairs execute all13 established workloads with matching firmware, ROM, initial profile, script, sources and host identities. No competing project builds or tests ran during measurement. Request/result wall time excludes IDE rendering. Raw trials are retained under `artifacts/m19/bench-final`; [all samples and identities](../../evidence/m19/benchmark-summary.json) are committed.

| Workload | Matched M18 median (s) | M19 median (s) | Change |
|---|---:|---:|---:|
| `tiny_cold` | 7.0400 | 7.2140 | +2.47% |
| `tiny_repeat` | 0.8010 | 0.7757 | -3.16% |
| `small_math` | 2.6327 | 2.5061 | -4.81% |
| `medium` | 2.9845 | 2.9344 | -1.68% |
| `large_cold` | 3.0324 | 3.0619 | +0.97% |
| `large_repeat` | 1.3771 | 1.3782 | +0.08% |
| `edited_check` | 1.7771 | 1.7223 | -3.08% |
| `edited_run` | 1.4126 | 1.3625 | -3.54% |
| `alternate_tiny` | 2.6813 | 2.8818 | +7.47% |
| `syntax_error` | 1.7959 | 1.7974 | +0.08% |
| `after_error_tiny` | 2.7153 | 2.7926 | +2.84% |
| `tiny_after_restart` | 8.3574 | 8.3085 | -0.58% |
| `repeat_after_restart` | 1.0513 | 1.0263 | -2.38% |

Tiny repeat remains0.776s and large cold3.062s. No median regression exceeds the planned10% investigation threshold. Alternating tiny has the largest measured increase,7.47%; its sample ranges overlap (before2.631–2.958s, after2.806–2.935s). This remains visible rather than normalized away; no production scheduler or tracing change was introduced and no new speedup is claimed.

Baseline helper SHA256: `20bd0ba769256b941bb311e07417739026d1692df722ad783a2a54a097809165`. Final integrated helper SHA256: `37407156fcec205864a6eac6d9d7813edef43bd35bbf35c41c017239ec6654e4`. The separate worker runtime regression helper rebuilt byte-identically to M18 before the service-owner integration; its source and binary identities are recorded separately from this final integrated performance helper.

All48 exact container names recorded by the coordinator runs, and the actual cancellation CID, were independently absent after completion. Original firmware hashes and the pinned QEMU reference checkout remain unchanged. The three worker checkouts are clean, their handoffs are integrated and their ownership claims are released. Local checkpoints were created; nothing was pushed or published.
