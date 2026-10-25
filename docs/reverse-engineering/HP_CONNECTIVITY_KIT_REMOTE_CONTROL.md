# HP Connectivity Kit remote-control investigation

## Continuation findings — 2026-10-25 (supersedes unresolved statements below)

No physical-device operation was performed. Navigation, Check and Run remain
unsupported. The continuation establishes the software HID chain and screen
patch path; it does **not** establish physical G1 remote control.

### A. Binary provenance

The inspected executable is PE32/i386, preferred base 0x400000, resource
FileVersion **2.4.2 r15751**, ProductVersion 2.4.2. Its retained hash is
unchanged. Original installers were read and hashed, not run:

| Original file | SHA-256 |
|---|---|
| HP_Prime_Connectivity_Kit_20260909.exe | 43a16de1a126001e0ee2a796eebb4291a47faa384730e902d80f4fea32c8a493 |
| HP_Prime_Virtual_Calculator_x64_2026_09_09.exe | 8653a27d099d78e4fcdd5504dbca46373381d3646391f3364116eada02fbb6a9 |

The retained cab1.cab hash is
`e1d4c90ef09bfd42b1f07401944a1b222eabd77e929bd0b9f465b2222d477f1d`;
the earlier table omitted one `f` and is corrected below. The second retained
ConnectivityKit.exe extraction has the same PE32 hash. Historical PE64
addresses/hash in other reports were not revalidated and must not be mixed
with this build. Virtual Calculator installer hashing does not establish
its implementation or identity with the embedded host engine.

### B. HID transport implementation

[The HID call trace](HP_CONNECTIVITY_KIT_HID_CALL_TRACE.md) records actual
factories, virtual tables, queue/worker calls, Windows APIs, sizes, framing,
teardown and address mapping. This resolves the earlier imports-only gap.
Reports are descriptor-sized. The transport removes one Windows report-ID
byte before presenting data to the protocol worker. A historical 64-byte
G1 protocol report corresponds conditionally to a 65-byte Windows buffer,
including report ID zero; no fresh Windows descriptor was measured.
Legacy fragments have sequence 00…FE, wrapping to 00, and 63 message bytes
for that G1 size. Alternative negotiated transport branches exist and remain
unverified for the connected G1 configuration.

### C. EC: concrete receiver and bounded outgoing result

Receive dispatcher **59e050**, through the `CCalc` listener wrapper
**4bbac0**, handles application opcode EC at **59e2b5**. It reads the BE
body length using **59dfe0**, copies packet +6, and appends a 20-byte queue
record to CCalc +1fc4 under its lock at +1fc0. Record fields are source
endpoint ID, byte pointer, byte count, cursor and next pointer. It sets
CCalc +2b0 bit 0x20. This is an incoming message to the **embedded host
calculator engine**, not the HID writer.

The previously missing consumer is **59c5b0**, called from **4ba16b**.
It reads one byte at **59c680**. Reconstructed dispatch:

| Byte | Observed consumer behavior |
|---|---|
| ED | Reset/local state path at 59cff6 |
| EE | LE16 string length and text conversion at 59c6aa |
| EF | Local state operations at 59cfd7 |
| F0 | Local event type 1/value D1 and queue/wake handling at 59cf4f |
| F1 | Structured subtype/coordinate input, local event type 5 at 59d14c |
| FB | Framebuffer/state query with flags/cookie/checksum at 59c75e |
| Other bytes, including F2…FA | Default at 59d225: enqueue local event type 1 with the byte as value |

**553a20** writes those default events to the host's eight-entry event ring
(global 113a650, initialized at 474448 onward), then **55d8f0** wakes the
local engine. This demonstrates local byte-valued input semantics, not a
physical G1 GETKEY table or a firmware-compatible press/release encoding.
Historical one-byte hplp payloads resemble the default branch, but reserved
commands and the receiver's identity prevent extrapolating acceptance.

EC constructors at **59cd85/59ce61** are FB query replies, containing flags,
cookie/checksum and optionally PNG/state data. Generic reply router
**59fb10** resolves the source endpoint and calls **4cb220** at 59fbfe.
Consequently an EC **reply can statically reach HID output if its resolved
endpoint is physical**. It would be incorrect to claim EC can never leave
the host. That is not a UI key producer, and neither transmission nor this
host receiver establishes a physical remote-input implementation.

The GUI message dispatcher **44f110** switches F2…FD; EC falls through its
default release path. Different registered listeners may still receive it.
The transport control report `[FF EC …]` is a distinct negotiation message,
not this application EC command stream. Evidence: `ec-incoming.asm`,
`ec-consumer.asm`, `ec-routing.asm`, `ui-dispatch.asm` and `hid-receive.asm`.

### D. 450320

[Project trace](HP_CONNECTIVITY_KIT_PROJECT_TRACE.md) closes this lead:
**450320 constructs a two-byte FC screenshot request**, with caller-selected
format; manual capture 4289e0 selects 08. Its 0x26 allocation comprises
0x24 metadata bytes and two serialized bytes. Callback 4648c0 selects and
sends to an endpoint, reaching the physical worker when that endpoint is a
HID client. It is not a key operation. This conclusion uses caller/payload/
queue evidence absent from the original investigation.

### E–F. Monitor/Project and screenshot comparison

[Project trace](HP_CONNECTIVITY_KIT_PROJECT_TRACE.md) supplies the complete
scheduler→request→HID→assembly→Qt event→PNG→cached-image path. Ordinary
Project requests use **FC04**, with **FC08** for full frames. Monitor uses
05/07/0D/15 and corresponding full variants 09/0B/11/17. The consumer reads
coordinates and composites rectangular PNG patches. Embedded host producer
code independently confirms incremental versus forced-full semantics, but
is not physical firmware 15157. A ten-second pending deadline is not a
polling interval. No official Windows FPS was measured.

All 100 preserved format logs passed offline framing/CRC parsing. The
recovered compositor explains why 1×1 PNGs cannot be treated as standalone
screens. The first small 0D patch changes nine scaled pixels; subsequent
seventeen leave the replay unchanged. Physical FC04 behavior and its
pre-response delay remain unmeasured. Existing 0.950/2.579 FPS observations
are retained, not replaced by a claimed official speed advantage.

### G. Remote-control feasibility

Inspected Project/Monitor handlers resize, select, capture, copy or save.
Screen-view keyboard handlers are inherited Qt methods; its custom double
click emits a local resizing signal. The recovered event filter belongs to
the file browser. No verified screen UI→key packet→G1 response path exists.
The host EC consumer narrows the interpretation substantially but cannot
justify enabling physical navigation. Prior unsuccessful physical probes
remain evidence only for their tested encodings and conditions.

### H. Program management

Tree expansion **4315b0** requests identity if needed and invokes **4503b0**
→ **450460**, constructing a one-byte **F9** backup/inventory request.
It uses the same endpoint/HID route. F7 responses go through **44f280**
and model dispatch **42ff90**, which handles multiple object types 0…0B;
not every returned object is a program. CRC handling calls **5a2d20**.
F9 completion dispatches **44f206 → 42ff60**, marking model +26 complete
and clearing pending +27. This establishes an official application's
terminal marker interpretation, not proof that a captured prefix is complete.

Named reads at **44fc07** build a name/type packet using **59dec0**, change
the opcode to **F8**, calculate CRC and send via **5a21e0**. Name sizing
and termination differ across paths and must not replace Rust's physically
tested G1 read encoding without verification. No proven pagination scheme
or independent object-count guarantee was recovered. Rust's 54 objects /
five programs remain an incomplete inventory because no validated terminal
marker was observed for that run. Add terminal-F9 recognition only after
capturing its actual G1 response and validating framing/session boundaries.

### I. Physical Check and Run

Program editor Qt metadata (111c720, 111c0b4, 111c690) exposes editing,
save, tab and presentation actions; no physical Check/Run invocation was
established. Embedded calculator compilation/execution and host editing
are distinct from sending source and from execution on G1. No credible
new physical invocation packet was recovered. Capability gates remain.

### J. Reproducible controlled Windows experiment (not performed)

No suitable Windows analysis image was identified in the checked local
locations. Availability of qemu/virsh alone is not a Windows test environment.
Installer execution, USB passthrough and physical testing require explicit
user authorization; this continuation prepared the procedure only.

1. Prepare a disposable Windows VM snapshot without the calculator attached.
   Verify installer hashes above; retain a clean baseline. Disable automatic
   firmware update and synchronization in the analysis environment. Do not
   proceed if the application requires storage changes to observe screens.
2. After authorization, attach only the selected 03f0:1541 interface. Start
   USBPcap capture before attachment, filter Wireshark by that device address,
   and record descriptor/report descriptors. Save raw pcapng, timestamps,
   software versions and hashes. Avoid unrelated USB devices and personal data.
3. Use Process Monitor and debugger/API trace to identify the PID owning the
   HID path. Log CreateFileW arguments/result, attributes, HIDP_CAPS input/
   output lengths, ReadFile/WriteFile lengths, report-ID byte and completed
   asynchronous byte counts. Set breakpoints at module-base+RVA, not preferred
   VA: writer +14820, WriteFile call +1488f, reader +14900, scheduler +cce00,
   EC consumer +19c5b0. Correlate OVERLAPPED completion with its original call.
4. Record separate bounded idle, Monitor start/stop, Project start/stop and
   manual-capture sessions. Capture initial FA/control negotiation, exact FC
   formats, report lengths and timestamps. Compare request→first response,
   response transfer and next-request intervals. Test only one selected device.
5. With separately authorized input observation, log clicks/keys in Project
   and local UI while capturing outgoing packets and independently recording
   the calculator display. A local shortcut is not remote input. Require an
   exact packet plus attributable physical response before proposing a Rust
   input implementation. Never try arbitrary opcodes or brute-force key maps.
6. For approved screenshot-only testing, compare full FC08 against observed
   official incremental FC04, including unchanged image, visible movement,
   format switches and reconnect/full recovery. Verify coordinates, PNG,
   CRC, reconstructed pixels and timing. Do not infer speed from byte savings.
7. Inventory and execution investigation must be separate, explicitly scoped
   sessions. Do not upload, delete, run or alter calculator objects while
   tracing screens. Preserve captures and VM snapshot; do not touch firmware.

### K. Ranked Rust recommendations

1. **Completed:** offline PE trace tooling and preserved-log parser/replay;
   high evidence quality, no device risk, makes further claims reproducible.
2. **Best pending performance candidate:** FC04 RGB565 patch capture with a
   persistent canvas/full-frame recovery. High static evidence, unmeasured
   physical benefit and compatibility. Require the controlled capture above
   before implementing live transmission or changing defaults.
3. **Inventory correctness:** investigate terminal F9 on actual G1; moderate
   implementation effort, strong official UI evidence, no completeness claim
   until independent physical validation.
4. **Negotiated HID mode:** document/capture before implementing. Real code
   exists, but configuration, acceptance and throughput benefit are unknown.
5. **Physical keys/Check/Run:** retain unsupported capabilities; missing firmware
   acceptance and UI producer evidence outweigh host-side event resemblance.

No Rust production behavior was changed in this continuation. Host PNG
conversion remains a low-priority performance target given the measured
pre-response delay. No speculative key or stream command was added.

### L. Verification and limitations

Five offline Python unit tests passed, covering a hand-built PE fixture,
fragmentation including sequence wrap, malformed/truncated reports, CRC and
patch replay. All 100 preserved format captures passed offline parsing.
Focused disassembly and table evidence total less than 1 MB. No installer,
Windows process, physical USB operation, Cargo build or firmware test was
run in this continuation. `git diff --check` passed. No commits or pushes.

The two previously recorded asset-enabled cold-boot stable-screen timeouts
(m4a/m4b, 60-second deadline) remain separate failures; their thresholds were
not changed. They concern emulated boot/event-pump state. Neither PE static
analysis nor the preserved physical HID PNG evidence depends on those boot
tests, so they do not invalidate these traces. They limit claims about
emulated guest boot/input and are not reported as newly passing tests.

## Preserved earlier investigation

The following is the earlier evidence record. Its explicitly unresolved
caller/consumer statements are superseded by the continuation above; prior
physical observations and safety boundaries remain in force.


## Conclusion

No physical G1 remote-input operation is established by the inspected
Connectivity Kit binary. Its receive path recognizes opcode `EC` and queues
bytes from the packet as an internal event. The inspected code does not show
that those bytes are physical key identifiers, define key width/modifiers, or
prove that G1 build 15157 accepts this path. No packet from this investigation
was sent to a calculator.

The Rust CLI therefore continues to refuse UI navigation, Check, and Run on
physical hardware. Existing physical screenshots and read-only program reads
remain available through the serialized session owner. The partial inventory
continues to report `inventory_incomplete` because no validated completeness
condition has been established for the reference firmware.

## Evidence and provenance

### Extracted binary

The retained installer extraction is under the ignored
`scratch/research/autonomous-control-investigation/` directory. The extracted
32-bit executable is:

| File | SHA-256 |
|---|---|
| `ck/ConnectivityKit.exe` | `a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb` |
| `payload.cab` | `923797dcb582e94c13089a6d5b682af8da1096599c066555d61835418a988904` |
| `msi/cab1.cab` | `e1d4c90ef09bfd42b1f07401944a1b222eabd77e929bd0b9f465b2222d477f1d` |

The executable is PE32/i386 with image base `0x00400000`. Its `.text` section
has RVA `0x1000`, virtual address `0x00401000`, and file offset `0x400`.
Thus, for these `.text` addresses, `file_offset = VA - 0x00401000 + 0x400`.
This build is distinct from the 64-bit Connectivity Kit executable used in
other protocol research; its virtual addresses must not be mixed with that
binary. The original installer executable was not present in the retained
extraction, so this record does not claim an installer hash.

### Static binary observations

Addresses below are virtual addresses from the PE image. File offsets are
calculated using the `.text` mapping above.

| Address | File offset | Observation |
|---|---:|---|
| `0x450320` | `0x04F720` | Allocates a 0x26-byte internal message beginning with `FC` and a caller-supplied subcommand byte, initializes message fields, and passes it into the application's internal dispatch path. The caller's purpose and exact event semantics have not been established. It is **not** evidence of a screenshot request. |
| `0x59C5B0` | `0x19B9B0` | Internal byte-stream dispatcher. Its opcode switch begins at `0x59C680`. |
| `0x59C680` | `0x19BA80` | Dispatches the internal `ED`–`FB` opcode range. This is an application protocol parser, not a demonstrated key table. |
| `0x59E2B5` | `0x19D6B5` | The `EC` branch in the receive-side dispatcher. It calls the packet-length helper at `0x59DFE0`, copies packet bytes beginning at offset 6 into an allocated buffer, builds an internal event record, links that record into a queue, and signals the application's event mechanism. |

The `EC` branch's packet-to-event copy is consistent with delivery of an
opaque byte payload to another part of the Connectivity Kit application. This
observation does not identify the event consumer's interpretation. In
particular, the inspected branch alone does not prove whether these bytes mean
keys, commands, or another event payload; whether values are one or multiple
bytes; whether modifiers or press/release states exist; or whether a handshake
is required.

The executable's import table names `HID.DLL` functions
`HidD_GetHidGuid`, `HidD_GetAttributes`, `HidD_GetPreparsedData`,
`HidP_GetCaps`, and `HidD_FreePreparsedData`; `SETUPAPI.dll` functions for
enumerating and opening device interfaces; and `KERNEL32.dll` file I/O
functions including `CreateFileW`, `ReadFile`, and `WriteFile`. Imports prove
that the executable can use these APIs. They do not, by themselves, establish
the report sizes, complete connection handshake, framing on the physical
wire, or which UI event reaches which HID write.

The executable's retained strings and RTTI also name the UI types
`CCalcScreenProjectorWindow`, `CCalcScreenMonitorDockWidget`, and
`CCalcScreenCaptureWindow`, plus the tree actions `onTreeViewMonitor`,
`onProject`, and `onTreeViewGetScreenCapture`. This confirms that monitor,
projector, and capture UI components exist in this build. The string/RTTI
evidence does not establish that monitor/projector input is forwarded to the
calculator, nor that the widgets are display-only. A complete event-filter and
transport-caller trace for those widgets has not been established; inherited
Qt key handlers and widget names are not used to infer either result.

### Existing source and physical evidence

The pinned historical `hplp` source documents its version-1 `EC` message as
`EC 01 [count u32 BE] [one-byte values]`; that source and its supported
firmware list do not prove compatibility with build 15157. Prior physical
probes in the retained evidence used candidate EC encodings and did not
produce the expected screen changes. Those results rule out only the tested
encodings and conditions. They do not validate another encoding.

Local original firmware and the emulator's `GetEvent` mapping describe
firmware-internal input events. They are not proof of the Connectivity Kit's
USB wire encoding. The local original-firmware image is G1 2.4.2; the
reference physical calculator reports build 15157 and boot `V0.050.640`.
These firmware versions are kept separate in the evidence record.

The reference physical profile is USB `03f0:1541`, firmware build 15157,
boot `V0.050.640`. Its existing screenshots, exact program reads, and partial
inventory are read-only observations. No fresh screenshot-based navigation
acceptance was performed for this investigation.

## Rust implementation boundary

`ppl-device` retains the typed, exclusive `Prime`/screenshot session and the
bounded local IPC owner. The IPC request enum exposes status, screenshot,
program read, inventory, and host-side stop; it exposes no raw packet or key
operation. `ppl-cli` leaves `remote_keys`, `remote_navigation`, and
`remote_execution` disabled, and the physical UI/Run commands return
`unsupported_capability` before upload or input transmission.

The inventory parser can preserve CRC-valid returned objects, but a valid
prefix is not proof of a complete catalog. No verified terminal condition or
independent completeness proof exists for this G1 profile, so
`inventory_incomplete` is intentional.

## Reproduction

From the repository root, inspect the retained binary without executing it:

```sh
sha256sum scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe
file scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe
objdump -h scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe
objdump -d -Mintel --start-address=0x450320 --stop-address=0x4503a5 \
  scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe
objdump -d -Mintel --start-address=0x59c5b0 --stop-address=0x59c6d8 \
  scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe
objdump -d -Mintel --start-address=0x59e2b5 --stop-address=0x59e3a6 \
  scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe
```

These commands reproduce the listed machine-code observations. They do not
recover C++ symbols or prove the behavior of absent callers, downstream event
consumers, another Connectivity Kit build, or the physical calculator.

## Remaining blockers

Before a physical input probe can be specified, the event consumer and the
transport path must establish a concrete packet structure, command direction,
key/event semantics, and firmware compatibility. Then the proposed exact
payload and bounded actions must be reviewed and specifically approved by the
user before transmission. At present there is no justified candidate packet,
so no physical acceptance sequence was run and no navigation capability was
added. The monitor/projector event-filter and HID-caller trace is also
unfinished. An external actuator would be a separate hardware architecture
and is outside this verified USB capability.

Evidence categories: the addresses, fields, imports, and file offsets above
are static binary observations; the capability gates and session behavior are
Rust implementation observations; historical screen results are preserved
physical observations; the `hplp` format is external source documentation;
any interpretation beyond those facts is explicitly unverified.
