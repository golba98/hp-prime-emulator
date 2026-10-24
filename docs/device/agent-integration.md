# Agent integration with the physical G1

**Current reference G1: no unattended execution backend is verified.**
All execution capabilities remain false; execution commands refuse with exit 6.
Do not start a listener as a normal background service or treat the command
surface as proof that the objective is complete.

Use `ppl device ... --json`, not `ppl run`. Supported capabilities come from
`device status`; execution flags remain false. The separate
`object_management.direct_delete` flag is true for the verified deletion
profile, with `mechanism: "usb_delete"` and key injection unverified. A successful status call does
not establish that the listener is currently running. The user starts the
installed listener once and the host verifies it through fresh challenges.

Recommended workflow:

1. Read `device status --json` and select the device's cryptographic serial-id
   with `--device ID` if more than one calculator is connected.
2. Generate source with one fresh ZZT_ export, local helpers, and short LOCAL
   declarations. Use existing emulator checks for development while recognizing
   their builtin coverage and syntax limits.
3. Inspect syntax diagnostics before issuing `device exec-file FILE --json`.
4. Record request ID, upload state and execution state separately.
5. Accept a successful execution only when `execution_confirmed` is true and the
   request-correlated result is successful. Preserve evidence paths.
6. On timeout or disconnection, use `device status --request-id ID --json` to
   retrieve that request's eventual result; do not resubmit the code.
7. After a terminal calculator error, correct the source, generate a fresh ZZT_
   export/catalog name, and retry only when no request remains unresolved.

## Object deletion workflow

Program and note deletion is now unattended on G1 `03f0:1541`, build 15157,
boot V0.050.640. `device send` records a device/type/exact-name/content-bound
creation receipt; `device delete program|note NAME --json` syncs backups,
sends native USB `EB` once, and independently verifies absence. No helper,
Run press, or evidence note is needed on the normal path. This is not a
remote execution capability.

Use `delete verify --operation-id ID --json` for read-only recovery. Never
retry an uncertain deletion. `delete cleanup --operation-id ID --json`
removes only archived artifacts of a confirmed operation. `--manual-run`
preserves the old one-Run helper workflow; `--prepare-only` uploads only and
never deletes or injects keys. See [deletion](deletion.md).

Deletion output has `operation_id`, `mechanism`, `backup_created`, per-target
`results`, `deletion_confirmed`, `execution_confirmed`, `cleanup_state` and
an error string or null. Direct deletion keeps execution false. Exit 0
means the requested operation succeeded (preparation alone is not deletion);
exit 5 means refusal, I/O failure, or an unresolved outcome. Deletion syntax
errors also emit JSON with exit 2. Noninteractive deletion refuses missing
ownership receipts without prompting. Deletion supports `--device SERIAL_ID`.

## Output contract

Stdout contains one JSON object for status, listener and remote execution
commands. Progress is written to stderr. Schema version 1 includes:

| Field | Meaning |
|---|---|
| device | Observed model, PID, firmware, boot version, hashed serial-id |
| operation | Requested host operation |
| uploaded / upload_state | Program installation verification; independent of execution |
| request_upload_verified | Request note read-back verification, when applicable |
| request_id | Durable request identity for recovery |
| execution_state | not_requested, unknown, completed, runtime_error, mailbox_verified, or experimental fixed_mailbox_echo_verified |
| execution_confirmed | True only for a successful correlated evaluation; echo probes never qualify |
| result / result_type | Calculator text and native type code, or null |
| error | Structured kind, message and exit_code, or null |
| evidence_paths | Local artifacts supporting the response |

An installation can return exit 0 and `uploaded: true` while
`execution_confirmed: false`. Echo verification also leaves execution false.
`execution_state: unknown` is not permission to retry. A trapped runtime error
is distinct from successful execution and keeps execution_confirmed false.
Clap syntax/unknown-option errors retain conventional stderr diagnostics and
exit 2; they do not promise the operational JSON envelope.

| Exit | Meaning |
|---|---|
| 0 | Requested operation succeeded; inspect execution fields |
| 1 | Existing local compiler failure |
| 2 | Invalid request, input or refusal |
| 3 | Terminal calculator evaluation error |
| 4 | Existing emulator resource/interrupt limit |
| 5 | Device discovery, USB, filesystem or inconclusive preflight failure |
| 6 | Unverified/unsupported execution capability or identity |
| 7 | Result timeout; execution may remain unknown |
| 8 | Device locked, existing installation, collision or unresolved request |
| 9 | Invalid evidence or unverified upload |

Use one consistent working directory and --remote-dir for the device. Do not
run multiple USB tools concurrently. Never interpret a transport acknowledgment,
mock test, screenshot alone, or emulator result as request-correlated physical
execution. Examples in documentation are interfaces, not claims of hardware
success. See [operating instructions](remote-execution.md).

## Bounded research tool

`device listener fixed-mailbox-probe --json` installs one fixed-note receiver,
waits for one manual start, and tests two harmless 128-bit screenshot echoes.
It has no evaluator. `--resume ZZT_NAME` reuses only a stopped receiver whose
creation record, device, source/template and exact mailbox content verify.
Its same-connection owned-note writer cannot target programs/settings and has
a fifteen-minute lifetime. No mode is enabled by these experimental echoes.

On the recorded G1 the first update did not appear in the live grid, although
its value was retrieved from storage after exit. This does not establish the
internal timing of the write. A program's foreground loop and a working USB
link are insufficient for unattended execution.

The user confirmed brief-Esc exit on the queued-key revision. Older receivers
may miss a tap and require holding Esc or the physical On/Cancel interrupt.
The host does not inject a stop key or silently reset the calculator.
