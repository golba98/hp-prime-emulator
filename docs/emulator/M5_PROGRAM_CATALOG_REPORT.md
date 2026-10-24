# M5 — Genuine Program Catalog, Program Storage and PPL Source Import

**Date:** 2026-10-24 · **Status:** `M5 COMPLETE — IMPORTED PROGRAMS LISTED, OPENED AND SOURCE-VERIFIED IN THE FIRMWARE`

Firmware: HP Prime G1 `armfir.elf` 2.4.2, sha256
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`
(user-supplied, never committed), `PRIME_OS.ROM` sha256 `f6aeba16…a8999`.
Physical calculator: **not used** (no device access in M5). Reference
containers: `~/Downloads/advpmt/ADVPMT.hpprgm` (sha256 `bc812d64…b164a8`) and
`~/Downloads/leaseadvpmt/Leasing20.hpprgm` (`ac548308…c3459`), read-only,
copied before use, unchanged afterwards.

## 1. Summary

| State | HELLO | SQUARE | TEST | MULTI (CRLF, Unicode) | ADVPMT (genuine) | Leasing20 (genuine) |
|---|---|---|---|---|---|---|
| `imported` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `recognized_by_firmware` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `visible_in_catalog` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `opened_in_editor` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `source_verified` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| persisted (second boot, all four states again) | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |

All six were checked in one `verify-program` run that started from a fresh
factory boot (`/tmp/prime-m5-final/report.json`, copied to
`scratch/research/m5-evidence/verify/report.json`).

- **Storage.** The firmware stores user programs as `C:\DATA\<NAME>.hpprgm`,
  host path `<data-dir>/C/DATA/`. It enumerates them once at boot with
  `_wfindfirst/_wfindnext C:\DATA\*.hpprgm` and reads each one to EOF. The
  catalog does not rescan.
- **Format.** The container was decoded from three independent sources that
  agree on every structural byte: files the emulated firmware wrote itself,
  and the two genuine device files. The importer emits only what the
  firmware itself writes for a program it has not yet checked. It contains no
  compiled data; the firmware derives its own export cache.
- **Source proof.** Leaving the Program Editor makes the firmware save the
  program back. For every source import, that save equals the imported file
  byte for byte, apart from the export cache the firmware adds and the
  `name64` padding (§3.3).

## 2. Storage architecture (M5-A)

Evidence comes from `--trace-fs` (new) and `--storage-diff` (new), and is
summarised in `scratch/research/m5-evidence/new-program/report.json`.

- **First boot.** The firmware creates `C:\DATA` with `calc.hpsettings`,
  `settings`, `Custom Mode.hpexammode` and one `&<App>.hpappdir\&<App>.hpapp`
  per app. It also writes `C:\SYSTEM\MYSET.INI` (`[ENV] PBITS`).
- **Program enumeration.** Called from pc `0x3069A490`/`0x3069A550` at boot:
  `C:\DATA\*.hpprgm`. Each match is opened `rb+` (pc `0x3068DFD8`) and read in
  full (`fread` pc `0x3061556C`), until a read returns 0.
- **Program Catalog → New → name → OK.** The firmware writes
  `C:\DATA\HELLO.hpprgm` (`wb+`, pc `0x3068DFF8`) at once, before the editor
  opens: 1072 bytes, no source text, no export cache.
- **Leaving the editor (ESC).** The firmware rewrites the file with its source
  and an export cache (1232 bytes for the template), even when nothing was
  edited. There is no index file: no other file is touched.
- **Second boot of an initialised directory.** It goes straight to Home, with
  no lock screen and no wizard.
- **Catalog order.** The catalog lists programs in enumeration order; our
  VFS sorts names. Leaving the editor moves that program to the top and
  selects it. This most-recently-used order is not persisted across boots.
- **New-program dialog** (`scratch/research/m5-evidence/name-dialog/`):
  - `1A` gives "Error: Invalid input";
  - 31 letters are accepted (`ABCDEFGHIJKLMNOPQRSTUVWXYZABCDE.hpprgm` created);
  - 32 and 40 letters give "Invalid input";
  - an existing name, including an imported one, gives "A program with same
    name already exists".

  The name field is not alpha-locked: letters need ALPHA each time.

## 3. Container format (`src/hpprgm.rs`)

```
u32 0xB28A617C, u32 0xFFFFFFFE, u32 0
object*                        to the end of the file
object = u32 len (bytes after this field), u32 type, payload[len-4]
list   = object with (type & 0x3F) ∈ {0x0E, 0x3E}; payload = group*,
         group = u32 len, object*
```

### 3.1 Fields and evidence

| Type | Content | Evidence |
|---|---|---|
| `007FFF05`, `023FFF05`, `00BFFF05` | header ints | All files. Values 0/0/0 in firmware-made files, 0/0/13 in Leasing20 (meaning unknown). |
| `0100023E` | export cache: groups `(0040020B name64, 00800205 flags)` | Written by the firmware when it saves after a check; absent from the file written by New. The firmware **recomputes** it: ADVPMT's 9 became 89. Flags seen: 9 (0 args), 25 (1 arg), 89 (5 args), 8, 24, 56 — consistent with `9 \| args<<4` for exported programs, but not verified. |
| `00C001BE` | exported variables `(name64, flags, value)` | Leasing20 only; never written by the importer. |
| `014000BE` | source files: one group `(0040008B name64, 00800085 int 0, 00C0009B string)` | All files. The name is `"Main"` on 2.4.2 and empty in ADVPMT (the editor title then reads `ADVPMT:(PPL)`). A program created but never edited has no string object. |
| `00C0009B` | UTF-16LE source plus one NUL, **no padding** | SQUARE's 82-byte string is saved by the firmware unaligned (the next object is at `0x14A`). The other samples happen to be 4-byte aligned. |
| `0240028E` | 10 slots `(0040028B name64, 00800284 8×FF)`; Leasing20: `00800286 16×FF` | Every sample; names empty; meaning unknown. Reproduced as the 2.4.2 firmware writes it. |

Line endings are LF in all three sources; the importer converts CRLF/CR and
reports the conversion. Non-ASCII text (`≥ → é Σ`) and non-BMP characters
round-trip.

### 3.2 Byte-level agreement

- The importer's output for the firmware's own template equals the firmware's
  save minus its export cache: 1136 bytes, identical after canonicalising
  `name64` padding. Pinned by the env-gated
  `m5a_the_firmware_stores_a_new_program_as_c_data_name_hpprgm` and by the
  hand-assembled bytes in `tests/hpprgm.rs`.
- The firmware's re-save of each imported program (HELLO, SQUARE, TEST,
  MULTI) equals the import the same way: `source_verified` evidence,
  "whole file equals the import except the export cache: true".

### 3.3 Divergence noted (not fixed)

In emulated saves, the bytes after the NUL of each `name64` field and the
`0240028E` slot names hold stale heap data; in device files they are zero.
`lmalloc` does not zero memory, and neither does the PrimU2 reference, so no
fix was made without evidence of the real OS's behaviour. These bytes are not
significant to the firmware: its parsers stop at the NUL, and every
emulator-written file loads and opens. `hpprgm::canonical_padding` zeroes
them for comparisons. Firmware-made files from the emulator therefore contain
memory fragments; they stay in the gitignored `scratch/`.

## 4. Import (M5-B, `src/import.rs`, CLI `import`)

`prime-emulator import --program FILE --data-dir D [--name N] [--replace] [--as-source] [--report R]`

- **Inputs.**
  - Source text: UTF-8 (with or without BOM), or UTF-16LE/BE with BOM.
    Encoding and line-ending conversion are reported.
  - Genuine containers: strict parse (magic, every length, header ints,
    exactly one source section, UTF-16 with a single NUL), then copied
    **unchanged**.
  - A `.hpprgm` without the magic (e.g. all nine `tests/fixtures/*.hpprgm`)
    is refused: "plain text under a .hpprgm name; import it as source with
    `--as-source`".
- **Names.** An ASCII letter, then letters, digits or `_`, at most 31
  characters; the length limit and the leading-digit rule are
  firmware-verified (§2). Conflicts are checked case-insensitively, as on the
  guest's FAT file system.
- **Transaction.**
  1. Validate everything.
  2. Require a firmware-initialised `C/DATA`.
  3. Remove stale `.prime-import-*.tmp` files (reported); they can never match
     `*.hpprgm`.
  4. Write `C/DATA/.prime-import-NAME-PID.tmp` and fsync.
  5. `rename` it over the final name, then fsync the directory.
  6. With `--replace`, remove a differently-cased old file after the new one
     is in place.
  7. Hash `C/` before and after, and report `other_files_unchanged`.
- **Report.** Paths, size, SHA-256, source characters, lines and SHA-256, the
  container's export cache, what was replaced, and the removed temp files.
- **Exit codes.** 0 imported; 7 rejected (duplicate, invalid name, invalid
  input), nothing written; 4 I/O error or uninitialised directory.

  Exercised: duplicate → 7; `--replace` → 0 (case change handled);
  `1HELLO` → 7; 32 characters → 7; `tests/fixtures/tester.hpprgm` → 7; a
  700-byte truncated ADVPMT → 7 ("object at 0x280: length 0x39c exceeds its
  container"); missing `C:\DATA` → 4.

`prime-emulator inspect-program FILE` prints the object tree, export cache and
source of any container.

## 5. Catalog and editor verification (M5-C/D, `src/progverify.rs`, CLI `verify-program`)

Sessions, each on its own copy of an initialised data directory:

- `init`: first boot plus the wizard, unless `--init-data` is given;
- `control`: no programs, `SHIFT 1`;
- `session1`: imports, `SHIFT 1`, then for each program DOWN×row, the Edit
  soft key (`TAP:26:229`), then ESC;
- `session2`: a second boot of `session1`'s data, the same keys.

The catalog model is enumeration order, with an edited program moving to the
top. Each program's state rests on its own evidence:

- **`recognized_by_firmware`.** The boot-time fs trace shows `wfindnext`
  returning `NAME.hpprgm`, `wfopen rb+` of it, and `fread` bytes equal to the
  file size.
- **`visible_in_catalog`.** The firmware's enumeration order equals the
  predicted catalog order. The program's row band (y = 42 + 21·row) in the
  catalog screenshot differs from the control catalog (285–6720 px), and the
  row after the last program is identical to the control.
- **`opened_in_editor`.** The Edit press redraws the screen including the
  title bar (bbox `[0,3,319,239]`). The ESC step, attributed by the new
  `step` stamp on fs events, makes the firmware save exactly one program
  file: this one.
- **`source_verified`.** The firmware saved the file during the session. Its
  source string equals the imported source exactly, and for source imports
  the whole file equals the import except the export cache and padding.

In the final run, the firmware's enumeration was `ADVPMT, HELLO, Leasing20,
MULTI, SQUARE, TEST`. The opening rows followed the MRU model: for example,
MULTI was listed at row 3 and opened from row 5. Each prediction was confirmed
by the file the firmware saved.

### 5.1 Screens (PNG file sha256; `scratch/research/m5-evidence/`)

| Screen | File | sha256 |
|---|---|---|
| Control catalog (no programs) | `verify/control/shots/02_1.png` | `14d114f4…63c4` |
| Catalog with 6 programs, session 1 | `verify/session1/shots/02_1.png` | `9a0868ec…5f66` |
| Same, after reboot (identical) | `verify/session2/shots/02_1.png` | `9a0868ec…5f66` |
| Editor `HELLO:Main(PPL)` | `verify/session1/shots/04_TAP_26_229.png` | `ee9d8202…cc1c` |
| Editor `SQUARE:Main(PPL)` | `verify/session1/shots/10_TAP_26_229.png` | `c8bf1904…ebfeec7` |
| Editor `TEST:Main(PPL)` | `verify/session1/shots/17_TAP_26_229.png` | `fa85fa30…87e3` |
| Editor `MULTI:Main(PPL)` (`≥`, `→`) | `verify/session1/shots/24_TAP_26_229.png` | `df7436f2…0712` |
| Editor `ADVPMT:(PPL)` | `verify/session1/shots/30_TAP_26_229.png` | `d824753a…549f` |
| Editor `Leasing20:Main(PPL)` (`é`) | `verify/session1/shots/37_TAP_26_229.png` | `50a8617d…351b` |
| New Program, `HELLO` typed | `new-program/18_0x4F.png` | `7481e1c2…b51e` |
| Firmware template in its editor | `new-program/20_TAP_293_229.png` | `07f54f96…f19` |
| Dialog: "Invalid input" (`1A`) | `name-dialog/leading-digit-1A.png` | `be26ccd4…3dce` |
| Dialog: "same name already exists" | `name-dialog/duplicate-HELLO.png` | `a234ce0d…0e58` |

The firmware-made reference files are in `firmware-made/`:

- `HELLO.on-create.hpprgm`: `c597dfe7…`, 1072 B;
- `HELLO.on-editor-exit.hpprgm`: `1e61698d…`, 1232 B;
- `HELLO.imported-then-firmware-saved.hpprgm`: `9d4dbb13…`;
- `SQUARE.imported-then-firmware-saved.hpprgm`: `32343916…`.

## 6. Emulator changes

| Area | Change |
|---|---|
| `services/fsops.rs` | **Fix:** `WritePrivateProfileString` read its file name from the stack; the ABI and PrimU2 pass it in `r3`. It now writes `C:\SYSTEM\MYSET.INI`, and an empty file or section writes nothing (returns 0). Regression test in `tests/loader_files.rs`. Structured `--trace-fs` events for every file service. |
| `services/fs.rs` | **Fix:** drive-relative paths were prefixed with the drive twice (`C/C/DATA`). **Fix:** `chdir("C:\")` was refused. **Fix:** host paths contained `C//DATA`. Unit tests compare the path text too: `PathBuf` equality hides doubled separators. |
| `trace.rs`, `runner.rs`, `script.rs` | `--trace-fs` (bounded at 50 000 events, each with op, thread, caller pc, time and script step); `--storage-diff` (created/modified/deleted with SHA-256); fs events stamped with the key-script step. |
| `keys.rs` | `ALPHA:TEXT` script items. Letters are their alpha legends' ASCII codes `0x41–0x5A`, read from the firmware's device-code table (A = Vars, key 14 … Z = "2", key 43). H, E, L and O were seen on screen, and the full A–Z set via the 40-letter name. |
| new | `hpprgm.rs`, `import.rs`, `progverify.rs`, `storage.rs`; CLI `import`, `inspect-program`, `verify-program`. |

The native PPL crates and the native GUI were not touched, and the firmware
editor is not connected to the GUI.

## 7. Tests

Synthetic (no firmware):

- `tests/hpprgm.rs` (12): hand-assembled firmware layout, unpadded strings,
  round trip including non-BMP characters, export cache, unedited program,
  padding canonicalisation, and rejection of bad magic and plain text, bad
  preamble and header, truncation at four cuts, bad lengths, group overrun,
  trailing bytes, missing NUL, odd or lone-surrogate UTF-16, data after the
  NUL, missing or duplicate source, NUL or CR in build input.
- `tests/import.rs` (11): firmware-layout output, encodings and line endings,
  name rules, invalid names write nothing, case-insensitive duplicates and
  `--replace`, three coexisting programs, uninitialised directory, plain-text
  `.hpprgm`, genuine container copied unchanged, corrupt container refused,
  stale temp removal, failed write leaves nothing.
- Unit tests: VFS paths (2), storage diff (1), `ALPHA:` (1), catalog model
  (2); `tests/loader_files.rs` adds the INI ABI regression.

Env-gated (`PRIME_TEST_FIRMWARE`, `PRIME_TEST_FACTORY_A`), in
`tests/firmware_programs.rs`, 3 tests, 44 s:

- the firmware creates `C:\DATA\HELLO.hpprgm`, writing it on OK and again on
  leaving the editor, and the importer reproduces it;
- HELLO, SQUARE and TEST verified through both sessions;
- the firmware refuses a New program with an imported program's name, writes
  nothing, and never enumerates a leftover importer temp file.

Results:

- **M4 regression gate.** `firmware_display` 4/4 and `firmware_boot` 1/1
  pass. `verify-calc` (`2+2`, `4`, `2+3`) passed **10/10** consecutive runs,
  21.5–22.8 s each.
- **Workspace.** `cargo fmt --all -- --check` clean. `clippy --workspace
  --all-targets -D warnings` clean. `test --workspace`: 51 suites,
  **1066 passed, 0 failed**, 3 ignored (pre-existing).

## 8. Performance (release, Fedora host)

| Step | Time |
|---|---|
| CLI import (HELLO/SQUARE/TEST/Leasing20, 128 KiB `C:` tree hashed twice) | < 10 ms each (`/usr/bin/time` 0.00 s), ~7 MB RSS |
| `init` first boot plus wizard | 6.6 s |
| `control` | 2.9 s |
| `session1`, 6 programs | 20.3 s |
| `session2` | 19.7 s |
| Whole `verify-program`, 6 programs | about 50 s |
| Whole `verify-program`, 3 programs (with `--init-data`) | 19.5 s |

## 9. Limitations

- Name rules beyond the probed ones (`_`, lower case, non-ASCII, reserved
  words) follow PPL identifier rules and were not probed in the firmware.
- `verify-program` handles at most 8 programs, because the catalog is not
  scrolled. Catalog visibility is a pixel-difference check, not OCR. Row
  identity comes from the editor-save attribution and the firmware's
  enumeration order.
- Programs are only seen at boot: importing while the firmware is running is
  not supported, because the firmware does not rescan.
- The meaning of the export flags, header int 3 and the `0240028E` table is
  unknown. The importer never writes the first; the other two are reproduced
  as the firmware writes them.
- The emulated allocator does not zero memory (§3.3).
- No comparison with the physical G1 in M5, by choice. Device-format evidence
  comes from the two genuine files, whose firmware versions are unknown.
- Running imported programs (Run soft key, PRINT output) is outside M5.

## 10. Reproduction

```bash
FW=scratch/research/firmware/extracted
scripts/emulator/prepare_factory_a.sh $FW/PRIME_APP.DAT /tmp/prime-factory
scripts/emulator/run.sh build --release -p prime-emulator
BIN=../.cache/prime-emulator-target/release/prime-emulator
# Import into an initialised data directory (one firmware boot first):
$BIN import --program HELLO.ppl --data-dir /tmp/prime-m5/data --report /tmp/prime-m5/import.json
$BIN inspect-program /tmp/prime-m5/data/C/DATA/HELLO.hpprgm
# End to end: init boot, control, import, catalog/editor, persistence:
$BIN verify-program --firmware $FW/fat/armfir.elf --os-rom $FW/PRIME_OS.ROM \
  --factory-a /tmp/prime-factory/A --out-dir /tmp/prime-m5-final \
  --program HELLO.ppl --program SQUARE.ppl --program TEST.ppl
# Firmware-level observation of any key script:
$BIN run --firmware $FW/fat/armfir.elf --os-rom $FW/PRIME_OS.ROM --data-dir D \
  --on-unsupported zero --keys "SHIFT,1,TAP:78:229,ALPHA:HELLO,ENTER,TAP:293:229,ESC" \
  --screenshot-dir D/shots --trace-fs --storage-diff --report D/report.json
# Tests:
scripts/emulator/run.sh test -p prime-emulator
PRIME_TEST_FIRMWARE=$PWD/$FW/fat/armfir.elf PRIME_TEST_OS_ROM=$PWD/$FW/PRIME_OS.ROM \
  PRIME_TEST_FACTORY_A=/tmp/prime-factory/A scripts/emulator/run.sh test -p prime-emulator \
  --release --test firmware_programs --test firmware_display --test firmware_boot -- --test-threads=1
```

## 11. Safety and repository notes

- The physical G1 was not contacted.
- Firmware files and the extracted `A:` tree were only read; every run used
  a `/tmp` copy.
- `ZZT_BSTVIS.json` (sha256 `41edf9b8…b6a2`) and
  `ownership/…/program-ZZT_BSTVIS.json` (`62551773…6179`) were untouched;
  their mtime predates M5.
- `.gitignore` was not changed, and nothing was committed. `docs/**/*.md`
  is gitignored, so this report and the untracked `crates/prime-emulator`
  persist only locally until they are added deliberately.
- Evidence lives in the gitignored `scratch/research/m5-evidence/`. It
  contains no factory tree, but its firmware-made `.hpprgm` files include
  emulator memory fragments (§3.3).
