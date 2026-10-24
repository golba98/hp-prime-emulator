# M2 Implementation Report

**Milestone:** Phase 2 / M2 — Firmware Execution & Headless Boot
**Date:** 2026-10-23 · **Status:** `M2 PARTIAL — FIRMWARE EXECUTION VERIFIED`
**Crate:** `crates/prime-emulator` (new; no existing code touched)
**Firmware under test:** official HP Prime G1 firmware 2.4.2 (2026-09-09)
`armfir.elf`, sha256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`
(user-supplied; obtained from HP's channel; never committed).

## 1. What was actually built

A headless firmware-execution backend:

```
crates/prime-emulator/
  build.rs                 links libatomic (Unicorn 2.x TCG requirement)
  .cargo/config.toml        space-free build dir (Unicorn CMake constraint)
  src/
    lib.rs                  module map
    memory.rs               host-backed guest memory blocks + dynamic heap
                            (free-list, coalescing, aligned PE allocations,
                            bounded string access, no unbounded reads)
    firmware.rs             real ELF32/ARM loader (goblin) + PRIME_OS.ROM
                            header parse + the verified static memory map
    engine.rs               per-thread Unicorn engine: SVC/IRQ dispatch hooks,
                            memory-fault hook, slice run loop, synchronous
                            ISR execution with full register save/restore,
                            optional instruction trace, the firmware's
                            yield-idiom workaround (word 0xEE17FF7E)
    services/
      mod.rs                742-entry registry, verified SVC ABI dispatcher
      names.rs              generated: SDKLIB export names (721 named, 21
                            deliberately unnamed)
      handlers.rs           demand-driven installation (execution order)
      sync.rs               host events/semaphores/reentrant critical sections
      threading.rs          guest threads (own engine per native thread),
                            sync services with guest descriptors
      fs.rs                 guest VFS: drive letters, path jailing,
                            case-insensitive resolution, find/IN/DOS dates
      fsops.rs              file services + loader file descriptors
      system2.rs            LCD object, devices, PE module loading, INI,
                            system variables
      pe.rs                 PE32/ARM loader: sections, exports, IAT
                            resolution, base relocations, return-zero
                            Thumb stubs for unresolved imports
    trace.rs                bounded structured diagnostics: stages, maps,
                            SVC calls, call-site counters, JSONL + human
    runner.rs               boot orchestration, thread spawner, virtual IRQ
                            pump, limits, heartbeat detection, run report
    main.rs                 `prime-emulator` CLI (exit codes 0-4)
  tests/  loader.rs (7) · memory.rs (11) · svc.rs (7) · firmware_boot.rs (1)
scripts/emulator/run.sh      cargo wrapper (space-free CARGO_TARGET_DIR)
```

## 2. Verified startup parameters (from Phase 1 evidence, now validated live)

| Parameter | Value | Source |
|---|---|---|
| Entry | `0x30604AE8`, ARM state | ELF header; first SVC (`GetCurrentPathA`) reached from the entry path, confirming the state |
| SP | top of a 4 MiB heap block at `0x20000000` | PrimU2 model; 42M+ instructions of error-free execution |
| r0 | 0 (`argc != 4` path) | entry disassembly; firmware took the expected branch |
| OS ROM | mapped inert at `0x30000000` | header-verified |
| Segment | single RWX `PT_LOAD` at `0x30600000` (filesz 0x76F634 / memsz 0x7756C0) | readelf + goblin |
| DLL bases | pre-mapped at preferred images (SDKLIB @ `0x10000000` etc.) — required because WinCE DLLs here have **no .reloc** | observed load failure without it |

## 3. The SVC ABI (M2-verified correction of a Phase 1 detail)

The firmware's stub table starts at `0x307FBBE4` (not `0x307FBBEC` as Phase 1
recorded): each stub is `push {r0}; push {lr}; svc #(0x10000+N)`, reached by
plain `bl`. Dispatch convention (validated by 16,950+ live calls and unit
tests): `[SP]`=caller return, `[SP+4]`=saved r0, caller stack args at
`[SP+8]`, return via `R0=result, SP+=8, PC=[SP]`.

## 4. Demand-driven service implementation (86→97 services, wave ledger)

Wave 1 (boot blockers in strict mode): `lmalloc/lcalloc/lrealloc/_lfree`,
`GetCurrentPathA`, `ProgramIsRunningA/W`, `GetSysTime`, `GetEvent`,
`OSSleep`, `BatteryLowCheck`, `Interrupt*`.
Wave 2 (diagnostic-run ranked): thread creation + all sync primitives,
`GetActiveLCD`/`LCDOn`/`SetSystemVariable`, `GetMasterIDInfo`,
`WriteComDebugMsg`, devices (`CreateFile`/`DeviceIoControl`/`CloseHandle`),
the complete VFS file set (`__wfopen`, `_fread/_fwrite/_fclose`,
`__fseek`/`_ftell`/`_filesize`, `_wmkdir/_wrmdir/_wchdir`, `_wfindfirst/
_wfindnext/_findclose`, loader descriptors `_OpenFile/_OpenSubFile/_ReadFile/
_FseekFile/_FileSize/_CloseFile`, INI, `_afnsplit/_afnmerge`, `__wremove`).
Wave 3: `_LoadLibraryA/W`, `_GetProcAddress`, `_FreeLibrary`,
`_GetModuleFileNameA` — a real PE32/ARM loader.

Two ABI bugs were found and fixed by evidence: PrimU2's `__GET` is a
translate-to-host (we initially double-dereferenced stack pointers), and
`0x1001F`'s verified export name is `InterruptEnable` (PrimU2's enum label
`InterruptMask` is a mislabel; registration by verified name fails loudly on
such mismatches — caught at boot).

## 5. Key engineering findings (new in M2)

1. **`SDKLIB.DLL` is fixed-base** (no `.reloc`): it must load at its
   preferred image base; the boot routine pre-maps every
   `A:\WINDOW\SYSTEM\*.DLL` at its preferred base.
2. **The firmware's timer ISR is a dispatcher** (decoded at `0x306073C8`):
   it increments a tick counter (`0x30D69DC0`) and calls 3 handler slots
   (`0x30D69DB8`) against deadlines (`0x30D69DB0`). Our virtual IRQ pump
   runs this real guest ISR every tick.
3. **Concurrent ISR engines race the main thread.** A dedicated ISR engine
   (PrimU2's model) produced torn guest callback pointers →
   `blx`-into-ARM-mode crashes at random points (traced via the
   instruction-trace ring). Fix: deterministic IRQ delivery between main
   slices on the *same* engine with full register save/restore — matching
   real-hardware preemption far better. After the fix: zero faults over
   240 s.
4. **The main thread reaches its idle loop**: `EnterCrit → OSWaitForEvent →
   LeaveCrit` at `0x3069341c`, 4,700+ iterations per minute, with the tick
   ISR running. This is genuine firmware steady state — but the firmware's
   timer-handler registration path has not been reached yet, so the loop
   currently waits on an event nothing sets. That gap is the M2 remainder
   (see M2_KNOWN_BLOCKERS.md).

## 6. Test evidence

- 26 synthetic tests: 7 loader, 11 memory, 7 SVC/dispatch, 1 optional
  firmware integration. All passing.
- Full workspace regression via `scripts/emulator/run.sh`:
  `cargo fmt --all -- --check` clean, `clippy --workspace --all-targets
  -D warnings` clean, `test --workspace --all-targets` 30/30 suites ok
  (including the pre-existing 700+ PPL tests — nothing regressed).
- Real-firmware integration (env-gated, 30M instructions, 45 s): passing
  with evidence gates (entry `0x30604AE8`, ≥5M instructions, >1000 SVC
  dispatches, IRQ delivery recorded).

## 7. Repository-integrity note (required by the Phase 2 task §17)

`docs/**/*.md` is intentionally untracked by this repository's
`.gitignore` (session-generated engineering docs). All M2 and Phase 1
documents therefore exist only on disk. No `.gitignore` change was made.
Risk: research is lost if the working tree is cleaned. Recommendation:
either commit an archive of `docs/emulator/` in a separate branch/PR, or
change the policy for `docs/emulator/` specifically. Awaiting user
direction; nothing was altered silently.
