# Physical scenario capture (M16)

The `ppl-compat::capture` module and `ppl capture` CLI prepare, retain and
compare stateful scenario evidence offline. They do not open USB, upload a
program, inject keys, change calculator settings, delete objects or run a
physical calculator. No new physical measurements were made for this work.
The existing 77 historical physical references remain in their original
evidence format and retain their original observation scope.

## Offline commands

```sh
ppl capture prepare tests/differential/state.toml \
  --capture-id G1_SESSION_001 --output /private/capture-plan.json
ppl capture seal --plan /private/capture-plan.json \
  --recording /private/recording.json --artifacts /private/raw \
  --output /private/sealed-session
ppl capture validate /private/sealed-session \
  --fixture tests/differential/state.toml
ppl capture compare tests/differential/state.toml /private/sealed-session \
  --backend-run /private/native-or-hle-backend-run.json
```

`prepare` consumes the same versioned fixture as `ppl differential`. Its plan
contains the resolved source, full scenario specification, initial state,
firmware constraints, expectations and exact ordered input actions. Prepare
refuses to overwrite an existing plan. `compare` accepts a single structured
`BackendRun` from an executing Native/HLE/future full-system backend, not a
whole differential report. It compares all captured semantic fields using
the existing differential comparison engine. Types, strings, errors, lists,
matrices, state and pixels retain their exact representation; there is no
new whitespace, numeric or error normalization.

Schema-2 named programs are frozen as resolved source text, alongside the
shared initial/default/edit source-hash manifest. Their aggregate identity
covers every named program and edit. Validation never reopens a captured
relative source-file path. Missing/extra or changed secondary sources and
changed source manifests fail validation. Schema-1 single-program scenarios
and initial schema-1 capture plans retain their original source identity.

CLI exit codes are 0 for a valid/matching result, 1 for a semantic mismatch,
and 2 for invalid input or infrastructure errors. JSON stdout includes the
verification category and detailed backend, step and JSON-pointer mismatches.
All four commands perform local filesystem I/O only.

## Capture schema 1

The public Rust types are the schema definition; unknown document fields are
rejected. A `Recording` binds its capture ID and plan SHA-256 to:

- A `DeviceRecord`: model, G1 PID, firmware build, boot version and salted
  serial fingerprint. Manually physical captures require two distinct raw
  reassembled FA01 replies retained before and after the sequence. Their
  build/version/serial fields must agree with the device record. These raw
  replies contain the serial number: keep bundles private or review them
  before sharing.
- UTC capture time in `YYYY-MM-DDTHH:MM:SSZ` form. Invalid calendar dates fail.
- An initialization confirmation bound to the exact source SHA-256. The
  operator confirms source import, specified settings and authored setup.
- One ordered `CapturedStep` per planned action, including action SHA-256,
  manual-delivery confirmation, elapsed milliseconds, exact observation data
  and artifact names. A read-only confirmation is permitted for a screenshot
  capture action. Missing, duplicate, reordered, modified or undelivered steps
  fail validation.
- Artifact kind, filename, byte length and SHA-256. Every artifact must be
  referenced by its observation or the identity evidence list.

Result notes or user-confirmed transcripts contain UTF-8 JSON with this
`ObservationTranscript` shape:

```json
{
  "schema_version": 1,
  "capture_id": "G1_SESSION_001",
  "source_sha256": "<64 lowercase hex characters from the plan>",
  "step": "first",
  "action_sha256": "<SHA-256 of compact JSON for the exact planned action>",
  "data": {"status": "success", "return_value": {"type": "real", "value": "1"}}
}
```

The artifact bytes are retained exactly and the parsed fields must equal the
recorded observation. A result note in another encoding or legacy harness
format needs an explicit adapter; the new module does not silently reinterpret
legacy note bytes. Human transcription uses `user_confirmed_transcript` and
must be reviewed by the operator. Retain the original screenshot alongside
the transcript when it is the source of a transcribed result.

Graphics observations require a retained screenshot whose decoded pixels
exactly equal the observation's `image` field. `screenshot_png` is a normal
PNG. `prime_screenshot_png` is HP's original packed RGB555 FC08 PNG and uses
the existing dedicated decoder. Do not relabel packed screenshots as normal
grayscale PNGs. A screenshot alone establishes only status and pixels; other
values/state require a correlated result transcript.

Physical firmware build constraints are checked. A physical firmware SHA-256
constraint is refused: FA reports a build, not a hash of installed firmware.
The capture layer does not pretend it can establish that hash over USB.

## Scripted collection interface and device safety

`collect(plan, time, operator, reader)` sequences initialization, manual action
confirmation and read-only observation collection. `ManualOperator` returns
explicit confirmations or cancellation. `CaptureReader` exposes identity,
observation and retained identity evidence only; it has no write, delete or
key-injection method. The collector rechecks device identity after every
observation and aborts on changes. Controlled readers exercise this flow in
normal CI without requiring hardware.

There is no concrete live USB scenario reader in this increment. The next
binding should reuse `ScreenshotSession`'s exclusive transport ownership and
existing read-only FA/F8/FC operations. It must retain first/last FA replies
and correlate probe results to this capture/source/action. A failed screenshot
session must be closed; stale responses cannot be reused as another command.

Program preparation/upload continues through the existing guarded compare
workflow and requires separate physical-device authorization. Use new
disposable `ZZT_*` objects, preserve the append-only name ledger, refuse
existing names and uncertain retry, and never automate deletion. Physical
Run/key/touch/power actions remain manual. EC remote input is not promoted
to a working scenario transport: delivery on firmware 15157 remains unverified.

## Integrity, publication and bounds

A sealed capture has `capture.json` and an `artifacts/` directory. The manifest
contains a versioned payload and SHA-256 of its canonical JSON encoding. Each
artifact has its own size/hash. Validation rereads every retained artifact.

Publication reserves a new directory, writes and flushes artifacts, then
atomically publishes the final manifest without overwriting. Directory
metadata is flushed on Unix. A termination before manifest publication leaves
an incomplete directory that cannot validate or be reused. Existing bundles
and incomplete directories are never overwritten. Filesystems without Unix
directory syncing provide weaker power-loss durability; checksum validation
still fails closed on incomplete data.

Limits are 1,024 steps, 256 artifacts, 8 MiB per artifact, 32 MiB total artifact
data and 16 MiB per JSON document/source. Artifact names are flat ASCII
filenames; absolute paths, path separators, leading dots and parent traversal
are refused. Artifacts must be regular files; leaf symlinks and a symlinked
sealed artifact directory are refused. Input directories are explicit caller
paths and validated files are copied into the new bundle. PNG dimensions must
be 320×240 before decoding, preventing allocation through forged dimensions.

Checksums detect corruption and bind the collected components together. They
are not a hardware signature, and they cannot prove that a human attestation
is truthful. A malicious author able to replace all evidence can recompute
hashes. This is the same fundamental trust boundary as the existing manual
physical-evidence workflow, with stricter source/action/raw-identity binding.

## Verification categories

`classify_execution` derives Native-only, HLE-verified or Firmware-versus-Native
from successful executing-backend comparisons. `classify_historical` labels
successful qualified legacy replay as Historical physical replay; it adds no
new measurements. `compare_capture` grants Newly physical-verified only to a
matching, validated manually physical capture with complete identity/delivery/
raw-observation evidence. It never grants that category to a controlled mock,
even when a mock reproduces a Native result exactly. Failed comparisons and
mock infrastructure checks have category Untested, with their actual attempt
and mismatch information retained in the report.

The categories describe verification strength, not broad calculator parity.
A newly verified result covers only the source, firmware build, actions and
observations recorded in that scenario. HLE results, historical physical replay
and newly captured physical observations remain separate provenance classes.

## Validation performed

Focused tests use real Native execution as input to an explicitly controlled
mock reader. They cover capture/validate/compare, exact graphics, typed-result
mismatches, source edits, stale transcripts, identity changes, cancelled input,
incomplete/changed steps, path rejection, corruption, interrupted publication,
symlinks, PNG allocation limits and immutable plan publication. Raw FA parser
tests use synthetic bytes and produce no physical evidence. CLI tests verify
offline preparation, overwrite refusal and incomplete-bundle errors. A real
subprocess CLI test exercises prepare → seal → validate → compare with relative
paths and controlled mock observations; it covers a publication bug where a
bare relative output name caused a spurious parent-directory sync failure.

The capture-specific checks comprise 21 unit tests, two CLI command tests and
one subprocess CLI cycle. Formatting and `cargo clippy -p ppl-compat -p ppl-cli
--all-targets -- -D warnings` pass. The initial single-program checkpoint also
passed all 113 `ppl-compat` tests. After schema-2 integration, the ordinary
`ppl-compat` run passes 123 tests. That run does not enable the optional real
firmware scenarios; actual M16 workspace/firmware validation is recorded
separately by the coordinator.

No new physical-device claim, automated USB capture, full-system backend or
cryptographic hardware attestation is implemented here.
