//! M4 real-firmware integration tests (display, input, calculation).
//!
//! Requires the proprietary HP firmware, never bundled:
//! - `PRIME_TEST_FIRMWARE` → armfir.elf (and optionally `PRIME_TEST_OS_ROM`);
//! - `PRIME_TEST_FACTORY_A` → the device's `A:` tree extracted from
//!   PRIME_APP.DAT (`scripts/emulator/prepare_factory_a.sh`).
//!
//! Each test copies the factory tree into a fresh temporary data directory
//! (first boot: lock screen, then the first-run wizard). Skips when unset.

use std::path::{Path, PathBuf};
use std::time::Duration;

use prime_emulator::calcverify::{self, CalcVerifyConfig, CalcVerifyOutcome, DEFAULT_SETUP_KEYS};
use prime_emulator::keys::{KEYS, parse_keys};
use prime_emulator::runner::{self, Outcome, RunConfig, RunReport};
use prime_emulator::script::ScriptConfig;
use prime_emulator::services::UnsupportedPolicy;

/// armfir.elf's loaded text/data segment (single PT_LOAD).
const FW_START: u32 = 0x3060_0000;
const FW_END: u32 = 0x3060_0000 + 7_820_992;

struct Env {
    firmware: PathBuf,
    os_rom: Option<PathBuf>,
    factory_a: PathBuf,
}

fn env() -> Option<Env> {
    let get = |k: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let firmware = get("PRIME_TEST_FIRMWARE").filter(|p| p.is_file());
    let factory_a =
        get("PRIME_TEST_FACTORY_A").filter(|p| p.join("programs/misc/armfir.dat").is_file());
    match (firmware, factory_a) {
        (Some(firmware), Some(factory_a)) => Some(Env {
            firmware,
            os_rom: get("PRIME_TEST_OS_ROM"),
            factory_a,
        }),
        _ => {
            eprintln!("skipping: PRIME_TEST_FIRMWARE / PRIME_TEST_FACTORY_A not set");
            None
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

/// Boots the firmware on a fresh copy of the factory tree and runs `keys`.
fn run_script(env: &Env, keys: &str, lcd_watch: bool) -> (Outcome, RunReport, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    copy_tree(&env.factory_a, &tmp.path().join("data/A"));
    let cfg = RunConfig {
        firmware_path: env.firmware.clone(),
        os_rom_path: env.os_rom.clone(),
        data_dir: tmp.path().join("data"),
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        max_instructions: 4_000_000_000,
        max_seconds: 300.0,
        script: Some(ScriptConfig {
            keys: parse_keys(keys).unwrap(),
            screenshot_dir: Some(tmp.path().join("shots")),
            ..ScriptConfig::default()
        }),
        lcd_watch,
        ui_timers: true,
        ..RunConfig::default()
    };
    let summary = runner::run(&cfg).expect("run produces a report");
    (summary.outcome, summary.report, tmp)
}

fn in_firmware(pc: u32) -> bool {
    (FW_START..FW_END).contains(&pc)
}

#[test]
fn m4a_framebuffer_is_written_by_the_firmware() {
    let Some(env) = env() else { return };
    // The lock screen after boot, then one unlock swipe.
    let (outcome, r, _tmp) = run_script(&env, "SWIPE:160:120:310:120:12", true);
    assert_eq!(outcome, Outcome::ScriptComplete, "{}", r.evidence);
    let d = &r.display;
    let view = d.view.expect("LCD located");
    assert_eq!((view.width, view.height, view.stride), (320, 240, 1280));
    assert_eq!(view.fb_va, view.lcd_va + 616, "window1_bufferstart");
    let script = d.script.as_ref().unwrap();
    let initial = &script.captures[0];
    assert!(
        initial.unique_colors > 2,
        "a real screen, not a cleared buffer"
    );
    // The pixels come from guest stores in the firmware's own code.
    assert!(d.fb_writes.unwrap() > 100_000);
    assert!(!d.fb_writer_pcs.is_empty());
    assert!(
        d.fb_writer_pcs.iter().all(|&(pc, _)| in_firmware(pc)),
        "{:x?}",
        d.fb_writer_pcs
    );
    assert!(r.outcomes.framebuffer_capture == Some(true));
}

#[test]
fn m4b_a_digit_key_draws_on_the_home_edit_line() {
    let Some(env) = env() else { return };
    let script = format!("{DEFAULT_SETUP_KEYS},IDLE,2");
    let (outcome, r, _tmp) = run_script(&env, &script, true);
    assert_eq!(outcome, Outcome::ScriptComplete, "{}", r.evidence);
    let keys = &r.display.script.as_ref().unwrap().keys;
    let idle = &keys[keys.len() - 2];
    let two = &keys[keys.len() - 1];
    // Negative control: the same settle window with no input changes nothing.
    assert_eq!(idle.name, "IDLE");
    assert_eq!(idle.stable_change.changed, 0, "{idle:?}");
    // The key: delivered to the UI pump thread, then a lasting glyph on the
    // edit line, drawn by firmware code.
    assert_eq!(two.code, 0x5A);
    assert!(two.delivered_ms.is_some());
    assert_eq!(two.delivered_to_thread, 1, "the UI pump (guest thread 1)");
    let [x0, y0, x1, y1] = two.stable_change.bbox.expect("a lasting change");
    assert!(two.stable_change.changed > 20, "{two:?}");
    assert!(
        x0 < 16 && x1 < 32 && y0 > 190 && y1 < 225,
        "edit line: {:?}",
        two.stable_change.bbox
    );
    assert!(two.fb_writes.unwrap() > 0);
    assert!(two.fb_writer_pcs.iter().all(|&(pc, _)| in_firmware(pc)));
    assert_eq!(r.outcomes.input_delivery, Some(true));
    assert_eq!(r.outcomes.input_to_display, Some(true));
}

#[test]
fn m4c_two_plus_two_is_four_on_the_firmware() {
    let Some(env) = env() else { return };
    let out = tempfile::tempdir().unwrap();
    let cfg = CalcVerifyConfig {
        firmware_path: env.firmware.clone(),
        os_rom_path: env.os_rom.clone(),
        factory_a: env.factory_a.clone(),
        out_dir: out.path().to_path_buf(),
        setup_keys: DEFAULT_SETUP_KEYS.to_string(),
        settle_ms: 300,
        boot_timeout: Duration::from_secs(60),
        key_timeout: Duration::from_secs(20),
        max_instructions: 4_000_000_000,
        max_seconds: 300.0,
        lcd_watch: false,
        verbosity: 0,
    };
    let (report, outcome) = calcverify::verify(&cfg);
    assert_eq!(outcome, CalcVerifyOutcome::Passed, "{}", report.evidence);
    assert!(report.a_matches_b && report.a_differs_from_c && report.expressions_differ);
}

#[test]
fn key_codes_match_the_firmwares_own_table() {
    let Some(env) = env() else { return };
    // The firmware translates device codes with a 192-byte table (literal
    // at 0x30606FE4, read by 0x30606FC0); 0xE047 is special-cased to 5.
    let data = std::fs::read(&env.firmware).unwrap();
    let phoff = u32::from_le_bytes(data[0x1C..0x20].try_into().unwrap()) as usize;
    let p_offset = u32::from_le_bytes(data[phoff + 4..phoff + 8].try_into().unwrap()) as usize;
    let p_vaddr = u32::from_le_bytes(data[phoff + 8..phoff + 12].try_into().unwrap());
    let at = |va: u32| p_offset + (va - p_vaddr) as usize;
    let table_va = u32::from_le_bytes(
        data[at(0x3060_6FE4)..at(0x3060_6FE4) + 4]
            .try_into()
            .unwrap(),
    );
    let index_of = |code: u16| -> i8 {
        if code == 0xE047 {
            5
        } else {
            data[at(table_va) + code as usize] as i8
        }
    };
    // Documented 0–50 physical key numbering (HP Prime GETKEY codes).
    let expected: &[(&str, i8)] = &[
        ("0", 47),
        ("1", 42),
        ("2", 43),
        ("3", 44),
        ("4", 37),
        ("5", 38),
        ("6", 39),
        ("7", 32),
        ("8", 33),
        ("9", 34),
        ("+", 50),
        ("-", 45),
        ("*", 40),
        ("/", 35),
        (".", 48),
        (",", 29),
        ("ENTER", 30),
        ("ESC", 4),
        ("HOME", 5),
        ("APPS", 0),
        ("ON", 46),
        ("LEFT", 7),
        ("UP", 2),
        ("RIGHT", 8),
        ("DOWN", 12),
        ("BACK", 19),
        ("SHIFT", 41),
        ("ALPHA", 36),
        ("SYMB", 1),
        ("PLOT", 6),
        ("NUM", 11),
        ("VIEW", 9),
        ("CAS", 10),
        ("HELP", 3),
        ("VARS", 14),
        ("TOOLBOX", 15),
        ("MATH", 16),
    ];
    assert_eq!(
        expected.len(),
        KEYS.len(),
        "every key of the table is checked"
    );
    for &(name, idx) in expected {
        let code = KEYS.iter().find(|k| k.0 == name).unwrap().1;
        assert_eq!(index_of(code), idx, "{name} ({code:#06X})");
    }
}
