# Uploading a program to a G1 (`ppl device send`)

Works on a calculator with **no** preinstalled `ZZT_*` object. `ZZT_CTRL` is not
needed; `--control type:name` is an optional extra probe and a missing control is
not an error. (`device run`, `cleanup` and `delete` still use a control by design.)

```bash
echo ZZT_MYPROG | ppl device --trace out.trace send ZZT_MYPROG prog.hpprgm
```

## Wire facts (firmware 15157)
- HID reports are 64 bytes, host report `[00][seq][<=63 bytes]`; `F7` = file, `F8` = request, `FF` = ready (reply `0x59`).
- **No write acknowledgement exists.** Only an `F8` read-back that returns an `F7` with the same name, type, valid CRC and equal text proves an install.
- An absent object, a busy calculator and a late reply all look the same: silence.
- Source is sent as UTF-16LE text (plain ASCII/LF `.hpprgm` files are fine; containers are decoded to text first). A 11,525-char file is 23,050 payload bytes.

## Outcome states printed by the CLI
| State | Meaning |
|---|---|
| Transfer initiated | writing started |
| Payload transmitted | all reports accepted by the HID layer (host side only) |
| Device acknowledgement | never available |
| Read-back verified | exact match; ownership + ledger written; exit 0 |
| Read-back differs | object exists but text differs; exit 5 |
| INDETERMINATE | no/failed read-back; **not retried**; line appended to `tests/compat/runs/transfer-attempts.jsonl` (never the ownership ledger); exit 5 |

## Preflight (clean device)
`FF` must answer twice, and the program and its result note (`NAME`+`R`) must stay silent twice while the device is responsive. Existing names are refused.

## Read-back and timeouts
Read-back sends **one** `F8` and listens 20 s (`READBACK_TIMEOUT`). The earlier loop re-sent `F8` every 2 s and drained the queue each time, which can discard a reply already in flight. Reply streams are capped per report (2 s), not by a short overall deadline.

## Safe retry policy
Never resend an indeterminate upload under a new name. Check the Program Catalog or run `ppl device get program NAME` later (read-only). Only silence from a *responsive* device (FF=0x59) is weak evidence of absence.

## Trace
`--trace FILE` writes a bounded table (op, endpoint, requested/actual bytes, status, command/response id, timeout stage, elapsed). No payload bytes are logged. Off by default.

## Size limit (the root cause of the BSTV2/V3/BSTVIS2 failures)
This G1 (fw 15157) **silently drops any program upload above about 16 KB** of UTF-16 payload. There is no error, no reply and no stored object. `MAX_PROGRAM_PAYLOAD` (15,928 B) is the largest size verified; `send` refuses larger payloads before sending anything. The cause (255-report sequence wrap or a firmware cap) is undetermined: sending 0xFF in the sequence and pacing writes did not help. Workaround: shorten the source, for example strip comments and indentation. `artifacts/device-upload/BSTVisualizer.min.hpprgm` (6,141 chars, 12,282 B) does exactly that and renders identically in the emulator.

## Evidence log (2026-10-24, G1 fw 15157, no ZZT_CTRL on device)
| Upload | Payload | Result |
|---|---|---|
| ZZT_PAD15 | 14,840 B | read-back verified |
| ZZT_PAD159 | 15,926 B | read-back verified |
| ZZT_PAD16 | 16,284 B | dropped (no reply, `get` silent) |
| ZZT_PAD17 | ~18 KB | dropped |
| ZZT_PAD20 / PAD20B (0xFF in seq) / PAD20C (5 ms pacing) | 21,080 B | dropped |
| ZZT_BSTVIS2 (full artifact), twice | 23,050 B | dropped (Spreadsheet app, then Program Catalog) |
| **ZZT_BSTVIS2 (minified artifact)** | 12,282 B | **read-back verified, exact match, CRC valid** |

Also: `ZZT_BSTVIS` (older 6,204-char BST) is present; `ZZT_BSTV2`/`ZZT_BSTV3` are absent. Attempts that did not verify are in `tests/compat/runs/transfer-attempts.jsonl`. Test objects `ZZT_PAD15` and `ZZT_PAD159` remain on the calculator (harmless, tool-owned; remove with `ppl device delete`).
