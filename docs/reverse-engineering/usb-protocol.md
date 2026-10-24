# HP Prime USB HID Protocol Reference

This document summarizes the reverse-engineered USB communication protocol used by the HP Prime graphing calculator in normal Human Interface Device (HID) mode.

> [!IMPORTANT]
> **Privacy Rule:** Device serial numbers must never be logged or committed in raw format. Use a cryptographic fingerprint (hash) such as `serial_id` for provenance tracking.

---

## 1. Physical & Transport Layer

- **Device Class:** USB HID (Human Interface Device). Does not require custom kernel drivers.
- **Vendor ID:** `0x03F0` (Hewlett-Packard)
- **Product IDs:**
  - `0x0441`: HP Prime G1 (original firmware < 8151)
  - `0x1541`: HP Prime G1 (firmware build ≥ 8151)
  - `0x2441`: HP Prime G2
- **Endpoints:**
  - `EP 0x81` (IN, Interrupt, 64-byte packets)
  - `EP 0x02` (OUT, Interrupt, 64-byte packets)
- **Report Size:** 64 bytes.

---

## 2. Framing & Sequence Protocol (`hplp`)

Messages exceeding 63 payload bytes are split into sequential 64-byte HID reports:
- Byte 0: Report ID (`0x00`)
- Byte 1: Sequence number `seq` (`0x00`, `0x01`, ... `0xFE`). Note that `0xFF` is skipped and wraps back to `0x00`.
- Bytes 2..64: Message chunk data.

### Frame Layout
```
Report: [0x00][seq][...up to 63 bytes of payload...]
```

---

## 3. Command Set

### 3.1 Check-Ready / Ping (`0xFF`)
- **Host Send:** `[0xFF]`
- **Device Reply:** Single byte `0x59` ('Y').
- Observed transport readiness reply only. It does not prove UI state, file availability, or program execution.

### 3.2 Get Calculator Info (`0xFA`)
- **Host Send:** `[0xFA]`
- **Device Reply:** Structured payload:
  - Header: `[0xFA, 0x01, size u32 BE]`
  - Body contains model name, boot version, firmware build, and device serial.
  - Offsets (from body end):
    - `len - 36`: Firmware build (u16 little-endian, e.g. `15157`)
    - `len - 32`: Firmware version string (UTF-8 / ASCII)
    - `len - 16`: Firmware-specific serial field (the reference G1 parser uses 10 ASCII characters)

### 3.3 Screenshot Capture (`0xFC`)
- **Host Send:** `[0xFC, format]`
  - Format `0x08`: Full screen (320×240 logical resolution).
  - Format `0x09` / `0x0A` / `0x0B` / `0x0D`: Scaled variants or 106×80 4-bit thumbnail.
- **Device Reply:**
  - For format `0x08`, returns a PNG image labeled as 16-bit Grayscale.
  - **Critical Hardware Fact:** The samples are actually big-endian **RGB565** pixels packed into 16-bit words.
  - Decoding formula:
    - `r = ((v >> 11) & 0x1F) * 255 / 31`
    - `g = ((v >> 5) & 0x3F) * 255 / 63`
    - `b = (v & 0x1F) * 255 / 31`

### 3.4 File Request (`0xF8`)
- **Host Send:** `[0xF8, 0x01, size u32 BE, type, namelen, crc lo, crc hi, name UTF-16LE + 0x00 0x00]`
- **Object Types:**
  - `0x00`: Settings (e.g., `calc.settings`, `cas.settings`)
  - `0x02`: App
  - `0x03`: List
  - `0x04`: Matrix
  - `0x05`: Note
  - `0x06`: Program
  - `0x08`: AppProgram
- **Critical Protocol Finding (Firmware 2.2.15157):**
  - The HP Prime firmware C string lookup routine expects the requested object name in the `F8` payload to be **null-terminated in UTF-16LE** (`0x00 0x00`).
  - If the null terminator is omitted, the firmware searches beyond the name into uninitialized packet memory, fails to find a matching object, and stays silent (causing a timeout).
  - Appending `0x00 0x00` in the payload ensures instantaneous response when the object is present (`ZZT_CTRL`, `tester`).
  - Names are case-sensitive (`tester` vs `TESTER`). Extensions like `.hpprgm` must not be appended for programs.
- **Device Reply:** If object exists, returns `0xF7` file packet. If object does not exist, device times out (no reply).

### 3.5 File Send (`0xF7`)
- **Host Send:** `[0xF7, 0x01, size u32 BE, type, namelen, crc lo, crc hi, name UTF-16LE, data...]`
- **Safe Program Rule:** Only send programs matching `ZZT_*`. Never overwrite existing programs or system objects.
- **Guarded Workflow:** Programs are only uploaded after `preflight_new_names` probes the control object (`ZZT_CTRL`) twice with matching CRC and verifies the target program name is silent twice. An immediate read-back via `F8` validates byte-for-byte integrity.

### 3.6 Remote Key Injection (`0xEC`)
- **Host Send:** `[0xEC, 0x01, count u32 BE, key_codes...]`
- Key codes are mapped on the 0–50 HP Prime matrix.
- **Safety Restriction:** Disabled by default; only authorized when explicit `--auto-run` flag is passed.

### 3.7 Remote Key Injection (`0xEC`) — Hardware Verification Result (2026-10-22, G1 build 15157)

Investigated per the required protocol-verification procedure:

1. **Framing verified against the hplp reference** (`libhpcalcs/src/prime_cmd.c`,
   `calc_prime_s_send_key`/`calc_prime_s_send_keys`): host message is exactly
   `[0xEC][0x01][count u32 BE][key codes...]`. Observed on the wire:
   `00 00 EC 01 00 00 00 01 04` (one Esc press) — byte-exact per hplp.
2. **No reply**: the calculator sends no acknowledgment to `EC` (nothing is
   drained before a following `FC`; the HID exchange log shows only the
   outgoing message).
3. **No observable effect**: 11 presses across a bounded, session-authorized
   probe — `home`, `down`, `esc`, `1`, `2`, `3`, `esc`, `on`, `1`, `1`, `1` —
   each followed by an `FC` screenshot. All 8 screenshots are byte-identical
   (md5 97d64a46…) including the pre-press baseline. Typing `1 2 3` into the
   entry line, `esc` (cancels dialogs), and `on` (wake/break) must all change
   a live screen; none did.
4. `FC` screenshots are proven live by the 47 previously verified core cases,
   so a stale framebuffer is not the explanation.

**Verdict: `EC` key injection as framed by hplp with `GETKEY` codes 0–50 does
not work on this G1 (firmware 2.2.15157).** Either the firmware ignores `EC`
entirely, or it expects a different (undocumented) payload encoding. hplp
itself defines no key-code table (`test_hpcalcs.c` asks the user to "Enter
key ID"), and no public source documents an alternative encoding. Per the
project rule (*never invent keycodes*), no speculative encodings were sent.

**Consequence for automation**: fully unattended physical execution is not
available on this firmware. Hardware runs keep the guarded manual step: the
program is uploaded by the harness and started by the user pressing Enter in
the Program catalog; results are then captured automatically (`FC` screenshot
or `F8` note fetch). The `ppl device key` / `ppl device session` CLI remains
as the (safe, budgeted) interface for any future verified key-injection
protocol. This does not prevent unattended deletion: subsequent Connectivity
Kit analysis and physical tests established the native `EB` command (§5).
The PPL helper remains available as an explicit manual fallback — see
[deletion](../device/deletion.md). All earlier EC probe evidence is preserved under
`artifacts/ec-verification/` (git-ignored).

## 4. Remote execution investigation (2026-10-23)

### Confirmed observations

The reference physical G1 identifies as 03f0:1541, build 15157, boot
V0.050.640. FA, FF (59), F8 reads while idle, guarded F7 creation/read-back,
and live FC screenshots work. None establishes remote invocation. No EC packet
was sent during this extension. The earlier EC verdict applies to its tested
encoding and firmware, not every imaginable execution facility.

Two uploaded sources with 17 locals in one declaration were rejected by the
G1 compiler. A revision using several declarations of six or fewer was accepted.
The exact LOCAL boundary was not measured on this G1. The conservative profile's
eight-variable ceiling follows published compiler measurements; it is not tagged
as an exact G1 hardware limit.

The note-catalog listener processed echo FC01C7DF71, but its exact PONG response
was recovered only after stopping PPL. A delayed reply correctly caused a later
control preflight to refuse another upload. ZZT_188349 was not uploaded. Existing
programs were never overwritten.

The optimized ZZT_ADACBA source displayed its foreground label. Source,
diagnostic-note and known control reads received no reply while it was active,
although screenshots worked. Merely lowering CPU load did not establish live
file communication.

The smaller ZZT_6E9E66 receiver enumerates no catalogs and writes no notes. It
read one pre-created, tagged note and displayed its initial 128-bit nonce in a
black/white grid. A same-connection, ownership-scoped F7 update was transmitted.
No matching replacement frame appeared for 30 seconds; the final grid still
contained the initial nonce. The second update was not sent. F8 for its mailbox
also received no reply while it ran. After one brief Esc tap exited it, F8
immediately returned the exact transmitted replacement value with valid CRC.

The update's precise storage time is unknown. Deferred file handling and a
running interpreter retaining an earlier object view remain explanations.
These results establish neither live command delivery nor mathematical
impossibility of every G1 execution mechanism. No arithmetic or program-call
request was sent, no execution-confirmed flag was asserted, and all execution
capability flags remain false.

The original held-key check could miss a tap during a long wait. Both source
generators now consume queued GETKEY events with 50 ms sleep slices; the user
confirmed one-tap exit on ZZT_6E9E66. Normal menus remain unavailable during a
foreground PPL program; this is not a background daemon.

See [exact evidence and procedures](../device/hardware-testing.md).

### Application protocols, not new USB opcodes

The note-catalog protocol uses existing F7/F8 messages, persistent receipts and
nonce-correlated terminal notes. The fixed-mailbox experiment uses an initially
verified, newly created note plus FC screenshot replies. Its application-level
grid magic C3A5 is not a calculator opcode. The latter sends only harmless nonce
updates and cannot evaluate source text.

Mutable-note capabilities can authorize only their own note, never a program or
settings object. Creation requires the existing absence guard and exact
read-back. Reopening requires a trusted creation record bound to the observed
device, matching deployed source/template and exact current tagged note data.
Capabilities bind to the exact open connection, expire after fifteen minutes,
and are not serialized. New-object absence guards retain their five-minute age
limit. All program uploads still require fresh ZZT_ names and guarded preflight.

### Sources and boundaries

- [hplp command definitions](https://github.com/debrouxl/hplp/blob/master/libhpcalcs/src/prime_cmd.h)
  and [implementation](https://github.com/debrouxl/hplp/blob/master/libhpcalcs/src/prime_cmd.c):
  ready, info, screenshot, backup/file transfer, chat, keys and clock operations.
  No standalone PPL evaluation/run operation appears in that implementation.
  Its supported historical firmware does not establish behavior on build 15157.
- [HP's 2016 G1-era User Guide](https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf):
  Notes catalog access/assignment (printed pp. 635–636), string operations
  (pp. 590–592), EXPR evaluation (p. 430), and exported program functions.
  These language facilities do not guarantee that a running interpreter sees
  USB catalog changes or can resolve newly uploaded functions.
- [HP Connectivity Kit guide](https://h10032.www1.hp.com/ctg/Manual/c05332952.pdf):
  program editing/saving, monitoring, screenshots and messaging. Saving content
  must not be represented as running it.
- [PrimeDev README](https://github.com/BeatSkip/PrimeDev/blob/master/README.md)
  explicitly scopes its current protocol support to G2. It cannot establish G1
  execution support.
- [USB-HID peripheral experiment](https://forum.arduino.cc/t/hp-prime-as-an-arduino-leonardo-input-output-device/1128143)
  uses G2 as host for an Arduino HID peripheral. USBOpen/USBRecv/USBSend in that
  topology are not a demonstrated PC-to-G1 application channel.
- [Compiler-limit measurements](https://github.com/JordiRigau/hp-prime-kit/blob/main/docs/topics/ppl.md#ppllocal-limit)
  distinguish G2/virtual measurements from unverified claims. The long LOCAL
  limit informed a conservative workaround, not an assumed G1 capability.

### Remaining hypotheses

A resident PPL listener may communicate over ordinary transferred notes if
firmware services transfers and refreshes Notes during execution. Dynamic
program dispatch through EXPR, immediate visibility of newly uploaded exports,
completion retrieval during execution, and continued service after errors all
require separate physical tests. Listener restarts and queued requests require
persistent receipts and durable host recovery. No firmware change was attempted.
GETKEY (printed p. 607) returns buffered key events; ISKEYDOWN (p. 609) reports
only the current held state, explaining the original missed-tap bug.

No reliable standalone execution command, documented safe debugging invocation,
or alternative key encoding has been found. Chat/poll payloads are not treated
as source code. Firmware modification, bootloader/debug-memory interfaces and
hardware exploits are outside this implementation. Absence from the inspected
sources is recorded as an unresolved capability, not mathematical impossibility.

See [architecture and operating instructions](../device/remote-execution.md).


## 5. Native USB object deletion (`EB`) — verified 2026-10-23

### Primary evidence and packet construction

HP's [Connectivity Kit guide](https://h10032.www1.hp.com/ctg/Manual/c05332952.pdf)
describes deleting notes/programs from the computer (printed page 22), but
provides no wire format. The following serialization was established by static
inspection of the actual Connectivity Kit executable, then physical tests.
This supersedes earlier statements that absence from hplp's public table meant
there was no native deletion operation.

- Binary: Connectivity Kit **2.4.2 r15751**, PE resource FileVersion.
- SHA-256: `477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1`.
- Delete dispatcher: virtual address `0x1400589f0`.
- At `0x140058a35`, calls named-object builder `0x1401e8f50` with zero data.
- At `0x140058a3d`, writes `EB` into byte 0; byte 1 is selected from the
  connection's format field. The tested G1 uses `01`.
- At `0x140058a4a`, calls `0x1401ee980`: zero CRC bytes 8–9, read declared
  length, compute CRC, store the u16 little-endian result at bytes 8–9.
- For format `01`, CRC covers exactly `size` bytes from message start,
  matching existing v1 F7/F8 behavior; other header formats are not enabled.
- The builder writes type at byte 6, UTF-16LE name byte length at byte 7,
  name at byte 10. Deletion has no object body or UTF-16 null terminator.
- The resulting message proceeds to the existing outbound transport path.

The v1 message is:

```
EB 01 [size u32 BE] [type] [name byte length] [CRC lo hi] [UTF-16LE name]
size = 4 + name byte length
type = 05 (note), 06 (program)
```

Independent fixed fixture for program `ZZT_TEST`:

```
EB 01 00 00 00 14 06 10 66 C9 5A 00 5A 00 54 00 5F 00
54 00 45 00 53 00 54 00
```

A G1 host report prepends `00 00` (report ID and sequence). CRC was
independently calculated with Python `binascii.crc_hqx` and is tested against
the literal fixture. The Rust serializer is not used to generate its own
expected test value. The CRC is calculated **after** choosing EB, rather than
reusing F7's CRC. No guessed destructive packet variant was transmitted.

This is a native named-object management command. It does not require catalog
navigation, a helper, a synchronizing replacement upload, or interpreter
execution. The command itself supplies no completion proof; the implementation
queries actual device storage afterward and never retries it automatically.

### Physical acceptance

Reference G1: `03f0:1541`, build `15157`, boot `V0.050.640`; cryptographic
serial-id `5f3d8dbdb38ae2a0` (raw serial excluded).

Fresh `ZZT_AUTOP1` and `ZZT_AUTON1` established the first program/note cases.
Then the public default commands deleted fresh `ZZT_TEST` and `ZZT_NOTE`.
Every case had a CRC-valid read-back, immutable backup, one EB submission,
and two silent F8 rounds with live controls. Independent verification opened
a new connection. `ZZT_AUTOS1`, a separate fresh sentinel, remained unchanged
through the deletion tests. No calculator touch or keypress was involved.

Default operation IDs: `ZZT_DC20DC` (program) and `ZZT_D4269D` (note).
Both reported `deletion_confirmed: true`, `helper: null`, and
`execution_confirmed: false`. Local artifacts are under
`artifacts/automatic-deletion/` and `artifacts/delete/operations/`.
Unknown firmware profiles and G2 do not inherit this verification.

### Keypress investigation and its limits

[hplp at revision 7b07ea9790ef253aafa39bf75d994fb996282048](https://github.com/debrouxl/hplp/blob/7b07ea9790ef253aafa39bf75d994fb996282048/libhpcalcs/src/prime_cmd.c)
serializes one-byte key values as `EC 01 [count u32 BE] [bytes]` and reads
no key acknowledgment. It defines no separate press/release fields or
verified GETKEY-to-USB mapping. Its README's historical SDKV0.30/5447/partial
6030 support does not establish modern G1 behavior. G1 PID changes at build
8151 likewise do not prove key compatibility.

A new bounded session sent exactly `EC 01 00 00 00 01 05` (candidate Home)
from an observed Program Catalog. Screenshots before, immediately afterward,
and on a later poll had identical RGB pixels in `(0,16)-(320,240)`; only the
status clock was excluded. The expected transition to Home did not occur.
`artifacts/automatic-deletion/ec-probe.json` preserves image SHA-256 values,
the crop, session budget and result. The session was stopped after one press.

No program invocation followed. No key-down/key-up encoding was invented.
The calculator exposes a vendor HID protocol, not a standard USB keyboard
usage-page interface. Therefore transmitted Program Catalog navigation and
PPL invocation remain unsupported for this tested encoding. Direct deletion
is physically verified independently of these execution limitations.
