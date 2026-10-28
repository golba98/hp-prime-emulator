//! M5 real-firmware tests: program storage, import, Program Catalog and
//! Program Editor.
//!
//! Requires the proprietary HP firmware, never bundled (see
//! `firmware_display.rs`): `PRIME_TEST_FIRMWARE`, `PRIME_TEST_FACTORY_A`,
//! optionally `PRIME_TEST_OS_ROM`. Skips when unset. Every run works on
//! temporary copies; nothing outside the temp directories is written.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use prime_emulator::calcverify::{DEFAULT_SETUP_KEYS, copy_tree};
use prime_emulator::hpprgm;
use prime_emulator::import::{self, ImportRequest, TEMP_PREFIX};
use prime_emulator::keys::parse_keys;
use prime_emulator::progverify::{self, ProgramSpec, ProgramVerifyConfig};
use prime_emulator::runner::{self, Outcome, RunConfig, RunReport};
use prime_emulator::script::ScriptConfig;
use prime_emulator::services::UnsupportedPolicy;
use prime_emulator::trace::TraceConfig;

const TEMPLATE: &str = "EXPORT HELLO()\nBEGIN\n\nEND;\n";
const HELLO: &str = "EXPORT HELLO()\nBEGIN\n  PRINT(\"Hello from HP Prime!\");\nEND;\n";
const SQUARE: &str = "EXPORT SQUARE(N) BEGIN RETURN N*N; END;\n";
const TEST: &str = "EXPORT TEST() BEGIN LOCAL A; A := 42; PRINT(A); END;\n";

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

fn run(env: &Env, data: &Path, keys: &str, shots: &Path) -> (Outcome, RunReport) {
    let cfg = RunConfig {
        firmware_path: env.firmware.clone(),
        os_rom_path: env.os_rom.clone(),
        data_dir: data.to_path_buf(),
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        max_instructions: 8_000_000_000,
        max_seconds: 300.0,
        trace: TraceConfig {
            fs_trace: true,
            ..TraceConfig::default()
        },
        script: Some(ScriptConfig {
            keys: parse_keys(keys).unwrap(),
            screenshot_dir: Some(shots.to_path_buf()),
            start_with_keys: false,
            ..ScriptConfig::default()
        }),
        storage_diff: true,
        ui_timers: true,
        ..RunConfig::default()
    };
    let s = runner::run(&cfg).expect("run produces a report");
    (s.outcome, s.report)
}

/// One first boot through the setup wizard, shared by the tests (copied,
/// never modified).
fn initialised(env: &Env) -> &'static Path {
    static INIT: OnceLock<PathBuf> = OnceLock::new();
    INIT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let data = dir.join("data");
        copy_tree(&env.factory_a, &data.join("A")).unwrap();
        let (outcome, r) = run(
            env,
            &data,
            &format!("{DEFAULT_SETUP_KEYS},IDLE"),
            &dir.join("shots"),
        );
        assert_eq!(outcome, Outcome::ScriptComplete, "{}", r.evidence);
        assert!(data.join("C/DATA/calc.hpsettings").is_file());
        data
    })
}

fn fresh_copy(env: &Env) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    copy_tree(initialised(env), &t.path().join("data")).unwrap();
    t
}

fn hpprgm_writes(r: &RunReport) -> Vec<String> {
    r.fs_events
        .iter()
        .filter(|e| {
            e["op"] == "wfopen"
                && e["mode"].as_str().is_some_and(|m| m.starts_with('w'))
                && e["path"].as_str().is_some_and(|p| p.ends_with(".hpprgm"))
        })
        .filter_map(|e| e["path"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn m5a_the_firmware_stores_a_new_program_as_c_data_name_hpprgm() {
    let Some(env) = env() else { return };
    let t = fresh_copy(&env);
    let data = t.path().join("data");
    // Program Catalog → New → name → OK (editor opens) → ESC.
    let keys = "SHIFT,1,TAP:78:229,ALPHA:HELLO,ENTER,TAP:293:229,ESC";
    let (outcome, r) = run(&env, &data, keys, &t.path().join("shots"));
    assert_eq!(outcome, Outcome::ScriptComplete, "{}", r.evidence);
    // Written on OK (creation) and again on leaving the editor.
    assert_eq!(
        hpprgm_writes(&r),
        vec!["C:\\DATA\\HELLO.hpprgm", "C:\\DATA\\HELLO.hpprgm"]
    );
    let made = std::fs::read(data.join("C/DATA/HELLO.hpprgm")).unwrap();
    let c = hpprgm::parse(&made).unwrap();
    let s = c.source().unwrap();
    assert_eq!((s.name.as_str(), s.source.as_str()), ("Main", TEMPLATE));
    assert_eq!(c.exports(), vec![("HELLO".to_owned(), 9)]);

    // The importer's container for the same source is the firmware's file
    // without the export cache it adds (name64 padding aside: uninitialised
    // memory in the emulated save, zero in device files).
    let mut stripped = c.clone();
    stripped.objects.retain(|o| o.ty != hpprgm::SECTION_EXPORTS);
    let ours = hpprgm::build_program(TEMPLATE).unwrap().to_bytes();
    assert_eq!(
        hpprgm::canonical_padding(&stripped.to_bytes()).unwrap(),
        ours
    );
}

#[test]
fn m5cd_imported_programs_open_in_the_firmware_catalog_and_editor() {
    let Some(env) = env() else { return };
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let programs = [("HELLO", HELLO), ("SQUARE", SQUARE), ("TEST", TEST)]
        .iter()
        .map(|(name, text)| {
            let p = src.path().join(format!("{name}.ppl"));
            std::fs::write(&p, text).unwrap();
            ProgramSpec {
                path: p,
                name: None,
                as_source: false,
            }
        })
        .collect();
    let cfg = ProgramVerifyConfig {
        firmware_path: env.firmware.clone(),
        os_rom_path: env.os_rom.clone(),
        factory_a: env.factory_a.clone(),
        out_dir: out.path().to_path_buf(),
        programs,
        init_data: Some(initialised(&env).to_path_buf()),
        setup_keys: DEFAULT_SETUP_KEYS.into(),
        settle_ms: 300,
        boot_timeout: Duration::from_secs(60),
        key_timeout: Duration::from_secs(20),
        max_instructions: 8_000_000_000,
        max_seconds: 300.0,
        persistence: true,
        verbosity: 0,
    };
    let r = progverify::verify(&cfg);
    let json = serde_json::to_string_pretty(&r).unwrap();
    assert_eq!(r.status, "passed", "{json}");
    for (p, name) in r.programs.iter().zip(["HELLO", "SQUARE", "TEST"]) {
        assert_eq!(p.name, name);
        for s in [p.session1.as_ref().unwrap(), p.session2.as_ref().unwrap()] {
            assert!(
                s.recognized_by_firmware.ok && s.visible_in_catalog.ok,
                "{json}"
            );
            assert!(s.opened_in_editor.ok && s.source_verified.ok, "{json}");
            // The firmware's own check derived the export from the source.
            assert_eq!(s.firmware_exports[0].0, name);
        }
    }
    // Enumerated by the firmware in both boots.
    for s in r.sessions.iter().filter(|s| s.label.starts_with("session")) {
        assert_eq!(
            s.enumerated,
            vec!["HELLO.hpprgm", "SQUARE.hpprgm", "TEST.hpprgm"]
        );
    }
}

#[test]
fn m5_firmware_refuses_a_new_program_named_like_an_imported_one() {
    let Some(env) = env() else { return };
    let t = fresh_copy(&env);
    let data = t.path().join("data");
    let src = t.path().join("HELLO.ppl");
    std::fs::write(&src, HELLO).unwrap();
    let rep = import::import(&ImportRequest {
        program: src,
        data_dir: data.clone(),
        name: None,
        replace: false,
        as_source: false,
    })
    .unwrap();
    // A leftover importer temp file must never reach the catalog.
    std::fs::write(data.join(format!("C/DATA/{TEMP_PREFIX}X-1.tmp")), b"half").unwrap();
    let keys = "SHIFT,1,TAP:78:229,ALPHA:HELLO,ENTER,TAP:293:229";
    let (outcome, r) = run(&env, &data, keys, &t.path().join("shots"));
    assert_eq!(outcome, Outcome::ScriptComplete, "{}", r.evidence);
    let listed: Vec<&str> = r
        .fs_events
        .iter()
        .filter(|e| e["op"] == "wfindnext")
        .filter_map(|e| e["name"].as_str())
        .filter(|n| n.ends_with(".hpprgm") || n.starts_with(TEMP_PREFIX))
        .collect();
    assert_eq!(listed, vec!["HELLO.hpprgm"]);
    // "A program with same name already exists": nothing written, the
    // imported file is intact.
    assert!(hpprgm_writes(&r).is_empty(), "{:?}", hpprgm_writes(&r));
    let now = std::fs::read(data.join("C/DATA/HELLO.hpprgm")).unwrap();
    assert_eq!(prime_emulator::sha256::hex(&now), rep.sha256);
    let ok = r.display.script.as_ref().unwrap().keys.last().unwrap();
    let [_, y0, _, y1] = ok.stable_change.bbox.expect("a message box");
    assert!(
        y0 > 60 && y1 < 240,
        "dialog, not a screen change: {:?}",
        ok.stable_change.bbox
    );
}
