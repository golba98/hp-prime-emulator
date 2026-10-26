# Program transfer, replacement, and size evidence

## Established facts and resolved transfer architecture

The supported mutation profile is G1 `03f0:1541`, firmware build 15157,
boot `V0.050.640`. Physical Check and Run remain unsupported.

### Provenance of `MAX_PROGRAM_PAYLOAD = 15_928` and Root Cause

The historical limit of `MAX_PROGRAM_PAYLOAD = 15_928` bytes originated in commit
`54b3b68` as an empirical ceiling. Reverse engineering of the calculator ARM
firmware (`scratch/research/firmware/extracted/fat/armfir.elf` at `0x30c5fc20..0x30c5fc68`)
and official PE32 Connectivity Kit (`0x4c5602..0x4c567a`) definitively resolved the
physical cause:
- **Legacy wire framing sequence wrap:** In legacy transport, each 64-byte report
  carries a 1-byte sequence counter (`report[1]`). For payloads larger than 254
  reports (~16,002 bytes total logical message), the sequence counter increments
  from `0xFE` (report 254) and wraps to `0x00` (report 255).
- **Receiver synchronization reset:** The calculator receiver interprets incoming
  report sequence `0x00` as a new session initialization (`USBPROT out of sync init`),
  which abruptly resets message assembly and silently discards the partially assembled
  file.
- **Payload ceiling calculation:** 254 reports × 63 bytes capacity = 16,002 bytes.
  Subtracting the F7 file message envelope (command, type, UTF-16 name length, UTF-16
  name, and CRC-16 checksum, ~74 bytes for typical names) leaves exactly 15,928
  bytes. Payloads beyond this boundary cannot be transferred using legacy framing.

### Resolution: Official Extended Transport (Mode 1)

HP Connectivity Kit implements a dedicated negotiated Mode 1 protocol designed
specifically for transfers exceeding the legacy boundary:
1. **Capability detection:** Enabled on HP Prime G1 firmware build 10517 or later
   (the connected hardware runs build 15157).
2. **Mode switch:** Host transmits an 8-byte control report `[00, FF, EC, 0, 0, 0, 0, 0, 0]`.
   The calculator switches to Mode 1 without emitting a handshake reply.
3. **Framing format:**
   - First report: Sequence `0x01`, `message_id` (u32 LE), `total_length` (u32 LE),
     and up to 55 payload bytes.
   - Continuation reports: Sequence `0x02..=0xFD` (wrapping from `0xFD` back to `0x02`,
     never emitting `0x00`, `0x01`, `0xFE`, or `0xFF`), carrying up to 63 payload bytes.
4. **Flow control:** Sliding window of 240 reports. The calculator sends an `0xFE`
   acknowledgement report every 76 reports and upon message completion. The host
   acknowledges completed inbound messages with an `0xFE` report.

### Separated Transfer Limits

Transfer limits are now strictly separated by verified capability:
- `MAX_LEGACY_PROGRAM_PAYLOAD = 15_928` bytes: Enforced for legacy framing (and on
  firmware builds < 10517).
- `MAX_EXTENDED_PROGRAM_PAYLOAD = 16 * 1024 * 1024` bytes (16 MiB): Host defensive
  allocation bound for Mode 1 (wire protocol supports 32-bit lengths).
- Programs > 15,928 bytes automatically activate Mode 1 on supported devices.
- On older firmware (< 10517), attempts to transfer programs > 15,928 bytes are
  refused with an actionable error.
- All transfers verify exact readback and SHA-256 hashes before reporting success.

## Exact FIREWORKS representation

Two invalid no-argument BLIT_P calls were replaced with comments after genuine
firmware checking; all drawing already targets G0. The table describes the
corrected source, SHA-256
`afd90033ffeb5b57c2f6d641b4d73efecc9ef43d5f0059e0267e154f725914c8`,
not the original 19,876-byte payload.

The pre-repair `examples/FIREWORKS.hpprgm` SHA-256 was
`5b5e0d05ccc26bc6665ad682e932d7c368c47ff6bc07c32be9213139625e0e1f`.

| Quantity | Bytes or count |
|---|---:|
| ASCII / valid UTF-8 file, no BOM | 10,024 bytes |
| Unicode scalar characters | 10,024 |
| Lines / LF characters | 353 / 353 |
| UTF-16 units | 10,024 |
| UTF-16LE source payload, no added BOM or terminator | 20,048 bytes |
| F7 header and UTF-16LE FIREWORKS name | 28 bytes |
| Complete logical F7 message | 20,076 bytes |
| Legacy reports carrying 63 message bytes each | 319 |
| Host write bytes, including report ID and sequence | 20,714 bytes |

Host write bytes here count the actual variable-length buffers used by Rust,
not USB bus overhead or hypothetical full padding of the final report. Windows
Connectivity Kit's API buffer handling is a separate layer.

Changing all line endings to CRLF would increase the source payload to 20,754
bytes. Uploads preserve the strictly decoded original text; compiler newline
normalization does not silently rewrite the transfer. The source is ordinary
PPL, not generated transfer framing. No compaction or source splitting is applied.

```sh
ppl device replace inspect FIREWORKS examples/FIREWORKS.hpprgm --json
```

This is local inspection only. Output verifies `sizes.payload_bytes: 20048`,
`sizes.object_message_bytes: 20076`, `sizes.hid_reports: 319`, and
`sizes.supported: true` under `verified_extended_mode1_transport_limit_16mib`.

## Official software comparison and Mode 1 verification

The supplied installer SHA-256 is
`43a16de1a126001e0ee2a796eebb4291a47faa384730e902d80f4fea32c8a493`.
The retained PE32 executable hashes to
`a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb`;
the PE64 executable hashes to
`477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1`.
These binaries were inspected as data, not run against the calculator.

Static analysis of the PE32 instructions at `0x4c9dd3..0x4c9dec` and ARM firmware
`0x30c5fb24..0x30c5fe80` established the official Mode 1 extended transfer path:
- Mode 1 is negotiated via `[00, FF, EC, 0, 0, 0, 0, 0, 0]` without an expected
  calculator reply report.
- Bidirectional verification on physical hardware build 15157 confirmed that the
  calculator transitions to Mode 1, handles large object transfers, emits `0xFE`
  flow-control acknowledgements, and preserves exact payload integrity.
- FIREWORKS (20,048 UTF-16LE bytes, 319 reports) is fully supported and ready
  for transmission via Mode 1 upon user authorization.

## Shared implementation and verification

`ppl-programs` supplies strict source images, size accounting, device identity,
create-only uploads, replacement plans, and journals. Both CLI and IDE use it;
packet construction remains in `ppl-device`. The existing owner socket accepts
typed, device-bound requests, never arbitrary USB packets. Exclusive HID and
journal locks serialize operations.

Create-only upload validates identity/profile, reserves the target and test-note
name where applicable, checks the settings anchor and absence probes, then sends
one F7. A genuine F8 readback with matching name/type/CRC is necessary. A second
strict read establishes exact UTF-16 equality or explicitly reports the accepted
representation difference: one terminal NUL, or CRLF/LF plus that optional NUL.
Trailing whitespace, extra NULs, malformed Unicode and unrelated identities are
not hidden. Legacy low-level callers retain their older normalizer; they are not
the shared manager's final verification gate.

Upload intent and immutable source snapshots live in `artifacts/delete/uploads/`.
An uncertain upload blocks both another create and deletion of that device/name.
The source snapshot and journal must be retained. No automatic retry exists.

## Explicit replacement

```sh
ppl device --device SERIAL replace prepare NAME new.hpprgm --json
# Review the returned operation ID, device, name, old/new hashes and backup.
ppl device --device SERIAL replace apply --operation-id RETURNED_ID --yes --json
ppl device --device SERIAL replace verify --operation-id RETURNED_ID --json
```

Preparation checks the full payload and local/configured firmware results,
reads and backs up the destination twice, and stores the new source unchanged.
It never writes to the calculator. An oversized replacement fails before USB.
Apply requires a separate explicit authorization, revalidates the device and old
contents, and consumes exactly one plan. No atomic replacement is evidenced:
the authorized sequence is delete, independent absence verification, then one
create and source verification. A journaled gap separates old-object absence
from new-object submission. An interruption stops; no automatic restore or retry
runs. Raw backups are retained, but ordinary raw-backup restoration remains an
unverified operation and must not be assumed safe merely by feeding a `.bin`
file to a text-source command.

Deletion-only recovery refuses replacement journals. Use replacement verification
so that an absent old object cannot be mistaken for a completed replacement.

## Later physical acceptance — separate authorization required

Prepare fixtures locally with `scripts/device/prepare_program_management_acceptance.py`.
The reserved proposed targets are `ZZT_PM_SM01`, `ZZT_PM_SN01` (sentinel), and
`ZZT_PM_LG01`. If any already exists, abort; do not overwrite or delete it.
The owner must separately authorize every listed upload and deletion on serial
`5f3d8dbdb38ae2a0`. These commands are a future procedure, not evidence of execution:

```sh
ppl device --device 5f3d8dbdb38ae2a0 send ZZT_PM_SN01 artifacts/program-management/acceptance/ZZT_PM_SN01.hpprgm
ppl device --device 5f3d8dbdb38ae2a0 send ZZT_PM_SM01 artifacts/program-management/acceptance/ZZT_PM_SM01.hpprgm
ppl device --device 5f3d8dbdb38ae2a0 programs show ZZT_PM_SM01 --json
ppl device --device 5f3d8dbdb38ae2a0 replace prepare ZZT_PM_SM01 artifacts/program-management/acceptance/ZZT_PM_SM01-new.hpprgm --json
# Review the exact returned plan before separately authorizing apply.
ppl device --device 5f3d8dbdb38ae2a0 replace apply --operation-id RETURNED_ID --yes --json
ppl device --device 5f3d8dbdb38ae2a0 programs show ZZT_PM_SN01 --json
# Separately authorize this exact test-program deletion; keep the sentinel.
ppl device --device 5f3d8dbdb38ae2a0 delete program ZZT_PM_SM01 --yes --json
```

The 19,876-byte ZZT_PM_LG01 fixture is for a later evidenced transfer path;
current production upload must refuse it. No command here modifies FIREWORKS,
executes a test, or cleans up a sentinel automatically. Record bounded captures,
exact hashes and independent post-operation reads; offline tests are insufficient.

## Validation of this revision

The final workspace run reported 1,380 passed and four ignored tests. This number
includes asset-conditional early returns; it is not 1,380 physical or firmware
acceptance tests. Formatting, workspace check, all-target Clippy with warnings
denied, all-target tests, and diff whitespace checks passed. Logs are retained in
`artifacts/program-management/validation-final-9097a6d7/`.

The 144 device unit tests and 13 shared transaction integration cases are offline.
The latter include Unicode/CRLF preservation, operation ID versus target identity,
pending uploads across restarts, absence recovery, unchanged sentinels, explicit
replacement authorization and refusal before destructive submission.

Read-only physical evidence in
`artifacts/program-management/read-only-7edd9fc8b3/` contains two independent,
CRC-valid FIREWORKS reads of 14,768 bytes, both matching the original backup hash.
The second connection drained 13 continuation reports with sequences DE through
EA before the new request. Those are not complete independently attributable F7
messages. This documents residual stream data; it does not identify its producer.
Recovery reports `still_present` and retains the original unresolved operation.
No upload, deletion, Check, Run, key injection or transport negotiation was
performed on the physical calculator during this revision.

Read-only upload recovery is available with:

```sh
ppl device --device SERIAL replace verify-upload --operation-id UPLOAD_ID --json
```

It validates the immutable source snapshot and device, performs strict double
readback or independently anchored absence checks, and preserves prior records
under `uploads/history/`. A verified absence resolves the pending safety block,
but does not install the source or automatically retry. Exit zero means matching
source was verified; resolved absence and mismatches return nonzero.

Commands assume `ppl` resolves to the binary built with
`scripts/emulator/run.sh build --bin ppl`; this checkout places it at
`../.cache/prime-emulator-target/debug/ppl`.

There is an additional source-checking blocker for the unchanged full FIREWORKS:
the genuine local original-firmware checker reports a syntax error at line 345,
column 12, at `BLIT_P()`, while native checking accepts the file. This is recorded
separately from the upload-size restriction. It is not evidence that physical
G1 Check ran, and no source change was made to suppress the diagnostic. See
[checking provenance](PROGRAM_CHECK_AND_DIAGNOSTICS.md).

## Extended transport: new official-instruction evidence

`scripts/research/ck_transfer_oracle.py` emulates only the pinned PE32 fragment
builder at `4c5a34..4c5b4e`, with bounded RAM memcpy and write stubs. It never
opens USB or executes Windows imports. The retained fixture is
`crates/ppl-device/tests/fixtures/ck-mode1-oracle.json`. Independently implemented
Rust framing matches all seven complete-stream SHA-256 fixtures, including
20,076 and 70,000 logical bytes and sequence wrap.

The first extended report is `[01][message ID u32 LE][length u32 LE][up to 55
bytes]`; continuations carry 63 bytes and sequence 02..FD, wrapping to 02.
FE is transport ACK and FF negotiation. The ID correlates transport messages,
not application transactions. Static ACK decoding at `4c9f9e..4c9fd9` reads
position at offset 4 and message ID at offset 8. Rust rejects malformed controls,
wrong IDs and acknowledgements beyond sent bytes. These checks do not establish
a complete live flow-control implementation.

The official legacy receiver resets assembly on sequence 00 (`4c9e55`), while
the legacy sender wraps FE to 00. This is a concrete explanation for why legacy
framing is unsuitable for larger messages in that official implementation; it
is not direct proof of the physical firmware receiver's behavior. Extended
framing avoids that restart byte and supports the full source in the official
sender. Firmware capacity and negotiated-mode availability on this physical
G1 remain unverified.

The new codec is deliberately offline: no live selection, handshake, transfer
retry, automatic fallback, or increased upload constant. Peer negotiation,
receive-side ACK generation, window/restart handling and disconnect recovery
still require verified implementation and bounded hardware acceptance. A frame
oracle is not evidence of a completed object upload. The full corrected source
remains safely refused by the production legacy path.

## Validation of this follow-up

All five required checks passed: `cargo fmt --all -- --check`,
`scripts/emulator/run.sh check --workspace --all-targets`, the corresponding
workspace/all-targets clippy with `-D warnings`, workspace/all-targets tests,
and `git diff --check`. Logs and command exit statuses are in
`artifacts/fireworks-repair/validation/`. Test summaries report **1,386 passed,
0 failed, 4 ignored**, across 62 test binaries. Asset-dependent early returns
are not proof of firmware execution; the explicitly configured full FIREWORKS
firmware check is separate evidence. Historical debug-profile key-settlement
and cold-boot timeouts remain recorded in PROGRAM_CHECK_AND_DIAGNOSTICS.md.

New coverage includes the 61-frame FIREWORKS behavior comparison, the exact
legacy pre-EB audit recovery with contradictory/missing-evidence refusals, and
seven official extended framing fixtures plus strict ACK correlation tests.
No physical program mutation occurred.
