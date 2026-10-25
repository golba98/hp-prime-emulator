# Remote-input investigation: G1 build 15157

## 2026-10-25 continuation

The [updated investigation](HP_CONNECTIVITY_KIT_REMOTE_CONTROL.md) traces
EC through its previously missing consumer and traces the HID output boundary.
See its sections C/G/I for input conclusions and J for the unperformed,
authorization-dependent Windows experiment. The earlier findings below are
preserved; unresolved consumer/transport claims are superseded by that report.

Application EC reaches the embedded host calculator's queue at 59e2b5 and
consumer 59c5b0. Default bytes enqueue local input events through 553a20;
ED/EE/EF/F0/F1/FB are reserved local commands. FB queries construct EC
state/image replies. Those replies can use generic endpoint routing to HID,
but no Monitor/Project physical key producer or firmware-15157 acceptance
was demonstrated. The separate FF EC transport-control report is not a key
packet. Do not infer a physical GETKEY map from the numeric opcode.

450320 is now verified as a two-byte FC screenshot builder; manual capture
passes 08. Automatic Project uses 04 with full-frame 08 recovery, and the
viewer composites returned rectangles. Details and evidence are in the
[Project trace](HP_CONNECTIVITY_KIT_PROJECT_TRACE.md) and
[HID trace](HP_CONNECTIVITY_KIT_HID_CALL_TRACE.md).

No new physical traffic was sent. Existing navigation, Check and Run gates
remain disabled. Offline replay and mock tests are not hardware acceptance.

## Earlier record

## Result and scope

**No reliable physical remote-input mechanism is established.** The new UI and execution commands therefore refuse before uploading or transmitting input. This investigation sent no EC packet, no alternative control opcode, no program upload, and no execution request. Existing historical probe evidence is not relabeled as a new successful experiment.

Target: physical G1 `03f0:1541`, firmware build 15157, boot `V0.050.640`. Identity is represented in reports by its existing cryptographic fingerprint, never the raw serial. Local original-firmware assets are G1 2.4.2 and are not assumed interchangeable with this build.

## Candidate evidence matrix

| Candidate | Source/framing | Compatibility evidence | Physical acknowledgment/transition | Decision |
|---|---|---|---|---|
| Historical EC keys | Pinned hplp: `EC 01 [count u32 BE] [one-byte values]` | README lists historical SDKV0.30, 5447 and partial 6030; no modern key table | Prior build-15157 probes transmitted successfully; no acknowledgment or expected screen transition | Tested encoding ineffective; disabled |
| GETKEY values 0–50 | Existing `ppl-device`/native VM key table | PPL language identifiers, not proof of USB encoding | Prior probe using these values had no effect | Do not reuse as verified wire keys |
| Original-firmware device events | `prime-emulator/src/keys.rs`, PrimU2 LCD/MCP tables, firmware event mapping | Local 2.4.2 GetEvent input works inside the emulator | No physical USB test of this interface | Emulator-only evidence |
| Modern Connectivity Kit EC structures | Actual extracted 32-bit CK executable, static inspection described below | Structured EC packets exist; key semantics and 15157 support not established | Not transmitted; no physical transition evidence | Research evidence only; not an input backend |
| PPL note/listener bridge | Existing note-catalog and fixed-mailbox experiments using F7/F8/FC | Live updates were not observed by the running G1 program | Earlier screenshot nonce failed to change; replacement read back after manual exit | No live evaluator enabled |
| Standard host keyboard / PrimU2 TCP, pipe, GDB interfaces | Vendor HID transport versus host-side emulator automation | No verified host-keyboard or physical G1 automation endpoint | No physical validation | Do not substitute emulator interfaces |

A successful transport write is never treated as a pressed key. No Home, Program Catalog, Up/Down, Enter, Esc, Shift, editor selection, Check, or Run action is enabled by these candidates.

## Historical EC evidence

Pinned source revision: `7b07ea9790ef253aafa39bf75d994fb996282048`:

- [prime_cmd.c](https://github.com/debrouxl/hplp/blob/7b07ea9790ef253aafa39bf75d994fb996282048/libhpcalcs/src/prime_cmd.c), `calc_prime_s_send_key` and `calc_prime_s_send_keys`: version-1 EC framing with one-byte payload elements.
- The corresponding receive functions perform no acknowledgment operation.
- [README](https://github.com/debrouxl/hplp/blob/7b07ea9790ef253aafa39bf75d994fb996282048/README.txt): historical firmware support; it does not establish build-15157 behavior.

Existing `artifacts/ec-verification/` baseline and after-key PNGs have identical SHA-256 hashes. A later single candidate Home packet from Program Catalog also failed to change the relevant image region; `artifacts/automatic-deletion/ec-probe.json` preserves that probe. Clock pixels were excluded in the later comparison. No outcome is generalized to every possible EC encoding.

The emulator's firmware codes differ materially from GETKEY codes: for example, local firmware Enter is `0x0D`, Esc is `0x01`, and Home is `0xE047`, while the native GETKEY table uses 30, 4, and 5. This establishes that the interfaces differ; it does **not** justify sending these alternate values over USB. Width, event semantics, and actual USB dispatch must first be understood.

## Connectivity Kit static inspection

The available `HP_Prime_Connectivity_Kit_20260909.exe` installer was inspected as an archive, never executed. Embedded CAB/MSI content was extracted only under ignored `scratch/research/autonomous-control-investigation/`.

The extracted **32-bit** `ConnectivityKit.exe` SHA-256 is:

`a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb`

This is not the earlier 64-bit executable used for verified EB deletion; its offsets must not be conflated with that binary.

Static x86 inspection found, among other protocol handlers:

- `0x59CD85`: writes EC to an allocated message; nearby construction writes FB at byte 6 and structured data thereafter, then sets a version/header and length.
- `0x59CE61`: writes an `EC 01` header into a stack packet; `0x59CE68` sets a 14-byte declared body, and `0x59CE6E` writes FB at byte 6. Nearby code constructs additional state fields.
- `0x59C680` dispatches an internal byte range ED–FB. These internal branches and their structured packets are not a documented remote-key table.

Follow-up disassembly mapped these addresses to `.text` file offsets and
traced the receive-side `EC` branch: it copies packet bytes beginning at
offset 6 into an internal event record, links the record into a queue, and
signals the application's event mechanism. That still does not identify the
consumer's interpretation as physical keys. It does not establish key widths,
modifiers, a handshake, or compatibility with build 15157.

The earlier lead at `0x450320` was described as screenshot-request
construction. The disassembly supports only that it constructs a 0x26-byte
internal message beginning with `FC` and a caller-supplied subcommand byte,
then dispatches it within the application; the purpose of that event is not
established. It is not used here as screenshot evidence.
See the address/offset mapping and reproduction commands in
[HP Connectivity Kit remote-control investigation](HP_CONNECTIVITY_KIT_REMOTE_CONTROL.md).

The retained immediate-index file is `scratch/research/autonomous-control-investigation/ck-command-immediates.json`. A linear instruction scan is a research index, not proof that all callers or operations have been identified. Structured EC uses must not be guessed to mean Home, a keypress, or execution; they could affect unrelated state. No such packet was transmitted.

HP's [Connectivity Kit guide](https://h10032.www1.hp.com/ctg/Manual/c05332952.pdf) documents program content editing, inventory, backup, monitoring, and screenshots. Its computer-editor keyboard instructions are not evidence of physical calculator key injection. Saving a program is not running it.

## Firmware and alternate-control limits

The Rust firmware runner supplies local device events through its GetEvent implementation. Its USB/USBHOST service stub returns success for controller calls; it does not establish an end-to-end physical USB linking handler. PrimU2's TCP command server, Windows named-pipe bridge, GDB interface, and host tooling are emulator facilities and cannot be used as evidence of physical G1 control.

No matching build-15157 USB input handler has been identified in the available local original-firmware image. An address or event mapping observed in the newer pinned image must not become a build-15157 capability flag. No firmware dump, memory write, patch, bootloader operation, clock change, settings change, exam-mode operation, exploit, or arbitrary-opcode scan was attempted.

## Prepared acceptance procedure

A new physical input experiment requires specific user authorization covering the candidate's exact encoding, permitted actions, starting state, press budget, deadline, and evidence directory. The present investigation does not supply a sufficiently justified alternate packet to send.

Once a legitimate candidate is established, the first probe should:

1. Hold one verified device connection and capture a fresh full-resolution baseline.
2. Establish a harmless known starting screen; refuse a confirmation, active program, or unknown screen.
3. Send only the specifically authorized action, once.
4. Capture fresh screenshots within a bounded deadline, without blindly repeating input.
5. Require the expected semantic screen transition, not a clock change, acknowledgment, or successful write.
6. Preserve the result and keep every other action disabled until separately validated.

No Run or upload follows a successful navigation probe automatically. Check and Run require their own evidence and authorization. New controller code cannot enable itself from mock transport fixtures or manually edited capability booleans.

## New read-only evidence

Read-only verification established exact physical FIREWORKS retrieval and a CRC-validated partial F9 inventory. The inventory receiver obtained 54 objects including five programs, but saw no terminal marker; it correctly reports `inventory_incomplete`. Screenshot inspection preserved two fresh 320×240 frames and reported uncertainty because local OCR was unavailable. These observations prove neither a newly invoked Check nor physical execution. See [inventory](../device/program-inventory.md) and [control](../device/autonomous-control.md).
