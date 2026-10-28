# Deleting programs and notes on HP Prime G1

Current shared-manager behavior and evidence boundaries are documented in
[DELETE_TRANSACTION_RECOVERY.md](DELETE_TRANSACTION_RECOVERY.md). The historical measurements below are retained;
claims about an exact firmware size limit or legacy pending-state handling are
superseded by that report.

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
checks. Deleting a disposable `ZZT_*` object requires no prompt for a matching
creation receipt. Ordinary non-`ZZT_` programs sent with `send` are
audit-logged only (no receipt or ledger entry): they are never test-cleanup
candidates, but can be deleted deliberately (next section).
The physical demonstration used these exact names with fresh objects, then
verified each absence on a new USB connection. No helper was uploaded or
executed. An unrelated, freshly created sentinel note remained unchanged.

## Deleting ordinary programs and notes

```sh
ppl device delete program FIREWORKS --yes       # one command, no interaction
ppl device delete program FIREWORKS --yes --json # same, machine-readable
ppl device delete program FIREWORKS              # asks you to type the name
ppl device delete note MyNote                    # notes: always asks
ppl device get program FIREWORKS --out /tmp/FIREWORKS-backup.hpprgm   # optional
```

`--yes` authorizes **one** deletion attempt of **exactly** the named program
on the selected calculator. It is recorded in `authorization.json` bound to the
device fingerprint, type, name and content hash; it does not read stdin, need
a creation receipt, a helper, a Run press or a `ZZT_CTRL`. `--json` alone never
authorizes anything. `--yes` is refused with several names, with `note`, and
with `--manual-run`/`--prepare-only`; it works for `ZZT_*` names too (still no
receipt involved), but `ZZT_CTRL` and reserved names stay protected. With
several calculators connected, pass `--device SERIAL_ID` (ambiguity is refused).

Example output:

```text
$ ppl device delete program FIREWORKS --yes
HP Prime G1
Firmware: build 15157

Target: program FIREWORKS

[OK] Device connected
[OK] Liveness anchor verified (settings:calc.settings)
[OK] Program found
[OK] Program contents verified (two reads, CRC valid)
[OK] Local backup created (artifacts/delete/operations/ZZT_Dxxxxx)
[OK] Deletion authorized
[OK] Native USB delete submitted
[OK] Independent absence verified

Result: FIREWORKS deleted successfully.
```

Each `[OK]` line is printed only after that step succeeded. Exit status is 0
only for `confirmed_deleted`.

Any program or note whose name is a valid calculator name can be deleted; no
`ZZT_` prefix is needed. The optional `get` is only a convenience: `delete`
always makes its own verified backup. Deletion is never silent, never
batched, and never implied by a creation receipt.

**Names.** 1–16 characters of `[A-Za-z0-9_]`, starting with a letter, not a
reserved word (`END`, `IF`, `ZZT_CTRL`, ...). Names are device object
identifiers, not paths: `/`, `.`, spaces and NUL are rejected, and the
`EB` protocol is not extended to filesystem paths. `program:FIREWORKS` and
`note:FIREWORKS` are different targets; the type must match the object on the
calculator or the request is refused. An ordinary request names exactly one
object (a companion note is never deleted implicitly) and cannot be mixed
with `ZZT_*` names.

**Order of operations** (any failure refuses the deletion):

1. Select the calculator, read its identity (`FA`, kept as a baseline), require
   the verified USB-delete firmware profile (`unsupported_device` otherwise),
   choose the liveness anchor (below), refuse if an earlier unresolved
   operation covers the same device/type/name.
2. Fetch the object twice with CRC validation; the two reads must agree and
   the anchor answers (with an `FA`+`FF` heartbeat) in between. A program that
   stays silent while the anchor answers is `not_found`; nothing is sent.
3. Write the **raw** object bytes to
   `artifacts/delete/operations/OPERATION_ID/program-NAME.bin` (create-new,
   `fsync`, directory sync), record its SHA-256 and the device fingerprint in
   `operation.json` and `metadata.json` (name, type, bytes, SHA-256, device
   fingerprint, firmware, timestamp), and read the backup back to confirm it.
   If any of this fails nothing is sent (`backup_failed`).
4. With `--yes` the authorization is recorded and this step needs no input.
   Without it, show the target and require the **exact name** to be typed:

   ```text
   HP Prime G1 — Delete program

   Device: <fingerprint>
   Object type: program
   Object name: FIREWORKS
   ...
   Type FIREWORKS to authorize deletion:
   ```

   Matching is case-sensitive and exact (only the line ending is ignored).
   Anything else cancels with nothing transmitted; the backup is kept.
   Piped input and `--json` never prompt and are always refused; there is no
   `--force` or `--yes`.
5. Record `authorization.json` (method, device, type, name, SHA-256) and the
   `submitting` state durably.
6. Re-probe the control and target, require unchanged content, send one `EB`.
   There are no retries, wildcards or inferred targets.
7. Verify absence independently (below) and record the outcome.

**Results.** The JSON `result` field is one of `confirmed_deleted`,
`still_present`, `not_found`, `indeterminate`, `device_unresponsive`,
`backup_failed`, `authorization_refused`, `unsupported_object_type`,
`unsupported_device` (other pre-flight refusals, such as invalid names, report
`refused`). Only `confirmed_deleted` exits 0. The JSON also carries
`object`, `type`, `device_serial_id`, `backup_path`, `backup_sha256`,
`submission_state` (`not_sent`/`submitted`), `verification`, `status`,
`liveness_anchor` and `recovery`.

**Liveness anchor (why no `ZZT_CTRL` is needed).** A silent calculator is never
counted as deleted: absence needs two `F8` rounds, each bracketed by a
CRC-valid reply from an *anchor* plus an `FA` identity check (same
fingerprint/build/version as at the start) and an `FF` ready check. By default
the anchor is the built-in `settings:calc.settings` object, so nothing has to
be installed; `--control TYPE:NAME` or an installed `ZZT_CTRL` still work. If
no anchor answers, deletion is refused before anything is backed up or sent
(`unsupported_device`). `FA`/`FF` alone are *not* enough: they keep answering
while file requests are silent (documented with a foreground program running).

*Evidence and limits.* Read-only probe on the reference G1 (build 15157,
`V0.050.640`, 2026-10-24, `artifacts/anchor-probe/`): `calc.settings` and
`cas.settings` each answered 3/3 with valid CRC in ~55 ms. The calculator also
answers any other type-0 name (`bogus.settings`) with an empty reply, so the
anchor proves the file-request handler is alive, not that the settings object
exists. It has not been tested while a PPL program is running; keep the
calculator idle at the Home screen. `ppl device probe-anchor` repeats the
check.

**Indeterminate outcomes.** Once `EB` has been sent, a timeout, missing
reply or failed read-back leaves the operation `unresolved`. The command is
never resent. Reconcile read-only:

```sh
ppl device delete verify --operation-id ZZT_Dxxxxx
```

An unresolved operation blocks new deletions of that target until it is
confirmed. Operation IDs keep the internal `ZZT_Dxxxxx` form regardless of
the object name.

**Recovery.** The `.bin` backup is the exact byte stream the calculator sent
(UTF-16LE source text for programs and notes). Re-sending it with
`ppl device send` is the expected recovery route, but **restoration has not
been independently verified for ordinary objects**; keep the backup and
verify a restore before relying on it.

**Records are separate.** Ordinary deletions are `class: "user"` operations.
They never read, write or retire creation receipts and never enter the
disposable-test ledger; a receipt for `ZZT_NAME` cannot authorize an ordinary
object and an ordinary deletion cannot authorize other cleanup. Records
written before this feature have no `class` and load as `test` records.
Authorization is bound to device fingerprint, type, name and content hash; a
different calculator, type or changed content never matches.

**Object types.**

| Type | Fetch/backup | `EB` delete | Status |
|---|---|---|---|
| program (0x06) | verified | verified on reference G1 (`ZZT_*` objects) | supported (`--yes` or typed name); ordinary names covered by mock-transport tests, not yet physically deleted |
| note (0x05) | verified | verified on reference G1 (`ZZT_*` objects) | supported; same caveat |
| list, matrix, real, complex | no evidence | no evidence | **unsupported** (`unsupported_object_type`) |
| app, app note/program, settings | not applicable | not applicable | protected, never deletable |

Other types stay unsupported until fetch, raw backup and `EB` encoding are
evidenced on hardware. No arbitrary-path or firmware/system deletion exists.

**Physical testing.** Deleting a non-`ZZT_` object on a real calculator needs
separate, explicit authorization from the owner. Until then the ordinary path
is verified only against the in-memory mock transport, plus the read-only
anchor probe above; the first physical check should use a throw-away program
(`send` one, then `delete ... --yes`). Only build 15157 / `V0.050.640` is
supported; other firmware and the G2 are refused.

## Test-object (`ZZT_*`) deletion

Everything below describes the disposable-test path: `ZZT_*` names, creation
receipts, optional batches of up to 32, and the explicit manual helper. It is
unchanged by ordinary-object deletion.

## Mechanism and safety

The preferred operation is Connectivity Kit's native `EB` named-object
management message, **not PPL execution or UI automation**. Its serializer
was traced in Connectivity Kit 2.4.2 r15751 and then tested on the physical G1.
See [protocol evidence](../reverse-engineering/usb-protocol.md#5-native-usb-object-deletion-eb--verified-2026-10-23).
The earlier conclusion that there was no USB delete command was based on
incomplete public command tables and is superseded by this evidence.

The workflow is:

1. Select and lock the calculator; read its identity and firmware profile.
2. Validate exact names (`ZZT_*` for the receipt-authorized test path), types,
   count, and protected controls.
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
| Ordinary (non-`ZZT_`) program deletion with `--yes`, settings anchor | Implemented; mock-transport tests; anchor probe read-only on hardware; no physical ordinary deletion yet |
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
