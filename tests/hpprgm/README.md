# HPPRGM file compatibility corpus

50 authored file fixtures cover binary containers, text encodings, source
fidelity, path/name handling, unedited programs, caches, unknown metadata,
large inputs, malformed lengths, truncation and invalid Unicode. Expected
source is stored separately under `sources/`, with its original line endings.
`cases.json` pins every input's size and SHA-256, expected representation,
explicit source mode, catalog-name override and host-import outcome.

No firmware binary or downloaded program is distributed here. `generate.py`
constructs the documented object layout independently of the Rust serializer.
Reproduce the fixtures with `python tests/hpprgm/generate.py`; this must leave
the tracked corpus unchanged. The largest authored source is 262,189 UTF-8
bytes (262,189 UTF-16 units for its ASCII text); this is a tested size, not a
claim about firmware's maximum.

Host regression tests live in `crates/prime-emulator/tests/file_compat.rs` and
`import_transactions.rs`. The corpus test resolves files from
`CARGO_MANIFEST_DIR`, independent of the process working directory. Duplicate
and case-variant names, replacement, concurrent processes, stale temporary
files and an interrupted import lock are exercised with isolated profiles.

`firmware_file_compat.rs` contains explicitly ignored local-firmware tests:
a 26-case import/editor-save/reboot capture and a genuine editor modification
followed by two reboots. They require absolute `PRIME_TEST_FIRMWARE`,
`PRIME_TEST_OS_ROM`, `PRIME_TEST_FACTORY_A`, `PRIME_TEST_INIT_DATA` and a fresh
absolute `PRIME_FILE_COMPAT_OUT`. Run with `--release -- --ignored
--test-threads=1`. Raw profiles and screenshots belong under ignored
`artifacts/`, never in this corpus. Firmware saves may include stale heap
padding; only sanitized results and hashes should be published.

Use a dedicated, space-free `CARGO_TARGET_DIR` when working alongside other
agents. The workspace's default target directory is shared between worktrees.
Firmware observations are `local-firmware`, never physical-device VERIFIED.
