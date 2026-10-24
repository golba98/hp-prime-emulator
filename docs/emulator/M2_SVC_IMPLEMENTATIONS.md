# M2 SVC Implementation Ledger

97 of 742 SDKLIB services have handlers (up from PrimU2's 85/742
reference coverage; 48+ of ours were *exercised by the real firmware* in
the final run). Status evidence: **ref** = PrimU2 commit 461d999,
**fw** = observed in the real 20260909 firmware execution.

## Implemented (exercised by the firmware, live counts from the 60 s run)

| ID | Name | Status | Live calls | Evidence |
|---|---|---|---|---|
| 0x10000 | OSCreateThread | partial | 1 | ref: own engine per thread, 0x54-byte descriptor (magic@0, tid@0x14, func@0x18); fw: thread spawned and ran |
| 0x1000D | OSCreateEvent | impl | 2 | ref: 0x14-byte descriptor, magic 0x201, flag@4, latch@8 |
| 0x1000E | OSWaitForEvent | impl | 4763 | ref: wait-result ABI 0x82/0x83/0x84; fw: main idle loop at 0x3069341c |
| 0x10012 | OSInitCriticalSection | impl | 3 | ref: magic 0x202, recursion count@8; host: reentrant per host thread |
| 0x10013 | OSEnterCriticalSection | impl | 4770 | ref + fw |
| 0x10014 | OSLeaveCriticalSection | impl | 4770 | ref + fw |
| 0x1001E | InterruptInitialize | partial | 1 | ref: registers (irq=r0, isr=r2), returns 1; fw: ISR 0x306073C8 registered for IRQ 14 |
| 0x10020 | InterruptDisable | partial | 1 | ref: unregister ISR; fw: honored |
| 0x10021 | InterruptDone | partial | 373 | ref: returns 1; fw: called by the guest ISR every tick |
| 0x10029 | LCDOn | impl | 1 | ref: returns 0 |
| 0x1002F | SetAutoPowerOff | partial | 1 | unverified; zero observed harmless (ref: not implemented either) |
| 0x10037 | lmalloc | impl | 215+198+147+... | ref: heap pointer, 0 on failure |
| 0x10038 | lcalloc | impl | 2 | ref (we zero; ref does not — documented divergence) |
| 0x10039 | lrealloc | impl | 2402 | ref: new+copy+free; old size from our heap |
| 0x1003A | _lfree | impl | 950 | ref: returns its argument |
| 0x1003F | GetEvent | partial | 1 | ref: blocks until an input batch; ui_event_prime_s layout verified (124 bytes) |
| 0x1004C | SetSystemVariable | partial | 2 | ref: var 89→-1; var 6/type 2→brightness |
| 0x1008D | GetActiveLCD | partial | 3 | ref LCD layout: 320x240 ARGB32 host convention, magic block + buffer |
| 0x100A5 | GetSysTime | impl | 1 | ref: SYSTEMTIME u16 fields, returns r0 |
| 0x100C1 | GetMasterIDInfo | partial | 2 | ref layout 172 bytes; identity string is ours, not the reference's (documented) |
| 0x100C5 | _afnsplit | impl | 1 | ref: split semantics, ext=[sp+8] |
| 0x100C6 | _afnmerge | impl | 1 | ref: merge semantics, out=r0 |
| 0x100CA | _fclose | impl | 21 | ref |
| 0x100CB | _filesize | impl | — | ref |
| 0x100CF | __fseek | impl | — | ref: whence 0/1/2 |
| 0x100D0 | _ftell | impl | — | ref |
| 0x100D4 | _fread | impl | 422 | ref: (dest,size,count,handle) |
| 0x100D7 | _fwrite | impl | 51 | ref: (src,size,count,handle) |
| 0x100DA | _findclose | impl | 2 | ref: releases sfn/lfn guest strings |
| 0x100EC | _OpenFile | impl | 1 | ref: loader fd (0x20 bytes: cart, parent, base, size, offset) |
| 0x100EF | _CloseFile | impl | 36 | ref |
| 0x100F0 | _ReadFile | impl | 44 | ref: bounded by fd window |
| 0x100F1 | _FseekFile | impl | 40 | ref |
| 0x100F2 | _FileSize | impl | 27 | ref |
| 0x100F3 | _OpenSubFile | impl | 53 | ref: subfile windows into parents |
| 0x10111 | _GetPrivateProfileString | partial | 1 | ref ABI: (app,key,def,r3=dest, size=[sp+8], file=[sp+0xC]) |
| 0x10112 | _WritePrivateProfileString | partial | 3 | ref ABI: (app,key,value, file=[sp+8]) |
| 0x10119 | GetCurrentPathA | partial | 1 | ref routes to GetCurrentExecutable (image path); fw: called at entry |
| 0x1011A | ProgramIsRunningA | partial | 3 | ref: singleton zeroed 0x250-byte record |
| 0x1026F | __wfopen | impl | 108 | ref: VFS handle |
| 0x10270 | _wfindfirst | impl | 23 | ref: find_context_t (38 bytes), DOS attrs |
| 0x10271 | _wfindnext | impl | 2 | ref: returns -1 at end (fixed the 32k-call scan loop) |
| 0x10274 | __wremove | impl | — | ref |
| 0x10277 | _wmkdir | impl | 183 | ref |
| 0x10278 | _wrmdir | impl | 2 | ref |
| 0x10279 | _wchdir | impl | 3 | ref: per-drive cwd |
| 0x10286 | _LoadLibraryA | impl | 1 | ref PELoader; fw: SDKLIB etc. loaded |
| 0x10298 | _GetProcAddress | impl | — | ref: export lookup by module base |
| 0x102A1 | WriteComDebugMsg | partial | 2 | ref: guest printf; ours logs the format string |
| 0x102A7 | CreateFile | partial | 3 | ref: every name becomes a virtual device |
| 0x102AC | DeviceIoControl | partial | 5 | ref ABI: (h,req,in,size, out=[sp+8], outlen=[sp+12], retlen=[sp+16]) — single-deref fixed from our first attempt |
| 0x102AD | CloseHandle | partial | 3 | ref |
| 0x1001F | InterruptEnable | partial | — | ref no-op returning 1 (verified export name; PrimU2 mislabels it InterruptMask) |

## Registered but not yet exercised by this firmware path

OSCreateSemaphore/OSWaitForSemaphore/OSReleaseSemaphore/OSCloseSemaphore,
OSSetEvent/OSResetEvent/OSCloseEvent, OSDeleteCriticalSection,
OSExitThread/OSTerminateThread/OSSuspendThread/OSResumeThread/
OSWakeUpThread/OSSetThreadPriority/OSGetThreadPriority, OSInitCriticalSection,
_LoadLibraryW, _FreeLibrary, _GetModuleFileNameA, _wrmdir, _filesize,
__fseek, _ftell, __wremove, GetEvent (1 call so far).

## Unsupported services observed (zero-return diagnostic mode, logged, never counted as success)

| ID | Name | Calls | Note |
|---|---|---|---|
| 0x102CB | GetApplicationHeadInfoA | 2 | unknown semantics; ref: not implemented |
| 0x10041 | SetEventType | 2 | UI event-type filter; unknown |
| 0x10042 | GetEventType | 2 | UI event-type filter; unknown |

These three are the current candidates for the remaining boot gap.
