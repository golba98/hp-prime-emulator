# Physical program inventory and source export

## Implemented behavior

```sh
ppl device programs list
ppl device programs list --json
ppl device programs show FIREWORKS
ppl device programs show FIREWORKS --json
ppl device programs export FIREWORKS --out /tmp/FIREWORKS.hpprgm --json
```

Listing uses the physical calculator, not a ledger of locally uploaded names. It sends the documented one-byte F9 backup request and reads the stream of version-1 F7 objects. Every returned object must have valid lengths, HID sequencing, strict UTF-16 name encoding, and the existing device-protocol CRC. Duplicate objects are rejected. Unknown object types remain opaque and are reported by type and payload size without exposing system content.

The receiver bounds each message to 16 MiB, total reports to 64 MiB, object count to 4,096, total duration to 120 seconds, and idle response time to five seconds. Unused bytes after a declared message are HID padding, not another object. Reports reset their sequence at each object and skip sequence FF on wrap. No backup restore, settings write, firmware command, or arbitrary opcode interface is added.

On parsing failure, the receiver consumes the remainder within the same byte/time limits and retires the connection. It never sends another command while consuming that response. Reaching silence, a limit, a malformed object, or an unverified termination produces `inventory_incomplete`, not a complete empty catalog. A failed transfer may return only its validated prefix.

The hplp source describes an empty version-1 F9 packet as a terminal marker. Its semantics are not verified on this G1; even observing it does not currently promote completeness. `capabilities.program_inventory` therefore remains false: it means a verified **complete** inventory, not absence of a partial-read implementation.

## Actual read-only G1 evidence

The implementation session retrieved 54 CRC-valid objects, including these five program objects:

| Program | UTF-16 wire payload bytes |
|---|---:|
| ZZT_PAD15 | 14,842 |
| FIREWORKS | 14,762 |
| ZZT_PAD159 | 15,928 |
| ZZT_BSTVIS2 | 12,284 |
| ZZT_BSTVIS | 12,408 |

No terminal marker was observed before the idle deadline. These are verified present programs, **not a claim that no other program exists**. The result is preserved at `artifacts/device-debug/inventory-911ac4d340c9/result.json` on the development machine. Sizes describe exact returned payload lengths, not calculator RAM usage or Connectivity Kit container size.

Earlier rejected reads exposed unspecified report padding. Their failed/incomplete records were preserved rather than called successful. The receiver now handles that padding according to hplp's declared-length behavior. No programs were created, overwritten, deleted, or executed during inventory verification.

## Exact program reads

`show` uses F8 with the required UTF-16LE name terminator. It checks the returned F7 command, version, declared length, name, type, and CRC before exposing source. Names are case-sensitive. A silent request is a timeout, never proof of absence.

JSON includes the exact decoded source, UTF-16 byte/unit lengths, encoding, SHA-256 of the returned payload, source-text hash, hashed device identity, and verification status. CRLF, BOM codepoints, and trailing NULs are not silently removed. Invalid UTF-16 is reported as uncertain instead of replaced by guessed characters.

`show` without JSON prints source unchanged on stdout and metadata on stderr. `export` uses exclusive creation, preserves the exact device payload, and creates a `.hpprgm.metadata.json` sidecar with provenance and hashes. Existing destinations and sidecars are refused. A sidecar write failure is an error; a successfully created payload is retained for recovery rather than silently deleted.

The export is the USB source payload, usually BOM-less UTF-16LE. It is **not** a synthesized Connectivity Kit binary container and no BOM is inserted. Existing local readers may require a BOM for UTF-16 text; use the JSON source field to create an intentional UTF-8 working revision, preserving the raw export as evidence. Exporting never changes the installed program.

The physical FIREWORKS source independently read with SHA-256 `d2bc3da5907267fe930ff3cf404d067e9a1f878799779fdcc7db19e782d1f576` matched its inventory payload hash. This does not establish identity with the current local example.

## Sources

- [Pinned hplp commands](https://github.com/debrouxl/hplp/blob/7b07ea9790ef253aafa39bf75d994fb996282048/libhpcalcs/src/prime_cmd.c): backup request, file reception, terminal handling, and CRC.
- [Pinned hplp packet assembly](https://github.com/debrouxl/hplp/blob/7b07ea9790ef253aafa39bf75d994fb996282048/libhpcalcs/src/prime_vpkt.c): declared message lengths, per-message report sequencing, and padding truncation.
- [HP Connectivity Kit guide](https://h10032.www1.hp.com/ctg/Manual/c05332952.pdf): calculator content lists and read-only backup, printed pages 11–15.
