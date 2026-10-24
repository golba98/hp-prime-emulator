# Unattended deletion on HP Prime G1

**Physically verified on 2026-10-23:** G1 `03f0:1541`, firmware build
`15157`, boot `V0.050.640`. Programs and notes can be deleted directly over
USB, without opening the Program Catalog or pressing Run.

```sh
ppl device send ZZT_TEST ./test.hpprgm
ppl device delete program ZZT_TEST --json

ppl device send ZZT_NOTE ./note.txt --note
ppl device delete note ZZT_NOTE --json
```

`send` retains its existing typed name confirmation and guarded creation
checks. Deletion itself requires no prompt for a matching creation receipt.
The physical demonstration used these exact names with fresh objects, then
verified each absence on a new USB connection. No helper was uploaded or
executed. An unrelated, freshly created sentinel note remained unchanged.

## Mechanism and safety

The preferred operation is Connectivity Kit's native `EB` named-object
management message, **not PPL execution or UI automation**. Its serializer
was traced in Connectivity Kit 2.4.2 r15751 and then tested on the physical G1.
See [protocol evidence](../reverse-engineering/usb-protocol.md#5-native-usb-object-deletion-eb--verified-2026-10-23).
The earlier conclusion that there was no USB delete command was based on
incomplete public command tables and is superseded by this evidence.

The workflow is:

1. Select and lock the calculator; read its identity and firmware profile.
2. Validate exact `ZZT_*` names, types, count, and protected controls.
3. Fetch each target twice with CRC validation and unchanged contents.
4. Match the device/type/exact-name/content creation receipt. A prefix or
   legacy name-only ledger entry is insufficient for unattended deletion.
5. Sync immutable backups, authorization, and a prepared operation record.
6. Sync the submitting state, recheck live control and target contents,
   then send each exact deletion once.
7. Establish absence through two independent `F8` rounds, bracketed by
   CRC-valid control replies. Late drained replies invalidate absence.
8. Sync outcomes and retire the corresponding creation receipts.

Wrong names, changed content, missing backups, invalid CRCs, stale replies,
and lost controls cannot become successful deletion results. Failed or
uncertain submissions are never automatically retried. Other firmware
profiles fail with recovery instructions rather than injecting keys or
silently entering the manual workflow. `ZZT_CTRL` and the configured
control are always protected. At most 32 targets are accepted per request.
Do not edit objects or run another USB client concurrently.

Creation receipts live under `RUNS_DIR/ownership/SERIAL_ID/`. They are
written from an additional exact, CRC-valid device read after successful
`send`, and bind to the observed stored bytes. Legacy callers may still
explicitly authorize a backed-up `ZZT_*` object by typing its exact name in
an interactive terminal. JSON and piped deletion never prompt for missing
ownership. Unrelated personal objects are outside the supported namespace.

## Result and recovery

A successful direct deletion returns exit 0 and one JSON object:

```json
{
  "schema_version": 1,
  "operation": "delete",
  "mechanism": "usb_delete",
  "backup_created": true,
  "deletion_confirmed": true,
  "execution_confirmed": false,
  "helper": null,
  "helper_uploaded": false,
  "cleanup_state": "not_required",
  "error": null
}
```

The full output includes `operation_id`, device fingerprint, backup directory,
per-target status and SHA-256, and recovery instructions. Partial success is
preserved per target. A transport write is never described as execution.
Nonzero results can still contain confirmed deletions: inspect all fields.
Invalid CLI syntax returns exit 2; operational refusals/I/O failures return 5.
Deletion JSON also covers argument-parsing failures; help remains ordinary text.

Operations are stored in
`artifacts/delete/operations/OPERATION_ID/operation.json`, with immutable
`program-NAME.bin` / `note-NAME.bin` backups and `authorization.json`.
`artifacts/delete/journal.jsonl` retains the audit trail. Operation IDs use
`ZZT_Dxxxxx` but do not imply a calculator helper exists.

```sh
# Read-only reconciliation; never reissues deletion or execution.
ppl device delete verify --operation-id ZZT_Dxxxxx --json

# Existing independent-query interface remains supported.
ppl device delete verify program ZZT_TEST --json
ppl device delete verify note ZZT_NOTE --json

# Select a calculator using its cryptographic ID from device status.
ppl device delete program ZZT_TEST --device SERIAL_ID --json
```

Keep the operation store and its backups together. Recovery refuses records
for another calculator, malformed records, and corrupt backups. A conflicting
unresolved operation blocks a new request for the same target. Read-only
verification may establish a delayed outcome; it does not cancel an operation.

## Manual helper compatibility

The established PPL catalog-assignment helper remains available explicitly:

```sh
ppl device delete program ZZT_TEST --manual-run
ppl device delete note ZZT_NOTE --prepare-only --json
ppl device delete verify program ZZT_TEST --helper ZZT_Dxxxxx --json
```

`--manual-run` requires the existing single Run press. `--prepare-only`
uploads and reads back the helper, without sending `EB`, remote keys, or
performing deletion. Its successful preparation is not deletion or execution.
New helper sources use `@@PPLDELETE|HELPER|working` and
`@@PPLDELETE|HELPER|done` markers; old `done` notes remain readable by legacy
verification. The new marker path is covered by offline tests; the original
manual helper was physically verified in the earlier `ZZT_DELTP1` and
`ZZT_DELTN1` tests. No new automatic helper execution is claimed.

Remote Home key injection was retested harmlessly. Catalog screenshots
remained pixel-identical below the clock/status bar. No program invocation
was attempted through keys. `device run --remote` and `device exec "2+2"`
remain gated by their existing unverified execution capabilities.

## Temporary object lifecycle

Normal direct deletion creates **no calculator helper or evidence note**.
There is consequently no recursive cleanup cycle or accumulating evidence.

For manual helper operations, keep the terminal note until targets are
independently absent. Archive its exact contents and SHA-256 locally before
cleanup, then delete only the recorded note and any verified matching helper.
Raw cleanup backups are synced under the operation's `cleanup-backups/`.
Cleanup uses `EB`; it creates no further helper or evidence object.

```sh
ppl device delete cleanup --operation-id ZZT_Dxxxxx --json
```

Unresolved operations and changed artifacts are retained. If cleanup was
interrupted after submission, another cleanup call first reconciles absence;
it never resubmits a still-uncertain command. `deletion_confirmed` and
`cleanup_state` remain separate. New manual helpers can be cleaned once their
terminal evidence is archived; read-only `verify` itself performs no cleanup.

Old result notes, including `ZZT_D33C29R` and `ZZT_D24731R`, can be imported
only through an explicitly named cleanup operation. Migration requires the
original same-device prepared event, matching archived target hashes, an
independent successful historical verification, fresh absence of the old
target and helper, and an unchanged CRC-valid `done` note. Both named historical notes have now been archived and physically cleaned;
independent checks are preserved with the hardware evidence. Missing or
inconsistent evidence is retained. The historical FNV identity is used only
to bind the old journal to the current observed device; new records use the
existing salted SHA-256 fingerprint.

## Evidence and capability status

| Capability | Status |
|---|---|
| Default direct program/note deletion on the reference G1 profile | Implemented and physically verified |
| Independent absence checks after reconnecting; unchanged sentinel | Physically verified |
| Backup, receipt, partial-outcome and recovery guards | Implemented; offline fault tests and successful hardware path |
| Cleanup of the two confirmed historical evidence notes | Physically verified; raw evidence retained locally |
| Manual original helper | Previously physically verified; preserved explicitly |
| New correlated helper markers and interruption handling | Implemented and offline tested; new helper execution not hardware verified |
| `EC` with tested GETKEY-style Home encoding on build 15157 | No visible effect; not an execution backend |
| Other G1 firmware / G2 direct deletion | Unsupported by the default profile gate |
| Arbitrary remote execution / listener experiments | Experimental and unverified; capability gates remain closed |

The exact default demonstration is preserved in
`artifacts/automatic-deletion/hardware-summary.json`, its stdout/stderr and
USB logs, independent query results, and immutable operation backups. Physical
experiments created only fresh tool-owned `ZZT_*` objects. Existing personal
programs and notes, settings, firmware and the clock were never modified.
