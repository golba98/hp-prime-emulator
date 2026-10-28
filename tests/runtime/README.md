# Application compatibility suite (runtime)

Real PPL programs and `.hpprgm` applications, executed by the original
HP Prime G1 2.4.2 firmware under the emulator and compared with recorded
expectations. Compiler-only fixtures stay in `tests/diagnostics/`.

```
cases/        one TOML per application case (schema: crates/prime-emulator/src/appsuite.rs)
programs/     sources written for this suite (existing fixtures are referenced in place)
reference/
  local-firmware-g1-2.4.2/   checkpoint PNGs + provenance JSON recorded from local firmware
  physical-g1/               physical-calculator evidence (none recorded yet)
  masks/                     declared ignore masks (each case states why)
```

Each case records the source path and SHA-256, entry, arguments, initial
state, the input/checkpoint script replayed after function entry, the
expected outcome / return / PRINT output / runtime error / Check diagnostic,
framebuffer checkpoints, known gaps and any open defect it reproduces.

## Running

```bash
# Every case on a fresh boot, compared with the references (exit 0 = all as expected)
prime-emulator app-suite --firmware armfir.elf --os-rom PRIME_OS.ROM \
  --factory-a factory/A --init-data initial-ready/data \
  --suite tests/runtime --out-dir /tmp/app-suite [--only <id>]

# The same as an integration test (skipped, with a log line, without firmware)
PRIME_TEST_FIRMWARE=… PRIME_TEST_FACTORY_A=… PRIME_TEST_OS_ROM=… PRIME_TEST_INIT_DATA=… \
  cargo test --release -p prime-emulator --test app_suite -- --nocapture

# Compare any two framebuffer PNGs (exit 0 only for exact pixel parity)
prime-emulator frame-diff actual.png reference.png [--mask m.png] [--out diff.png]
```

`--out-dir` receives `<id>.txt` (the per-application compatibility report),
`<id>.json`, `SUMMARY.md` and, for differing checkpoints, `<id>-<label>-diff.png`.

`--record` writes references only from runs whose outcome matched every
expectation; the sidecar JSON states `provenance = "local-firmware"`, the
firmware and source hashes. Recording never produces physical evidence.

## Input and checkpoint script (`runtime_keys`)

Items are key names, `TAP:x:y`, `SWIPE:…`, plus:

- `SNAP:label`: wait for a settled screen with the guest idle, then capture.
- `DELAY:ms`: capture after a fixed time. Use it for animations and for
  programs that poll (`WAIT(-1)`, `GETKEY` loops) and so never look idle.

The `return` checkpoint is the frame at the function's return, before Home
redraws.

## Verdicts

Only exact pixel equality outside a declared mask is `MATCH`; similarity
figures describe a difference, they never excuse one. A frame that cannot be
compared (random animation) is `NOT COMPARED` with its reason. Keyboard and
touch are `MATCH` only when the scripted input produced the expected result.
`Physical G1 Test` is `NOT VERIFIED` unless `reference/physical-g1/<id>.json`
holds a physical-calculator observation; local-firmware agreement is never
reported as physical parity.
