# Fedora Linux Compatibility Report

Environment used (all evidence from this machine, 2026-10-23):

| Component | Detected version |
|---|---|
| OS | Fedora Linux 44 (x86-64) |
| rustc / cargo | 1.96.0 (2026-05-25); workspace `rust-version = 1.85` satisfied |
| GCC | 16.2.1 (Red Hat 16.2.1-2) |
| clang | 22.1.8 |
| Capstone | runtime rpm 5.0.6-4.fc44; Python bindings 5.0.7 (working) |
| Unicorn | **not installed**; rpm `unicorn`/`unicorn-devel` **2.1.4 available** in Fedora repos; `unicorn-engine-sys` 2.1.5 (bundled build) built from source successfully |
| mtools | 4.0.49 (FAT16 disk extraction verified with `MTOOLS_SKIP_CHECK=1`) |
| cmake / ninja | present (used by the Unicorn build) |
| qemu-system-arm | 10.2.2 available in repos (for future full-system work) |
| cross toolchain | `gcc-arm-linux-gnueabi` available (not needed by HLE; useful for test kernels) |

## 1. Why PrimU2 is Windows-only — the actual dependency inventory

| PrimU2 subsystem | Windows dependency | Category | Fedora replacement |
|---|---|---|---|
| `PrimU.cpp` entry | hard-coded `prime_data\A\...` backslash path | (2) minor | path abstraction |
| Sync primitives | `Win32SyncPrimitives.cpp`: `CreateEvent`/`CreateSemaphore`/`CRITICAL_SECTION`/`Sleep`/`SwitchToThread` | (2) minor | `std::sync`/`parking_lot`; the `ISyncFactory` seam already exists |
| `executable.cpp` | `<windows.h>` PE structures, `memcpy_s` | (2) minor | `goblin`/`object` PE types; `memcpy_s` → checked copy |
| `PELoader.cpp` | `<windows.h>` IMAGE_* headers | (2) minor | pure PE parse (no loader behavior change) |
| `LCD.cpp` display | Win32 window class, `WM_*` message loop, GDI `BitBlt`, `SetTimer` | (3) replace | framebuffer surface + `winit`/`softbuffer` (already used by `ppl-gui`) or `egui` |
| Keyboard/touch | `WM_KEYDOWN`/`WM_LBUTTONDOWN` VK mapping | (2) minor | keymap is data; feed from winit or scripted input |
| `HpInterOp.cpp` | Named Pipes (`CreateNamedPipe`), `HANDLE` | (3) replace | Unix domain socket / TCP; the protocol is the HP USB packet framing, transport-independent |
| `CommandServer.cpp` | Winsock (`WSAStartup`, `SOCKET`, `closesocket`) | (2) minor | POSIX sockets, API is the same |
| UI toolkit | Dear ImGui + SDL2 (vendored) | (4) not needed for milestone | headless core first; any UI later |
| Bundled `lib/*.dll` | unicorn.dll, capstone.dll, MinGW runtime DLLs | (2) minor | system unicorn/capstone or Rust crates that build from source |
| MSVC project | `PrimU.sln`/`.vcxproj`, `packages.config` | (4) | not ported at all (Rust rewrite) |
| `QuickTransfer` | C#/.NET, named pipes | (4) for milestone | our existing `ppl-device` Rust USB stack + future emulator bridge |
| `GB2UTF8.ps1` | PowerShell | (4) | n/a |
| **Missing file** | `memory.h` included by `executor.cpp`/`dbgout.cpp` but absent from repo | (1) blocker | does not compile as published — moot for the Rust path |
| Unicode conversion | `std::codecvt` (deprecated; removed in C++26) | (2) | Rust `String`/`WString` conversions are trivial |

Categories: (1) already portable, (2) portable with minor adaptation,
(3) requires substantial replacement, (4) not necessary for the initial
emulator milestone. The core emulation logic — executor, SVC dispatcher,
`Marshal.h`, MemoryManager, svc_* handlers, Thread model,
Interrupt/TimerControllers, GdbStub — is portable C++ with STL only.

**Conclusion: nothing in PrimU2's emulation core is fundamentally
Windows-bound.** The Windows-ness is concentrated in the UI shell, the
Connectivity-Kit pipe bridge, and the build system.

## 2. Unicorn on Fedora — validation results (tests actually run)

Isolated project: `scratch/research/validation/prime-phase1-validation/`
(copied to `/tmp/prime-validation` to run, because of finding F-1 below).
Dependencies: `unicorn-engine = "2.1.5"`, `goblin = "0.10"`.

| Test | What | Result |
|---|---|---|
| T-ARM-1 | Hand-assembled ARM (`mov r0,#65; add r0,#7; bx lr`) executed on Unicorn 2.x built from source; register verified (r0 = 72) | **PASS** |
| T-SVC-1 | `push {lr}; svc #0x10042; pop {pc}` with a `UC_HOOK_INTR` hook implementing PrimU2's exact dispatch convention (SVC from `*(pc-4) & 0xFFFFF`, return addr from `[sp]`, `sp += 8`, result in `r0`) | **PASS** |
| T-MEM-1 | Host `Vec<u8>` mapped into guest via `mem_map_ptr`; bidirectional read/write visibility verified | **PASS** |
| T-ELF-1 | Real firmware `armfir.elf` parsed with `goblin`: EM_ARM, entry 0x30604AE8, 1 PT_LOAD at 0x30600000 (matches readelf) | **PASS** |

### Constraints discovered (must-fix for Phase 2)

1. **F-1: Unicorn's CMake refuses paths containing spaces.** Building under
   `.../3-HP Prime Emulator/...` fails with `ERROR: main directory cannot
   contain spaces nor colons`. Our repository path contains a space. Phase 2
   options: (a) relocate the repo, (b) set `CARGO_TARGET_DIR` to a
   space-free path, (c) build Unicorn once out-of-tree and link the system
   library. Any CI must account for this.
2. **F-2: link with `-latomic`.** `unicorn-engine-sys` 2.1.5 pulls QEMU TCG
   helpers that reference `__atomic_*_16`; Fedora's toolchain + rust-lld does
   not automatically link libatomic. `RUSTFLAGS="-C link-arg=-latomic"`
   resolved it (verified). Build scripts must set this (or link the Fedora
   `unicorn` rpm and use its `-lunicorn` instead).
3. **F-3: no ARM disassembler by default.** Fedora binutils lacks ARM
   support and llvm-objdump is absent; Capstone (python 5.0.7) worked for our
   static analysis, and `capstone` rust crate or `llvm-tools` should be part
   of the Phase 2 dev toolchain.

## 3. Library feasibility on Fedora (verified or evidence-based)

| Need | Choice | Status |
|---|---|---|
| ARM execution | `unicorn-engine` 2.1.5 crate (builds Unicorn 2.x from source) or Fedora `unicorn` 2.1.4 rpm | **verified working** |
| Disassembly (fault reports) | Capstone (`capstone` crate; system 5.0.6) | python bindings verified on armfir.elf |
| ELF/PE parsing | `goblin` 0.10 | **verified on armfir.elf**; also parses PE (SDKLIB export table) |
| FAT16 disk access | mtools (verified) / pure-Rust reader for the emulator VFS | verified via mtools; Rust side is straightforward (FAT16, small disk) |
| Sockets (automation, GDB) | std::net | trivially portable (PrimU2's Winsock usage is 1:1) |
| Threading | std::thread + channels | the PrimU2 model ports directly |
| UI (later phase) | `winit`/`softbuffer` (already in workspace for `ppl-gui`) or `eframe` | existing precedent in this repo |

## 4. Legal and distribution constraints (Fedora packaging view)

- Unicorn: GPLv2 — fine to *link* (LGPL-style exception? No: Unicorn is
  GPLv2 with a linking exception? **Unicorn Engine is GPLv2**; dynamic
  linking from an MIT Rust host is the model used by many projects but the
  licensing of the combined work must be reviewed by a lawyer before any
  distribution. Vendoring Unicorn in-repo is not required on Fedora since
  the distro ships it.)
- Capstone: BSD-3 — no issue.
- goblin: MIT — no issue.
- Firmware images: proprietary HP; never committed, never redistributed;
  users obtain their own copies from HP's channel (documented in
  FIRMWARE_ARCHITECTURE.md).
- PrimU2: GPL-2.0 — reference only, no code reuse in the MIT project (see
  PRIMU2_SOURCE_AUDIT.md §11).
