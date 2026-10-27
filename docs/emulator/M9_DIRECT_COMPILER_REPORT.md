# M9 — Direct original-firmware compiler Check

Date: 2026-10-27. Firmware: G1 2.4.2 `armfir.elf`, SHA-256
`ad9779ff…fb39`. Local emulation only; no physical calculator was used.

## Result

The original firmware compiler can be called directly inside the running
emulated firmware, without simulating key presses or opening the Program
Catalog or editor. A warm direct Check takes about **12 ms** end to end
(p95 about 19 ms), against 6.9 s for a fresh checker and 10.6 s for the
UI-driven session. Inside the firmware, the whole sequence takes about
1.3 ms, and 1.0–1.3 ms of that is the compiler. Over the 75-case corpus,
in four orders and with every case checked three times in one session, the
results are identical to the stored firmware reference.

## Firmware call chain (static disassembly + register probes)

| Address | Role (verified) |
|---|---|
| `0x308F2C60` | Program Editor Check handler: editor `[[g]+0x34]`, commits text (`[vtable+0x2c]` = `0x309DBAC0`), calls the compiler with `([editor+0x64], 1, 0, sp)` |
| `0x306BB740` | Program compiler. Compiles **in place** into the program context: source items at `ctx+0x314` (count `+0x318`), UTF-16 text pointer at `item+0x40`, parser `0x306D4BC8`; resets with `0x306AE1A4(ctx, 0)`, finalizes with `0x306AE210`; with `r1 = 1` it skips the global error record; failure record `{index, line, utf16_offset}` at `r3` |
| `0x30B43FDC` | `Programs(name):=value`: for a string clears both source lists (`0x306AEA88(ctx+0x314, 0)`, `0x306AE930(ctx+0x31C, 0)`), `new(0x48)` (`0x30800124`), copies the text (`0x306BC954` = length `0x306D20F0` + duplicate `0x30690DB0`), initializes the item (`0x306ADFD8(item, "Main", 0, copy)`; the item owns the copy), appends it (`0x306AEA00`), and compiles with **Check's arguments** (`ctx, 1, 0, &record`; return address `0x30B4407C`) |
| `0x306C3CD8` | Resource text lookup (tail-calls `0x306C072C(id, 0)`); returns a table pointer, no allocation |
| `0x308F2CA8` / `0x308F2D04` | Check handler's dialog resources: `mov r0, #0x410` (success) and the literal `0x3416` (error); a failure whose record index is −1 shows no dialog |

Probes (`PRIME_PROBE_PCS`, an opt-in diagnostic register log) confirmed on
the running firmware:
- every Check and every `Programs():=` compile runs on guest thread 0;
- `ZZT_IDE`'s context was `0x2045C870` for all 75 UI Checks of an M2 session,
  so it is the program's own object;
- the loader's `Programs("ZZT_IDE"):=s` compiles the new source with `r1 = 1,
  r2 = 0` and a result record, exactly like Check.

Allocation: the firmware's `malloc`/`free` are SVCs (`lmalloc` `0x10037`,
`_lfree` `0x1003A`) served by the emulator's host-backed guest heap, so a
buffer from `GuestMemory::heap_alloc` is an ordinary guest allocation.

## Mechanism

- **Where:** guest thread 0's main-loop wait, `OSWaitForEvent` called from
  the `wait_event` shim (return address `0x3069341C`). The UI is then between
  two events, which is where Check itself runs. Thread 1 is the event pump
  in `GetEvent` (M3) and is never used.
- **How** (`guest_call.rs`): the wait's SVC handler saves every register,
  points the PC back at the SVC, builds a frame below SP, and enters the
  target with `LR` = a two-word trap (`svc #0xFFFF0`) in guest heap memory.
  The trap's dispatch restores the saved registers, so the thread simply
  waits again. A `CallPlan` chains calls back to back; a pending plan wakes
  the wait within 10 ms.
- **What** (`direct.rs`): the same calls as `Programs():=`, in the same order
  and with the same arguments, then the compiler, then the Check handler's
  resource choice and `0x306C3CD8`. Before running, the plan checks a
  SHA-256 of the six code ranges it relies on (182 bytes), and refuses a
  program context in use (`ctx+0x24` bit 0).
- **Validation:** a verdict requires exactly one direct compiler entry with
  `LR` = the trap, the requested context and record, Check's arguments, ARM
  state, exactly one parsed source whose SHA-256 and UTF-16 length equal the
  request, one parser return, a return of 0 or 1, a non-empty firmware
  message, and on rejection a source index other than −1. Anything else is
  no verdict.
- **Context discovery:** the first request in a session runs the existing
  UI loader once and takes `ZZT_IDE`'s context from the firmware's own
  `Programs():=` compile of that exact source (`kind = "assign"`); every
  request, including that first one, is then checked directly.

Positions map through the same `checkprog::firmware_diagnostic` /
`compiler::position` code as the fresh checker; nothing is estimated.

## Equivalence (75 cases, stored reference, 0 differences each)

| Capture | Checks | Differences | Repeat differences |
|---|---|---|---|
| `--engine direct` (corpus order) | 75 | 0 | — |
| `--reverse` | 75 | 0 | — |
| `--shuffle 20261027` | 75 | 0 | — |
| `--shuffle 7 --repeat 3` (one session) | 225 | 0 | 0 |

Compared fields: verdict, status, message, line, column, UTF-16 offset.

State: guest heap live bytes after each Check were identical for every case
between rounds 2 and 3 of the 225-check session, with the same peak in all
three rounds (only the first request, which also ran the loader, differs).

## Latency

| Path | Cold | Warm p50 | Warm p95 |
|---|---|---|---|
| Fresh checker | 6.9 s per Check | — | — |
| Session, UI-driven | 15.5 s | 10.6 s | 11.1 s |
| **Session, direct** | 10.6 s (boot + one loader run) | **12.3 ms** | **16–23 ms** |

Warm direct stages (median): compiler call 1.2 ms, the other firmware calls
under 0.03 ms each, message lookup 0.02 ms, whole plan about 10 ms (mostly
waiting for the UI thread's next 10 ms wait poll), host round trip
about 12 ms. Primary (< 1 s) and stretch (< 250 ms) targets are met for warm
Checks; cold start still needs one firmware boot and one loader run.

## Integration

- Session command `DirectCheck { request, source }`; the verdict returns in
  `Event::Result.check`, like the UI path.
- `ppl diag-parity capture --engine direct [--reverse | --shuffle SEED]
  [--repeat N]`.
- `ppl_engine::direct_check::DirectChecker`: one headless session on a
  scratch copy of `PPL_FIRMWARE_INIT_DATA`. Before use it must reproduce two
  stored results exactly (canary: `v01_hello` accepted; `e12_unclosed_paren`
  rejected at 3:13, offset 35, "Error: Syntax Error").
- GUI Check: uses the warm direct checker, starting it in the background on
  the first Check. Until it is ready, or whenever it reaches no verdict, the
  fresh checker gives the verdict, and the console says which path was used.
  Results go through the same revision-bound recording, markers and Problems
  provenance (`guest-firmware`).

## Risks

- The addresses and code hash are pinned to this firmware; another image
  disables the path rather than calling unknown code.
- The calls skip `Programs():=`'s post-compile work (error object creation,
  file save), which the Check handler skips too; the check program is
  session-owned (`ZZT_IDE` in a scratch profile).
- A guest fault inside a call would end that session; the GUI then falls
  back to the fresh checker.
- Cold start is about 10.6 s per GUI session (boot plus one UI loader run).
