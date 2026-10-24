# M4 Firmware Display Report

**Milestone:** M4 — firmware LCD output, keyboard→screen interaction, first calculation
**Date:** 2026-10-24 · **Status:** `M4 COMPLETE — FIRMWARE DISPLAY AND CALCULATOR VERIFIED`
**Firmware:** HP Prime G1 2.4.2 (20260909), `armfir.elf`
sha256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39` (user-supplied, never committed)
**Backend:** the original ARM firmware under Unicorn (`crates/prime-emulator`). The native Rust PPL VM, `ppl-graphics` and the IDE are not involved.

> `docs/**/*.md` is gitignored in this repository: this report is untracked.
> Add it with `git add -f` if it should be kept (the M2/M3 reports have the same caveat).

## 1. Summary

| Stage | Result | Evidence |
|---|---|---|
| M3 baseline (user command) | reproduced ×3 | heartbeat, 14.8 M instr., caller `0x306070B4`, tid 1, 5 delivered, 6 re-entries, `GetActiveLCD` ×3, 0 faults |
| M4-A authentic LCD capture | **PASS** | guest stores from firmware code (`0x3061FBE8`, `0x306A92B0`, …) into `window1_bufferstart`; 22–848-colour screens |
| M4-B keyboard→screen | **PASS** | key `2` delivered by GetEvent to the pump (tid 1); 58 px glyph on the edit line; 154–232 k firmware framebuffer stores; IDLE control: 0 px |
| M4-C 2+2 | **PASS** | result glyph of `2+2` pixel-identical to that of `4` (sha256 `dcb90b6d…456e`), differs from `2+3` (`f24b7133…89ce`); reproduced in independent runs |

## 2. Root causes found and fixed (in order of discovery)

Each was found with traces/disassembly and fixed in the emulator — nothing
draws, computes or writes guest state on the firmware's behalf.

1. **Thumb code resumed as ARM** (`engine.rs`). Unicorn's ARM `emu_start(begin)` takes the instruction set from bit 0 of `begin`; `run_slice` and the virtual-IRQ return resumed at the even PC, so any slice boundary inside Thumb code switched to ARM decoding. Seen as `UC_ERR_INSN_INVALID` at `0x307FB134` (Thumb `movs r0,#1` with CPSR.T clear, LR odd) once boot reached Thumb-heavy code. Fix: resume at `pc | CPSR.T`. Test: `tests/thumb_resume.rs` (fails without the fix).
2. **SDKLIB.DLL never loaded** (`runner.rs`, `services/pe.rs`, `services/system2.rs`). COREDLL, MD5DLL and SDKLIB all prefer `0x10000000`; the boot pre-map gave the base to the first file by name (MD5DLL, relocatable), so the fixed-base SDKLIB (no `.reloc`) could not be placed and `LoadLibrary("sdklib.dll")` returned 0 — in M3 too (there the file was simply absent). Fix: pre-map fixed-base images only; relocatable images always load into the arena; a preferred base is used only when a block starts exactly there. Tests: `pe::base_tests`.
3. **Loader sub-files closed the resource container** (`services/fsops.rs`). The guest-visible `loader_file_descriptor_t` (PrimU2 layout: cart, parent_fd, subfile_base, size, offset, flag) was laid out differently, and a sub-file copied its parent's `parent_fd` (0 for a root file) instead of pointing at the parent descriptor, so closing the first resource closed `armfir.dat` and every later read failed: **the firmware could not load its fonts and drew no text**. Fix: reference layout and `parent_fd = parent`. Test: `tests/loader_files.rs`.
4. **No UI timer events** (`services/handlers.rs`, `runner.rs`). The pump subscribes to `DEVICE_TIMER` (`SetEventType(GetEventType()|0x20)`) and dispatches `0x20` → `0x306099C0`/`0x3060A2A4` and `0x4000` → `0x306098A8`; GetEvent delivered only key batches. Fix: deliver `SYS_TIMER`/`DEVICE_TIMER` at PrimU2's cadence (11 and 61 ticks of 60 Hz) before input, as the reference's `special_event` slot does. `--ui-timers auto|on|off` (auto: on with `--keys`, off otherwise so the M3 command reproduces exactly).
5. **Event producers starved** (`runner.rs`). Injection, timers and the key script ran between main-thread slices, but the main thread blocks in `OSWaitForEvent(0x20402430)` (caller `0x3069341C`) until the pump sets it — and the pump only runs when GetEvent has something. Each wait ran to its ~5 s timeout (585 s for one script). Fix: a host *device thread* produces timer/input events and runs the key script; IRQ delivery stays on the main engine.
6. **No touch input** (`services/handlers.rs`, `keys.rs`). The first boot shows a slide-to-unlock lock screen (`SlideUnlock` string resource; keys are delivered but ignored), and the wizard's soft keys are touch targets. Fix: `UI_EVENT_TYPE_TOUCH` (0xF) delivery with PrimU2 `TouchUpdate` semantics (statuses 1/2/8, x at entry+6, y at +8 — the offsets the pump's 0xF branch at `0x30607180` reads); `TAP`/`SWIPE` script items. A `TAP` is held ~300 ms: shorter presses could fall between two SYS_TIMER ticks and were missed (2 of ~12 runs at ~96 ms; 0 of 10 at ~300 ms).
7. **Missing factory files.** The M3 command boots an empty data directory: no `A:\programs\misc\armfir.dat` (fonts, images) and no DLLs. M4 runs use the device's factory `A:` tree from `PRIME_APP.DAT` (`scripts/emulator/prepare_factory_a.sh`).

## 3. LCD memory layout and ownership

`GetActiveLCD` (SVC `0x1008D`) returns the OS-side LCD object (PrimU2 `LCD.h`
"BLIGLCD"), allocated by the OS service on first call (guest heap, e.g.
`0x204024E0`/`0x20402550` depending on the data directory), cleared black:

| Offset | Field | Value |
|---|---|---|
| 0 | `LCDMagicPtr` | object + 592 |
| 4 | `secondBufferstart` | object + 616 (same buffer: single-buffered) |
| 8 | `bufferSize` | 307 200 |
| 12 | `itself` | object |
| 468 / 470 | `xRes` / `yRes` | 320 / 240 |
| 592 | `LCD_MAGIC` | x_res 320, y_res 240, `pixel_bits` 32, brightness 2 |
| 592+16 | `window1_bufferstart` | object + 616 |
| 616 | framebuffer | 320 × 240 × 4 bytes, stride 1280 |

**Pixel format:** little-endian 32-bit `0x00RRGGBB` (bytes B, G, R, unused). Verified from guest stores (`0x00F2F2F2` grey written at `fb+0`), and consistent with PrimU2's renderer reading B/G/R at a 4-byte stride. The top byte is 0 (not alpha). PrimU2's brightness scaling is display-only and is not applied to captures.

**Rendering flow:** the firmware draws into its own surfaces and blits to the panel buffer with the 32-bit copy loop at `0x3061FB74` (`str r5,[r1],#4` at `0x3061FBE8`; destination `= [desc+0x10] + (y*[desc+8] + x)*4`). Other direct writers seen during key handling: `0x306A92B0`, `0x30B9498C`/`0x30B94994`, `0x3061DB3C` — all in `armfir.elf`'s text. No OS drawing service (FillRect, _BitBlt, CreateCompatibleLCD …) is called on these paths: only `LCDOn` and `GetActiveLCD`.

## 4. Implemented functionality

- **Capture** (`display.rs`): `LcdView::locate` validates the object (magic pointer, self pointer, resolutions, pixel depth, buffer size, buffer = object+616, buffer inside one mapped block) → `NotInitialized` / `InvalidLayout` / `OutOfBounds`. `capture` is one guest-memory read between slices; PNG via the `png` crate (not `ppl-graphics`).
- **Key/touch scripts** (`keys.rs`, `script.rs`): `--keys "2,+,2,ENTER"`, raw `0xNN`, `TAP:x:y`, `SWIPE:x0:y0:x1:y1[:steps]`, `IDLE` (negative control). A step settles when the input was delivered by GetEvent, the pump is parked in GetEvent with nothing queued, the main thread spends ≥ 80 % of the window in its event wait (and is waiting now), and no *new* screen state has appeared for `--settle-ms` (blinking cursors cycle through known states). Per-step evidence: delivery time and thread, first-change latency, change outside the idle mask, change excluding flicker, writer PCs (`--lcd-watch`), PNG + sha256.
- **CLI** (`main.rs`): `--keys`, `--screenshot`, `--screenshot-dir`, `--settle-ms`, `--idle-observe-ms`, `--boot-timeout-s`, `--key-timeout-s`, `--lcd-watch` (range-limited write hook installed once the framebuffer exists), `--ui-timers`; exit 5 = script failed. `verify-calc` runs the A/B/C differential (exit 0 pass, 6 not verified).
- **Report** (`--report`): independent `outcomes` (`firmware_execution`, `framebuffer_capture`, `input_delivery`, `input_to_display`, `calculation` — the last only from `verify-calc`), `display` (view, final capture, script evidence, writer PCs).

## 5. Verified key mappings

Codes come from PrimU2 (`mcp_server.py` and `LCD.cpp vk_to_device_keymap` agree). Every code in `keys::KEYS` is also checked against the **firmware's own table** (192 signed bytes at `0x30C09B7C`, via literal `0x30606FE4`, read by `0x30606FC0`; `0xE047` special-cased to 5): each maps to the documented 0–50 key index of the same name (e.g. `0x5A`→43 "2", `0xB9`→50 "+", `0x0D`→30 Enter, `0x55`→37 "4", `0x33`→44 "3", `0x01`→4 Esc, `0xE047`→5 Home, `0x83`→46 On) — test `key_codes_match_the_firmwares_own_table`. Observed on screen: `2`, `3`, `4`, `+`, `ENTER`.

**Event model:** each key is one GetEvent `KEY_BATCH` (`0x100010`) with a down (`0x10`) + up (`0x100000`) pair; the pump translates the code, keeps a 64-bit key-state mask and posts to the UI thread (`0x306A5CCC`).

## 6. Evidence

### M4-A (lock screen at boot, then unlock)
`--lcd-watch` during the unlock swipe: 1 536 000 framebuffer stores, all from firmware PCs. Boot screen: 22 colours (padlock); Home: 527–848 colours.

### M4-B (`<setup>,IDLE,2`)
| Step | Delivered to | First change | Lasting change | Framebuffer stores |
|---|---|---|---|---|
| IDLE | — | — | 0 px | — |
| `2` (0x5A) | tid 1 (pump) | 16–58 ms | 58 px, bbox (1,203)–(8,218) | 154 786–232 377 (top PC `0x306A92B0`) |

### M4-C (`verify-calc`, two independent invocations, identical)
| Run | Keys after setup | Result glyph bbox | sha256 |
|---|---|---|---|
| A | `2,+,2,ENTER` | (311,189)–(318,198) | `dcb90b6d810fdfec192e4391a6a0fe724743aef31910ae72ff102388d2df456e` |
| B | `4,ENTER` | (311,189)–(318,198) | `dcb90b6d810fdfec192e4391a6a0fe724743aef31910ae72ff102388d2df456e` |
| C | `2,+,3,ENTER` | (311,189)–(317,198) | `f24b7133dfd64b37c3c244b9e9d918e939aa1003164682a6156ce188fe4189ce` |

`a_matches_b`, `a_differs_from_c` and `expressions_differ` all true. The firmware's own Home screens read `2+2 … 4` and `2+3 … 5`. The result glyph is the right-most cluster of pixels changed by ENTER in the lowest changed band (status-bar clock changes are a separate band and are ignored).

Reliability: the full unlock + wizard + `2+2` script completed with identical calculation-step changes (58/56/74/275 px) in 10/10 runs.

## 7. Tests

| Suite | Firmware? | Covers |
|---|---|---|
| `tests/display.rs` | no | synthetic guest: GetActiveLCD → GetEvent → per-key pixel; ordering; timers/touch ignored; parked-flag sync; capture read-only; layout validation (each corrupted field); RGBA/PNG round trip; `measure_capture_and_encoding` (ignored) |
| `tests/thumb_resume.rs` | no | Thumb/ARM resume across slices and IRQ return |
| `tests/loader_files.rs` | no | descriptor layout, sub-file close keeps container, nested windows, bounds |
| `src/keys.rs`, `src/verify.rs`, `src/sha256.rs`, `pe::base_tests` | no | key/touch parsing, differential verifier (incl. wrong result, invalid control, static rule, status-bar change), SHA-256 vectors, DLL base policy |
| `tests/firmware_display.rs` | env-gated | M4-A, M4-B (with IDLE control), M4-C, firmware key table |
| `tests/firmware_boot.rs` | env-gated | M3 heartbeat (unchanged assertions) |

Commands run (final tree):

```text
cargo fmt --all -- --check                                              PASS
scripts/emulator/run.sh check --workspace --all-targets                 PASS
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings PASS (0 warnings)
scripts/emulator/run.sh test --workspace --all-targets                  PASS: 38 binaries, 1033 passed, 0 failed, 3 ignored
PRIME_TEST_FIRMWARE=… PRIME_TEST_OS_ROM=… PRIME_TEST_FACTORY_A=… \
  scripts/emulator/run.sh test -p prime-emulator --release \
  --test firmware_display --test firmware_boot -- --test-threads=1      PASS: 5/5 (32 s)
```

## 8. Performance (release, Fedora host)

| Measurement | Value |
|---|---|
| Boot to stable lock screen (factory data) | 0.5 s (≈2.3 s on an empty data dir) |
| Full setup + `2+2` script | 6.8–7.8 s, 128–191 M instructions |
| Throughput incl. idle waits | ≈ 19–28 MIPS (M2 compute bursts ≈ 35 MIPS) |
| Capture (guest read + decode) | 0.047 ms |
| Settle fingerprint | 0.045 ms |
| SHA-256 / PNG encode per frame | 0.83 ms / 0.96 ms (4.4 KB PNG for a 640-colour test frame) |
| Key queue → first screen change | 13–99 ms |
| `--lcd-watch` overhead | same wall time for the full script (range-limited hook); the earlier universal hook made Home ≈ 100× slower |
| Memory | LCD object 307 816 B guest; script driver keeps ≤ 32 distinct 300 KB frames per step (≤ 9.8 MB) |

## 9. Remaining limitations

- The OS LCD object is the reference's host-side layout (32 bpp); the real G1 panel is RGB565 — the firmware evidently adapts to `pixel_bits`, but hardware-panel fidelity is not modelled.
- `_FindResourceW` (SDK version resource) and `GetApplicationHeadInfoA` remain zero-returned (PrimU2 does the same for the former; the firmware handles both failures).
- First boot needs the lock-screen swipe and wizard; settings written to `C:\DATA` are not reused across runs (each test uses a fresh data directory).
- `GetEvent` still ignores the `SetEventType` mask (timers are delivered at the reference cadence regardless).
- Timing is host-paced (timer cadence, settle windows); results are reproducible in practice (10/10) but not cycle-deterministic.
- The JSONL trace can interleave lines from concurrent guest threads (1 corrupt line seen in one run).
- Only Home-screen arithmetic is verified; CAS, apps, programs and the Program Catalog are untested (M5+).

## 10. Reproduction

```bash
FW=scratch/research/firmware/extracted
scripts/emulator/prepare_factory_a.sh $FW/PRIME_APP.DAT /tmp/prime-factory     # once
SETUP="SWIPE:160:120:310:120:12,ENTER,ENTER,TAP:293:229,TAP:293:229"

# Firmware screenshot + 2+2 through the Home screen
mkdir -p /tmp/prime-calc/data && cp -r /tmp/prime-factory/A /tmp/prime-calc/data/
scripts/emulator/run.sh run --release --bin prime-emulator -- run \
  --firmware $FW/fat/armfir.elf --os-rom $FW/PRIME_OS.ROM \
  --data-dir /tmp/prime-calc/data --on-unsupported zero \
  --keys "$SETUP,2,+,2,ENTER" --screenshot-dir /tmp/prime-calc/shots \
  --screenshot /tmp/prime-calc/result.png --max-seconds 240 --max-instructions 8000000000 \
  --report /tmp/prime-calc/report.json

# M4-C differential verification (A: 2+2, B: 4, C: 2+3)
scripts/emulator/run.sh run --release --bin prime-emulator -- verify-calc \
  --firmware $FW/fat/armfir.elf --os-rom $FW/PRIME_OS.ROM \
  --factory-a /tmp/prime-factory/A --out-dir /tmp/prime-verify --lcd-watch \
  --report /tmp/prime-verify/differential.json

# M3 baseline (unchanged command; UI timers off outside key scripts)
scripts/emulator/run.sh run --release --bin prime-emulator -- run \
  --firmware $FW/fat/armfir.elf --os-rom $FW/PRIME_OS.ROM \
  --data-dir /tmp/prime-m4-baseline/data --on-unsupported zero \
  --inject-key 0x83:250 --heartbeat-min-events 5 -v 1 --report /tmp/prime-m4-baseline/report.json
```

## 11. Provenance note

Part of the M4 code was written by a parallel agent session while this
session was paused (2026-10-24 03:42–03:56): `calcverify.rs` (A/B/C
orchestration), the `verify-calc` CLI, `GuestMemory::heap_stats`, and the
`..RunConfig::default()` line in `tests/firmware_boot.rs`. Those sessions
were then stopped; their code was reviewed and kept, with the comparison
replaced by `verify.rs` (theirs panicked on a missing cluster and did not
check the expressions).

## 12. Next milestone

M5 — Program Catalog: load `.hpprgm` files into the firmware's program
storage (`C:\DATA`) and open them through the Program Catalog using the
verified key/touch input and screen capture from M4.
