# Phase 2 Implementation Plan

Goal: a **headless Rust emulator core** that boots real HP Prime G1 firmware
(`armfir.elf`) on Unicorn, services its OS calls, and exposes a scriptable
automation interface — no GUI, no kernel, no hardware modeling beyond what
the SVC boundary requires. The smallest useful executable milestone is
deliberately early and binary (see M2).

Assumptions carried from Phase 1 (each is a gate that can invalidate the
plan, not a certainty):

- A1: the 20260909 firmware (analyzed) is representative; the pinned test
  target is 20250915 (PrimU2's), to be obtained the same legitimate way.
- A2: armfir boots with SVC interception alone (PrimU2 is existence proof on
  20250915; must be re-proven on our target).
- A3: the PrimU2 memory map (`common.h`) is correct enough for boot.

## Milestones

### M0 — Workspace and build foundation (1–2 weeks)

- New crates under `crates/` per EMULATION_ARCHITECTURE.md §5, starting with
  `prime-arm` (Unicorn FFI) and `prime-memory` only. No other crates until
  M2 needs them.
- Resolve the two build constraints found in Phase 1:
  - spaces-in-path breaking Unicorn's CMake (document `CARGO_TARGET_DIR` at
    a space-free path, or relocate repo — decide once, record in README);
  - `-latomic` link flag on Fedora (build script).
- Decision: bundle-build Unicorn via `unicorn-engine` crate (default) vs
  link Fedora `unicorn` rpm. Default to the crate for reproducibility.
- **Verification:** the Phase-1 validation tests (T-ARM-1/T-SVC-1/T-MEM-1/
  T-ELF-1) become `prime-arm` integration tests, run in CI on a space-free
  checkout path.

### M1 — Firmware image tooling (1–2 weeks, parallel with M0)

`prime-firmware`:
- Parse `PRIME_APP.DAT` header + FAT16 at offset 8192 (pure Rust read/write;
  mtools parity tests against the real image, which stays gitignored).
- Loaders verified in Phase 1: OS-ROM header (entry/loadaddr/"V5J\0"),
  armfir ELF PT_LOAD mapping (goblin), PE32 DLL parse (SDKLIB export table
  ↔ service-id table generation: build the 742-entry table from the DLL at
  build time, replacing hand-copied constants).
- **Verification:** `prime-firmware` round-trips the disk, extracts
  armfir.elf/SDKLIB.DLL, and generates a service table matching PrimU2's
  `interrupts.h` enumeration (diff test).

### M2 — First firmware execution (the pivotal milestone, 2–4 weeks)

`prime-services` (dispatch + the PrimU2-minimum handler set), `prime-core`
(boot orchestration):
1. Map PRIME_OS.ROM (inert), map armfir PT_LOAD at 0x30600000, main stack at
   0x10000000, dynamic heap at 0x20000000, static regions per the verified
   map.
2. Implement the SVC dispatcher (validated convention) with full logging;
   unimplemented services return 0 and are **logged, counted, and ranked** —
   the ranking is the worklist, not a guess.
3. Start a single guest thread at `0x30604AE8`; iterate: crash → fault
   report (Capstone) → implement the blocking service → repeat.
4. Deliverable: the firmware runs its first instruction stream to a stable
   hang/idle without host exception, with a documented log of every
   unimplemented SVC hit.
- **Gate:** if armfir jumps into kernel/DLL memory (violating A2), stop and
  pivot: either map inert DLL exports (KRNLLIB is tiny: 104 exports) or
  escalate the full-system path (Architecture C) for a re-evaluation.
- **Success criterion (binary, honest):** defined *before* chasing it —
  the guest reaches its event loop (repeated `GetEvent` polls observed) or a
  measurable heartbeat (timer tick loop). "Booting" claims are only allowed
  with this evidence; a framebuffer showing the HP logo would be celebrated
  but is not required for M2 success.

### M3 — Virtual disk + filesystem services (2 weeks)

`prime-filesystem`: drive-letter VFS over the FAT16 image with a **host
sandbox** (fix PrimU2's traversal hole: drive-absolute paths must be
lexically normalized and jailed under the image), loader file descriptors,
INI, findfirst/findnext. Persistence = the image file.
- **Verification:** scripted guest file operations produce the same disk
  state as mtools-performed equivalents (golden image diff).

### M4 — Display and input as data, not UI (2 weeks)

`prime-display`/`prime-input`: framebuffer surface snapshot API (the PrimU2
LCD object layout), device keycode event queue, scripted input injection.
No window; PNG dump (reuse `ppl-graphics`) for humans.
- **Verification:** scripted key sequence (Home, Apps, digits) produces
  frame snapshots that change correspondingly; a PPL program entered
  through the program editor via scripted keys compiles and runs on the
  guest — **first end-to-end PPL-through-HP-compiler execution**.

### M5 — Automation server and test harness (2 weeks)

`prime-testing` + `prime-debug`:
- TCP command server (`key`/`touch`/`screenshot`/`state`, PrimU2-compatible
  plus `svclog`), GDB stub (Unicorn-native), fault/trace tooling.
- Extend `ppl-compat` with an emulator backend: the existing differential
  cases run against (a) local PPL VM, (b) firmware emulator, (c) hardware.
- **Verification:** one full differential case passes on all three engines
  with identical results (numbers via PRINT-to-screen OCR-free comparison
  is *not* acceptable; use the M9-matrix technique from hp-prime-kit adapted
  to the emulator: read back guest variables through the disk).

### M6+ (Phase 3 candidates, deliberately out of Phase 2 scope)

- Threading/scheduler fidelity (cooperative deterministic scheduling),
  InterruptController/TimerController completion, PE loader completion,
  PPL compiler entry-point RE (PPL_COMPILER_RESEARCH.md §7), version pinning
  for 20250915 vs 20260909, full-system QEMU path (Architecture C) reusing
  all SVC knowledge.

## Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| armfir depends on live kernel state (A2 false) | Medium | Plan-breaking for HLE | M2 gate + early exit; pivot defined; qemuPrime exists as fallback direction |
| Guest threads race on host-shared memory (PrimU2 model unsound at scale) | Medium | Heisenbugs | Start single-threaded (main thread only); serialize; deterministic scheduler in Phase 3 |
| SVC count/semantics drift between firmware versions | High (low harm) | Service table mismatch | Generate table from the pinned firmware's SDKLIB.DLL (M1), never hand-copy |
| Unicorn API/build friction on Fedora | Low (already solved) | Slowness only | M0 encodes the Phase 1 fixes in build scripts |
| Legal: Unicorn GPLv2 linking, firmware redistribution | Low | Distribution constraints | Document; link system unicorn; firmware never committed; lawyer review before any binary distribution |
| Fidelity illusions ("it boots") | Medium | False progress | Binary success criteria per milestone; all claims need logs/traces |
| Effort creep into GUI/kernel prematurely | High | Scope explosion | GUI is Phase 3; kernel is Architecture C, not Phase 2 |

## Dependencies

- Firmware image: user-supplied, downloaded from HP's channel (hpcalc mirror
  documented), gitignored. A `prime-firmware` self-test skips when absent.
- Upstream references remain read-only research material; no code reuse
  (GPL-2.0 PrimU2; see PRIMU2_SOURCE_AUDIT.md §11).
- Physical G1 remains the final oracle via existing `ppl-device`
  (unchanged safety rules from AGENTS.md).
