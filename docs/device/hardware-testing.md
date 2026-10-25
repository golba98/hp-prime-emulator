# Reproducible G1 hardware testing

## Scope and current conclusion

Physical reference: G1 03f0:1541, build 15157, boot V0.050.640. Raw serials are
excluded. Experiment uploads create only fresh ZZT_ programs/notes (ordinary user programs can be sent with `ppl device send NAME file`, see program-upload.md), with positive controls,
collision probes and CRC-valid read-back. Mutable-mailbox tests update only the
note created by that test. No firmware, clock, undocumented memory or EC keys
are involved. Existing user programs remain intact. The only deletions are
explicit, tool-owned `ZZT_` objects removed by `ppl device delete` (see
[deletion](deletion.md)): each one is backed up first, deleted by a generated
allowlist helper after one Run press, and confirmed only through two probe
rounds with the control object answering.

Unattended expression/program execution has **not** been demonstrated. All
execution capabilities remain false. A successful upload is not execution.

## Recorded observations, 2026-10-23

| Test | Actual observation |
|---|---|
| Identity, FF, F8 ZZT_CTRL | Correct reference identity; ready byte 59; valid control source |
| ZZT_6B831C and ZZT_99B587 | Sources uploaded/read back, then rejected by G1 syntax checks; each declared 17 locals in one statement |
| ZZT_FC01C7 | Split declarations accepted; user started it manually |
| FC01C7DF71 echo | Uploaded/read back; initial result polls timed out; exact PONG:ZZT_EC7BDD recovered after stopping the listener |
| ZZT_188349 candidate | Not uploaded: an unrelated late result answered a control probe, so the guard refused |
| ZZT_ADACBA | Optimized cache/native lookups and visible foreground screen; user started it; source/state/control file reads received no reply while active, but screenshots worked |
| ZZT_407901 fixed mailbox | Source and initial mailbox verified; startup wait expired before the user ran it, so no live updates were sent; later screenshot showed a valid initial grid |
| ZZT_6E9E66 fixed mailbox | Queued-key handler, fixed note read and no periodic note writes; source and mailbox verified; fresh initial nonce decoded from live screenshot |
| Live update to ZZT_6E9E66M | One tagged update transmitted; screen remained at the initial nonce for the 30-second window; second update was not sent |
| File read during that loop | ZZT_6E9E66M received no F8 reply within eight seconds |
| Brief Esc tap | User confirmed that one tap exits ZZT_6E9E66 |
| Mailbox after exit | Immediate F8 returned valid CRC and exactly the transmitted nonce |

Initial nonce: 8EA33C89AB6CEB0B2686397B2B69EBB5. Transmitted nonce:
A2DB41A93B2023BE07736D99E29EFEFB. The last live grid still decoded to the initial
nonce. The replacement's storage was verified only after stopping; its precise
storage time is unknown. Deferred USB file handling and stale interpreter
object state remain alternative explanations. Neither observation establishes
that all G1 execution mechanisms are impossible.

Evidence: artifacts/remote-execution/5f3d8dbdb38ae2a0/, especially
fixed-6E9E66-hardware-summary.json, fixed-ZZT_6E9E66.json and the ready/echo PNGs.
Parent-directory HID logs include fixed-queued-esc-probe.log and
fixed-mailbox-after-stop.log. runtime-error.png records an expected User abort;
optimized-active.png and fixed-active.png show actual foreground screens.

The calculator was stopped after testing. No arbitrary expression or program
invocation request was sent. No execution-confirmed flag was asserted.

## Reproduce the fixed-mailbox experiment

1. Close other USB applications, stop existing PPL programs, and leave ZZT_CTRL
   available. Use the same repository working directory and journal directory.
2. Run `cargo run --bin ppl -- device status --json` and verify the reference
   identity. Unknown firmware must not inherit execution capabilities.
3. Run `cargo run --bin ppl -- device listener fixed-mailbox-probe --json --log
   fixed-probe.log`. Wait for READY FOR MANUAL START, after both read-backs.
4. Run the exact generated program on the G1 within the ten-minute start window.
   Leave the grid open without touching the calculator during both challenges.
5. Require both fresh, full 128-bit nonces in valid 320x240 FC frames. A static
   initial frame, successful HID write or file acknowledgment is insufficient.
6. On failure, preserve the report and stop the receiver with one brief Esc tap.
   Then read its exact mailbox with `device get note NAME --out result.bin`.
   Compare its UTF-16 content with the recorded nonce; storage after stopping is
   not live-processing evidence.
7. If only the startup wait expired, stop first, then use
   `listener fixed-mailbox-probe --resume ZZT_NAME --json`. Resume refuses an
   unverified device/source/template or a mailbox whose exact tagged data differs.
   It never overwrites another program. Older source templates cannot be resumed
   by a changed generator merely because their names match.

The host keeps one exclusively locked HID connection throughout the experiment.
Owned-note capabilities are not serialized or transferred across connections.
No F8 query is needed between the two live screen challenges. A screenshot's
nonce must match the new request, not an earlier cached frame. Mock-generated
images test only the decoder and are not accepted as physical proof.

## Note-catalog experiments and future acceptance

`listener install`, manual Run, `listener probe`, and `listener verify` implement
the separate note-catalog approach. Do not use it as a normal service on this
firmware: its live gates have not passed. `verify` requires arithmetic results,
a trapped error followed by recovery, and 30 from a newly uploaded two-argument
program while the listener remains active. These steps have not been reached.

Any future supported backend must demonstrate repeated request processing,
argument-bearing calls, new-code installation, correlated result retrieval and
recovery without additional manual Run presses. Program installation, invocation
and completion must be separately evidenced. Images, mocks, emulator results,
or F7 acknowledgments alone cannot satisfy that requirement.

Never resend an uncertain evaluation. Recover by request ID. A receipt without
a terminal result can represent interrupted work, while an unreceipted request
may still execute on a later manual start. A timeout does not stop the calculator.
An interactive/non-returning user program may require physical On/Cancel.

No automatic cleanup is implemented for these objects. Retain matching receipts
while requests remain. Use an explicit ownership-verified procedure for any
later cleanup; the campaign ledger does not establish mailbox ownership.

## Automated checks

Run cargo test --workspace, cargo fmt --all -- --check, and
cargo clippy --workspace --all-targets -- -D warnings. The Xvfb smoke test keeps
its existing ignored-by-default requirement. Coverage includes malformed inputs,
EOF parser handling, declaration limits, native message framing, collision and
read-back guards, stale/wrong nonce and CRC evidence, disconnects, short writes,
durable unresolved journals, OS locks, connection-bound guards, owned-note
reopening, firmware gating, JSON, and released Esc taps in the VM.

Deletion hardware evidence (2026-10-23): `ZZT_DELTP1` (program) and
`ZZT_DELTN1` (note) were each uploaded fresh, confirmed present, backed up,
deleted by one Run of the generated helper, and verified absent while
`ZZT_CTRL` answered; both helpers self-deleted and their `done` result
notes were initially retained (subsequently archived and cleaned below). Error cases (missing, invalid, protected, wrong type,
repeat) all refuse without writing. See [deletion](deletion.md).

Earlier remote-execution baseline: 834 passed, 0 failed, 1 existing Xvfb-dependent smoke test ignored. Formatting and workspace/all-target Clippy with warnings denied passed. See artifacts/remote-execution/validation-summary.json and workspace-tests.log.


## Unattended deletion acceptance (2026-10-23)

The reference G1 (03f0:1541, build 15157, boot V0.050.640) now supports
physically verified direct `EB` deletion. This does not establish remote PPL
execution. A harmless single Home probe produced no catalog-to-Home transition;
no execution key sequence was sent.

Fresh `ZZT_AUTOP1` / `ZZT_AUTON1` established the initial direct program/note
cases. The public default commands then deleted fresh `ZZT_TEST` and
`ZZT_NOTE`, without `--auto-run`, an experimental flag, a helper, or calculator
interaction. Separate CLI processes independently confirmed both absences.
`ZZT_AUTOS1` remained byte-for-byte unchanged throughout the test, then was
safely deleted after its purpose was fulfilled.

Default evidence is in `artifacts/automatic-deletion/hardware-summary.json`,
with source fixtures, upload logs, deletion JSON/USB logs, independent queries,
sentinel snapshots and SHA-256 values. Its program operation is `ZZT_DC20DC`;
its note operation is `ZZT_D4269D`. All raw target backups remain in those
operation directories. A second reproducible fresh-name acceptance run is in
`artifacts/automatic-deletion-tests/0F220E/summary.json`: program, note and
sentinel were all created and removed, and each deletion was independently
verified after reconnecting. All steps returned exit 0.

The already-confirmed historical result notes `ZZT_D33C29R` and `ZZT_D24731R`
were separately cleaned as requested, after importing their same-device
prepared/verify journals, checking original backup hashes, confirming the old
target/helper remained absent, and archiving current CRC-valid `done` contents.
Cleanup returned `complete` for both, created no helpers/notes, and preserved
raw note backups in their operation's `cleanup-backups/`. This was recovery
and cleanup of known confirmed operations, not destructive experimentation
on unrelated existing objects.

### Reproduce safely

```sh
cargo build --bin ppl
python3 scripts/device/verify_automated_deletion.py --hardware
```

The `--hardware` switch is required. The script uses fresh random `ZZT_*`
names, existing guarded Rust upload/deletion paths, immutable backup checks,
a sentinel and independent query processes. It sends no raw guessed packets
or keys. A failure retains the exact request evidence and stops without
retrying an uncertain action or attempting speculative cleanup. Successful
runs remove their own sentinel after checking it, leaving no test fixtures
on the calculator. Do not operate the calculator or run another USB tool
concurrently. Unknown deletion firmware profiles fail safely.

Offline coverage includes direct-delete CRC fixtures, connection-bound and
single-use authorization, changed source, late replies beside a live control,
corrupt controls/results, final-control loss, partial success, ignored EB,
prepare-only without mutation, journal failure before upload, finite cleanup,
changed/unresolved artifact retention, restart recovery without retransmission,
legacy proof validation, receipt scoping/retirement, firmware gating and JSON
for operational and argument-parser errors. GUI stress expectations were also
updated for the workspace's current semantic highlighter, retaining exact
string coverage and comment-boundary checks without changing GUI behavior.

Final unattended-deletion validation: 891 workspace tests passed, 0 failed, 1 existing GUI smoke test ignored. Workspace build, formatting, all-target Clippy with warnings denied, and diff checks passed. See `artifacts/automatic-deletion/validation-summary.json`. No commits or pushes were made.
