# SVC Coverage Report

System-service inventory for PrimU2 at commit `461d994`, cross-checked
against the official 20260909 firmware image. A **declared** service ID does
not mean the service is implemented: coverage here is derived from source
code (`PrimU/Services.cpp` registration calls), not from README claims.

## 1. The service ID space

- Service IDs are `0x10000..0x102E5` — **742 services**
  (`PrimU/interrupts.h`), which is exactly the export ordinal space of the
  firmware's `SDKLIB.DLL` (742 named exports, verified by parsing the DLL in
  `PRIME_APP.DAT`). The SVC immediate is `0x10000 + export ordinal - 1`,
  read by the dispatcher as `*(pc-4) & 0xFFFFF`.
- `PrimU/syscalls_sdk.json` names 721 of the 742; IDs `0x102D1..0x102E4`
  exist in the enum and the DLL but carry no name in the JSON (logging shows
  them raw).
- armfir.elf reaches every service through a 742-entry stub table at VA
  `0x307FBBEC` (`svc #id; push {r0}; push {lr}`) — all 742 are *reachable*;
  which are *used* at runtime is a trace question, not a static one.

## 2. Coverage summary

| Metric | Count |
|---|---|
| Declared service IDs | 742 |
| Named in `syscalls_sdk.json` | 721 |
| Handlers registered in PrimU2 | **85** (11.5%) |
| Unimplemented at runtime (return 0 + log) | 657 |

A Rust emulator must treat the 86 as the *known-minimum* set, then grow the
set empirically: run the firmware, log `Unimplemented SVC`, implement in
order of boot-blocking.

## 3. Implemented handlers (85, from `Services.cpp`, status from code reading)

Status legend: **Complete** — behavior plausibly matches the platform;
**Partial** — works for the firmware's main path but with known gaps or
invented data; **Stub** — presence-only (logs, returns a constant, or exits).

| `0x10000` | OSCreateThread | Thread/Sync/MsgQue | Complete |
| `0x10001` | OSTerminateThread | Thread/Sync/MsgQue | Complete |
| `0x10002` | OSSetThreadPriority | Thread/Sync/MsgQue | Complete |
| `0x10003` | OSGetThreadPriority | Thread/Sync/MsgQue | Complete |
| `0x10004` | OSSuspendThread | Thread/Sync/MsgQue | Complete |
| `0x10005` | OSResumeThread | Thread/Sync/MsgQue | Complete |
| `0x10006` | OSWakeUpThread | Thread/Sync/MsgQue | Complete |
| `0x10007` | OSExitThread | Thread/Sync/MsgQue | Complete |
| `0x10008` | OSSleep | Thread/Sync/MsgQue | Complete |
| `0x10009` | OSCreateSemaphore | Thread/Sync/MsgQue | Complete |
| `0x1000A` | OSWaitForSemaphore | Thread/Sync/MsgQue | Complete |
| `0x1000B` | OSReleaseSemaphore | Thread/Sync/MsgQue | Complete |
| `0x1000C` | OSCloseSemaphore | Thread/Sync/MsgQue | Complete |
| `0x1000D` | OSCreateEvent | Thread/Sync/MsgQue | Complete |
| `0x1000E` | OSWaitForEvent | Thread/Sync/MsgQue | Complete |
| `0x1000F` | OSSetEvent | Thread/Sync/MsgQue | Complete |
| `0x10010` | OSResetEvent | Thread/Sync/MsgQue | Complete |
| `0x10011` | OSCloseEvent | Thread/Sync/MsgQue | Complete |
| `0x10012` | OSInitCriticalSection | Thread/Sync/MsgQue | Complete |
| `0x10013` | OSEnterCriticalSection | Thread/Sync/MsgQue | Complete |
| `0x10014` | OSLeaveCriticalSection | Thread/Sync/MsgQue | Complete |
| `0x10015` | OSDeleteCriticalSection | Thread/Sync/MsgQue | Complete |
| `0x10018` | OSCreateMsgQue | Thread/Sync/MsgQue | Complete |
| `0x10019` | OSPostMsgQue | Thread/Sync/MsgQue | Complete |
| `0x1001A` | OSSendMsgQue | Thread/Sync/MsgQue | Complete |
| `0x1001B` | OSPeekMsgQue | Thread/Sync/MsgQue | Complete |
| `0x1001C` | OSGetMsgQue | Thread/Sync/MsgQue | Complete |
| `0x1001D` | OSCloseMsgQue | Thread/Sync/MsgQue | Complete |
| `0x1001E` | InterruptInitialize | Interrupt | Partial |
| `0x1001F` | InterruptMask | Interrupt | Stub |
| `0x10020` | InterruptDisable | Interrupt | Partial |
| `0x10021` | InterruptDone | Interrupt | Partial |
| `0x10032` | SysPowerOff | Time/Power | Stub |
| `0x10036` | BatteryLowCheck | Time/Power | Stub |
| `0x10029` | LCDOn | LCD | Stub |
| `0x1008D` | GetActiveLCD | LCD | Complete |
| `0x10037` | lmalloc | Memory | Complete |
| `0x10038` | lcalloc | Memory | Complete |
| `0x10039` | lrealloc | Memory | Complete |
| `0x1003A` | lfree | Memory | Complete |
| `0x1003F` | GetEvent | Event/input | Partial |
| `0x1004C` | SetSystemVariable | Other | Partial |
| `0x100A5` | GetSysTime | Time/Power | Partial |
| `0x100C1` | GetMasterIDInfo | Other | Stub |
| `0x100CA` | fclose | Filesystem | Complete |
| `0x100CB` | filesize | File (loader fd) | Complete |
| `0x100CF` | fseek | Filesystem | Complete |
| `0x100D0` | ftell | Filesystem | Complete |
| `0x100D4` | fread | Filesystem | Complete |
| `0x100D7` | fwrite | Filesystem | Complete |
| `0x100D8` | afindfirst | Filesystem | Complete |
| `0x100D9` | afindnext | Filesystem | Complete |
| `0x100DA` | findclose | Filesystem | Complete |
| `0x100DD` | aremove | Filesystem | Complete |
| `0x100E0` | amkdir | Filesystem | Complete |
| `0x100E2` | achdir | Filesystem | Complete |
| `0x10111` | GetPrivateProfileString | File (loader fd) | Complete |
| `0x10112` | WritePrivateProfileString | File (loader fd) | Complete |
| `0x100C5` | afnsplit | Filesystem | Complete |
| `0x100C6` | afnmerge | Filesystem | Complete |
| `0x100EB` | FSGetDiskRoomState | Filesystem | Stub |
| `0x100EC` | OpenFile | File (loader fd) | Complete |
| `0x100EF` | CloseFile | File (loader fd) | Complete |
| `0x100F0` | ReadFile | File (loader fd) | Complete |
| `0x100F1` | FseekFile | File (loader fd) | Complete |
| `0x100F2` | FileSize | File (loader fd) | Complete |
| `0x100F3` | OpenSubFile | File (loader fd) | Complete |
| `0x10119` | GetCurrentPathA | Filesystem | Partial |
| `0x1011A` | ProgramIsRunningA | Loader/PE | Complete |
| `0x10283` | ProgramIsRunningW | Loader/PE | Partial |
| `0x10286` | LoadLibraryA | Loader/PE | Complete |
| `0x10287` | GetModuleFileNameA | Loader/PE | Complete |
| `0x10293` | FindResourceW | Loader/PE | Complete |
| `0x10297` | FreeLibrary | Loader/PE | Complete |
| `0x102A7` | CreateFile | File (loader fd) | Stub |
| `0x102AC` | DeviceIoControl | Other | Partial |
| `0x102AD` | CloseHandle | Other | Partial |
| `0x1026F` | wfopen | Filesystem | Complete |
| `0x10270` | wfindfirst | Filesystem | Complete |
| `0x10271` | wfindnext | Filesystem | Complete |
| `0x10274` | wremove | Filesystem | Complete |
| `0x10277` | wmkdir | Filesystem | Complete |
| `0x10278` | wrmdir | Filesystem | Complete |
| `0x10279` | wchdir | Filesystem | Complete |
| `0x102A1` | WriteComDebugMsg | Debug | Complete |

Notes on the classifications:

- `GetEvent` is Partial: it *blocks* the guest in a host poll loop instead of
  modeling the OS event queue/timing; `GetSysTime` maps host wall-clock
  (timezone-sensitive); `GetMasterIDInfo` returns invented identity strings;
  `SysPowerOff` calls host `ExitProcess`.
- `CreateFile`/`DeviceIoControl`/`CloseHandle` are the virtual-device layer
  (`BAT`, `ARCH`, `FACTORY`, `USB`, `USBHOST`, `INDICATOR`); unknown devices
  return `0xFF`-filled buffers — Partial by construction.
- The PE/loader group (`_LoadLibraryA` etc.) is Partial: unresolved imports
  become return-zero Thumb stubs, so any DLL function the firmware calls
  silently no-ops.
- Memory, thread, sync, message-queue, and filesystem groups are the strong
  core and are the right starting set for a Rust reimplementation.
- PrimU2's own registration macro carries a second status field: 48
  registrations are tagged `HANDLE_IMPLEMENTED` and 37 `HANDLE_NAMEONLY`
  (present primarily so the SVC log shows a name; behavior is minimal) —
  several of the "Complete" rows above are `HANDLE_NAMEONLY` and should be
  re-audited when implementing them in Rust.

## 4. Major missing functionality (657 unimplemented, grouped by name)

| Gap | Approx. share | Examples |
|---|---|---|
| GDI / graphics primitives (SDKLIB widget toolkit) | ~170 | `DrawLine`, `FillRect`, `DrawCircle`, bitmap, icon, cursor ops |
| UI toolkit (buttons, lists, menus, editors, rich view, scrollbars, windows) | (overlaps above) | `CreateButton`, `ChangeListItemSum`, `ConstructRecEdit`, `Richview` |
| Text/font services | ~20 | `GetCharWidth`, `WriteString`, `Printf`, `PrintfXY` |
| Pen/touch + key services | ~35 | `GetPenEvent`, `CheckPenEvent`, `SetSysKeyState` |
| File/disk extras | ~30 | flash/FAT-adjacent, disk room, sync |
| Event/timer/LCD control | ~40 | `LCDOff`, `CheckLCDOn`, `SetTimer1IntHandler`, `GetLCDContrast`, `Buzzer`, `KeyBeep` |
| Error/context services | — | `OSSetLastError`, `OSGetLastError`, `getvect`/`setvect`, `RemapMemory`, `InterruptSetMode` |
| Everything else | — | `ShowGraphic`, `CreateIcon`, ... |

**Key open question (Phase 2 M2):** the calculator's own renderer evidently
survives without most of the SDKLIB GDI group (PrimU2 boots and draws the UI
without them), implying armfir contains its own drawing code and the SDKLIB
widget APIs are used by *other* apps (`diagnose.exe`, BESTA UI tools). This
must be confirmed by boot tracing rather than assumed.

## 5. Full inventory file

A machine-readable 742-row inventory (id, enum name, JSON name, status) is
kept at `scratch/research/svc_coverage_full.tsv`, and the SVC-id usage counts
scanned from armfir.elf at `scratch/research/firmware_called_svcs.json`
(research artifacts, gitignored).
