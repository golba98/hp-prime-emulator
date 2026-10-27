# Program deletion: protocol and acceptance

The [HP Connectivity Kit guide](https://h10032.www1.hp.com/ctg/Manual/c05332952.pdf)
documents deleting calculator programs, but does
not specify the wire format. The existing [protocol investigation](../reverse-engineering/usb-protocol.md#5-native-usb-object-deletion-eb--verified-2026-10-23)
traces Connectivity Kit 2.4.2 r15751's named-object builder and delete dispatcher
and records prior physical tests on G1 build 15157. The [Connectivity Kit protocol audit](CONNECTIVITY_KIT_DELETE_PROTOCOL_AUDIT.md) adds independently pinned x86/x64 and newer ARM firmware-handler evidence, complete live inventory, actual settings names and format-3 read support. The retained firmware is not an exact build-15157 image. No new physical deletion was performed.

The evidenced legacy logical message is `EB 01 [size:u32 BE] 06 [name-byte-len]
[CRC:u16 LE] [UTF-16LE name]`, with `size = 4 + name-byte-len`, no body or name
terminator. CRC-16/XMODEM is computed over the first `size` message bytes with
the CRC bytes zeroed. G1 HID legacy framing prepends report ID and sequence;
the fixed fixture in the protocol investigation is independently calculated.
No application-level completion acknowledgement or protected/missing-object
error response has been established. Host validation and independent F8
queries determine target status; a successful USB write does not prove deletion.

Only G1 `03f0:1541`, build `15157`, boot `V0.050.640` is permitted for direct
deletion. EB over Mode 1 has not been independently established and is refused;
reconnect in legacy mode. Large target reads that cannot be safely completed in
legacy mode fail before deletion; this restriction does not change extended
program transfer. Silence, malformed frames, CRC errors, lost liveness or a
failed write cannot prove absence. A potentially sent EB is never retried;
use `ppl device delete verify --operation-id ID` to reconcile it read-only.

`ppl device delete program NAME --dry-run` checks the profile, anchor and two
CRC-valid identical target reads, reports name, size and hash, and sends no EB.
It creates no backup and gives no authorization. A real invocation repeats the
checks and saves a verified raw backup before exact-name confirmation or
`--yes`, then sends at most one EB and verifies absence. Both forms require
exactly one valid program name, including for `ZZT_*`; no helper fallback is
available for this command. An unresolved operation blocks another attempt.

## Current readiness and proposed Stage B — not authorized

Stage A created `ZZT_DEL901` once under separate approval. It remains present;
never upload it again or overwrite it. The ownership receipt binds its 92-byte
raw source to SHA-256
`d5287d9df0c955dbdae6330d904c39ec3bb0c71a39f1605835e7429e13273e7d`.

The audit repaired live inventory completeness. Real Home/CAS settings are
`calc.hpsettings` and `cas.hpsettings`; `calc.settings` remains only a liveness
anchor. Home matches its opaque baseline. CAS bytes vary across F9 and its
semantic preservation remains unresolved. Stage A is not fully accepted.

The official client negotiated wire format 3, which persisted even after USB
cable reconnection. Disassembly of Connectivity Kit x86 (`0x44fc60`), x64 (`0x1400589f0`),
CRC calculator (`0x5a2d20` / `0x1401ee980`) and ARM firmware (`0x30689380`) confirmed
that format 3 sends `EB 03` with full-message CRC coverage (and without zlib compression
because message length < 128 bytes). Rust deletion has been updated to support wire formats
1 and 3 on Legacy transport. Physical dry-run against the G1 reports
`mutation_wire_format_supported: true` and `submission_state: not_sent`.

## Stage B physical execution — VERIFIED and COMPLETED

On 2026-10-27, Stage B single-target program deletion of disposable test object `ZZT_DEL901`
was authorized and executed against physical HP Prime G1 (`03f0:1541`, build `15157`,
boot `V0.050.640`, serial ID `5f3d8dbdb38ae2a0`) under operation ID `ZZT_D3250C`:

```text
HP Prime G1
Device: 5f3d8dbdb38ae2a0
Firmware: build 15157

Target: program ZZT_DEL901

[OK] Device connected
[OK] Liveness anchor verified (settings:calc.settings)
[OK] Program found
[OK] Program contents verified (two reads, CRC valid)
[OK] Local backup created (artifacts/delete/operations/ZZT_D3250C)
[OK] Deletion authorized
[OK] Program confirmed deleted
```

Post-deletion physical verification results:
1. **Target Status:** `ZZT_DEL901` is confirmed deleted (`confirmed_deleted` in `operation.json`).
2. **Backup Preserved:** Pre-deletion raw backup verified at `artifacts/delete/operations/ZZT_D3250C/program-ZZT_DEL901.bin` (92 bytes, SHA-256 `d5287d9df0c955dbdae6330d904c39ec3bb0c71a39f1605835e7429e13273e7d`).
3. **Surviving Programs:** Full live F9 enumeration confirmed exactly 7 programs remaining, all byte-for-byte identical to baseline:
   - `ZZT_LG901` (17,042 bytes, SHA-256 `50eb2bfe...`)
   - `FIREWORKS` (20,050 bytes, SHA-256 `b3424548...`)
   - `ZZT_PAD15` (14,842 bytes, SHA-256 `b44521f3...`)
   - `ZZT_PAD159` (15,928 bytes, SHA-256 `cbc516b2...`)
   - `ZZT_BSTVIS2` (12,284 bytes, SHA-256 `79a62714...`)
   - `ZZT_BSTVIS` (12,408 bytes, SHA-256 `f897184a...`)
   - `ZZT_SM901` (88 bytes, SHA-256 `d6073781...`)
4. **Inventory Delta:** Record count reduced from 64 to 63 objects (exactly delta = -1). Terminal observed and validated.
5. **No further mutations:** No other program or object was modified or deleted.

## Automated verification

On 2026-10-27, `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, and `cargo test --workspace --quiet` passed.
The original feature device library ran 155 tests, including the literal EB fixture, unsupported
firmware and transport, lost liveness, malformed replies, and existing Mode 1
transfer cases. CLI mock tests cover dry runs, confirmation, disconnection during
EB and blocking an ambiguous retry. Environment-dependent tests that require
Xvfb or private hardware fixtures remained ignored. Mocks are not hardware
evidence.

The subsequent protocol audit passes formatting, workspace Clippy and workspace
tests, with 164 device-library tests. See its report and ignored evidence logs
for the final read-only checks; automated passes do not accept physical deletion.
