# Physical HP Prime G1 execution

## Current result: unattended execution is not supported

On the reference physical G1 (03f0:1541, build 15157, boot V0.050.640), no
working unattended execution mechanism has been demonstrated. The new CLI,
validation, guarded transfers, request journals, capability gates and bounded
experiments are implemented. **They do not constitute a working remote evaluator.**
Expression/program execution capabilities remain false and the corresponding
commands refuse before uploading. The existing manual-run harness still works.

The tests established that a transmitted note update can be retrieved after
stopping PPL, while the running receiver continues to show its initial value.
File reads also receive no reply during the tested foreground loops; FA info
and FC screenshots still work. This is consistent with deferred file handling
or interpreter state that does not observe external updates. The exact internal
cause has not been established. CPU optimization alone did not resolve it.

No standalone execution opcode was found in the inspected G1 protocol sources.
The previously tested hplp EC encoding was ineffective on this firmware. This
extension sends no EC, changes no firmware or clock, performs no memory exploit,
and deletes no calculator data. G2 USB-host examples are not evidence for G1
PC communication.

## Implemented interface

```sh
cargo run --bin ppl -- device status --json
cargo run --bin ppl -- device listener status --json
```

`status` reports the observed identity and locally recorded capability evidence.
`listener_running: null` deliberately means that historical installation does
not establish present liveness. `listener status` also tries its diagnostic
state note. In a foreground loop, even source/state-note reads may time out.
A READY state note is historical, not proof that the program is currently alive.

These execution interfaces are implemented but **remain gated off** on the
reference calculator:

```sh
ppl device exec '2+2' --json
ppl device run TEST --remote --args '10,20' --json
ppl device exec-file ./ZZT_TEST.hpprgm --args '5,10' --json
```

`ppl run` still means emulator execution. Without --remote, `device run` retains
its existing manual Run/result-note workflow; it does not initiate execution.

The device profile accepts one complete expression, including calls, but not
assignments or statement sequences. Arguments support nested collections and
quoted commas. Initial exec-file validation requires one exported function
matching a fresh ZZT_ catalog name; helpers may be local, exported globals are
rejected. Source identifiers are never silently renamed. Existing callable
program names may be ordinary names such as TEST. Long LOCAL declarations are
rejected by the conservative device profile; generated code uses six or fewer
variables per declaration. Local parsing does not prove firmware compilation.

Unknown firmware, old G1 protocol variants and G2 do not inherit support. Runtime
errors have a generic diagnostic until a reliable G1 error-code channel is
verified. A user program may have side effects or interactive/non-returning
behavior; the host does not claim to sandbox arbitrary calculator programs.

## Experimental note-catalog listener

Use only for deliberate investigation, not as a normal background service:

```sh
ppl device listener install --json
# Run the exact generated name once on the calculator.
ppl device listener probe --json
ppl device listener verify --json
```

Installation probes a fresh program and its diagnostic note while the positive
control program ZZT_CTRL responds twice, uploads, and verifies source read-back.
It records exact source, SHA-256 and hashed device identity. Installation never
starts a program. `--new-session` preserves the previous installation and creates
another program. A failed candidate never replaces a verified installation record.

The listener polls Notes, processes fresh ZZT_Q<session><id> requests, creates a
persistent ZZT_A... receipt before evaluation and a ZZT_R... terminal result.
Every request contains a random 128-bit nonce. Requests and result names are
reserved by guarded preflight. The host validates type/name, CRC, protocol
version, request identity, nonce, terminal status and END marker. A receipt is
not completion. The receiver never re-executes a request with an existing receipt.

`probe` requires two fresh echoes without manual intervention. `verify` then
checks arithmetic, a trapped error followed by recovery, and a newly uploaded
addition program. No capability is enabled before its physical test passes.
The reference G1 has not passed these gates. No evaluation request was sent in
the recorded hardware session.

The optimized source caches handled requests, uses native POS lookups, scans
only on catalog size changes or a ten-second fallback, and writes no periodic
idle heartbeat. It displays a foreground-mode label. The diagnostic note changes
only at startup, initial readiness, error, or normal stop.

## Fixed-mailbox/screenshot proof of concept

```sh
ppl device listener fixed-mailbox-probe --json --log fixed-probe.log
```

This alternative intentionally does not evaluate PPL text. It creates and
read-back verifies a fresh receiver and one tagged mailbox note before asking
for a manual start. The source reads only that fixed note once per second,
releases temporary strings before waiting, and draws only when its nonce changes.
It does not enumerate Notes or write notes from the calculator.

After recognizing the fresh initial nonce in a 320x240 FC screenshot, the host
updates only its own pre-created mailbox and waits for a matching 128-bit screen
nonce. It attempts a second echo only if the first succeeds. The black/white
screen grid uses application-level magic C3A5 plus 32 hex nonce digits. This is
not an invented USB command: the only wire operations are existing F7 and FC.

Mutable-note access uses an ephemeral, connection-bound capability, issued only
following guarded creation and exact CRC-valid read-back. It cannot target
programs or settings, cannot be serialized, expires after fifteen minutes, and
cannot cross Prime instances. The exclusive device-use session excludes other
USB writers and manual mailbox edits. A successful write is only transmission;
only the screenshot challenge can confirm processing. No unchecked program
replacement is introduced.

If the startup window expires, stop the receiver before resuming:

```sh
ppl device listener fixed-mailbox-probe --resume ZZT_NAME --json
```

Resume verifies the recorded device, deployed source SHA-256/template, current
source read-back, and the mailbox's exact current tagged content before issuing
another update capability. Differing or busy data is refused. A fresh ready
nonce prevents an older ready image from satisfying the next handshake. A source
template upgrade requires a new program rather than overwriting an old one.
The manual-start window is ten minutes; the calculator is not started remotely.

The tested ZZT_6E9E66 receiver displayed its initial nonce correctly, but never
showed the transmitted replacement nonce during the 30-second window. The
replacement was read back after the user exited. Therefore the live bridge is
unverified and enables no execution mode.

## Responsiveness and stopping

PPL listeners are foreground programs. Normal calculator menus are unavailable
while they run; they are not background services. The original held-key check
could miss a short Esc tap during a one-second wait. Both source templates now
consume queued GETKEY events and use 50 ms sleep slices. The user confirmed that
one brief Esc tap exits ZZT_6E9E66. VM tests separately exercise a released tap
with ISKEYDOWN always false, and verify sleeping rather than busy spinning.

Older test programs may require holding Esc for two seconds or pressing the
physical On/Cancel interrupt. A User abort dialog after an interrupt is not a
syntax failure. The host timeout never interrupts a calculator program. Stop
before using normal menus or editing mailbox objects.

## Journals, locking, recovery and retention

Journals are persisted before any execution-request F7 can be sent. Timeout,
disconnection or invalid evidence after submission leaves execution unknown;
there is no automatic resend. Retrieve that same request only:

```sh
ppl device status --request-id REQUEST_ID --json
```

An unresolved evaluation blocks further execution. Only harmless echoes can be
closed with `listener abandon-probe ID`, a host-only action that does not cancel
calculator work. Their eventual replies remain recoverable. A request without a
receipt can still run when its listener next starts; do not assume cancellation.
A receipt without a result can mean interrupted evaluation and cannot safely be
replayed.

Actual HID handles hold a shared OS advisory lock for their full lifetime. A
second cooperating process fails busy. Guards and owned-note tokens also bind
to the exact connection, preventing transfer of mock/stale guards to another
Prime instance. Close Connectivity Kit and unrelated USB tools; these cannot be
controlled by the project's advisory lock. Use one working directory and one
--remote-dir rather than bypassing unresolved journals with another directory.
Linux is the validated host platform; Windows hardware behavior is unverified.

Artifacts live under artifacts/remote-execution/<serial-id>/: sources, candidate
and active manifests, request journals, terminal notes, screenshots and optional
HID logs. Serial IDs use the existing salted SHA-256 convention. New remote
command logs exclude FA replies containing raw serials. No automatic cleanup is
performed; no existing user program was overwritten. Keep receipts while their
requests remain, and do not apply another campaign's ownership ledger to these
objects. Repository conventions ignore these artifacts and Markdown under docs/.

See [protocol findings](../reverse-engineering/usb-protocol.md),
[agent integration](agent-integration.md), and [hardware testing](hardware-testing.md).
