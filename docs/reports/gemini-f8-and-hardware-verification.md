> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# HP Prime G1 Hardware Verification & F8 Investigation Report

**Date:** 2026-10-22
**Branch:** `rust-rewrite`
**Device:** HP Prime G1 (`03f0:1541`)
**Firmware Build:** 15157 (`2.2.15157`)
**Boot Version:** `V0.050.640`
**Serial ID:** `203c2bc6b51950db` (anonymized fingerprint)

---

## 1. Executive Summary

During the takeover of the interrupted Claude engineering session:
1. **Control Program Confirmed:** Read-only screenshot inspection of the physical calculator confirmed that the user had created `ZZT_CTRL` (with the underscore) in the Program Catalog.
2. **F8 Protocol Bug Identified and Fixed:** The cause of `F8` object retrieval timeouts was isolated: the HP Prime G1 firmware (build 15157) requires the requested object name in the `F8` request payload to be **null-terminated in UTF-16LE (`\0\0`)**. Without this terminator, the firmware's C string parser scans into packet memory, fails to match the name, and remains silent. Appending `\0\0` resolved the issue completely.
3. **Confirmed Object Retrieval:** Both `ZZT_CTRL` (62 bytes) and `tester` (10,774 bytes) were retrieved with 100% valid CRC-16 checks. Missing objects consistently timeout, confirming clean presence/absence discrimination.
4. **Guarded Upload Succeeded:** The guarded upload workflow for `ZZT_998C5E` (smoke test) ran through `preflight_new_names`:
   - `FF` check-ready succeeded (`0x59`).
   - Control `ZZT_CTRL` answered with valid CRC in Round 1.
   - Target `ZZT_998C5E` was silent in Round 1.
   - Control `ZZT_CTRL` answered identically in Round 2.
   - Target `ZZT_998C5E` was silent in Round 2.
   - `WriteGuard` was issued and `F7` send completed (4,230 bytes).
   - Immediate `F8` read-back matched the source byte-for-byte.
5. **GUI & Debugger End-to-End Verified:** Tested under Xvfb with `PPL_GUI_SMOKE=1`. Window presentation, 2x framebuffer scaling, 4-color rendering, graceful exit on finish, prompt closing, and SIGTERM cancellation were verified.

---

## 2. F8 Root Cause Analysis & Empirical Evidence

### Protocol Framing Defect
In `ppl-device::files::encode_file_message`:
Previously, the UTF-16LE encoded name was placed into the payload without trailing null bytes.

When tested against the physical device:
| Name Sent | Null Terminator (`\0\0`) | Report Padded | Result |
|---|---|---|---|
| `tester` | No | No | TIMEOUT |
| `tester` | No | Yes | TIMEOUT |
| `tester` | Yes | No | **REPLY (10,774 bytes, CRC OK)** |
| `tester` | Yes | Yes | **REPLY (10,774 bytes, CRC OK)** |
| `ZZT_CTRL` | Yes | No | **REPLY (62 bytes, CRC OK)** |
| `ZZT_CTRL` | Yes | Yes | **REPLY (62 bytes, CRC OK)** |
| `NONEXISTENT` | Yes | No | **TIMEOUT (Clean absence)** |

### Fix Implemented
In `crates/ppl-device/src/files.rs`:
```rust
pub fn encode_file_message(cmd: u8, ty: FileType, name: &str, data: &[u8]) -> Vec<u8> {
    let mut name16: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    if cmd == crate::CMD_REQ_FILE && !name16.ends_with(&[0, 0]) {
        name16.extend_from_slice(&[0, 0]);
    }
    let size = (4 + name16.len() + data.len()) as u32;
    ...
```
And in `decode_file_message`:
```rust
    let name = String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_string();
```

---

## 3. Smoke Test `ZZT_998C5E` Status

- **Program:** `ZZT_998C5E`
- **Suite:** `g1_smoke` (5 cases: `smoke_add`, `smoke_mul`, `smoke_third`, `smoke_round`, `smoke_strcat`)
- **Status:** **UPLOADED & READ-BACK VERIFIED**
- **Collision Safety:** Collision-safe `ZZT_*` namespace, preflight verified.
- **Side Effects:** Zero persistent side effects; restores `HAngle`, `HFormat`, `HDigits`.
- **Next User Action:**
  1. On the calculator, press `[Shift]` + `[1]` (Program Catalog).
  2. Select `ZZT_998C5E` and press `Run`.
  3. Keep the terminal on screen.
  4. Run `cargo run --bin ppl -- compare tests/compat/cases/g1_smoke.toml --device screenshot --run ZZT_998C5E` to record authentic physical evidence.

---

## 4. Test Suite Metrics

- Workspace Unit & Integration Tests: **250 passed, 0 failed**
- Xvfb GUI Smoke Test: **1 passed** (`gui_renders_and_closes_promptly`)
- Cargo Formatting: **Clean (`cargo fmt --all -- --check`)**
- Clippy: **Clean with `-D warnings` (`cargo clippy --workspace --all-targets -- -D warnings`)**
- Release Build: **Clean (`cargo build --workspace --release`)**
