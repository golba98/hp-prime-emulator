# M2 Execution Trace & Evidence

All runs used the official G1 firmware 2.4.2 (20260909):
`armfir.elf` sha256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`,
`PRIME_OS.ROM` mapped inert. Working command (from the repository root):

```bash
scripts/emulator/run.sh run --release -- \
  --firmware scratch/research/firmware/extracted/fat/armfir.elf \
  --os-rom   scratch/research/firmware/extracted/PRIME_OS.ROM \
  --data-dir /tmp/m2data/prime_data \
  --on-unsupported zero \
  --max-instructions 400000000 --max-seconds 240 \
  -v 1 --report report.json
```

The data dir must contain the extracted `PRIME_APP.DAT` (FAT16) contents
under `A/` (mtools; Phase 1 procedure). `--on-unsupported zero` is the
diagnostic policy; every hit is logged as unsupported, never counted as
success. Exit codes: 0 heartbeat / 1 fault / 2 strict unsupported stop /
3 limit / 4 config error.

## Verified boot sequence (live, final run)

```
load:   ELF32 ARM ok: entry 0x30604AE8, 1 PT_LOAD segment(s), 8306652 bytes
load:   OS ROM mapped inert: 1048576 bytes, entry 0x30000020
cpu:    main thread: entry 0x30604AE8, sp 0x20400000 (stack at 0x20000000)
guest:  "check sdk version"                         (firmware's own debug SVC)
guest:  "check data format, read header..."         (firmware's own debug SVC)
map:    platform DLLs pre-mapped at preferred bases (SDKLIB 0x10000000, ...)
guest:  ISR irq=14 isr=0x306073C8 registered        (timer dispatcher)
device: '\\?\ARCH' opened as handle 0x1             (hardware revision query)
thread: guest thread 1 spawned: func 0x306A3984     (own engine, own stack)
vfs:    firmware creates its C:\DATA app structure (183 _wmkdir, 108 __wfopen,
        44 _ReadFile, 53 _OpenSubFile — first-boot data setup)
irq:    virtual IRQ dispatcher delivers tick 14 → guest ISR runs,
        calls InterruptDone, tick counter 0x30D69DC0 increments
steady: main thread idles in EnterCrit/OSWaitForEvent/LeaveCrit
        at 0x3069341c (~80 Hz), tick ISR at 10 Hz — no faults for 240 s
```

## Quantitative evidence (60 s run, release)

| Metric | Value |
|---|---|
| Instructions executed (main thread) | 37,200,000 |
| SVC dispatches | 16,950 |
| Distinct services exercised | 51 |
| Implemented services exercised | 48 |
| Virtual IRQs delivered (IRQ 14) | 54 in 60 s (rate limited by blocking waits) |
| Guest faults | 0 over the window (240 s run: 0) |
| Throughput | ~35 MIPS in compute bursts; 0.6 MIPS average while idling in host-blocked event waits |

Top repeating call sites (heartbeat/stall analysis data):

| Service | Caller | Count |
|---|---|---|
| OSEnterCriticalSection | 0x3068DE38 | 4,770 |
| OSLeaveCriticalSection | 0x3068DE4C | 4,770 |
| OSWaitForEvent | 0x3069341C | 4,763 |
| _fread | 0x3061556C | 422 |
| lmalloc | 0x30802B93 | 215 |

## Historical progression (all observed live, this milestone)

1. Run 1 (strict policy): stopped at `_LoadLibraryA` called from
   `0x30604F44` — the PE loader became a demand-driven requirement.
2. Diagnostic runs ranked the service frontier (the `_wfindnext` 32k-call
   loop was a busy loop caused by the zero-return stub — fixed by a real
   implementation returning -1 at end-of-directory).
3. InterruptEnable mislabel caught at registration (fail-loud name check).
4. Concurrent ISR engine caused torn callback pointers → traced with the
   instruction ring (`blx r4` into ARM mode from a Thumb callback read at
   `0x307FB1FC`) → replaced with deterministic same-engine ISR delivery.
5. After the fix: 42.6M instructions reach the idle loop; 229M over 240 s
   with zero faults.

## What does NOT count as success here (per the M2 criteria)

Executing instructions, ELF parsing, or single SVCs — those are the
synthetic-test domain. The 42.6M-instruction mark and the idle loop are
genuine firmware behavior, but the **heartbeat criterion (GetEvent pump
round-trip with injected input) is not yet met**: GetEvent has been called
only once and the idle loop waits on an event the firmware's own timer
handlers would set. See M2_KNOWN_BLOCKERS.md.
