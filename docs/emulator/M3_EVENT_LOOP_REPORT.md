# M3 Event-Loop Report

**Milestone:** Phase 3 / M3 — Firmware Initialization and Event-Loop Recovery
**Date:** 2026-10-23 · **Status:** `M3 COMPLETE — VERIFIED EVENT-LOOP HEARTBEAT`
**Firmware:** HP Prime G1 2.4.2 (2026-09-09), `armfir.elf`
sha256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`
(user-supplied, never committed) · unchanged from M2.

## 1. Root-cause analysis

**The M2 "stall" was not an initialization failure.** The system had
already reached its designed runtime topology, and the two remaining M2
hypotheses (missing services, empty timer table) were both wrong:

1. **The system is a two-thread producer/consumer runtime.**
   Static decoding (capstone) of the wait path showed:
   - `wait_event(obj, timeout)` shim at `0x30693404` (the `0x3069341C`
     loop M2 observed) is a thin wrapper over `OSWaitForEvent`.
   - The UI **event pump** lives at `0x3060708C`: prologue
     `GetEventType() → r0 |= 0x20 → SetEventType(r0)`, then a
     `GetEvent(&stack_event)` loop dispatching on `event_type`
     (`0x100` system, `0x20` timer, `0xF` touch, `0x10` key).
   - Guest thread 1 (entry `0x306A3984`) is a trampoline:
     `blx [arg+4]` — and its callback **is the pump**.
2. **Thread 1 never exited** (M2 hypothesis C, disproven). It parks
   *inside* the `GetEvent` handler waiting for input; M2 simply had no
   input source and a per-thread counter that only updated on completed
   slices, so it looked "dead". With a 10k slice it reports progress.
3. **The timer handler table was never empty** (disproved with the new
   `--watch 0x30D69DB8,12` watchpoint): the firmware actively
   registers/clears handler `0x3067ECBC` in slot 0 — 31 writes / 30
   clears in 90 s — a periodic re-arm cycle. The tick counter
   (`0x30D69DC0`) increments in the ISR; deadlines at `0x30D69DB0`.
4. **The event the main thread waits on (`0x20402430`) is produced by
   the pump thread itself**: after processing an input batch, the
   firmware calls `OSSetEvent(0x20402430)` from `0x3069AAD4` (the sole
   `OSSetEvent` wrapper in the image, `tid=1` in the live trace).

So the missing piece was **a legitimate input source**: the pump was
starved, the main thread therefore never got its event, and the system
sat in its normal idle state — exactly what a real calculator with no
keys pressed does.

## 2. Rejected / disproven hypotheses

| Hypothesis (M2) | Verdict | Evidence |
|---|---|---|
| `GetApplicationHeadInfoA` (0x102CB) blocks init | **Rejected** | ABI decoded from call site `0x3061590C`: `(r0 = app handle, r1 = 0x84-byte buffer)`, returns 0 on failure; the firmware has an explicit failure path (`cmp r0,#0; bne`) that falls back to `ProgramIsRunningA`. Zero-return is the conservative correct answer; boot proceeds. |
| `SetEventType`/`GetEventType` missing blocks init | **Rejected** (but now implemented) | Both calls come from the pump prologue; the firmware tolerates absent semantics. Implemented as a stored filter (the pump ORs 0x20 in; GetEvent currently ignores the filter — documented divergence). |
| Thread 1 exits early / may be an init dependency | **Disproven** | Thread 1 *is* the UI pump; it parks in `GetEvent` and services all input. |
| Timer table empty because registration never ran | **Disproven** | Watchpoints show active register/clear cycling of `0x3067ECBC`. |
| Event `0x20402430` needs timer handlers to fire | **Disproven** | The producer is the pump thread after input processing, not a timer callback. |

## 3. Implementation (smallest sound corrections)

| Module | Change | Why |
|---|---|---|
| `runner.rs` | `--inject-key CODE:MS` headless input source (documented device semantics: `UIMultipressEvent` KEY/KEY_UP pair, PrimU2's exact CommandServer model, keycode 0x83 = ON) | The missing external input pathway; no GUI introduced |
| `runner.rs` | spawned-thread slice granularity 10k | per-thread progress reporting for threads that park inside blocking services |
| `services/mod.rs`, `trace.rs`, `engine.rs` | thread-id plumbed through dispatch into `SvcCtx`, SVC/JSONL traces, heartbeat evidence | "which thread" was the central unknown of the M2 blocker |
| `services/mod.rs` | `GuestArgs::stack_slots` ([sp+8], [sp+12]) | wrapper-frame diagnostics (carefully documented as *not* return addresses — an earlier "parent caller" interpretation was caught and renamed during review) |
| `engine.rs` | guest write watchpoints (`MEM_WRITE` hook, bounded 64-line log) | proves guest-side state machine activity |
| `services/handlers.rs` | `SetEventType`/`GetEventType` stored-filter handlers | the pump's prologue pair now round-trips real state |
| `services/threading.rs` | event lifecycle logs (create/wait/wait-resolved/set/reset, with desc/tid/caller) | causal timeline, bounded, v2-gated |
| `tests/svc.rs` | +6 tests (below) | regression coverage |

No production code signals guest events, injects fake return values, or
alters the IRQ pump; the virtual IRQ pump still executes the *real*
guest ISR.

## 4. Service coverage

Newly implemented: `SetEventType` (0x10041), `GetEventType` (0x10042) —
99 registered handlers total. `GetApplicationHeadInfoA` (0x102CB)
remains deliberately unimplemented with its ABI documented; its
zero-return is a legitimate guest-visible failure that the firmware
explicitly handles (not a fabricated success).

## 5. Verified thread and event behavior (live, 60 s runs, 3× reproducible)

```
tid 0 (main):  init → creates events 0x204010C0, 0x20402430 →
              tick-wait loop: OSWaitForEvent(0x20402430, slice) ×377,
              5 wait-resolves (auto-reset consumes; guest flag cleared)
tid 1 (pump):  trampoline → pump prologue (Get/SetEventType) →
              GetEvent @ 0x306070B4 → [input arrives] →
              processes batch (KEY_BATCH, down+up pair) →
              OSSetEvent(0x20402430) ×10 → GetEvent again…
virtual IRQ:   10 ms tick → guest ISR 0x306073C8 → tick counter++ →
              timer slot re-arm (register 0x3067ECBC / clear) ×31
```

No lost wakeups observed: every delivered input batch corresponds to a
main-thread wait resolution; `HostEvent` auto-reset set-while-unwaited
semantics are unit-tested (edge-triggered).

## 6. Firmware execution evidence (M3 vs M2 baseline)

| Metric | M2 (60 s, no input) | M3 (60 s, input 0.83:250 ms) |
|---|---|---|
| Outcome | limit-reached | **heartbeat** (exit 0), 8+ runs |
| Instructions | 37.2 M | 13.0–13.4 M to heartbeat |
| GetEvent pump calls | 1 (blocked) | **214 in 120 s** (107 round trips) |
| Events delivered | 0 | 5 per heartbeat run |
| Pump re-entries (0x306070B4) | 1 | 6 per heartbeat run |
| OSSetEvent(0x20402430) | 0 | 10 (live trace) |
| Main-thread wait resolves | 0 | 5 (live trace) |
| Guest faults | 0 | 0 (120 s run: 524.6 M instructions, 0 faults) |
| Timer slot writes (watch) | unobserved | 31 reg / 30 clear / ticks advancing |
| IRQs delivered | 54 | 509 (120 s run) |

## 7. Heartbeat evidence (the concrete round-trip sequence)

For each injected ON-key press (down+up pair, 250 ms interval):
1. Host input queue receives the pair (device pathway, no guest writes).
2. `GetEvent` returns a `KEY_BATCH` (`ui_event_prime_s`, type `0x100010`)
   to the pump thread (tid 1) at the pump call site `0x306070B4`.
3. Firmware dispatches on `event_type` and processes the batch.
4. Firmware calls `OSSetEvent(0x20402430)` from `0x3069AAD4`.
5. Main thread's `OSWaitForEvent` resolves (auto-reset consumes).
6. Main loop continues, re-arms, waits again.
7. Pump re-enters `GetEvent` (re-entry count ≥ deliveries).

Heartbeat criterion: ≥3 delivered events **and** matching pump
re-entries from the verified call site — asserted by the env-gated
integration test including `thread_id == 1` and
`caller_pc == 0x306070B4`.

## 8. Test results (commands actually run)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS (clean) |
| `scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings` | PASS (0 warnings) |
| `scripts/emulator/run.sh test --workspace --all-targets` | PASS — 30 suites, 0 failures (all M2 tests intact) |
| `PRIME_TEST_FIRMWARE=… cargo test -p prime-emulator --test firmware_boot` | PASS — now asserts the heartbeat: outcome, call site, thread id, deliveries, re-entries (debug build, 19.6 M instructions, 1.1 s) |
| Release run ×3 (60 s) | heartbeat ×3, identical evidence, 0 faults |
| Release run (120 s) | 524.6 M instructions, 214 pump calls, 509 IRQs, 0 faults |

New regression tests (13 svc total): host event set/wait/reset and
auto-reset consumption; set-while-unwaited (lost-wakeup) case;
event-type filter round trip; synthetic GetEvent round trip with
host-injected input (asserts KEY_BATCH struct contents, keycode pair,
delivery thread id); guest-write watchpoint; thread-id flow through
dispatch.

## 9. Remaining limitations

1. `GetEvent` does not filter on the `SetEventType` mask yet (the pump
   only ORs bits; all queued batches are delivered). Documented
   divergence.
2. The ON-key injection is a fixed-rate headless source; a proper
   keypad/touch automation interface (per the PrimU2 TCP command model)
   is future work.
3. The pump's dispatch of specific event types beyond KEY_BATCH
   (touch, system) is not yet exercised.
4. Unsupported services are still zero-returned in diagnostic mode
   (`GetApplicationHeadInfoA` remains the notable one); strict mode
   stops on the first one.
5. `docs/**/*.md` remains untracked per repository policy — see the
   persistence-risk note in M2_IMPLEMENTATION_REPORT.md §7 (applies to
   this document too).

## 10. Next milestone (smallest next objective)

Exercise the pump's key-dispatch path to a **screen-visible state**:
inject a digit key + ENTER and capture the LCD framebuffer
(`GetActiveLCD` object already mapped) to verify the firmware's UI
drawing is reached — the first step toward screen-output evidence for
PPL-program automation, without building a graphical frontend.
