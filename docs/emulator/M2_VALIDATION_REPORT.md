# M2 Validation Report

## Test commands and results (all actually executed)

| # | Command | Result |
|---|---|---|
| V1 | `cargo fmt --all -- --check` | **PASS** (clean; one `cargo fmt --all` applied mid-milestone) |
| V2 | `scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings` | **PASS** (0 warnings, includes the new crate; the wrapper's space-free `CARGO_TARGET_DIR` is required — see §build) |
| V3 | `scripts/emulator/run.sh test --workspace --all-targets` | **PASS** — 30 suites, 0 failures; includes all pre-existing PPL tests (no regression) and the 26 new tests |
| V4 | Firmware integration test (env-gated): `PRIME_TEST_FIRMWARE=... cargo test -p prime-emulator --test firmware_boot` | **PASS** (30M instructions, 3,026 SVCs, 59 IRQ deliveries; asserts entry `0x30604AE8`, ≥5M instructions, >1000 SVCs, IRQ delivery; skips without env) |
| V5 | Release firmware run, 240 s | **PASS with limits** — 229M instructions, 0 faults, steady idle loop; heartbeat not reached (documented) |

## New synthetic tests (26)

- **loader.rs (7):** valid ARM ELF parse (entry/segments/bytes); bss >
  file data; rejects non-ARM machine, non-EXEC type, truncated files,
  memsz < filesz, 32-bit overflow segments. Uses a programmatic synthetic
  ELF builder — no firmware needed.
- **memory.rs (11):** map/read/write roundtrip; unmapped and
  cross-boundary access rejection; overlap and alignment rejection;
  bounded C/wide string reads (unterminated = error, never unbounded);
  heap alloc/free/coalescing; unknown-pointer free rejection; 64 KiB
  aligned allocation; page-round helper.
- **svc.rs (7):** verified name-table integrity (742 entries, round-trip,
  21 unknown IDs stay nameless, spot checks against Phase 1 evidence);
  registry status marking; stack-arg convention (`[sp+8]`); a real SVC
  round-trip through a Unicorn engine using the firmware's exact stub
  encoding (`push {r0}; push {lr}; svc`), asserting handler invocation,
  `r0` result, and `sp += 8`; strict policy stops on unsupported with the
  right ID; zero policy continues and returns 0; the firmware yield
  idiom (0xEE17FF7E) sets Z and execution continues.
- **firmware_boot.rs (1):** env-gated real-firmware run with hard
  evidence gates (see V4). Firmware is never bundled.

## Known limitation found by tests

The firmware's `push {r0}` uses register-list encoding `0xE92D0001`; an
early synthetic test used `0xE92D0000` (empty list = invalid) — the test
itself caught it, which also validated the engine's invalid-instruction
fault path.

## Pre-existing issues

None observed: the workspace was green at baseline (2026-10-23) and
remains green with the new crate added.

## Build-reliability validation (Phase 1 findings F-1/F-2)

- F-2 (`-latomic`): solved in `crates/prime-emulator/build.rs`; verified
  on every build of this milestone.
- F-1 (spaces in repo path break Unicorn's CMake): solved without
  renaming or moving the repository via a space-free build dir outside
  the repo:
  - `crates/prime-emulator/.cargo/config.toml` (invocations from inside
    the crate) and
  - `scripts/emulator/run.sh` (any invocation from the repo root).
  Both point at `<repo>/../.cache/prime-emulator-target` (space-free),
  leaving the existing `target/` and all other crates untouched. All
  quality gates above ran through the wrapper. Dev builds and release
  builds verified.
