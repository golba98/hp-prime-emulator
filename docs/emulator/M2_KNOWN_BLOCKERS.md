# M2 Known Blockers & Next Step

> **M3 addendum (2026-10-23):** Every suspect below was investigated and
> **disproven** in M3. The "stall" was in fact the firmware's designed
> two-thread runtime: thread 1 is the UI event pump (`0x3060708C`), parked
> *inside* `GetEvent` waiting for input, and the main thread waits on the
> pump's completion event. The timer handler table was never statically
> empty — handler `0x3067ECBC` registers/clears in a re-arm cycle
> (31 writes/90 s). The missing piece was a legitimate headless input
> source (`--inject-key`, ON-key UIMultipressEvent). The verified
> `GetEvent` round-trip heartbeat is documented in
> [M3_EVENT_LOOP_REPORT.md](M3_EVENT_LOOP_REPORT.md). The text below is
> preserved as the historical M2 record.

## Precise remaining blocker (deepest verified state)

**Verified state:** the real firmware initializes for 42–229M
instructions, loads its platform DLLs, builds its `C:\DATA` structure,
spawns guest threads, registers its timer ISR (which our virtual IRQ pump
executes — tick counter `0x30D69DC0` increments, `InterruptDone` fires
every tick), and then idles in a clean, fault-free loop
(`EnterCrit → OSWaitForEvent → LeaveCrit` at `0x3069341C`) — steady for
240 s with zero faults.

**The gap:** the main thread waits on event descriptor `0x20402430`, which
nothing sets. The firmware's timer ISR (decoded at `0x306073C8`) only
calls *registered timer handlers* (3 slots at `0x30D69DB8`, deadlines at
`0x30D69DB0`) — and those slots are empty because the code path that
fills them has not run yet. One of these should eventually `OSSetEvent`
and drive the system into the `GetEvent` UI pump (the heartbeat
criterion).

**Prime suspects (in investigation order):**

1. **`GetApplicationHeadInfoA` (0x102CB, 2 calls, unsupported).** Name
   suggests application-container metadata; the surrounding call site is
   in the boot path (`0x30615910`). Reverse-engineer its expected
   struct/return from the call-site disassembly (the same method that
   decoded the ISR and the stub table).
2. **`SetEventType`/`GetEventType` (0x10041/0x10042, 2 calls each,
   unsupported).** UI event-type filter state; plausibly gates which
   events the idle loop accepts. Semantics likely a simple get/set
   filter variable — find the caller at `0x30607094/9C`.
3. **Guest thread 1's early exit** (`0x306A3984`, one engine, no
   instruction record — it stops before its first slice completes).
   Its argument (`0x20405590`) suggests a loader job; trace its stop
   reason (likely an unsupported service inside its own path that
   zero-returns into an early return).
4. **Timer registration path.** No direct `bl` callers of the
   `SetTimer1IntHandler` stub exist; the tables at `0x30D69DB0/DB8` are
   written by firmware code near `0x3067C4DC` — find its trigger.

## Honest M2 verdict

`M2 PARTIAL — FIRMWARE EXECUTION VERIFIED`. Genuine deep execution with
a stable, deterministic steady state, all subsystems (threads, sync, PE
modules, VFS, virtual IRQs) working against real firmware, 48
implemented services exercised, 26 passing tests, zero workspace
regressions. The heartbeat (GetEvent pump round-trip) is **not** claimed.

## Recommended first milestone of M3 (smallest next executable step)

Decode `GetApplicationHeadInfoA` + `SetEventType`/`GetEventType` from
their call sites (static), implement, and re-run — expected outcome:
either the idle loop's event fires (heartbeat within reach) or the next
unsupported frontier is identified. This is a 1–2 day loop using the M2
tooling (`--trace-svc`, call-site report, strict mode).
