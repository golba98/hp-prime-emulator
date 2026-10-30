# M13 — Storage, memory and power state

Date: 2026-10-29. Branch `feat/m12-full-calculator` (canonical checkout), on
top of M12. No commit, push or merge. No physical calculator was contacted.
Firmware: local G1 build 15751 (`armfir.elf` SHA-256 `ad9779ff…2fb39`).

All results are **local firmware** evidence. Whether a physical G1 preserves
the same objects across OFF/ON is verified in M15. **Physical parity: NOT
VERIFIED.**

## 1. What the firmware does (discovery)

A traced run (`prime-emulator run --keys "SHIFT,ON" --trace-fs
--storage-diff`) showed what the firmware's own OFF (Shift+ON) does.

It writes its state as files under `C:\DATA`:

| File(s) | Contents |
|---|---|
| `calc.hpsettings` | Home settings, variables, Ans and history |
| `cas.hpsettings` | the CAS |
| `L0`–`L9.hplist` | the list variables |
| `M0`–`M9.hpmat` | the matrix variables |
| `&<App>.hpapp` | app files |

Then it calls `_fflushall` (`0x306AD0D4`) and `SysPowerOff`
(`0x306CB6F0`). Neither had a handler: both returned 0 at once, so the
firmware carried on as if woken immediately, and called `SysPowerOff` twice.

Two control runs followed:

- **OFF, then a fresh boot on the same storage.** Home history, `A` and
  `L1` were restored.
- **No OFF before ending (what the IDE's "Power off", "Restart" and
  closing did until now).** Nothing was saved. After the reboot `A` was 0
  and the history was empty.

So the calculator's own storage is the persistence layer. The emulator has
to let that path run, and actually sleep. No second, host-side persistence
system is needed or added.

Programs are different: `Programs():=` (IDE import and loader) writes the
program file at once, which is why M12 imports already survived restarts.

## 2. What was implemented

| Area | Change |
|---|---|
| `SysPowerOff` | The calling guest thread sleeps until ON is pressed, then the call returns and the firmware redraws Home. With this the firmware calls it **once** per OFF (twice when it returned at once). While asleep, only ON acts, and only to wake: that press and its release are not delivered. Other keys do nothing |
| `_fflushall` | Makes every open written file durable (fsync); returns the open stream count |
| `fclose` | A written file is fsync'd when closed (the guest's storage stands for flash) |
| Live session (protocol v3) | `Lifecycle::Sleeping`; `Command::PowerDown` (detailed below) |
| IDE | **Power off** = the firmware's OFF (sleep, RAM kept). **Power on** = ON. **Shut down** = PowerDown, then the process ends. **Restart** and **closing the IDE** also power down first, with a bounded wait (40 s), then end. While asleep the LCD is dark with "Off — press ON to wake" (the firmware never calls `LCDOff`; the panel being off is the hardware's state, not firmware pixels) |
| Memory reset | **Reset memory…** (confirmation dialog) powers down, then moves the profile to `<profile>.backup-<epoch>` (never deleted); the next Start seeds factory storage. See the limitation in §5 |
| Integrity | At start, unfinished files from an interrupted write (`*.pending`, importer temporaries) are reported, never deleted |
| Memory Manager Info | It showed "Memory: 4.0GB, Storage: 0KB". Details below |
| Script driver (CLI) | A key that puts the firmware to sleep counts as done; an ON that wakes it counts as delivered. A sleeping run still ends at its script end or its time limit, because the run loop's thread is the one asleep |
| Tests/automation | Steps `firmware_shut_down`, `firmware_reset`, `kill_helper` (battery pull); `run_m12.sh` takes `M12_PROFILE` (a kept profile, for close/reopen) and finds `script_<name>.json` |

`PowerDown` works in four steps:

1. A running program is stopped first, as Stop does.
2. The session presses the firmware's OFF.
3. Once `SysPowerOff` is entered, the state has been saved and the session ends.
4. If that doesn't happen within 30 s, it ends with "The firmware did not
   complete power-off; changes since the last OFF may be lost". It never
   pretends to have saved.

The Memory Manager Info figures were wrong because of two services:

- **Memory** comes from `SetSystemVariable(0x59)`, called as a getter at
  `0x308CE570`. The PrimU2-derived handler returned -1, shown as 4.0 GB. It
  now returns the guest heap's free bytes.
- **Storage** comes from `FSGetDiskRoomState(0, &buf)` (`0x308CE598`).
  The firmware shows `buf+0x34` on a 0 return. That struct was never
  filled before, hence 0 KB. It is now filled from a provisional emulated
  capacity of 256 MiB minus the profile's files.

The dialog now reads 250.98 MB / 255.98 MB.

## 3. Tests

Scenarios (real IDE under Xvfb, through the firmware UI):

- `m13_persist`:
  - A firmware-run program sets `A:=5; Z:=99; L2:={4,5,6}; M1:=[[1,2],[3,4]]; HAngle:=1; F1:="X^2"`.
  - Home `7*6`, then CAS `2*3`.
  - Power off → Sleeping; a non-ON key does nothing → ON; then Shut down → Start.
  - Ans, then `M13GET` reads back `{A, Z, L2(3), M1(2,1), HAngle}`. The scenario ends with `7*6`, then the IDE closes.
- `m13_reopen`: a new IDE process on the same profile repeats the checks.
- `m13_crash`: the same setup, then the helper is killed (a battery pull), then a reboot.
- `m13_reset`: Reset memory, then Start; checks `{A, Z, HAngle}`.

Each state is checked on four firmware screens (Home, Home Settings, the
Function Symbolic view, CAS). Frames are compared pixel-exactly outside the
status bar. Values are checked through firmware-decoded return values and
the M4 glyph method (Ans vs `42` vs `43`).

| Check group | Result |
|---|---|
| Sleep/wake: Sleeping, same session, a non-ON key ignored, ON wakes, all four screens identical | pass |
| Shut down → Start: a new session; all four screens identical; Ans = 42; `{5, 99, 6, 3, 1}` | pass |
| IDE close/reopen: Settings, Symb, CAS identical; Ans = 42; `{5, 99, 6, 3, 1}` | pass |
| Battery pull: reported as failed; boots again | pass. Changes since the last OFF are lost (as expected without the firmware's save) |
| Memory reset: backup folder exists; `{0, 0, 0}`; Home history cleared | pass |

Results of the M13 runs: **3 runs × 40 checks = 120/120 passed** (`artifacts/m13/run{1,2,3}/assert.txt`, `assert_m13.json`). The one note is the battery-pull observation above.

New unit and gated tests:

- **Power key logic** (`services::power_tests`): asleep, only ON acts; the
  waking press is consumed.
- **Memory reset** keeps the old memory and is refused while a session holds
  the profile.
- **Leftover scan**: unfinished files are found; ordinary files are not.
- **Gated `firmware_session`** now ends with `PowerDown`: Sleeping, then "saved its state and
  switched off", then Closed. `cas.hpsettings` is absent before the OFF and
  present after.

Regression: 

- `cargo test --workspace --no-fail-fast`: 1593 passed, 0 failed, 6 ignored.
- clippy and fmt: clean.
- Firmware-gated `prime-emulator` tests: 24/24.
- M12 IDE suite after the M13 changes: 91/91 (`artifacts/m12/20261029T212245Z-m13gate/`).

## 4. Changes in behaviour for M12 users

- "Power off" no longer ends the helper process: it is the calculator's OFF
  (sleep). The M12 reliability scenario's process cycles now use "Shut down",
  so they test the same thing as before.
- Closing the IDE, Restart and Shut down now take as long as the firmware's
  OFF takes (under 2 s observed), at most 40 s.

## 5. Limitations and open items

- **Battery pull / crash.** Only what the firmware saved at its last OFF
  survives a killed helper (or a host crash). This follows from the
  firmware's design: it saves at OFF. Whether a physical G1 also saves
  periodically is untested (not a safe device experiment).
- **Cold reset is host-level.** The G1's memory-clear key combination is read
  by its boot code, which is not emulated (execution starts at
  `armfir.elf`). The reset moves the storage aside and seeds factory storage,
  which is the outcome of a factory reset, not the firmware's own reset path.
- **Memory and storage figures are provisional.** The emulated heap is 256 MiB
  while the G1 has 32 MB of RAM, and the storage capacity is assumed to be
  256 MiB. Both are to be calibrated against the physical G1's Info screen
  (M15). Low-memory behaviour can differ until then.
- **Auto power-off** (`SetAutoPowerOff`) is still a stub: how the OS triggers
  it is not yet observed. Moved to M16 (timing).
- The OFF path itself is authentic. The host only parks the thread and wakes
  it on ON; `SysPowerOff`'s return value is not verified (0, as before).

## 6. Reproduction

```bash
cargo build --workspace --release
cd tests/ide_xvfb_smoke
R=$PWD/../../artifacts/m13/run; mkdir -p $R/profiles
for s in m13_persist m13_reopen; do M12_PROFILE=$R/profiles/calc M12_RUN=$R ./run_m12.sh $s 900; done
M12_RUN=$R ./run_m12.sh m13_crash 900
M12_PROFILE=$R/profiles/calc M12_RUN=$R ./run_m12.sh m13_reset 900
python3 assert_m13.py $R
# Discovery (headless): the firmware's OFF and its storage writes
prime-emulator run --firmware … --data-dir <copy of initialized data> --on-unsupported zero \
  --keys "SHIFT,ON,DELAY:3000" --trace-fs --storage-diff --report off.json
```
