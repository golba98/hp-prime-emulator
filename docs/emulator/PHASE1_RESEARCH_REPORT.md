# Phase 1 Research Report

**Period:** 2026-10-23 · **Repository state:** branch `rust-rewrite`, clean
tree at start, no project code modified · **Deliverables:** this
`docs/emulator/` set · **Research material:** isolated, gitignored, under
`scratch/research/` (upstream checkouts, firmware image, validation project).

## 1. What was investigated

1. **Repository state** — the actual project root is
   `Development/4-Rust/3-HP Prime Emulator` (the CWD at task start was an
   unrelated C++ project and was not touched). Existing Rust workspace: 10
   crates implementing a PPL re-implementation, differential testing against
   a physical G1, and USB tooling — all preserved untouched. Note: this
   repository's `.gitignore` intentionally excludes `docs/**/*.md`
   (session-generated engineering docs); the new documents live on disk and
   are intentionally not committed, consistent with repo policy.
2. **PrimU2** (upstream commit `461d994`, 2026-06-19) — full source audit:
   executor, ELF/PE loaders, SVC dispatcher and marshalling, memory manager,
   threads/sync, interrupt & timer controllers, LCD/input, VFS, devices,
   Connectivity-Kit bridge, GDB stub, TCP automation, MCP server. See
   PRIMU2_SOURCE_AUDIT.md.
3. **Auxiliary projects** — qemuPrime (QEMU S3C2416 machine, abandoned 2021),
   ripem (3rd-party firmware; PRIME_OS.ROM format), hp-prime-kit (measured
   PPL/platform reference; Virtual Calculator automation; `.hpprgm` format),
   hpprime-pplcompile (container packer, explicitly not a compiler).
4. **Official firmware** — HP Prime G1 Firmware 2.4.2 (2026-09-09) obtained
   from HP's distribution channel (hpcalc.org mirror); full static analysis of
   PRIME_OS.ROM, PRIME_APP.DAT (FAT16), armfir.elf, SDKLIB/KRNLLIB DLLs. See
   FIRMWARE_ARCHITECTURE.md.
5. **PPL compiler question** — evidence from the firmware image, the 2013
   Virtual Calculator binary (local, symbols intact), and hp-prime-kit's
   measured `.hpprgm` format. See PPL_COMPILER_RESEARCH.md.
6. **Fedora feasibility** — toolchain inventory + isolated Unicorn/Capstone/
   goblin experiments. See FEDORA_COMPATIBILITY.md.
7. **Architecture comparison** — A (Rust HLE) / B (PrimU2 port) / C
   (full-system QEMU). See EMULATION_ARCHITECTURE.md.

## 2. What was verified (with evidence)

| Claim | Evidence |
|---|---|
| Service ID space is SDKLIB.DLL's export table: 742 services `0x10000..0x102E5` | SDKLIB.DLL export parse (742 exports) ≡ PrimU2 enum; armfir stub table at `0x307FBBEC` with `svc; push {r0}; push {lr}` |
| PRIME_OS.ROM loads at 0x30000000, entry +0x20, magic "V5J\0"+"2416" | header parse of 20260909 image; matches ripem `osrom2elf` |
| PRIME_APP.DAT is FAT16 at byte offset 8192 | boot-sector parse (OEM "BXC.OS.1") + mtools listing of the full disk tree |
| armfir.elf: ELF32 ARM EABI5, 1×RWX PT_LOAD at 0x30600000, entry 0x30604AE8, ~7.7 MiB | readelf + goblin (T-ELF-1) |
| ARM execution + SVC interception + shared-memory model work in Rust on Fedora | T-ARM-1 / T-SVC-1 / T-MEM-1 (all PASS, unicorn-engine 2.1.5 built from source on Fedora 44) |
| PrimU2 implements 85/742 services (48 HANDLE_IMPLEMENTED, 37 HANDLE_NAMEONLY) | `Services.cpp` registration audit (one grep hit was the macro `#define` itself) |
| `.hpprgm` compiled block is a symbol-table cache, not bytecode; calculator rebuilds it from source | hp-prime-kit measurements (G2 fw 2.4.15515), 367 KB block walk; source record authoritative (modified-block experiment) |
| hpprime-pplcompile does not parse PPL | its own README ("pure format converter") |
| The G1 SoC is S3C2416 (ARM926EJ-S) and the platform OS is WinCE 4.20-derived | OS ROM magic, DLL PE headers ("Windows CE 4.20"), qemuPrime machine, KRNLLIB `nand_*` exports |
| Two build constraints on Fedora | spaces-in-path breaks Unicorn CMake (reproduced); `-latomic` needed for link (reproduced, fixed) |

## 3. Tests executed

| ID | Test | Command | Environment | Expected | Actual | Status |
|---|---|---|---|---|---|---|
| T-ARM-1 | Execute hand-assembled ARM (`mov/add/bx lr`) on Unicorn | `cargo run --release` in isolated project (`/tmp/prime-validation`, deps `unicorn-engine 2.1.5`, `goblin 0.10`) | Fedora 44, rustc 1.96.0, GCC 16.2.1, RUSTFLAGS `-C link-arg=-latomic` | r0 = 72 | r0 = 72 | **PASS** |
| T-SVC-1 | SVC #0x10042 intercepted via UC_HOOK_INTR; PrimU2 return convention (LR from [sp], sp+=8, r0=result, PC=LR) | same binary | same | service id captured, r0=0x1234 | same | **PASS** |
| T-MEM-1 | Host `Vec<u8>` mapped via `mem_map_ptr`; guest↔host bidirectional writes | same binary | same | writes visible both directions | visible | **PASS** |
| T-ELF-1 | Parse real armfir.elf with goblin | same binary, arg = extracted armfir.elf | same | EM_ARM, entry 0x30604AE8, 1 PT_LOAD at 0x30600000 | matches readelf exactly | **PASS** |
| F-1 repro | Build same project under a path containing a space | original location `scratch/research/validation/...` | same | build succeeds | Unicorn CMake: "main directory cannot contain spaces" | **BLOCKED (documented; workaround: space-free build path)** |
| FA-1 | PRIME_APP.DAT FAT16 listing | `MTOOLS_SKIP_CHECK=1 mdir -i app.img ::/` | mtools 4.0.49 | 2013-wiki layout approximately holds | full tree listed; `MTOOLS_SKIP_CHECK` required (zero geometry fields) | **PASS** |
| FW-1 | armfir SVC stub scan | Python+struct scan + capstone disasm | python-capstone 5.0.7 | stubs `svc #0x10000+N` | 742 stubs × 12 bytes at 0x307FBBEC; naive word-scan false positives identified and excluded | **PASS** |

Two mid-test mistakes were made and corrected during T-ARM-1/T-SVC-1 (wrong
hand-assembled immediate/register encodings); final results above are from
the corrected, passing runs. No firmware code was executed on any physical
device; the physical G1 was not connected during Phase 1.

## 4. What remains unknown

1. Whether armfir boots with SVC interception alone on 20260909/20250915
   without live kernel state (M2 gate; PrimU2 is evidence on 20250915 only).
2. Which of the 742 services are *actually exercised* at runtime (needs boot
   tracing; the static scan cannot answer this).
3. The in-RAM representation of a compiled PPL program (only the on-disk
   constants cache is characterized).
4. Purpose of `PRIME_MASTER.DAT` and `armfir.dat`.
5. Real PCLK/timer semantics behind PrimU2's 38 kHz guess.
6. Whether the 21 unnamed service IDs (0x102D1–0x102E4) changed semantics
   across firmware versions.
7. Why firmware 2013→2026 shows the LCD interface as 32bpp under PrimU2 while
   the physical panel is 16bpp (host convention vs. firmware expectation).

## 5. Recommended Phase 2 architecture

**Architecture A: Rust HLE** (Unicorn + Rust host services), with
**Architecture C (full-system QEMU/S3C2416)** preserved as a separate
long-term track and re-evaluated after M4. Architecture B (porting PrimU2)
is rejected: GPL-2.0, non-compiling published tree, Windows-bound shell, and
its real value — the design knowledge — is now captured in these documents.
See EMULATION_ARCHITECTURE.md for the full comparison and the crate layout.

**First implementation milestone (Phase 2 M2):** boot the real `armfir.elf`
headlessly under Unicorn with the SVC dispatch layer and reach a *measurable
guest heartbeat* (event-loop polling), with every unimplemented service
logged and ranked — not a GUI, not a full calculator.

## 6. Documents created

- `docs/emulator/README.md` — index and conclusions
- `docs/emulator/PRIMU2_SOURCE_AUDIT.md` — deliverable A
- `docs/emulator/FEDORA_COMPATIBILITY.md` — deliverable B
- `docs/emulator/FIRMWARE_ARCHITECTURE.md` — deliverable C
- `docs/emulator/PPL_COMPILER_RESEARCH.md` — deliverable D
- `docs/emulator/SVC_COVERAGE.md` — deliverable E
- `docs/emulator/EMULATION_ARCHITECTURE.md` — deliverable F
- `docs/emulator/PHASE2_IMPLEMENTATION_PLAN.md` — deliverable G
- this file — deliverable H

Research artifacts (gitignored, `scratch/research/`): upstream checkouts,
firmware image + extracted components, `svc_coverage_full.tsv`,
`firmware_called_svcs.json`, validation project. No project code, tests,
build files, or tracked files were modified; no commits were made (per
AGENTS.md authorization rules).
