# M15 — Physical HP Prime G1 parity

Date: 2026-10-30. Branch `feat/m12-full-calculator` (canonical checkout). No
commit, push or merge.

Device contact so far is read-only: `ppl device detect` and `device info`.
The reference G1 reports 03f0:1541, firmware build 15157, boot V0.050.640.
Nothing was uploaded, run, changed or deleted on it in M15. The physical
column below is the evidence recorded earlier (2026-10-22/23) by the existing
guarded workflow.

## 1. Version-matched local firmware

The device runs build 15157; the existing local firmware is build 15751.

**The package.** HP's G1 package for 2.2.15157 is `HP_Prime_Calculator_Firmware_20240901.zip`
(hpcalc.org lists "2.2.15157 (2024-09-01)"). hpcalc.org no longer serves it
(404), so it was taken from the Internet Archive's capture of hpcalc.org's own
file (`web.archive.org/web/20241209044057id_/https://www.hpcalc.org/prime/pc/…`).

- archive SHA-256 `5b93dfde…46edd`, zip integrity OK;
- **all 13 files match HP's own `Prime_FW.md5`**;
- release notes: "Date Released: 2024/09/01, Firmware Version 2.2".

**The local assets** live in `~/.local/share/hp-prime-emulator/firmware-15157/`,
outside Git:

- `armfir.elf` (SHA-256 `6f0bb2ab…69fdd4`), extracted from `PRIME_APP.DAT`
  with `scripts/emulator/prepare_factory_a.sh`;
- `PRIME_OS.ROM`;
- `factory-A/`;
- `initial-data/`, made by a first boot through the scripted first-run
  wizard (`DEFAULT_SETUP_KEYS`), then OFF.

**Distinct from 15751.** Its own UI differs: Home Settings page 2 has Time
before Date and "a b/c Key options", where 15751 has Date first and "Key
Options". It boots to Home and hits the same two known unsupported services.

## 2. Three-way runner (`scripts/compat/firmware_legs.py`)

For every recorded hardware run, the runner takes the **exact program the G1
ran** from its run manifest (SHA-256 checked against the evidence) and runs it
on each local build and on the native engine.

The batch path (`ppl run --engine firmware`) relies on runtime hooks at
addresses verified only in the 15751 image. It correctly refused 15157 ("no
verified compiler-observation profile"). The runner therefore runs programs
the way a user does, which works on any build:

1. Import the program before boot.
2. Set HDigits to the device's recorded value with the keypad (`4 Sto▸ HDigits`).
3. Open the Program Catalog and tap the program, which opens the editor.
4. Press **Check**, the firmware's compile.
5. At Home, type the name and press ENTER.
6. Press OFF.
7. Read the firmware's own note file.

A print-channel program's `PRINT(x)` lines are rewritten mechanically to append
to a note (case functions and record format unchanged; `note_program`). A
note-channel program runs unchanged. Every run's settings header reproduced
the device's (`HAngle|HFormat|HDigits`).

**Classes** follow ppl-compat's own rule (`evidence.rs`):

- **VERIFIED**: exact text and type.
- **PARTIAL**: equal only after its documented `ᴇ`→`E`, `−`→`-` mapping.
- **TRANSCRIPTION**: a class added here. Equal only after the screenshot
  transcriber's conventions: string quotes dropped, the private-use imaginary
  unit written `i`. Never counted as verified.
- **MISMATCH**.

A mismatch is attributed to the **firmware version** when the device's own
build (15157) matches the device and another build does not. It is attributed
to **emulation** when 15157-local differs from the 15157 device.

## 3. Results

Artifacts: `artifacts/m15/legs2/` (print channel) and `artifacts/m15/legs-note/` (note channel), each with `matrix.md`, `matrix.json` and raw run reports.

| Leg | VERIFIED | PARTIAL | TRANSCRIPTION | MISMATCH | of |
|---|---:|---:|---:|---:|---:|
| **local firmware 15157** (device's build) | 68 | 4 | 5 | **0** | 77 |
| local firmware 15751 | 67 | 4 | 5 | 1 | 77 |
| native engine (print-channel programs) | 49 | 0 | 4 | 0 | 53 |

(Print channel: 53 results from `core` and `g1_smoke`. Note channel: 24 from `gen_0135289e`.)

- **The emulator reproduces every recorded physical result on the matched build.**
- **The 24 note-channel results** are raw text fetched from the device, not
  transcribed, and match exactly, Unicode minus `−` included. This shows the 4
  PARTIAL cases (`−2`, `−3`, `1ᴇ499`, `1ᴇ−499`) are artefacts of
  transcribing the screen, not differences in what the firmware printed.
- **The one mismatch** is `num_one_third_mul_three`, `(1/3)*3`:

  | Device 15157 | Local 15157 | Local 15751 |
  |---|---|---|
  | 0.999999999999 | 0.999999999999 | 1 |

  **Firmware version**: build 15751 changed this arithmetic. It is not an
  emulation fault.
- **TRANSCRIPTION (5).**
  - Four are strings (`"ab1"`, `"ell"`, `"hello"`, `"ABC"`). Every local
    engine, the independent native engine included, prints them quoted; the
    transcripts drop the quotes. The low-quality screenshot appears to show a
    leading quote.
  - One is the complex `1+2*` + U+E003 written as `1+2*i`.
  - The device batch in §4 answers this by raw note fetch.

## 4. Pending device batch (needs the user)

The project's device docs record that unattended execution does not work on this G1 (EC
key injection had no effect on build 15157). Results therefore need a
**Run press on the calculator**. Four programs are prepared (`ppl compare
tests/compat/cases/m15_open_v1.toml --device prepare`, no USB traffic):

| Run | Cases | Local 15157 | Local 15751 |
|---|---|---|---|
| `ZZT_4AAB60` | `str_plain` `"ABC"`, `third` `1/3`, `third_times_three` | `"ABC"` (quoted), `0.333333333333`, `0.999999999999` | `"ABC"`, `0.333333333333`, `1` |
| `ZZT_5826E5` | `cas_int` `CAS("int(2*x,x)")` | `2*x^2/2` (type 8) | the same |
| `ZZT_8C98E9` | `cas_pi` `CAS("3*pi-pi")` | `3*π-π` (type 8) | the same |
| `ZZT_AD36A6` | `cas_subst` `CAS("subst(a*x+b,x=2)")` | `a*2+b` (type 8) | the same |

(`artifacts/m15/open-local/local.json`.) These answer:

- whether the G1 quotes strings;
- whether `CAS("…")` in a Home program returns unsimplified results on the
  device too. Locally both builds agree; the CAS view simplifies (M14).

They also settle M14's `real-third`. Through STRING, `1/3` is
`0.333333333333` on both local builds; M14's `1/3` was the host decoder's
display of 15751's returned object.

## 5. Not yet compared (untested on the device)

- Framebuffer pixel parity.
- Touch and key behaviour.
- Menu screens.
- App results (M14).
- Persistence across OFF/ON (M13).
- Memory Manager Info figures (M13 calibration).

Each needs either a Run press (programs that draw, then a read-only screenshot)
or the user navigating the calculator for a read-only screenshot. They are
listed as **untested**, not as matches.

## 6. Reproduction

```bash
python3 scripts/compat/firmware_legs.py \
  --build 15751=~/.local/share/hp-prime-emulator/firmware \
  --build 15157=~/.local/share/hp-prime-emulator/firmware-15157 \
  --device-build 15157 --out artifacts/m15/legs
# Device batch (user: typed confirmation, then Run on the calculator):
ppl compare tests/compat/cases/m15_open_v1.toml --device upload --run ZZT_4AAB60   # and the other three
ppl compare tests/compat/cases/m15_open_v1.toml --device fetch  --run ZZT_4AAB60   # read-only
```
