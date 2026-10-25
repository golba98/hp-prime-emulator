# Rust repository publication and Python retirement

The public repository now saves the native editor/compiler fixes, original-firmware
emulator, shared engine integration, tests, and M2–M8 engineering reports together.
The README documents the current interfaces rather than the retired Python pipeline.

## Removed and retained

All 34 tracked files under `legacy/python` were removed after a private source
snapshot. No current Rust crate or test helper imports that implementation. Git
history preserves it. The historical migration report and pytest baseline remain
available as migration evidence, not a supported Python execution mode.

Python desktop-test assertions and maintenance tools remain in their existing
locations. Removing the old interpreter does not remove these active helpers.

Reviewed documentation is explicitly tracked without changing the existing ignore
patterns. Proprietary firmware and generated guest drives remain local. A follow-up
save includes the existing transfer ledger and hashed-device ownership receipts
in Git at the user’s request, preserving their recorded outcomes unchanged. Original firmware and sample assets were not
modified. No physical-device operations were performed for this publication.

## Publication validation

The following commands exited 0 on Linux:

```bash
cargo fmt --all -- --check
scripts/emulator/run.sh check --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings
scripts/emulator/run.sh test --workspace --all-targets
scripts/emulator/run.sh build --release --bin ppl --bin prime-emulator
cargo check -p ppl-engine --all-targets --target x86_64-pc-windows-gnu
cargo run --release --bin ppl -- check examples/HELLO.ppl
cargo run --release --bin ppl -- run examples/HELLO.ppl
cargo run --release --bin ppl -- run examples/SQUARE.ppl --engine native --entry SQUARE --args '[7]' --json
```

Workspace result: **1,131 passed, zero failed, three ignored**. Environment-gated
firmware tests in that ordinary workspace run were not counted as genuine guest
validation. The Windows target check covers the shared engine; it is not a full
Windows execution test. Windows CI retains the native workspace and excludes the
Linux-only `prime-emulator` crate. Firmware requests on unsupported platforms
return an explicit unsupported result, never a native fallback.

With `PRIME_TEST_FIRMWARE`, `PRIME_TEST_OS_ROM`, `PRIME_TEST_FACTORY_A`,
`PRIME_TEST_INIT_DATA`, `PRIME_TEST_RUNNER` and `PRIME_TEST_UNIFIED_OUT` configured
as documented in the M8 report, this additional command exited 0:

```bash
scripts/emulator/run.sh test --release -p ppl-cli --test firmware_engines \
  m8_checks_source_identity_and_arguments -- --test-threads=1 --nocapture
```

It performed nine fresh firmware checks/runs in **124.17 seconds**: HELLO accepted;
BROKEN rejected by both check and run, without runtime invocation; HELLO PRINT
output recovered; SQUARE inputs 7, 3, 0 and -4 returned 49, 9, 0 and 16; two arguments
were rejected with the expected invocation status. Source identity and guest runtime
entry/completion assertions passed. Private full reports are retained under
`scratch/research/publication-evidence/` and are not distributed with guest storage.

Earlier M4–M8 graphics, runtime-error, cancellation and reliability evidence is
recorded in the respective milestone reports. Those larger suites were not all
repeated solely for removing the unused Python directory and rewriting the README.
