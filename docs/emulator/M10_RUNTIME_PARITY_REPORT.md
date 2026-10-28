# M10 — G1 runtime parity, milestone 1

Date: 2026-10-27. Branch `feat/g1-runtime-parity` (worktree
`../hp-prime-runtime-parity`), from `integrate/diagnostic-and-location-parity`
06d4531 (M3 direct compiler 851642b + location parity de4015a/9c560e3). No
commit, push or merge was made. No physical calculator was contacted.

Firmware: G1 2.4.2, `armfir.elf` SHA-256 `ad9779ff…2fb39` (same pinned image
as M6–M9). All results below are **local firmware** observations.
**Physical-device parity: NOT VERIFIED** for every case.

## 1. Runtime architecture map

```
ppl run FILE ──► ppl-cli unified.rs ──(engine: flag ▸ PPL_ENGINE ▸ firmware if assets resolve ▸ native)
                    │ firmware
                    ▼
            ppl-engine firmware.rs ── spawns `prime-emulator run-program` in a temp dir,
                    │                  parses backend.json (identity, hashes, evidence gates)
                    ▼
prime-emulator runprog.rs
  import::prepare      transactional C:\DATA\NAME.hpprgm (host file I/O, then firmware discovers it)
  checkprog            boot ▸ Program Catalog ▸ Edit ▸ Check soft key (real key/tap events)
                       compiler hooks: compiler entry, parser source, result, dialog
  runner::run          Unicorn ARM; one host thread per guest thread; SVC services in Rust
  Driver (runprog)     Home ▸ types the call with real key events (runtime_value::invocation_keys)
                       ▸ replays runtime_keys (keys, TAP/SWIPE, SNAP/DELAY checkpoints)
  runtime::Observer    passive code hooks at fixed firmware PCs:
                         FUNCTION_ENTRY 0x306D2B48, BODY_ENTRY 0x306D2B90,
                         EVALUATOR_RETURN 0x306D171C (result + LCD frame),
                         PRINT_* (terminal text/value), ERROR_TEXT/ERROR_CONTEXT
  classify()           outcome from firmware evidence (see §2)
  display.rs           LCD frame = guest framebuffer (XRGB), captured at return/checkpoints

ppl show FILE ──► ppl-gui (IDE, firmware backend selected, firmware Check)
                    │ FirmwareClient worker thread (UI never blocks)
                    ▼
            `prime-emulator session` (framed stdin/stdout, ppl-engine session.rs)
              frames ─► GUI texture (16 ms, changed frames only)
              Key/Touch commands ◄─ GUI keyboard, on-screen keypad, mouse
              session/job.rs: CheckLoader ▸ Load (ZZT_LOADER: Programs("ZZT_IDE"):=src)
                              ▸ CheckTarget ▸ Invoke ▸ Execute (same Observer/classify)
```

Authentic firmware: PPL compile, Check diagnostics, PPL execution, graphics,
fonts, dialogs, key/touch interpretation, Home evaluation and display.
Host re-implementations: OS services (742-entry SVC registry), filesystem,
scheduler/threads, timers and clock (**wall-clock, not cycle-deterministic**),
input queue, framebuffer capture, value decoding and outcome classification.
Native Rust compiler/VM: editor feedback and `--engine native` only.

## 2. Confirmed causes of behavioural differences

| # | Symptom | Root cause | Fix |
|---|---|---|---|
| C1 | `RETURN {1,2}` reported `unsupported` (exit 8) although the firmware completed (evidence text: "…returned guest value observed"); GUI: "Firmware runtime: Unsupported" | `Observer::finish_observed` merged *did the firmware complete* with *could the host decode every value*: any undecodable result/argument/PRINT value, a hook read error or >1024 calls made a completed run `Unsupported`; a test asserted it | `runtime::classify`: completion = root-evaluator teardown with a non-error result object. Decoding gaps become **caveats**; an unreadable header (could be an error) is `Indeterminate`; no evidence is `Indeterminate`, never `Unsupported` |
| C2 | Lists never decoded | No codec for kind-6 objects | Layout derived from 7 firmware captures (`{}`, `{5}`, `{1,2,3}`, `{-1.5,1E200,0.001}`, `{"a",2}`, `{{1},2}`) and confirmed against the firmware's own Home display; bounded recursive decoder (depth 8, 4096 items, two observed subtypes only) |
| C3 | Live GUI: an animating program reported an error after 20 s while still running | The invocation's ENTER can never "settle" while the program draws; the key-script failure was reported as the run's result | Once the guest entered the function, the outcome comes from the observer, not the key script |
| C4 | Runtime input/checkpoints never delivered to programs that poll (`WAIT(-1)`, `GETKEY` loops) | Runtime key scripts first waited for an idle, settled "boot" screen | Runtime input starts at the first item (`start_with_keys`) |
| C5 | Framebuffer captures starved while a polling program runs (batch checkpoints; also live frames) | The snapshot gate only captures when no guest engine is active; a polling guest is almost never parked | Bounded fair request, only after captures were busy ≥ 100 ms: new slice entries are held ≤ 20 ms; abandoned requests expire; nested entries never wait. (A first version held on *every* busy attempt; it throttled the guest, slowed the firmware suites ~15 % and coincided with 1 stalled boot in 12 — narrowed before the final runs.) |
| C6 | Check never settled for some programs in runtime runs (LINES 3/3) | The diagnostic framebuffer-store hook (mutex + map update per pixel store) slowed redraw-heavy editor screens past the 20 s window | The compatibility harness does not enable this diagnostic; without it LINES completed 2/2 |
| C8 | Live session: after Stop, every later Run refused ("The previous firmware program is still stopping") | An ON-key break unwinds without the evaluator-return hook, so the interrupted invocation never ended | Stop records the interruption; the invocation is closed as `Cancelled` once the firmware UI pump is parked in GetEvent with the main thread waiting and no input pending for 300 ms (back at Home) |
| C9 | Stop sometimes did not break an endless loop; the next Run was refused | One ON key event can arrive while the program is between break checks and be missed (diagnosed: UI pump parked, main thread busy for 5 s). The first idle criterion also required 300 ms of *continuous* idleness, which firmware timers (183 ms) interrupt | While the interrupted program keeps the main thread busy, ON is pressed again, once a second, at most 5 times (passing runs needed 2–3). Settling uses an idle share (≥ 80 % of 500 ms) |
| C10 | Live `WAIT(-1)` touch list sometimes "not decoded" | A list with subtype 0x01 (only 0x00 and 0x20 had been observed) | Captured bytes: same layout, and the first element lives in firmware ROM. Subtype 0x01 is accepted, others still fail closed. The bounded value probe is now on by default, so every undecoded value carries its bytes |
| C11 | A new IDE calculator profile stopped at the first-run setup screen | The profile was seeded from factory A only | Seeded from the configured initialized data (A and C); existing profiles are never changed. The OS ROM is taken from the config file too |
| C7 | Published `FIREWORKS.hpprgm` cannot run | **Authentic**: G1 2.4.2 rejects `BLIT_P()` without arguments — Check at 73:10 "Error: Syntax Error" (also seen in the existing IDE scenario at 3:9) | Recorded as a compile-parity case; a derived copy with `BLIT_P(G0)` is used for the interactive run |

## 3. Program compatibility test inventory

`tests/runtime/` (separate from `tests/diagnostics/`), 20 cases, each pinning
source path + SHA-256, firmware, entry, arguments, initial state, input script,
expected outcome/return/output/error/Check position and checkpoints:

| Case | Category | Program | Input |
|---|---|---|---|
| square | arithmetic | m7/SQUARE.ppl `[7]`, repeated ×2 in one boot | — |
| fact | functions | m7/FACT.ppl `[5]` (recursion) | — |
| demo | functions | m7/DEMO.ppl (local function, PRINT) | — |
| globaltest | variables | m7/GLOBALTEST.ppl, ×3 (1,2,3) | — |
| ctrlflow | control flow | programs/CTRLFLOW.ppl (FOR/WHILE/REPEAT/IF/CASE) | — |
| hello | text | m7/HELLO.ppl (PRINT) | — |
| strcat | text | programs/STRCAT.ppl (concat, STRING, é Σ) | — |
| listret, listnest | lists | m7/LISTRET.ppl, m7/LISTS.ppl | — |
| drawtest, lines | graphics | m7/DRAWTEST.ppl, programs/LINES.ppl | `return` frame |
| waiting | keyboard | m7/WAITING.ppl (`WAIT(0)`) | `SNAP:waiting,ENTER` |
| keyecho | keyboard | programs/KEYECHO.ppl (`WAIT(-1)`) | `DELAY:1500,ENTER` |
| touchecho | touch | programs/TOUCHECHO.ppl (`WAIT(-1)`) | `DELAY:1500,TAP:100:50` |
| errzero, errlist | errors | m7/ERRZERO.ppl, m7/ERRLIST.ppl | — |
| loop | lifecycle | m7/LOOP.ppl (deadline interrupt) | — |
| bouncetest | application | existing tests/fixtures/tester.hpprgm (40-frame animation) | `return` frame |
| fireworks | application | existing tests/fixtures/FIREWORKS.hpprgm (Check only) | — |
| fireworks-g1 | application | programs/FIREWORKS_G1.hpprgm (3× `BLIT_P(G0)`) | `SNAP:title,DELAY:3000,ESC` |

## 4. Changes implemented

- `prime-emulator`
  - `runtime.rs`: `classify()`, `Running`/`Indeterminate` statuses, caveats, checkpoints
  - `runtime_value.rs`: list codec, `display()`, `ReturnRepresentation`, `PRIME_VALUE_PROBE` research dump
  - `runprog.rs`: `return_representation`/`return_display`/`caveats` in the report; `Indeterminate` fallback; unverified service IDs named; checkpoints
  - `session/job.rs`: shared classification; observer terminal state; unsettled invocation keys
  - `keys.rs`/`script.rs`: `SNAP:label`, `DELAY:ms`, `start_with_keys`
  - `memory/snapshot.rs`: bounded fair capture request
  - new `framediff.rs` and `prime-emulator frame-diff`
  - new `appsuite.rs` and `prime-emulator app-suite`
- `ppl-engine`
  - `Status::Indeterminate`
  - `return_representation`/`caveats` in the report
  - completion gate accepts an undecoded (tagged) result
  - `AssetPaths`: flag ▸ env ▸ `~/.config/hp-prime-emulator/firmware.toml`, with config-relative paths
- `ppl-cli`
  - default engine is firmware when assets resolve; `PPL_ENGINE` override; native-only options keep native
  - `Return: not decoded…` and `Caveat:` lines
  - new `ppl show FILE`
- `ppl-gui`
  - `launch_ide_with` (firmware backend + Check at start)
  - config-file assets
  - automation steps `calc_key`, `calc_touch`, `wait_running`, and the firmware LCD `screenshot`
- Tests
  - firmware: `m7_list_returns_complete_with_decoded_values`, which replaces the test asserting Unsupported
  - unit: list decoding, classification, checkpoint items, frame diff, config paths, gate fairness
  - `tests/ide_xvfb_smoke/script_firmware_runtime.json` + `assert_firmware_runtime.py`
  - `run_ide_auto.sh` no longer hard-codes the main checkout

## 5. Before/after execution results

| Request | Before (06d4531) | After |
|---|---|---|
| `ppl run --engine firmware LISTRET.ppl` | `unsupported`, exit 8, no value | `Success`, `Return: {1, 2}` |
| `{}`, `{1,2,3}`, `{-1.5,1E200,0.001}`, `{"a",2}`, `{{1},2}` | `unsupported` | decoded; match the firmware Home display |
| KEYECHO (`WAIT(-1)` + Enter) | input never delivered → timed out | `30` (HP keycode of Enter) |
| TOUCHECHO (`WAIT(-1)` + tap at 100,50) | input never delivered → timed out | `{0, 100, 50}` |
| BounceTest (existing app) | not runnable by the harness (text `.hpprgm`) | completes, returns 40, final frame |
| IDE Run of `θ:=π; RETURN θ+(π*2)` (existing session scenario) | not asserted; previously the class reported "Firmware runtime: Unsupported" | "G1 firmware completed; return value not decoded by host (tag 0x1f/0x2)" — an exact multiple of π, not yet decoded |
| IDE: Run after Stop of an endless loop | refused indefinitely | cancelled, then the next Run returns 42 |
| `ppl run LISTRET.ppl` from an unrelated cwd, no flags/env, config file only | native engine | firmware engine, `Return: {1, 2}` |

## 6. Screenshot comparison results

Fresh-boot verification against references recorded by an earlier fresh boot
(`app-suite`, no `--record`):

| Checkpoint | Result |
|---|---|
| drawtest `return`, lines `return`, bouncetest `return` | exact pixel match |
| keyecho `DELAY:1500`, touchecho `DELAY:1500` | exact pixel match |
| waiting `waiting` (Home screen) | exact match outside a 279-pixel mask (status-bar clock = host `GetSysTime`; battery-charge animation = host power status), measured with `frame-diff` |
| fireworks-g1 `DELAY:3000` | NOT COMPARED: RANDINT animation |

Exact parity and similarity are reported separately. Only exact parity outside a declared mask counts as `MATCH`.

## 7. Runtime completion and error accuracy

Second fresh-boot verification run: 19/19 cases as expected. FIREWORKS-G1 is
an expected failure (XFAIL, see §10). Completed, runtime-error (codes 4 and 24
with firmware messages), compile-error (73:10) and deadline outcomes all match
their expectations. Return values come only from decoded guest objects. An
undecoded object is reported as such, with its tag. A run without terminal
evidence is `Indeterminate`.

## 8. Test results and regressions

- `cargo fmt --check`: clean. `cargo clippy --workspace --all-targets -D warnings`: clean.
- `cargo test --workspace` (no firmware): 1459 passed, 0 failed, 4 ignored (baseline 1449/0/4).
- Diagnostic parity (`ppl-compat diagnostic_parity`): 5/5. The 75/75 verdict and 32/32 location baselines are unchanged.
- Firmware suites (release, sequential), final code, two full passes:

  | Suite | Pass 1 | Pass 2 |
  |---|---|---|
  | boot | 1/1 | — |
  | display | — (skipped after the compiler failure) | 4/4 |
  | programs | — (skipped) | 3/3 |
  | compiler | 4/5 | 5/5 |
  | runtime | — (skipped) | 7/8 |
  | direct check | 1/1 | — |
  | M8 engines | 5/5 | — |

  Both failures are the intermittent Check-UI settle stall (gap 1): a tap is "not settled within 20s", in NESTERR (compiler) and HELLO (runtime). Each of those tests passed in the other passes (compiler 5/5 ×2, runtime 8/8 ×2).
- The stall class predates this branch's changes to shared code: it was seen in CTRLFLOW, DEMO and KEYECHO before the snapshot-gate change, and the Check path is otherwise unchanged. The M8 report already lists UI time-outs as a reliability limitation.
- A same-session baseline measurement of the compiler suite is **not available**. The baseline worktree was removed outside this session while the run was queued, so that run executed this branch instead; it is not counted.
- App suite: see §7.
- Boot reliability (`m8_independent_boot_reliability`, 12 fresh boots each): baseline 06d4531 24/24 (2 runs); this branch after narrowing C5 24/24 (2 runs).
- Xvfb GUI (private display, isolated HOME/XDG/profile):
  - new `script_firmware_runtime.json`: 9/9. It covers a list return, the Enter key through the GUI path to `WAIT(-1)` (30), a GUI tap returning `{0, 100, 50}`, Stop cancelling an endless loop, and a Run afterwards (42).
  - existing scenarios still pass: `script_firmware_check` 27/27, `script_firmware_session` 31/31, native workflow/syntax/debug/gfx/input/stop/runconfig 11/11.
- `ppl show FILE` opens the IDE on the firmware backend and runs the firmware Check. Run, keys and taps go through the same live-session path the Xvfb scenario drives.

## 9. Physical verification status

None. No USB operation was performed. `tests/runtime/reference/physical-g1/`
is empty. Every report states `Physical G1 Test: NOT VERIFIED`. Producing
physical references needs an approved upload of `ZZT_*` copies of the suite
programs, a manual run, and read-only `FC` screenshots. That has not been
requested or done.

## 10. Next highest-priority compatibility gaps

1. **Intermittent Program-Editor/Check UI stalls** (the top reliability issue: about 1 in 12–30 firmware sessions in this milestone's runs). A key or tap in the Catalog or Editor occasionally never settles. The guest keeps executing: 1.6e9 instructions in 120 s for FIREWORKS-G1.
   - **FIREWORKS-G1:** reproducible 3/3 under `run-program`, while `check-program` checks the same source in 7 s. It is not caused by the runtime observer hooks.
   - **Other programs** (HELLO, DEMO, KEYECHO, CTRLFLOW): seen occasionally.
   - This blocks the interactive FIREWORKS run and causes harness flakiness.
2. **Some return values are not decoded:**
   - integers (`#…h`), observed tag 0x11/0x40, e.g. `WAIT(-1)` touch events;
   - exact π multiples, observed tag 0x1f/0x2;
   - matrices, complex numbers and other kinds.

   These are reported as completed runs with "not decoded" plus the tag; no value is invented.
3. **Time is wall-clock.** WAIT, timers and the clock are not deterministic, so animation checkpoints are not comparable.
4. **A device-thread panic hangs the batch session.** Seen once during development, now fixed at its cause. A panic should still terminate the run.
5. **Runtime source positions** for runtime errors are not yet extracted.
6. **Physical-G1 reference capture workflow** (approval-gated).
7. **Framebuffer write-count diagnostic.** It should be made cheap (per-pixel mutex) before it is re-enabled in harnesses.
