# Emulation Architecture Comparison and Rust Proposal

## 1. Candidate architectures

**A — Rust HLE (Unicorn + host services).** Rust host boots `armfir.elf` on
Unicorn, intercepts the 742 SDKLIB SVCs, and implements services on the host;
memory, threads, FAT16 disk, LCD, and input are host-side abstractions.
PrimU2's architecture, rebuilt cross-platform.

**B — Port/adapt PrimU2.** Build the C++ tree on Linux (fill in Win32
seams), or embed PrimU2 behind an integration layer.

**C — Full-system emulation.** Model the S3C2416 SoC (qemuPrime is an
existing QEMU fork with an S3C2416 machine: INTC, GPIO keyboard, NT11002
touch, NAND, RTC) and run the real boot chain: BXCBOOT → PRIME_OS.ROM
kernel → armfir.

## 2. Comparison

| Criterion | A: Rust HLE | B: PrimU2 port | C: Full-system (QEMU) |
|---|---|---|---|
| Implementation complexity | Medium — SVC surface is bounded (742, ~dozens needed for boot), services map to host APIs | Low start, unbounded end: missing `memory.h`, MSVC-isms, Chinese-comment code, no tests; every feature addition happens in a foreign GPL codebase | High — S3C2416 machine must be completed (qemuPrime is abandoned since 2021, old QEMU base), WinCE-derivative kernel debugging, NAND/bootloader chain |
| Fidelity | High for calculator behavior (real firmware code executes); scheduler/timing/power are approximations; SVC boundary is clean *today* but any kernel state read by armfir must be faked | Same as A (it *is* A) | Highest — kernel and boot chain are real; but the SDKLIB/DLL ABI still matters for apps |
| Firmware compatibility | Excellent for armfir.elf (verified format); kernel not executed | Same | Requires bootloader images not shipped in current packages (BXCBOOT0.BIN etc. are 2013-era) |
| Development effort to first boot | Weeks–months (see Phase 2 plan) | Similar to A in practice once porting friction, missing file, and Windows-only tooling are counted | Large; kernel-bring-up problems with no community (ripem payloads are the only precedent) |
| Maintenance burden | Ours; clean Rust; workspace-integrated | Tracking upstream + port maintenance; GPL contaminates our MIT tree on any code reuse | Tracking a private QEMU fork across QEMU releases |
| Portability | Linux/macOS/Windows from one codebase | Windows-first forever without the port work | Same as QEMU (excellent) |
| Performance | Unicorn TCG ≈ same engine class as QEMU TCG; SVC interception adds a hook per call — adequate (PrimU2 runs the calc interactively) | Same | Same engine; slightly fewer traps |
| Debugging | GDB stub (Unicorn-native), SVC log, memory/regs, deterministic replay potential (deterministic thread scheduling is achievable in Rust where PrimU2's native-thread model is not) | What PrimU2 has (good tools, Windows-bound UI) | QEMU gdb + tracing (gold standard) |
| Reverse-engineering required | SVC semantics per service (bounded, empirically testable), memory map (done), thread ABI (PrimU2 shows the shape) | Less RE, more archaeology | Device-level RE largely done by qemuPrime, but kernel bring-up is its own RE mountain |
| Legal | Unicorn GPLv2 linking (review), firmware never redistributed; MIT code stays clean | GPL-2.0 forces relicensing of reused parts | QEMU GPLv2; same linking consideration |
| Long-term kernel/hardware research | Not supported by design (no kernel) | Not supported | Supported (it *is* the hardware) |

## 3. What HLE cannot reproduce faithfully

Stated plainly: intercepting SVCs is **not** emulating the G1 kernel.

- No real scheduling: guest priorities, preemption, and timing behavior are
  approximations of the WinCE-derivative scheduler. Firmware that behaves
  differently under load is out of scope.
- No power management reality (battery, sleep/wake paths, clock scaling).
- No MMU/cache behavior (ARM926 caches are invisible to Unicorn HLE).
- Hardware interrupt latency/jitter is synthetic (host timers).
- Anything armfir does by reading kernel memory directly (if it does) must be
  discovered and faked — the one boundary PrimU2 has already had to fight
  (`0xee17ff7e` idiom, INTC MMIO at 0x4A000000).

## 4. Recommendation

**Architecture A (Rust HLE) for Phase 2**, with **C preserved as a separate
long-term objective** (revisit after M4: once the HLE boots, a QEMU-based
kernel path can reuse all the SVC knowledge for cross-validation — qemuPrime
remains the reference for the S3C2416 machine model).

Reject B: PrimU2's value is its *knowledge*, not its code — 86/742 services,
a non-compiling published tree, GPL-2.0, and a Windows-only shell make
porting strictly worse than a clean Rust reimplementation whose design is
now documented (PRIMU2_SOURCE_AUDIT.md).

## 5. Proposed Rust architecture

Workspace-adjacent crate tree (conceptual; the existing PPL-VM crates remain
untouched — the emulator is a new consumer of `ppl-compat`'s differential
harness, not a replacement for anything):

```
crates/
  prime-core/        # Emulator orchestration: boot sequence, config, event loop,
                     #  owns Memory + Machine; no UI, no I/O policy
  prime-arm/         # unicorn-engine FFI wrapper: engine lifecycle, hooks,
                     #  register/mem access, panic-safety, -latomic build config
  prime-firmware/    # PRIME_OS.ROM header, ELF loader (armfir), PE loader
                     #  (SDKLIB/KRNLLIB/COREDLL), firmware version pinning
  prime-memory/      # memory map, static blocks, dynamic heap (cookie model),
                     #  host↔guest translation, VPtr<T> accessors
  prime-services/    # SVC dispatch table (742), typed service bindings
                     #  (macro-based Marshal.h equivalent), handle tables
  prime-filesystem/  # FAT16 image mount (PRIME_APP.DAT), drive-letter VFS,
                     #  sandboxed host paths, loader file descriptors
  prime-display/     # framebuffer surface abstraction (32bpp + RGB565 view),
                     #  no windowing — frontends subscribe to frames
  prime-input/       # device keycode model, event queue, scripted injection
  prime-debug/       # GDB stub, SVC/trace logging, fault reports (capstone),
                     #  snapshot/deterministic-replay hooks
  prime-testing/     # headless runner, TCP automation server (key/touch/
                     #  screenshot), test harness bridging to ppl-compat
```

Justification of boundaries: the seams chosen are exactly the ones that let
(a) the core run headless with zero UI dependencies, (b) services be tested
against recorded traces without firmware, and (c) the automation surface
(TCP) match what PrimU2 proved works for driving PPL through real firmware.
Merging `prime-display`/`prime-input` into `prime-core` is viable later if the
seams prove thin; do not start with fewer than the FFI boundary
(`prime-arm`), which must exist regardless for soundness.

### FFI and safety notes (evidence: T-SVC-1/T-MEM-1 runs)

- `unicorn-engine` 2.1.5 wraps all hooks as `Rc`-backed closures; hooks must
  not touch the engine after a weak-ref upgrade failure (crash class the
  wrapper already handles); our layer must keep guest-memory host buffers
  alive for the engine's lifetime — enforce with a `Memory` owner that
  outlives all `Engine` handles (T-MEM-1 pattern).
- The PrimU2 native-thread-per-guest-thread model is a soundness hazard
  (unsynchronized shared host memory). Rust alternative for Phase 2: keep the
  model (it works; the firmware tolerates it), but serialize engine creation
  and pin a documented happens-before relationship per SVC that touches
  shared state. Deterministic cooperative scheduling is the Phase 3 upgrade
  path and is where a Rust reimplementation can exceed PrimU2.
- Build: `prime-arm` build script must add `-latomic` on Linux (F-2) and the
  workspace must handle the no-spaces path constraint (F-1) via documented
  `CARGO_TARGET_DIR` or by relocating the repository.

### Relationship to existing crates

- `ppl-math`/`ppl-syntax`/`ppl-compiler`/`ppl-vm`: unchanged; they remain the
  fast local oracle and IDE engine.
- `ppl-compat`: extended in Phase 2 with an "emulator backend" runner so the
  same differential cases run against (1) the local VM, (2) the firmware
  emulator, and (3) physical hardware — one comparison matrix, three
  engines.
- `ppl-device`: untouched; physical-device safety rules apply unchanged.
