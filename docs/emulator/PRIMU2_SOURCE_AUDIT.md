# PrimU2 Source Audit

Upstream: https://github.com/telecomadm1145/PrimeU2 — commit `461d994`
(2026-06-19, "Updated QuickTransfer. MCP"), GPL-2.0. Isolated checkout at
`scratch/research/PrimeU2/`. This document describes what the source actually
does, not what the README claims. Deviations between README and code are
flagged.

PrimU2 is an MSVC C++ Win32 console application that boots the HP Prime G1
firmware's `armfir.elf` on the Unicorn Engine, intercepts the firmware's
system-service calls (SVC), and services them with host implementations.
No hardware is emulated below the SVC/MMIO abstraction; the boot ROM, NAND,
USB controller, and the kernel itself are not executed.

## 1. Process bootstrap

`PrimU/PrimU.cpp` (`main`):

1. `SVCNameRegistry::LoadFromFile("syscalls_sdk.json")` — service-id → name
   map used for logging.
2. `Executable exec("prime_data\\A\\programs\\misc\\armfir.elf")` — the
   firmware path is **hard-coded with a Windows backslash path**.
3. `sExecutor->Initialize(&exec)` → `sExecutor->Execute()` — an infinite loop;
   `Cleanup()` is unreachable in practice.

`ISyncFactory` is an abstraction seam (`SyncPrimitives.h`) with the only
concrete implementation `Win32SyncFactory` (`Win32SyncPrimitives.cpp`) — the
codebase was already structured for porting, but no non-Win32 factory exists.

**Build-blocker:** `executor.cpp` and `dbgout.cpp` `#include "memory.h"`,
which is absent from the repository (only `MemoryManager/Block/Chunk` are
checked in). The tree **cannot compile as published**; the missing file is
small but its absence means the published snapshot is not a reproducible
build.

## 2. Firmware loading sequence (`executable.cpp` — `Executable::Load`)

Verified order of operations:

1. `.\PRIME_OS.ROM` is read from the working directory and mapped via
   `sMemoryManager->StaticAlloc(0x30000000, size, UC_PROT_ALL)` with a raw
   `memcpy` of the whole 1 MiB image (header included). The kernel is mapped
   **but never entered**.
2. The ELF is parsed with ELFIO (`include/elfio/`). Requirements:
   `ELFCLASS32`, `EM_ARM`.
3. For each `PT_LOAD` segment: `StaticAlloc(vaddr, size, prot)` where
   - `size` is **hard-coded to 16 MiB per segment** (comment in source admits
     this; real segments are ~7.7 MiB total),
   - `prot` is derived from `PF_R/W/X`,
   - the whole region is zeroed, then `memcpy_s` of `p_filesz` bytes
     (correct bss behavior).
4. `_entry = reader.get_entry()`; state `EXEC_LOADED`.
5. If the file is not an ELF, a PE32 fallback loads it through the WinSDK PE
   structures with system directory `.\prime_data\A\WINDOW\SYSTEM`.

The main-thread SP is the top of a 4 MiB region at `0x10000000`
(`MEM_STACK`), allocated from the dynamic heap in `Thread::Thread`.
There is **no reset vector, no bootloader, no kernel entry**: execution
starts directly at the ELF entry point with `r0 = 0`, `SP = stack top`,
and one Unicorn `uc_emu_start` loop.

### Static memory map (from `common.h`, `MemoryManager.h`, `Executor::Initialize`)

| Region | Address | Size | Notes |
|---|---|---|---|
| Null/trap page | `0x00000000` | `0x10` | prot 3 (RW) |
| Main stack | `0x10000000` | 4 MiB | `MEM_STACK` |
| (declared dynamic) | `0x10400000` | — | `MEM_DYNAMIC`, unused by heap impl |
| Dynamic heap | `0x20000000` | `0x10000000` | **256 MiB** (header comment wrongly says "32MB") |
| Kernel ROM | `0x30000000` | 1 MiB | `PRIME_OS.ROM`, RWX |
| Vector/exception page | `0x31FFF000` | 4 KiB | prot 3 |
| armfir PT_LOAD | `0x30600000` | 16 MiB alloc | real segment ~7.7 MiB, RWX |
| S3C INTC | `0x4A000000` | — | SRCPND/INTPND written directly by guest ISRs |
| LCD register block | `0x4C800000` | `0x4E000000-0x4C800000` | `LCD_REGISTER` |
| S3C2416 PWM timers | `0x51000000` | `0x44` | TCFG0..TCNTO4 |
| RTC | `0x57000000` | `0x100` | `RTC_REGISTER` |
| PE images | scanned | 1 MiB steps | `AllocateAny` scans `0x10000000..0x70000000` |

### Dynamic heap (`MemoryManager.cpp`)

Pre-mapped 256 MiB host block; sub-allocation in host memory with a
16-byte alignment, **cookie canaries** (prefix+suffix `kHeapCookie`),
an address-ordered free list with coalescing, in-place realloc when the next
block is free, and `HeapAllocCount` instrumentation. No guest-observed OOM
semantics were identified beyond returning 0. This design (pre-mapped block +
host-side bump allocator) is directly reproducible in Rust and is a good
model: it avoids `uc_mem_map` churn per allocation.

## 3. ARM execution engine (`executor.cpp/h`, `Thread.cpp`)

- The Executor singleton opens one `uc_engine` with
  `uc_open(UC_ARCH_ARM, UC_MODE_ARM)` — but **every guest thread opens its own
  engine with `UC_MODE_THUMB`** (`Thread::Thread`). The main engine is
  effectively only the registration template.
- `Executor::Execute()`:
  1. Starts the GDB stub (port 2345; `ENABLE_GDB_STUB` is **#defined by
     default**, plus `PRIMU_GDB_PORT` env override) — the README's
     "External Debugger — Not Yet Implemented" is **outdated**; `GdbStub.cpp`
     (522 lines) implements a real remote-serial stub with per-thread
     breakpoints and all-stop.
  2. `sInterruptController->Start()` — ISR dispatch thread.
  3. `sThreadHandler->NewThread(entry, 0, THREAD_PRIORITY_NORMAL, 4 MiB)`.
  4. `CommandServer::Start(4321)` — TCP automation server.
  5. Sleeps forever.
- `RegisterHooksForEngine` installs, per engine: `SvcDispatcher` (INTR hook),
  `FaultHandler` (mem-invalid hook), `TimerController` (MMIO write hook).

### Per-thread execution loop (`Thread::ThreadProc`)

- Each guest thread is a native `std::thread` owning a `uc_engine` (THUMB
  mode). Guest memory is shared by mapping **host buffers** into each engine
  with `uc_mem_map_ptr(block->GetVAddr(), size, UC_PROT_ALL, hostPtr)` —
  one mapping per `MemoryBlock`. Guest VA == host pointer translation is done
  by the `MemoryManager` (`__GET` / `__ADDR` macros), *not* by Unicorn.
- Registers: `r0 = arg`, `SP = stack top`, `PC = start`.
- Loop: `uc_emu_start(pc, 0, 0, count)`. On `UC_ERR_INSN_INVALID`, the
  instruction word is checked against the constant `0xee17ff7e`; if equal, the
  Z flag is forced (`cpsr |= 1<<30`), `pc += 4`, continue. This is an
  **undocumented firmware idiom workaround** (an `MCR p15` cache-op word the
  firmware uses; treated as a no-op that sets Z). Any other Unicorn error
  terminates the thread silently (`break`).
- `Thread::ExecuteCustomCode(pc, args...)` runs arbitrary guest code on a
  thread's engine from host context with `LR = 0xDEADC0DE` sentinel; return is
  detected by faulting at the sentinel address. Used by `InterruptController`
  to run guest ISRs on demand.

### Threading assessment

- **Native-thread = guest-thread.** `OSSetThreadPriority` only stores a value;
  there is no scheduler, no preemption, no timeslicing. Firmware spin-wait and
  priority patterns work only by accident of host scheduling.
- Race-condition exposure: multiple engines share raw host memory with no
  memory model; guest atomicity (`SWP`, `LDREX/STREX` on ARMv5 absent — S3C2416
  is ARMv5TEJ, so no exclusives; spinlocks use the `0xee17ff7e` idiom above)
  is not modeled.
- `ThreadManager` tracks threads in an unordered map keyed by an atomic
  incrementing id; `currentThread` is a `thread_local` back-pointer.
- Guest-visible thread descriptors: `OSCreateThread` allocates a 0x54-byte
  descriptor in the guest heap with a `THREAD_MAGIC`, function pointer at
  +0x18, internal tid at +0x14 — i.e. PrimU2 *invents* a guest ABI for thread
  objects that the firmware already has its own layout for; this was reverse
  engineered to match the firmware's expectations.

### Interrupt and timer model

- `InterruptController` (modeled after S3C2440 INTC): registry
  `irq → guest ISR pointer`, a pending-IRQ queue, and a dedicated dispatch
  thread that executes the ISR via `ExecuteCustomCode(isr, irq)` with the IRQ
  number in `r0`. `AcknowledgeIRQ` is a no-op because guest ISRs clear the
  MMIO registers at `0x4A000000/0x4A000010` directly.
- `TimerController`: models the S3C2416 PWM block at `0x51000000` (5 timers,
  IRQs 10–14), with a host thread per running timer; **`PCLK` is assumed
  38,000 Hz (source comment: "TODO: ???")** — the real S3C2416 PCLK is tens of
  MHz; PrimU2 deliberately slows timers so firmware timeout loops do not
  spin the host.
- The LCD window's `WM_TIMER` (60 FPS) decrements `TCNTO4`
  (`*(0x51000040) -= 10000/60`) and enqueues synthetic
  `UI_EVENT_TYPE_SYS_TIMER` / `DEVICE_TIMER` events — the **display thread
  drives timer ticks**, an ordering coupling that would be a determinism
  hazard in a headless port.

## 4. SVC dispatch (`SvcDispatcher.cpp`, `Marshal.h`, `Services.cpp`)

The firmware calls OS services through per-export stubs inside `armfir.elf`
(verified in the 20260909 image at VA `0x307FBBEC`):

```
svc #0x10000+N ; stmdb sp!, {r0} ; stmdb sp!, {lr}
```

`svc_hook` (a `UC_HOOK_INTR` callback) therefore:

1. Reads `r0..r3, sp, pc`; `SVC = *(u32*)(pc-4) & 0xFFFFF`;
2. `lr = *(u32*)sp` (the pushed return address);
3. Builds `SystemServiceArguments{r0..r3, sp, caller_pc = lr-4}`;
4. Looks up `service_table[SVC - 0x10000]` (an `std::array` of 742
   function pointers); out-of-range or null handler ⇒ logs and returns 0;
5. Returns by writing `r0 = result`, `sp += 8`, `pc = lr`.

- Declared service range: `0x10000..0x102E5` = **742 IDs** (`interrupts.h`),
  matching the 742 exports of `SDKLIB.DLL` in the firmware image (see
  [SVC_COVERAGE.md](SVC_COVERAGE.md)).
- **85 handlers are registered** (`Services.cpp`; 48 tagged HANDLE_IMPLEMENTED, 37 HANDLE_NAMEONLY) ≈ 11.5% coverage.
- `Marshal.h` is the strongest engineering artifact in the codebase: a
  template metaprogramming binding layer (`Reg<N>`, `Stack<Off>`, `VPtr`,
  `VRef`, `VPtrDeref`, `GuestVal<T>` multi-level pointers, `GuestMarshal`
  custom conversions, `HandleTable<T, Domain>` with domain-tagged handles
  `File=0xDEAD0000 / Find=0xF14D0000 / Device=0xDEEF0000 /
  Event=0xE0EE0000 / CritSec=0xCC550000 / Generic=0xAABB0000`).
  `AutoBind<fn>::thunk<fn>` derives argument extraction from the C++ handler
  signature. A Rust equivalent is naturally expressible with declarative
  table macros.

## 5. Memory, sync, and thread services

- `lcalloc/lmalloc/lrealloc/_lfree` map to the `MemoryManager` heap.
- Sync services (`svc_thread.cpp`) create **host** primitives through
  `g_SyncFactory` (Win32 events/semaphores/critical sections), with
  magic-tagged guest descriptors; waits block the native thread, timeouts map
  1:1. Message queues are host mutex+condvar queues.
- Consequence: guest priority inversion and nested-wait semantics of the
  real OS are not modeled; a deadlock the real device would resolve by
  priority scheduling will hang PrimU2.

## 6. Display and input (`LCD.cpp/h`, `ui.h`, `svc_system.cpp`)

- `GetActiveLCD` returns the guest VA of a host-allocated `LCD` object
  ("BLIGLCD" layout in `LCD.h`): `LCD_MAGIC{x_res=320, y_res=240,
  pixel_bits=32, brightness, window1_bufferstart}` plus a
  `uint16_t buffer[320*240*3]` used as a **32-bit ARGB** framebuffer
  (initialized `0xFF000000`). **The README's "RGB565" claim is wrong** — the
  emulated panel is 32bpp; the real G1 panel is 16bpp RGB565, so PrimU2's LCD
  object is a host-side simplification the firmware evidently tolerates.
- Rendering: a Win32 window per LCD (`WindowThreadProc`), 60 FPS `WM_TIMER`
  repaint, brightness scaling, GDI `BitBlt`; a separate ImGui/SDL2 window
  provides the keypad (`Prime_compact.bmp`), SVC log, memory editor.
- Keyboard: `WM_KEYDOWN` → `vk_to_device_keymap` (VK → device keycodes, e.g.
  `Esc=0x01`, `Apps=0xB1`, `Home=0xe047`) → `EnqueueEvent(UIMultipressEvent)`.
  Touch: `WM_LBUTTONDOWN/MOUSEMOVE/UP` → `TouchUpdate(x,y,finger,status)`
  into 8 `touch_states`.
- `GetEvent` (the firmware's event pump) **blocks the guest thread** in a
  native 50 ms-sleep poll loop over the host queues (`goto restart`). Guest
  UI responsiveness is host-pacing-dependent; the guest never sees a queue
  timeout because the handler only returns when it has an event.

## 7. Filesystem (`svc_common.cpp/h`, `svc_filesystem.cpp`)

- `VMPath`: drives `A:`..`Z:` map to `prime_data\<drive>\`; 26 per-drive CWDs
  with a current-drive char. Drive-absolute paths (`X:\...`) are
  **concatenated without lexical normalization** — `A:\..\..\..` escapes the
  sandbox (path-traversal hazard; relative paths do get
  `lexically_normal()`). No host-escape guard exists.
- Unicode: `std::wstring_convert<codecvt_utf8_utf16>` (deprecated; removed in
  C++26) — a porting hazard.
- `VFileSystem`: mutex-guarded `FILE*` handle table (handles from
  `0xDEAD0001`), open/read/write/seek/tell/size, `with(handle, fn)` legacy
  escape hatch.
- Loader-level files: `_OpenFile/_OpenSubFile/_ReadFile/_FseekFile/_FileSize/
  _CloseFile` over a guest-visible 0x20-byte `loader_file_descriptor_t` with
  sub-file windows (base + offset within a parent) — the mechanism the
  firmware uses to read resources out of container files.
- Also implemented: INI get/set, `findfirst/findnext/findclose` with DOS
  wildcards, `afnsplit/afnmerge`, ANSI+Unicode variants.

## 8. Devices and connectivity (`svc_system.cpp`, `HpInterOp.cpp`, `QuickTransfer/`)

- `CreateFile` registers virtual devices by name suffix; `DeviceIoControl`
  dispatches:
  - `BAT` — battery level 4/4, ADC 1000 (~3.42 V);
  - `ARCH` — requests 258/259 reboot(+bootloader)/260 hardware type
    (VerA/VerC — the two G1 hardware revisions);
  - `FACTORY` — FRP mount/read/write/erase, persisted to `frp_data.bin`;
  - `USB` / `USBHOST` — S3C2416 device/host controller IOCTLs bridged to
    function pointers `usb_in_cb` / `usb_out_cb`.
- `HpInterOp.cpp`: a Windows **Named Pipe** server that connects those
  callbacks to the official HP Connectivity Kit, so the real CK sees the
  emulator as a calculator over "USB".
- `QuickTransfer` (C#): a named-pipe client speaking the HP USB protocol
  (64-byte packets, 60-byte payloads; `PING 0x01`, `READ_FILE 0x10`,
  `WRITE_FILE 0x11`, `LIST_DIR 0x12`, `DELETE 0x13`, `MKDIR 0x14`,
  `FILE_INFO 0x15`, `READ_MEM 0x20`, `WRITE_MEM 0x21`, `LOG_READ 0x30`,
  `KEY_INJECT 0x40`) — file injection and key automation into the emulator.
- `mcp_server.py`: an MCP (LLM-tooling) server wrapping the TCP command
  interface (key names → device keycodes, touch, zlib/base64 screenshots).

## 9. Debugging and automation (verified present)

| Mechanism | Evidence |
|---|---|
| SVC call log | `RollingLogBuffer<SVCCallRecord> g_svcCallLog(1024)` + ImGui window (`svclogwin.h`) |
| GDB remote stub | `GdbStub.cpp`, port 2345, all-stop, per-thread breakpoints, `WaitForDebugger` on thread 0; enabled by default |
| Fault reporting | `FaultHandler.cpp`: register dump + Capstone disassembly + FP-chain stack trace (max 64 frames) on `UC_HOOK_MEM_INVALID` |
| TCP command server | `CommandServer.cpp` port 4321: `key <code> <down|up|press>`, `touch <x> <y> <down|move|up>`, `screenshot` (BMP32: 4-byte BE size prefix, headers, raw ARGB), `state` |
| Guest printf | `dbgMsg`/`WriteComDebugMsg` SVC + `ee_vsprintf` custom vsprintf over guest memory (`vprintf.cpp`) |
| Memory editor | `imgui_memory_editor.h` windows in the SDL2 UI |
| MCP integration | `mcp_server.py` |

The *documented* "Not Yet Implemented" items that are actually absent from
the code: SDKLIB GDI drawing APIs, battery detail APIs, USB OTG beyond the
bridge, and the scheduler itself.

## 10. What does *not* exist in PrimU2

- No kernel execution (PRIME_OS.ROM is mapped but never entered), no NAND, no
  bootloader, no USB descriptor-level emulation (the bridge reuses HP's CK).
- No CE DLL loader beyond `PELoader`: `LoadLibraryA` maps sections and
  resolves imports against already-loaded images with **return-zero Thumb
  stubs for unresolved imports** (`CreateReturnZeroStub`) — silent
  under-implementation of any DLL API the firmware calls.
- No persistence of emulated state other than `prime_data` + `frp_data.bin`.
- No tests, no CI, no build outside MSVC; Chinese UTF-8 comments throughout;
  `GB2UTF8.ps1` hints at author-side encoding fixes.

## 11. Licensing

GPL-2.0. Any direct code reuse in our MIT-licensed Rust project would force
GPL-2.0 on the reused parts. **Recommendation: treat PrimU2 strictly as
reference/inspiration; reimplement in Rust from the behavior spec, do not
translate code.** (Consult a lawyer before any exception; none is needed for
the recommended path.)
