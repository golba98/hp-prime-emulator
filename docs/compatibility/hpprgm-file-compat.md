# Independent M5 — HPPRGM file and import compatibility

Implementation branch: `feat/hpprgm-file-compat`, isolated worktree
`hp-prime-file-compat`. Base: `06d45318a1c0dc2ca21e575f04a682aa78d893d6`,
the clean diagnostic/location-parity integration checkpoint. Corpus commit:
`8578fcc`; importer/strict-decoding correction commit: `eeeb4f8`.
No changes were made in the integration or runtime worktrees. No USB command,
physical calculator access, merge, push or pull request was performed.

## Format and loading audit

| Area | Baseline implementation and evidence | M5 disposition |
|---|---|---|
| Recognition | Binary signature `7c 61 8a b2`, followed by `fffffffe` and `00000000` little-endian words. Extension alone is not format evidence. | Keep signature detection; a recognized broken container never falls back to source recovery in the validated path. |
| Binary structure | Length-prefixed typed objects and list groups. Header values, caches, table slots and padding were established by firmware-generated files in the earlier M5 report. | Preserve imported container bytes exactly, including unknown objects. Do not manufacture compiled caches. |
| Text input | Firmware importer strictly decodes UTF-8 or BOM-marked UTF-16LE/BE. `.hpprgm` text requires explicit source mode. | Preserve that policy. Source viewing is independent of import permission and catalog naming. |
| Legacy syntax loader | Infallible `decode_source` uses lossy UTF-8, UTF-16 marker searches and stops at junk/surrogates; malformed containers can fall back to heuristic extraction. | Add `decode_program`/`decode_text` with typed errors. Engine loading and importer text decoding use the shared strict implementation. Keep/document the legacy API for its existing callers; it is not a compatibility oracle. |
| Unicode | Container source is NUL-terminated UTF-16LE; supplementary characters require surrogate pairs. | Strict decoding rejects malformed pairs and data after the terminator. Genuine firmware save/reopen tests cover BMP and supplementary characters. No Unicode normalization is applied. |
| BOM/endings | Text import removes an encoding BOM and normalizes CRLF/CR to LF. Raw container import does not rewrite its source. | Expose BOM, original text, normalized text and the line-ending transformation. Firmware testing includes a binary CRLF source that stays exact across saves. |
| Names | Name is derived from the file stem or explicit override, not from an EXPORT declaration or the internal `Main` source-file name. | Preserve program identity and capitalization. Invalid stems require explicit names for import; they do not prevent strict source decoding. |
| Name assumptions | ASCII-letter initial, ASCII letters/digits/underscore, at most 31 characters. Earlier firmware evidence covered a leading digit and the 31/32 boundary. | New firmware evidence confirms lowercase names and 31-character catalog names. Non-ASCII, spaces, underscore and reserved-word acceptance are not all firmware-probed; conservative host restrictions remain explicit policy. |
| Catalog | Authentic firmware enumerates files at boot. A host directory listing is not the firmware catalog. | Retain existing firmware catalog/editor proof and report `catalog_refresh: "reboot-required"` after host-file import. |
| Persistence | Editor ESC causes a firmware save; there is no fabricated catalog index. | Verify exact saved source on two boots, and a real editor modification followed by two more boots. |
| Limits | Container lengths are u32; source and files are loaded into host memory. No firmware maximum was established. | Test 64 KiB and 256 KiB authored sources. Do not equate USB transfer boundaries with program limits. More than 128 nested metadata lists is explicitly refused as a host resource policy. |
| Paths | Import accepts PathBuf paths; older verification examples often assumed repository cwd. | Corpus paths anchor to `CARGO_MANIFEST_DIR`; Source::file retains an absolute identity. Test spaces and non-ASCII parent directories and explicit filename overrides. |

The source parser accepts exactly one source-file group. Multiple exported
functions are supported and tested; multiple internal source-file groups are
not established as supported. Unknown non-source objects are preserved, but
host acceptance alone does not establish firmware compatibility for them.
Encrypted, tokenized, protected and other unrecognized formats are not claimed
supported. Unsupported files must not be heuristically recovered for execution.

## Confirmed defects and corrections

1. A partial list-group header could read bytes outside its containing object,
   then panic on unsigned subtraction. Reproduced at baseline through
   `inspect-program` (exit 101). Both parser copies now check the remaining
   group-header bytes before reading. Every prefix and thousands of byte
   mutations are exercised without panics; excessive nesting fails safely.
2. Twelve concurrent case-variant imports produced six successful reports,
   two host files for one guest identity, temporary-file errors and false
   `other_files_unchanged` results. Profile-wide interprocess exclusion,
   unique temporary files and no-clobber installation now ensure one winner.
   Thread and separate-process regressions exercise the actual importer.
3. `--replace` selected an arbitrary entry when both `HELLO.hpprgm` and
   `hello.HPPRGM` existed. Such ambiguous identities are now refused before
   modifying either file. Non-regular replacement targets are also refused.
4. Engine and importer had separate strict text decoders while another public
   decoder silently recovered malformed data. The validated paths now share a
   typed strict decoder, retaining raw/normalized views and existing interfaces.
5. Snapshot errors could be silently omitted while claiming unrelated files
   stayed unchanged. Transactional imports now use an error-propagating
   snapshot; the existing best-effort reporting API stays available.

The portable lock is an empty `.prime-import-lock` directory outside guest C:.
Normal exits remove it. After a crash it deliberately remains locked: verify
that no importer is active, then remove only that empty directory before
retrying. Lock ownership is never inferred from a timeout. This lock coordinates
cooperative importers, not running firmware or arbitrary filesystem writers.
Validation/conflict failures write no program. An I/O error after installation
may leave a complete program installed; inspect the final path before retrying.
No-clobber is the default; replacement remains explicit.

## Evidence and testing

All observations here are **local-firmware**, not physical-device VERIFIED.
User-supplied proprietary assets were read only; each firmware execution used
an isolated copied profile. The original assets and downloaded programs were
not committed. Firmware identity and sanitized per-case evidence are stored in
`tests/hpprgm/reference/local-firmware-g1-2.4.2.json`. Raw profiles, filesystem
traces and screenshots stay under ignored `artifacts/`.

Firmware: HP Prime G1 2.4.2.

- `armfir.elf`: `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`
- `PRIME_OS.ROM`: `f6aeba161189a5aa1acb57be428dc5730cfa5ff7cd534494e59225debd5a8999`
- `PRIME_APP.DAT`: `c341d7a6450cdfb050665fea65eedf270c8c09e2fbe3c275b3d99300beb7efba`

A binary import must match its original byte hash. A firmware save need not:
the firmware derives export flags/caches and may change name64 padding.
References retain the raw saved hashes, actual exports and stored-source hashes.
Source equality is checked independently of these serialization differences;
raw save hashes are evidence, not stable expectations across emulator runs.
The offline reference test rejects stale fixture hashes, source changes or
shrinking firmware coverage. It does not manufacture a new firmware verdict.

The editor scenario changes `RETURN 1;` to `RETURN 2;` with ordinary keys,
saves, reboots twice, reopens and saves again. It checks exact source and the
unchanged bytes of a separate sentinel program. This is not simulated host-file
editing. File transport validity is tested separately from PPL syntax validity.

## Interfaces for `ppl show` and live import

`ppl_syntax::source::decode_program(&[u8]) -> Result<DecodedProgram, FileError>`
is the filename-independent strict entry point. `decode_text` is explicit text
mode. DecodedProgram supplies original_text, LF-normalized text, encoding,
BOM presence, container identity, source-object offset and the line-ending
transformation. The offset is the object's length-field offset (or section
when there is no source string), not the first UTF-16 code unit.

`ppl_engine::source::Source::file` supplies the existing absolute identity,
original bytes and hashes, original_text, compiler text and firmware_text.
`ppl show` should use original_text for faithful viewing and expose normalized
text only as an explicit choice. It must not require a valid catalog name or
use the legacy recovery decoder. Import uses ImportRequest/prepare/import;
prepare validates without writing or initializing a profile. CLI flags and
runtime dispatch were intentionally not changed.

The older GUI editor, legacy file adapter and compatibility-export decoder
still call the infallible recovery API. Their owners should migrate to the
fallible API and present decoding failures before allowing save. They are not
covered by this milestone's strict-path guarantees. Existing runtime temporary
name fallback is likewise unchanged; the standalone importer requires explicit
valid names, as selected for M5.

The [compatibility matrix](../../tests/hpprgm/COMPATIBILITY.md) reports all 50
files: 28 host imports accepted, 22 rejected by explicit format/name/source
policy, and 26 genuine two-boot firmware observations. All 26 preserve exact
stored source; the fresh capture agrees with the initial capture's source
hashes. Only the largest tested sizes are claimed, not a maximum.

Live probing established three separate facts. Host-file installation did not
change the running catalog's pixels. Ordinary firmware Editor Check, Home
invocation and `Programs("NEWLIVE"):=source` created a new program visible in
the authentic catalog; it subsequently reopened in the editor after reboot
with exact assigned source. The existing managed session Run command instead
failed before execution with "The current firmware catalog order could not be
verified (observed [], expected 3 programs)". That is an infrastructure failure,
not a firmware rejection. The probe records it separately; no session, guest-call
or catalog-order implementation was changed.

A future runtime command could expose validated import requests and return
the actual catalog identity, saved-source hash and registration/save evidence.
The standalone importer remains boot-time oriented and explicitly reports the
restart requirement. The future runtime interface must reject collisions before
assignment and use firmware-owned registration. Raw binary metadata must not be discarded silently by a
source-only live bridge. Integration into runtime/session ownership is deferred.

## Reproduction

Use a space-free, dedicated target directory; do not share the runtime agent's
Cargo output. Set these paths to absolute paths to your local assets:

```sh
export CARGO_TARGET_DIR=/tmp/hpprgm-m5-target
export PRIME_TEST_FIRMWARE=/absolute/path/to/armfir.elf
export PRIME_TEST_OS_ROM=/absolute/path/to/PRIME_OS.ROM
export PRIME_TEST_FACTORY_A=/absolute/path/to/factory/A
export PRIME_TEST_INIT_DATA=/absolute/path/to/program-free-initialized/data
export PRIME_FILE_COMPAT_OUT=/absolute/path/to/fresh/artifacts/m5-capture

cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p prime-emulator --release --test firmware_file_compat -- --ignored --test-threads=1
cargo test -p prime-emulator --release --test firmware_boot --test firmware_display --test firmware_compiler --test firmware_programs -- --test-threads=1

export PRIME_FILE_COMPAT_RUNNER="$CARGO_TARGET_DIR/release/prime-emulator"
python tests/hpprgm/live_probe.py --out /absolute/path/to/fresh/artifacts/live-probe
```

Use `summarize.py --capture CAPTURE_DIR --live LIVE_DIR/summary.json
--runner-commit COMMIT --runner RUNNER --output NEW_REFERENCE.json` to prepare
sanitized reference data. Review any change before replacing the checked-in
reference. Never publish the raw profile tree or saved heap padding. Firmware
results do not establish compatibility with an untested firmware version.

## Final results

- Workspace: **1,458 passed, 0 failed, 6 explicitly ignored**. Optional
  environment-guarded firmware tests are counted separately below.
- Enabled existing firmware regressions: **13/13 passed** (boot 1, compiler 5,
  display 4, catalog/persistence 3).
- Enabled M5 firmware tests: **2/2 passed**, covering 26 files on two boots plus
  the editor modification and two subsequent reboots. Fresh run: 333.84 s.
- Formatting, Clippy with warnings denied, fixture reproduction and diff checks
  passed. Malformed inputs, source/reference hashes and thread/process import
  races pass the permanent regression tests.

Machine-readable check results and local log hashes are in
`tests/hpprgm/reference/checks.json`. Raw evidence from this run is retained in
`artifacts/hpprgm-final-capture/` and `artifacts/hpprgm-live-ui-final/` in the
feature worktree. These directories are intentionally not tracked. No production
runtime, scheduler, key/touch, framebuffer, result-extraction or SVC code changed.
