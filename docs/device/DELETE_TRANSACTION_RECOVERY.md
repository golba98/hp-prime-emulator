# Deletion transactions and conservative recovery

## What ZZT_D64210 means

`ZZT_D64210` is a host journal ID, not a calculator program selected for deletion.
Its single recorded target is `program:FIREWORKS` on serial fingerprint
`5f3d8dbdb38ae2a0`, G1 build 15157. `helper`, `helper_note`, and `helper_source`
are null: no ZZT_D64210 helper was installed by this operation.

The journal records preparation and authorization at 2026-10-26 13:03:04 UTC,
then unresolved verification at 13:03:06. The original double-read backup is
14,768 bytes with SHA-256
`76bd537100131e6c23179df04536121cccf51394d44d6f66b0a78aecaebe553d`.
The subsequent saved current-object read has the same size and hash.

The first stored failure is the pre-delete quiet-check refusal about a late
FIREWORKS reply, followed by a verification refusal with the same wording. In
the implementation producing that message, the failed quiet check precedes EB.
The lifecycle bug was that this pre-submission failure was returned inside a
report and then stored as a submitted, unresolved deletion. A new regression
reproduces a duplicate target response, zero EB packets, and that distinction.
There is no retained raw USB trace of the original invocation, so its complete
wire history cannot be reconstructed independently. The old record contains no
per-target USB trace; the narrow code-and-audit recovery below establishes that
this particular failure occurred before an EB write was attempted.

FIREWORKS is the expected response identity for this operation. Its appearance
does not mean that FIREWORKS was confused with a program called ZZT_D64210.
A target response proves presence at observation time, not deletion success.
Its original stale/duplicate producer remains unproven.

Read-only investigation additionally reproduced a source-read sequence reset:
`got 00, expected 9F`. A subsequent CRC-valid read matched the backup. Later
reads exposed queued reports between connections. These observations establish
stream ambiguity, not its physical cause or successful deletion. An incomplete
F9 inventory is never used as proof of target absence.

## Repairs and invariants

- Diagnostic tracing no longer consumes HID input mid-send. A mock with an
  immediate reply previously timed out only with tracing enabled; it now passes.
- Queued evidence is assembled per logical message, with sequence, length,
  padding, UTF-16 name and CRC validation. Concatenating all drained reports into
  one imaginary object is removed. Partial, unrelated and corrupt traffic cannot
  establish absence or complete a transaction.
- A framing or receive-I/O failure retires the connection. Later operations
  cannot reuse it; closing and reconnecting does not itself prove synchronization.
- Identity redaction preserves drained object evidence. Erasing an entire FA
  exchange from the log could erase the late-response safety evidence too.
- Each EB target has `not_sent`, `possibly_sent`, or `sent` transport evidence.
  A short/failed attempted write is possibly sent; it is never retried.
- All management frontends share guarded submission and persist `sent` /
  `awaiting_verification` evidence before the verification result. Authorization
  and submission intent are durable before EB. Journal write failures remain
  errors even if physical absence is observed.
- Pre-EB refusal is recorded as `not_submitted`, rather than manufacturing an
  unresolved submitted operation. A crash after intent but before a final record
  remains indeterminate; missing evidence is never inferred to mean no write.
- Valid late target replies disprove absence and produce still-present evidence.
  Wrong types/names, incomplete messages and lost anchors remain inconclusive.
- Device selection, exact name/type, unchanged double reads, backup durability,
  explicit authorization, one EB, and independent absence reads remain mandatory.
  EB has no observed application acknowledgement; host write success is not proof.

Lifecycle records retain legacy phase strings for compatibility. Submission
states are typed separately and serialized explicitly. `prepared` precedes
`submitting`; `sent` precedes `awaiting_verification`; final phases are
`completed`, `not_submitted`, or `unresolved`. A report's verification evidence
is independent of its transmission evidence.

## Recovery procedure

```sh
ppl device --device 5f3d8dbdb38ae2a0 programs show FIREWORKS --json
ppl device --device 5f3d8dbdb38ae2a0 delete verify --operation-id ZZT_D64210 --json
```

These are read-only calculator operations. Verification may append local journal
evidence. It verifies the device profile and backup, then uses exact target reads
and the settings anchor, supplemented with FA/FF identity/liveness barriers.
Known presence must not be called absence; silence without a live file handler
is not deletion. An ambiguous stream requires reconnecting and another bounded
read-only investigation, not resending EB.

The original journal, authorization, backups and audit lines must be retained.
Before adding new submission fields to an old operation, the store preserves
`record-v1-original.json`. It does not erase the old history. Journal locks prevent
concurrent recovery/submission updates. Conflicting unresolved uploads and
deletions block additional program mutations.

Legacy operations remain safety-blocked unless genuine evidence resolves
them. Current unchanged contents do not by themselves prove the original request
was never sent, nor justify deleting it again. There is no force-clear command,
automatic destructive retry, inferred ZZT_D64210 cleanup, or personal-program
mutation during investigation.

For a replacement journal, use `device replace verify --operation-id ID`.
Deletion-only recovery refuses it because old-object absence does not prove
new-program installation. Keep the backup and source snapshot through every
intermediate state. See [replacement and limits](PROGRAM_TRANSFER_AND_LIMITS.md).

## Regression and acceptance

Offline coverage includes immediate traced replies, late target replies, corrupt
or partial queued messages, multiple unrelated responses, retired connections,
no-EB preflight refusals, ignored EB, interrupted writes, device swaps, exact
content authorization, restart-persistent pending uploads, and retained backups
and sentinels. The shared transaction tests use an in-memory Transport and never
open HID. Existing verified EB bytes remain covered by the independent fixture.

Physical acceptance of the revised workflow is pending separate authorization.
Use only the exact fresh test objects and bounded procedure in the transfer
report. Never use FIREWORKS or ZZT_D64210 as experimental deletion targets.

## Current read-only recovery outcome

The revised command was run read-only on the reference serial. It returned exit
5, `phase: unresolved`, `result: still_present`, and the exact FIREWORKS backup
hash. Both source reads immediately preceding it also matched that backup with
valid CRC. The prior raw operation record was preserved as
`record-v1-original.json`; the current record contains `submission: []`, explicitly
retaining unknown legacy transmission evidence. No safety block was force-cleared
and no calculator object was deleted. Evidence is retained under
`artifacts/program-management/read-only-7edd9fc8b3/`.

## Evidence-driven resolution of ZZT_D64210

The reviewed producing revision is
`54b3b686b3e8520ffd5433d5312d032a2783d6b0`. Its single-target control flow
combines a pre-EB `refused: ... exists ... reply arrived late` error with a
separate `; verification: refused: ...` error. If EB had returned successfully,
the error would instead begin `verification:`. A partial/failed write would
carry a transport error. This exact earliest terminal audit therefore proves
no EB attempt through the reviewed implementation, despite lacking a raw trace.
Unchanged contents alone would not prove this.

`legacy_no_send.rs` checks the original version-1 metadata, exact target and
device, immutable backup, complete journal ordering, absence of any second
submission or contradictory modern evidence, this precise original error, and
two fresh strict reads identical to the backup. Verification-only errors,
partial-write errors, changed contents and repeated submission remain blocked.
It preserves original records and writes `legacy-no-send-evidence.json` with
reviewed Git blob IDs, record/journal hashes and the original audit prefix.
The proof is explicitly code-and-audit evidence, not a fabricated USB capture.

The subsequent read-only recovery on the reference device resolved the record
as `phase: not_submitted`, `result: verified_not_performed`,
`submission_evidence: ["not_sent"]`, with FIREWORKS still present and matching
its original backup. Exit 5 means no deletion was performed. The report no
longer requests recovery or blocks on this false classification; repeated
verification preserves the resolved state. No EB, F7 upload or remote key was
sent. The earlier unresolved report above remains historical evidence.
