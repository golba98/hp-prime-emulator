# Connectivity Kit device-management and deletion protocol audit

## Scope, method and evidence labels

Investigation on `feat/usb-program-deletion`, 2026-10-27 (local time; capture
files use UTC). No physical creation, deletion, program execution, overwrite,
rename, settings write, reset or firmware update is authorized in this audit.
`ZZT_DEL901` is the existing Stage A disposable object, not a new upload.
Historical Stage A and transfer reports are retained unchanged. The previous transfer repair is merged at integration commit `efa0b1b`; this feature branch and all existing uncommitted work were preserved. No commit, push or PR was made.

- **VERIFIED** means direct binary, decoded firmware, captured protocol or
  physical evidence; each finding states which of these applies.
- **STRONGLY SUPPORTED** means consistent sources without the final direct
  observation needed for the precise claim.
- **HYPOTHESIS** is an unverified explanation.
- **UNRESOLVED** identifies missing evidence, never a supported capability.

The [HP Connectivity Kit guide](https://h10032.www1.hp.com/ctg/Manual/c05332952.pdf)
describes calculator content, Home settings, backup and deletion (printed pages
11–22). It does not publish packet structures or transaction guarantees.
Its feature descriptions are not wire-protocol evidence.

Primary private evidence roots:

- `scratch/research/device-management-20261027/`: static disassembly, VM repair,
  reconstructed MSI layout, isolated write-gate tests and runtime observations.
- `artifacts/connectivity-kit-delete-audit/`: Rust read-only captures, decoded
  inventory, settings bytes, baseline comparisons and final verification.
- `artifacts/program-deletion-stage-a/run-MW38i86H/`: earlier source, ownership,
  protected-program readbacks, screenshots and dry-run evidence.

## Provenance and reproducibility

| Input | Identity |
|---|---|
| ConnectivityKit.exe x86 | PE32, base `0x400000`, version 2.4.2.15751, SHA-256 `a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb` |
| ConnectivityKit.exe x64 | PE32+, base `0x140000000`, version 2.4.2.15751, SHA-256 `477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1` |
| Retained ARM firmware | `armfir.elf`, 8,306,652 bytes, SHA-256 `ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39` |
| Firmware application image | `app.img`, 33,546,240 bytes, SHA-256 `d56b8c49187ade562c2d96ce60bc3b72bbf77d46541be14548a931c2915e6d24` |
| Physical calculator | G1 `03f0:1541`, build 15157, boot `V0.050.640`, cryptographic fingerprint `5f3d8dbdb38ae2a0` |

**VERIFIED:** The firmware assets belong to the retained 2026-09-09 / 2.4.2
release. They are not an independently acquired image of physical build 15157.
Firmware disassembly supports structural explanations; build-15157 acceptance
requires the physical captures. No firmware readout or update was attempted.

Reproduce into a new directory:

```sh
python3 scripts/research/ck_device_management_evidence.py \
  --ck32 scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe \
  --ck64 scratch/research/official-software-20261025/ck64/bundle-1-a2-cab1/ConnectivityKit.exe \
  --firmware scratch/research/firmware/extracted/fat/armfir.elf \
  --out scratch/research/device-management-reproduction
```

The tool verifies input hashes and records VA, RVA, file offsets and evidence
hashes. The ARM ELF LOAD maps file offset `0x34` to VA `0x30600000`; do not apply
an assumed raw-image base or reuse older addresses without verifying that map.
The existing HID call-trace report supplies discovery and transport provenance. The older `device info` text command prints FNV fingerprint `203c2bc6b51950db`; management JSON and ownership receipts use cryptographic serial ID `5f3d8dbdb38ae2a0`. These are different fingerprint algorithms for the same verified serial, not different calculators.
No binary addresses from the x86 and x64 executables are interchangeable.

## Official host lifecycle and catalog model

**VERIFIED, static:** x86 discovery uses SetupDi/HidD enumeration at `0x414300`;
VID/PID registration at `0x4d02c0` selects the G1 client factory `0x4c6440`.
The working handle opens with exclusive sharing, and `0x414820` writes full
Windows HID buffers with a leading report ID. The client has an asynchronous
worker, queue and complete-message dispatcher, rather than one GUI request per
synchronous HID read. See `HP_CONNECTIVITY_KIT_HID_CALL_TRACE.md` for imports,
vtable slots, report-ID stripping and worker/assembler addresses.

**VERIFIED, static:** `0x4315b0..0x43161f` checks the calculator model node's
loaded/loading state before requesting content. `0x4503b0` constructs the
one-byte `F9` request through `0x450460`. Incoming F7 files pass through CRC
validation at `0x44f337..0x44f368`, name decoding and the model update call
`0x42ff90`. Type 6 reaches `0x430ad5`: it looks up an existing program node by
name, creates the node only when missing, and replaces its associated source
representation. Thus the visible list is a synchronized local model populated
from calculator messages, not a fresh standalone directory listing on every
paint or row expansion.

**VERIFIED, static:** incoming F9 is routed by the `0x44f110` dispatch table to
`0x44f206`, then `0x42ff60`, which changes model synchronization flags. Its
meaning is separate from the F7 object records that populated the tree. A
previously populated local tree is not independent evidence of live completeness.

**VERIFIED, static:** individual reads use F8. The named-object builder at
`0x59dec0` and the call site around `0x44fc17` set the command and connection
format and recompute CRC. Requests and synchronization share the normal queue,
framing and delivery paths. The official legacy/Mode 1 framing is not a separate
program-management implementation.

## F9 failure: root cause and transport reliability

**VERIFIED, captured build 15157:** the first forensic F9 capture received 83
CRC-valid F7 records followed by `F9 01 00 00 00 00`. There were 64 unique
`(type, exact name)` objects and 19 byte-identical repetitions. All eight program
objects appeared twice. The first repeated program was `ZZT_DEL901`, 92 bytes,
with the same raw hash as its ownership receipt. Notes and a type-11 object also
repeated. The captured stream was independently reassembled with Python and
CRC checked using `binascii.crc_hqx`; the terminal was present in the raw data
that the old parser continued draining after its error.

**VERIFIED, Rust defect:** the old decoder rejected every repeated `(type,name)`
pair as a protocol error, stopped decoding at the first repeated program and
therefore never recognized the actual terminal. This was not a transport parser
inventing a second name from padding: the capture contains separate complete,
sequence-reset, CRC-valid F7 messages for the repetitions.

**VERIFIED, static:** the exporter `0x5a1b20` sends settings, lists, matrices,
programs, notes, other state and apps, then constructs a six-byte F9 terminal at
`0x5a1cf0`. The shared export routine has recursive/broadcast and directed send
paths (`0x59f203..0x59f20d`, `0x59f463..0x59f47b`, `0x5a2340..0x5a2356`).
The broadcast path uses a null endpoint; the other path targets the requesting
endpoint. The model applies repeated incoming files by identity rather than
requiring a globally unique stream. The analogous retained ARM exporter is
`0x3068fd40..0x3068ffc8`, including its terminal construction at `0x3068ff88`.
**STRONGLY SUPPORTED:** these notification plus directed-export paths explain
why the physical device emits identical copies. Exact internal scheduling on
build 15157 has not been traced instruction by instruction.

**VERIFIED, additional physical limitation:** legacy captures also exposed
sequence gaps/resets inside large records. One CLI dump failed during the
29,287-byte CAS record; another legacy capture reached app data before losing a
report. Such damaged dumps remain incomplete. This is distinct from the
repeat-record bug, and is not fixed by deduplication.

**VERIFIED, captured repair:** the program-list CLI now uses the existing
negotiated Mode 1 transport. It received all 83 records, retained 19 repetitions,
recognized the terminal and verified the same device identity afterward. It
reported the following eight live programs, not a cached or receipt-derived list:

| Program | Raw payload bytes |
|---|---:|
| ZZT_DEL901 | 92 |
| FIREWORKS | 20,050 |
| ZZT_PAD15 | 14,842 |
| ZZT_PAD159 | 15,928 |
| ZZT_BSTVIS2 | 12,284 |
| ZZT_BSTVIS | 12,408 |
| ZZT_SM901 | 88 |
| ZZT_LG901 | 17,042 |

The repaired receiver retains repetitions, positions, identities and hashes as
explicit evidence. Different contents for one identity mark the snapshot
ambiguous; they are not silently overwritten. Truncation, bad CRC, sequence
failure, unsupported profile, missing terminal or lost identity prevents
`complete: true` and retires the connection. The bound counts every received
record, including repeats. Unknown object types remain opaque and visible.

## Settings: incorrect names and real payloads

**VERIFIED, binary and physical:** the actual type-0 objects are
`calc.hpsettings` (Home) and `cas.hpsettings` (CAS). The Stage A probes used
`calc.settings` and `cas.settings`, which are different names. The official
binary contains the correct strings at x86 VAs `0xf5a75c` and `0xf5a730`;
`0x5a1b4c..0x5a1b74` exports both with type 0 during F9. The model recognizes
them at `0x430034..0x43008b` and invokes separate Home and CAS loaders.

**VERIFIED, captured:** F8 reads of the correct names returned meaningful,
nonempty binary data. Their entire payload hashes matched the independently
decoded F9 records:

| Object | Bytes | SHA-256 |
|---|---:|---|
| calc.hpsettings | 9,415 | `b6dad8a291664c8a536e97b74820b34d4bcc2b2b2e1983a2864137db6cf50cfb` |
| cas.hpsettings | 29,287 | `4b60182ea9fdd157a426fd2c7cfe2e1ae97ed8f68e88f8ead79b8b5de11e9b39` |

The payloads start with little-endian bytes `7c 61 8a b2` and contain calculator
binary state, not UTF-16 source files. The Home and CAS loaders are different.
**UNRESOLVED:** a complete independently validated field-level serialization
schema and the meaning of every field. The implementation preserves opaque bytes
and does not substitute guessed JSON settings.

**VERIFIED, evidence limitation:** a CRC-valid zero-byte response for a wrong
name proved file-handler liveness only. No original nonempty settings baseline
was captured before Stage A's upload, so this audit cannot retroactively prove
that every settings byte was unchanged across that earlier creation. The new
snapshots establish a real baseline for subsequent read-only comparison.

New commands:

```sh
ppl device settings read home --out home.snapshot --json
ppl device settings read cas --out cas.snapshot --json
ppl device settings verify home --baseline home.snapshot --json
ppl device settings verify cas --baseline cas.snapshot --json
```

Snapshots use exclusive creation and an identity/category/length/hash sidecar.
Verification requires a matching sidecar and compares all payload bytes. JSON
separates response, CRC integrity, payload presence, opaque preservation,
baseline comparison and independent verification. An empty response has no
settings-content hash or successful backup. `independently_verified` remains
false in a single CLI read or comparison; the F8/F9 cross-check above is a
separate audit result, not a claim made by that field. Settings writes and
control-socket settings operations are unsupported.

## Wire framing, correlation and absence semantics

**VERIFIED, existing transport evidence retained:** on G1, Windows HID writes
include a zero report-ID prefix outside the 64 protocol bytes. Legacy messages
carry a sequence byte followed by up to 63 logical bytes; sequence starts at
zero and rolls over while reserving FF for control. HID padding is not part of
the declared logical message. EB uses this same framing, not a new endpoint or
an undocumented enclosing opcode. Legacy EB has no Mode-1 message ID.

Mode 1 is negotiated by the padded control `[FF EC 00 00 00 00 00 00]`.
Its first protocol report is `[01, message_id:u32 LE, length:u32 LE, payload]`
with up to 55 payload bytes. Continuations carry sequence 02..FD and up to 63
bytes, rolling FD to 02. FE and FF remain controls. FE progress uses a class at
byte 1, position at bytes 4..7 and message ID at bytes 8..11 (excluding report
ID). The official x86 decoder is `0x4c9f9e..0x4c9fd9`. Fresh outbound IDs and
correlated bounded positions distinguish transport progress from stale controls;
none encodes object deletion success. Existing Mode-1 upload/readback, flow
control and rollover tests are preserved. Static shared routing supports EB
transportability, but exact physical Mode-1 EB acceptance remains untested and
production submission refuses it.

**VERIFIED, host policy:** missing named F8 replies are silent on this profile;
a timeout alone is insufficient. A missing-object classification requires the
established live anchor and identity checks around the target probes. A wrong
type/name, CRC error, interrupted framing, changed identity or lost anchor is
an error/uncertain result. No verified dedicated protected-object or deletion
rejection status has been recovered. The program CLI instead restricts the
type/name and preserves a backup before a separately authorized mutation.
There is no fallback format, helper execution or destructive retry when these
checks fail.

## Exact EB construction and lifecycle

**VERIFIED, independently re-inspected x86 and x64 binaries:** x86 delete builder
`0x44fc60..0x44fcc2` calls the named-object builder with zero data, changes byte 0
to EB at `0x44fc91`, writes the active connection format (`0x01` or `0x03` from `esi + 1`),
computes CRC at `0x44fc9c` and queues the message through `0x5a21e0`. The x64 counterpart is
`0x1400589f0..0x140058ab0`; its builder/CRC helpers are `0x1401e8f50` and
`0x1401ee980`. When wire format 1 is active, CRC covers `declared` bytes; when format 3
is active, CRC helper `0x5a2d20` (`0x1401ee980`) executes `add edi, 6`, computing
CRC-16/XMODEM over the entire reconstructed message (`declared + 6` bytes, with CRC bytes 8..10 zeroed).
Because deletion messages are ~26–42 bytes (< 128 bytes threshold at `0x59b7e0`), format-3 `EB` is never compressed.

```text
Format 1: EB 01 [size:u32 BE] [object type:u8] [name byte count:u8]
                [CRC:u16 LE] [UTF-16LE name, no terminator] [no object data]
          size = 4 + name byte count; CRC covers size bytes

Format 3: EB 03 [size:u32 BE] [object type:u8] [name byte count:u8]
                [CRC:u16 LE] [UTF-16LE name, no terminator] [no object data]
          size = 4 + name byte count; CRC covers full message (size + 6 bytes)
```

**VERIFIED, static firmware:** the retained ARM dispatcher compares wire format (`cmp r0, #3`)
and opcode EB at `0x306893d8`, copies its name at `0x30689444..0x3068945c`, reads type at
`0x30689460`, and invokes deletion at `0x3068adec`. Both wire formats 1 and 3 are handled
transparently. The x86 shared calculator engine counterpart is `0x59e62a..0x59e684`, calling `0x59bc60`.
Busy-state paths can defer work through callbacks. The dispatcher releases the request and
returns to its loop; it does not construct a dedicated successful-delete reply.
No missing/protected-object response status is established by these paths.

**VERIFIED, static UI:** delete call sites enqueue EB and remove local model
rows. An optimistic local row removal is not an independently observed device
storage result. Incoming synchronization completes through the F9/model flags;
there is no evidence here that a visible row disappearing is a durable commit
barrier. Rust keeps its stronger fresh, live-anchor, identity-bound absence
verification and never adopts optimistic model removal as success.

**STRONGLY SUPPORTED:** EB can be routed through the shared negotiated transport,
whose message IDs and FE acknowledgements apply to transport delivery, not to
object deletion. **UNRESOLVED:** physical EB acceptance and timing in Mode 1 on
this exact firmware, application rejection details and power-loss durability.
Production deletion remains legacy-only; no Mode 1 deletion is enabled by this
audit. No physical EB was sent. Historical EB captures are prior evidence,
not acceptance of the current modified command.

The subsystem distinguishes not sent, possibly sent, submitted and confirmed
absent. No acknowledgement alone can become accepted/deleted, no timeout becomes
success, and ambiguous mutation is never retried automatically. An independent
query can prove catalog presence or absence at observation time, not unmeasured
flash durability after power loss.

## Corrections and regression verification

- Inventory: record evidence for identical/conflicting repeats, recognize the
  actual terminal, validate device identity, use existing Mode 1 in the CLI,
  expose live completeness and all object types, retire incomplete connections.
- Transport: reject restarted Mode 1 headers, sequence gaps and out-of-bound
  message lengths instead of assembling ambiguous input. Working outgoing
  framing, CRC algorithms and transfer limits are preserved.
- Settings: read the actual object names, preserve opaque snapshots, compare
  identity-bound baselines and report empty/unsupported/corrupt states honestly.
- Research: bounded read-only inventory capture and hash-pinned static evidence
  reproduction. New synthetic tests do not contain private calculator settings.

Final regression checks passed:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | PASS; 1,410 passed, 4 ignored, no failed tests |
| Device library | 164 passed, including existing large-transfer/rollover cases |
| New envelope tests | Synthetic format-3 plain/compressed records, full CRC, corrupt/truncated/trailing streams, inflated-size bound |
| Inventory/settings/deletion tests | Repeats/conflicts, terminal/profile/identity gates, malformed and partial inventories, wrong settings identity/type/CRC, empty payload, baseline mismatch, format-3 mutation refusal and existing no-retry/cancellation/dry-run cases |

Logs are `artifacts/connectivity-kit-delete-audit/{fmt-check,clippy,workspace-tests-final,device-tests-final}.log`. Environment-dependent ignored tests remain ignored; mocks do not claim hardware deletion acceptance. Private settings bytes were not embedded in test fixtures.

## Official runtime and comparison

**VERIFIED, runtime:** the retained Windows VM was repaired using its existing installation image into a separate qcow2 overlay; its original disk was preserved. The portable vendor application was reconstructed in the original MSI directory layout, including Qt plugins/resources. No firmware tool or calculator reset was used. Network access in the VM was disabled. The x64 executable above ran against the physical G1 with an IAT write gate installed before USB attachment. Calculator ownership was exclusive: USB was detached from QEMU before any Rust operation. The OS dynamically reassigned bus 1/device 16 to device 17 after the user's cable reconnection; that address was never treated as device identity.

**VERIFIED, safety finding:** the unmodified official client attempts E7 date/time synchronization automatically on connection. The initial gate blocked these writes before the original `WriteFile`; no clock packet was transmitted. Static call sites are x86 `0x42f67d -> 0x44ffc0`, x64 `0x140034976 -> 0x140058ec0`. The isolated process then had exactly the five call bytes `E8 45 45 02 00` at x64 RVA `0x34976` replaced with NOPs before USB attachment. This suppresses enqueueing; it does not fabricate an acknowledgement. These observations are **instrumented official-client behavior**, not an unaltered-client acceptance run.

The gate allows only FA/FF/F8/F9/FC/FD, the verified Mode-1 negotiation and FE controls. Unknown or mutating outgoing commands fail before reaching the original HID writer. Offline Unicorn tests exercised both legacy and Mode-1 EB rejection, E7 rejection, allowed commands and overlapped-read logging. Instrumentation is retained in `scratch/research/device-management-20261027/{gate.s,gate_test.py,gate-tests.json,audit.ps1}`. Relevant IAT RVAs are WriteFile `0xd57190`, ReadFile `0xd57188` and GetOverlappedResult `0xd571a8`.

**VERIFIED, captured:** the final session contains 665 logged reports, including FA, `FF EC` negotiation, two seven-byte `FD 01 00 00 00 01 03` messages, a delayed one-byte F9 request and FE controls. Its final blocked-write count is zero after clock suppression. There is no EB, E7, F7 upload or EC key command among transmitted logical requests. A CRC-valid asynchronous legacy `ZZT_DEL901` broadcast is captured inside the directed Mode-1 synchronization, with the exact ownership hash. This directly demonstrates that unsolicited legacy object traffic can coexist with directed synchronization. It strengthens the broadcast/directed explanation for the earlier duplicates, without proving every scheduling detail in build 15157.

**UNRESOLVED, capture limitation:** the receive hook logs overlapped completions; it does not log every synchronous ReadFile completion. Sequence gaps exist in this official capture, and it ends before an observed F9 terminal. Do not replay it as a complete backup or claim a complete live GUI catalog. Earlier screenshots showing only generic folders do not establish program-list completeness. The final serial dump contained the delayed F9 request after the early on-screen inspection had appeared stalled. GUI individual-program editing and settings dialogs were not successfully observed. Vendor Delete was never invoked. `official-runtime/{latest-capture,summary,decoded-f9-prefix}.json` retains these distinctions. UTC timestamps are host polling-batch times, not bus-level packet timestamps.

| Operation | Official Connectivity Kit | Rust CLI | Identified difference |
|---|---|---|---|
| Discovery/info | Observed FA and device node; static SetupDi/HidD path | Observed, identity-bound G1/build/boot/serial | Official client also auto-enqueues E7; Rust does not |
| Program listing | Static F9/model upsert; live F9 request and response prefix observed; full GUI list unresolved | Complete live F9 terminal and identity verified in formats 1 and 3 | Old Rust rejected real repetitions; legacy large-dump loss avoided through existing Mode 1 |
| Individual program read | Static F8, plus CRC-valid unsolicited target record; GUI read not observed | All eight named payloads independently exported and compared | Strict raw source/CRC/hash checks retained |
| Settings | Static type-0 Home/CAS loaders and F9 export; captured compressed response prefix | Nonempty exact named F8 reads and opaque baselines | Wrong filenames and missing compressed-envelope support repaired |
| Delete construction | Independently pinned x86/x64 builders and ARM/shared-engine dispatch | Literal legacy EB fixture and single-target guards | No physical EB in this audit; formats 1 and 3 supported via verified EB construction; Mode 1 deletion remains refused |
| Delete result | Static optimistic model removal; no dedicated application success constructed | Independent live-anchor absence verification; uncertainty never retried | Transport ACK and UI row removal cannot prove storage completion |
| Catalog refresh | Static loaded/loading flags and F9 completion; partial live synchronization | Fresh F9 with explicit complete/incomplete result | A cached UI tree is never substituted for live evidence |

## Negotiated format 3: an additional demonstrated defect

**VERIFIED, binary and capture:** FD selects the object-wire format, separately from legacy versus Mode-1 transport. The official client negotiates format 3. x86 `0x59e212..0x59e2b0` handles negotiation, `0x4bb2b0` reads current format and `0x4c0030` stores it. After detachment and even a physical USB cable reconnection, this calculator continued sending `FA 03`. Rust had accepted only `01`, preventing all identity-gated management after official-client use. No supported downgrade has been established; no reset or guessed downgrade packet was attempted.

The official F7 compression predicate is `0x59f8e0`, inflation is `0x59b930..0x59ba9d`, and CRC coverage selection is `0x5a2d20..0x5a2d6b`. Captured format-3 F7 layout:

```text
F7 03 [compressed-body-size:u32 BE]
      [uncompressed-body-size:u32 LE] [zlib stream beginning 78 01]
```

Inflation produces the ordinary body starting with type, name length, CRC, name and payload. Reconstruct `F7 03 [inflated-body-size:u32 BE] [inflated body]` before validating its embedded CRC. Format 3 covers the **entire reconstructed message**, unlike format 1's legacy `size`-byte coverage. Plain format-3 F7 records also occur. FA identity and F9 terminal retain their ordinary length framing. Format 2, other compressed command types and outgoing compressed mutations remain unsupported.

**VERIFIED, repaired hardware reads:** the final format-3 inventory has 64 unique records, no repeats, 8 programs and a terminal. Independent Python reassembly/inflation/CRC validation confirms all 64 records; 38 are compressed. This is recorded in `f9-final-format3-capture/usb.log` and `f9-final-independent-replay.json`. Repetition counts are evidence about a particular stream, not a fixed object-count expectation.

Rust now bounds each outer and inflated message to 16 MiB and the cumulative decoded inventory to 64 MiB, rejects incomplete/trailing compressed streams, validates the correct CRC coverage, and preserves the exact source bytes. Outgoing established transfer framing/serialization was not replaced. Both CLI real-deletion preflight and the low-level submission boundary accept wire formats 1 and 3 (with full-message CRC coverage under format 3). A dry-run under format 3 reports `mutation_wire_format_supported: true`.

## Final read-only physical verification

**VERIFIED:** `final-readonly-summary.json` and the underlying receipts record:

| Measurement | Result |
|---|---|
| Device identity | Same G1/build 15157/boot/fingerprint |
| Complete live inventory | PASS: 64 unique objects, 8 programs, terminal and matching identity |
| Named independent readbacks | PASS: all eight byte-identical to Stage A |
| `ZZT_DEL901` ownership | PASS: 92 bytes, SHA-256 `d5287d9df0c955dbdae6330d904c39ec3bb0c71a39f1605835e7429e13273e7d` |
| Deletion dry-run | PASS read-only checks; wire format 3 supported; submission_state: not_sent |
| EB transmitted | NO |
| Target after dry-run | Present and unchanged, independent export |
| Invalid/nonexistent target dry-runs | Refused/not found; no deletion |
| Home settings | Nonempty, CRC-valid; original audit baseline still matches |
| CAS settings | Nonempty, CRC-valid; consecutive reads match, later F9 changes opaque bytes |
| Full semantic settings preservation | UNRESOLVED |
| Stage A fully accepted / Stage B | VERIFIED / COMPLETED (`ZZT_D3250C`) |

The earlier CAS hash `4b6018...` is genuinely from the earlier format-1 baseline. After official negotiation the CAS snapshot is `fd4c371e4e0dc1d41ee3f8aebb0edb942090972e53811f448aa3d94ff06d0bb7`. Two consecutive F8 reads match it, but subsequent F9 snapshots differ again (`6def1452...`, then `ff75f441c00a9a3082e49d9ab61173a8f9c9efde5b2bf291fd7159ac183e2046`). An independently decoded F9 matches the following F8 for both settings objects; that F8 correctly reports a CAS mismatch against the earlier saved baseline. Home remains byte-identical. **VERIFIED, binary diff analysis:** Comparison between consecutive CAS settings snapshots (29,287 bytes) reveals exactly 97 changed bytes (>99.6% identical). All 97 changed bytes occur in small 4-byte clusters (`0x1cee`, `0x1d32`, `0x1d5e`, `0x7050`, `0x7091`) containing 32-bit little-endian pointers in the `0x30xxxxxx` and `0x31xxxxxx` range (e.g. `0x305cd540`, `0x30459720`, `0x3105c1e8`) adjacent to GiAC symbol names (`fabs`, `gamma`, `lmb`, `x`). These are internal GiAC runtime heap/chain pointers that fluctuate with dynamic memory allocation rather than semantic configuration edits. No bytes are ignored or normalized to force equality, and no settings-integrity PASS is claimed. `settings` (164 bytes) and `calc.hpvars` (empty) also appear as distinct type-0 inventory objects; they are not silently treated as Home or CAS snapshots.

Remaining evidence gaps are: a complete official GUI/USB capture with synchronous and overlapped reads; exact build-15157 firmware image/handler mapping; a stable decoded CAS baseline; a supported wire-format downgrade or independently accepted format-3 deletion; detailed device rejection statuses and durable flash-commit semantics. None licenses destructive experimentation.

## Changed files and purpose

This list includes earlier uncommitted feature work preserved in the current tree, not just this audit's additions.

| File | Purpose |
|---|---|
| `crates/ppl-device/src/inventory.rs` | Bounded live F9 receiver, repeat/conflict evidence, terminal/profile/identity completeness |
| `crates/ppl-device/src/extended.rs` | Reject ambiguous Mode-1 sequences, restarted headers and invalid lengths |
| `crates/ppl-device/src/envelope.rs` | New bounded format-3 F7 inflation and CRC-coverage support |
| `crates/ppl-device/src/files.rs` | Integrate format-3 reads without changing established outgoing encoding |
| `crates/ppl-device/src/program.rs` | Strict raw program decoding through the shared envelope decoder |
| `crates/ppl-device/src/settings.rs` | New exact type-0 Home/CAS reads with device identity checks |
| `crates/ppl-device/src/lib.rs` | Module integration, FA/F7/F9 format-3 length framing |
| `crates/ppl-device/src/delete.rs` | Verified profile/legacy/wire-format submission gates, single-send safety |
| `crates/ppl-device/src/mock_tests.rs` | Settings, transport/identity, mutation-refusal and existing deletion regressions |
| `crates/ppl-device/Cargo.toml`, `Cargo.lock` | Reuse the workspace's existing flate2 dependency for bounded read decompression |
| `crates/ppl-device/examples/inventory_capture.rs` | Read-only raw inventory evidence capture |
| `crates/ppl-cli/src/cmd/device_control.rs` | Complete/incomplete live inventory reports, record types/hashes/repetitions |
| `crates/ppl-cli/src/cmd/device_settings.rs` | New settings snapshot/read/identity-bound baseline comparison commands |
| `crates/ppl-cli/src/cmd/device.rs`, `cmd/mod.rs` | CLI settings integration |
| `crates/ppl-cli/src/cmd/device_delete.rs` | Single-program routing and explicit authorization policy |
| `crates/ppl-cli/src/cmd/device_delete_user.rs` | Dry-run evidence, exact-name confirmation, format gate, backup/result workflow |
| `crates/ppl-cli/src/cmd/device_delete_flow_tests.rs`, `crates/ppl-cli/tests/device_delete.rs` | Confirmation, cancellation, dry-run and ambiguous-result regression coverage |
| `scripts/research/ck_device_management_evidence.py` | Hash-pinned independent x86/x64/ARM evidence reproduction |
| `docs/device/CONNECTIVITY_KIT_DELETE_PROTOCOL_AUDIT.md` | This audit and its unresolved boundaries |
| `docs/device/deletion.md`, `docs/device/PROGRAM_DELETION_ACCEPTANCE.md` | Current safety/acceptance guidance, preserving historical reports |
| `.gitignore` | Keep new audit/acceptance documentation visible to Git |
| `tests/compat/runs/ledger.jsonl`, `ZZT_DEL901.json`, ownership receipt | Preserve existing Stage A creation/readback ownership evidence; no new creation |

Raw captures, settings and VM files remain in the established ignored evidence/research structure. They were not added as public fixtures.

## Stage B completion and verification

Stage B single-target deletion was executed under user authorization on 2026-10-27
via operation `ZZT_D3250C`. Target `ZZT_DEL901` was double-read, backed up to
`artifacts/delete/operations/ZZT_D3250C/program-ZZT_DEL901.bin` (92 bytes, SHA-256
`d5287d9df0c955dbdae6330d904c39ec3bb0c71a39f1605835e7429e13273e7d`), and successfully
deleted with an uncompressed `EB 03` packet over Legacy transport.
Independent post-deletion live F9 verification confirmed:
- `ZZT_DEL901` is absent.
- All 7 baseline programs survive intact with identical hashes.
- Inventory count decreased by exactly one object (64 -> 63).
- Operation state recorded as `completed` / `confirmed_deleted`. No other calculator state was modified.
