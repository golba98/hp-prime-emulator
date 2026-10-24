//! Genuine M6 firmware tests. Missing environment skips; explicitly supplied
//! invalid assets fail. Every guest writes only into an isolated copy.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use prime_emulator::calcverify::{DEFAULT_SETUP_KEYS, copy_tree};
use prime_emulator::checkprog::{self, CheckConfig, CheckReport, Status};
use prime_emulator::hpprgm;
use prime_emulator::import::{self, ImportRequest};
use prime_emulator::keys::parse_keys;
use prime_emulator::runner::{self, Outcome, RunConfig, RunReport};
use prime_emulator::script::ScriptConfig;
use prime_emulator::services::UnsupportedPolicy;
use prime_emulator::trace::TraceConfig;
use prime_emulator::verify;

const HELLO: &str = "EXPORT HELLO()\nBEGIN\n  PRINT(\"Hello from HP Prime!\");\nEND;\n";
const BROKEN: &str = "EXPORT BROKEN()\nBEGIN\n  PRINT(\"Missing parenthesis\";\nEND;\n";
struct Env {
    firmware: PathBuf,
    rom: Option<PathBuf>,
    factory: PathBuf,
    out: PathBuf,
}
fn env() -> Option<Env> {
    let Some(firmware) = std::env::var_os("PRIME_TEST_FIRMWARE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    else {
        eprintln!("SKIP M6: PRIME_TEST_FIRMWARE is unset");
        return None;
    };
    let factory = PathBuf::from(
        std::env::var_os("PRIME_TEST_FACTORY_A")
            .expect("M6 requires PRIME_TEST_FACTORY_A when firmware is supplied"),
    );
    assert!(firmware.is_file());
    assert!(factory.join("programs/misc/armfir.dat").is_file());
    let rom = std::env::var_os("PRIME_TEST_OS_ROM").map(PathBuf::from);
    if let Some(p) = &rom {
        assert!(p.is_file());
    }
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    let out = OUT
        .get_or_init(|| {
            let p = std::env::var_os("PRIME_TEST_COMPILER_OUT")
                .map(PathBuf::from)
                .unwrap_or_else(|| tempfile::tempdir().unwrap().keep());
            std::fs::create_dir_all(&p).unwrap();
            p
        })
        .clone();
    Some(Env {
        firmware,
        rom,
        factory,
        out,
    })
}
fn scripted(e: &Env, data: &Path, out: &Path, keys: &str, observe: bool) -> RunReport {
    let s = runner::run(&RunConfig {
        firmware_path: e.firmware.clone(),
        os_rom_path: e.rom.clone(),
        data_dir: data.into(),
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        max_seconds: 300.0,
        max_instructions: 8_000_000_000,
        trace: TraceConfig {
            compiler: observe,
            fs_trace: true,
            verbosity: 0,
            ..Default::default()
        },
        script: Some(ScriptConfig {
            keys: parse_keys(keys).unwrap(),
            screenshot_dir: Some(out.join("shots")),
            ..Default::default()
        }),
        ui_timers: true,
        storage_diff: true,
        ..Default::default()
    })
    .unwrap();
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&s.report).unwrap(),
    )
    .unwrap();
    assert_eq!(s.outcome, Outcome::ScriptComplete, "{}", s.report.evidence);
    assert!(s.report.outcomes.firmware_execution);
    s.report
}
fn initialized(e: &Env) -> &'static Path {
    static INIT: OnceLock<PathBuf> = OnceLock::new();
    INIT.get_or_init(|| {
        if let Some(p) = std::env::var_os("PRIME_TEST_INIT_DATA") {
            let p = PathBuf::from(p);
            assert!(p.join("C/DATA").is_dir());
            return p;
        }
        let out = tempfile::Builder::new()
            .prefix("initial-")
            .tempdir_in(&e.out)
            .unwrap()
            .keep();
        let data = out.join("data");
        copy_tree(&e.factory, &data.join("A")).unwrap();
        scripted(e, &data, &out, &format!("{DEFAULT_SETUP_KEYS},IDLE"), false);
        data
    })
}
fn check(e: &Env, name: &str, source: &str) -> CheckReport {
    let out = tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&e.out)
        .unwrap()
        .keep();
    let path = out.join(format!("{name}.ppl"));
    std::fs::write(&path, source).unwrap();
    let report = checkprog::check(&CheckConfig {
        program: path,
        name: None,
        as_source: false,
        firmware: e.firmware.clone(),
        os_rom: e.rom.clone(),
        factory_a: e.factory.clone(),
        out_dir: out.clone(),
        init_data: Some(initialized(e).into()),
        settle_ms: 300,
        boot_timeout: Duration::from_secs(60),
        key_timeout: Duration::from_secs(20),
        max_seconds: 300.0,
        max_instructions: 8_000_000_000,
    });
    std::fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "M6 {name}: {:?}, artifacts {}",
        report.status,
        out.display()
    );
    assert_eq!(report.program, name);
    assert!(report.source_preserved, "{}", report.evidence);
    report
}
#[test]
fn m6_hello_and_broken_have_authentic_independent_results() {
    let Some(e) = env() else { return };
    let hello = check(&e, "HELLO", HELLO);
    assert_eq!(hello.status, Status::Valid, "{}", hello.evidence);
    assert!(hello.diagnostics.is_empty());
    assert_eq!(
        hello.observation.as_ref().unwrap().message.as_deref(),
        Some("No errors in the program")
    );
    assert_eq!(hello.exports_after, vec![("HELLO".into(), 9)]);
    let broken = check(&e, "BROKEN", BROKEN);
    assert_eq!(broken.status, Status::Invalid, "{}", broken.evidence);
    let d = &broken.diagnostics[0];
    assert_eq!(d.message.as_deref(), Some("Error: Syntax Error"));
    assert_eq!((d.line, d.column), (Some(3), Some(30)));
    assert_eq!(d.raw_position.as_ref().unwrap().offset_utf16, 51);
    assert_eq!(
        broken.observation.as_ref().unwrap().parser_returns,
        vec![22]
    );
    assert_ne!(hello.source_sha256, broken.source_sha256);
}
#[test]
fn m6_reference_programs_and_diagnostic_positions() {
    let Some(e) = env() else { return };
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        accepted: bool,
        line: Option<usize>,
        column: Option<usize>,
        exports: Vec<(String, u32)>,
    }
    // Each expectation was measured through this G1's genuine Check action.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/compiler_cases.json")).unwrap();
    for case in cases {
        let r = check(&e, &case.name, &case.source);
        assert_eq!(
            r.status,
            if case.accepted {
                Status::Valid
            } else {
                Status::Invalid
            },
            "{}: {}",
            case.name,
            r.evidence
        );
        assert_eq!(r.exports_after, case.exports, "{}", case.name);
        if !case.accepted {
            assert_eq!(
                (r.diagnostics[0].line, r.diagnostics[0].column),
                (case.line, case.column),
                "{}",
                case.name
            );
            assert_eq!(
                r.diagnostics[0].message.as_deref(),
                Some("Error: Syntax Error")
            );
        }
    }
}
#[test]
fn m6_repeated_independent_boots_and_corrected_source() {
    let Some(e) = env() else { return };
    for i in 0..10 {
        let r = check(&e, "HELLO", HELLO);
        assert_eq!(r.status, Status::Valid, "iteration {i}: {}", r.evidence);
        let r = check(&e, "BROKEN", BROKEN);
        assert_eq!(r.status, Status::Invalid, "iteration {i}: {}", r.evidence);
    }
    let corrected = BROKEN.replace("parenthesis\";", "parenthesis\");");
    let r = check(&e, "BROKEN", &corrected);
    assert_eq!(r.status, Status::Valid, "{}", r.evidence);
    assert!(r.diagnostics.is_empty());
}
#[test]
fn m6_navigation_back_to_back_checks_and_home_calculation() {
    let Some(e) = env() else { return };
    let out = tempfile::Builder::new()
        .prefix("navigation-")
        .tempdir_in(&e.out)
        .unwrap()
        .keep();
    let data = out.join("data");
    copy_tree(initialized(&e), &data).unwrap();
    for (name, source) in [("HELLO", HELLO), ("BROKEN", BROKEN)] {
        let p = out.join(format!("{name}.ppl"));
        std::fs::write(&p, source).unwrap();
        import::import(&ImportRequest {
            program: p,
            data_dir: data.clone(),
            name: None,
            replace: false,
            as_source: false,
        })
        .unwrap();
    }
    // Sorted catalog: BROKEN then HELLO. Editor exit moves a row to the top.
    let r = scripted(
        &e,
        &data,
        &out,
        "SHIFT,1,TAP:26:229,TAP:293:229,ENTER,ESC,ENTER,DOWN,TAP:26:229,TAP:293:229,ENTER,ESC,DOWN,TAP:26:229,TAP:293:229,ENTER,HOME,ESC,2,+,2,ENTER",
        true,
    );
    let trace = r.compiler.as_ref().unwrap();
    assert_eq!(trace.checks.len(), 3);
    for (step, source, code) in [(3, BROKEN, 0), (9, HELLO, 1), (14, BROKEN, 0)] {
        assert_eq!(
            checkprog::validate(trace, source, step)
                .unwrap()
                .return_value,
            Some(code)
        );
    }
    for (name, source) in [("HELLO", HELLO), ("BROKEN", BROKEN)] {
        let saved = std::fs::read(data.join(format!("C/DATA/{name}.hpprgm"))).unwrap();
        assert_eq!(
            hpprgm::parse(&saved).unwrap().source().unwrap().source,
            source
        );
    }
    let captures = &r.display.script.as_ref().unwrap().keys;
    let frame = |p: &Option<String>| verify::load_png(Path::new(p.as_ref().unwrap())).unwrap();
    let result = verify::history_line(
        &frame(&captures[20].after.png),
        &frame(&captures[21].after.png),
    )
    .unwrap()
    .1;
    // Independently type 4 after all the compiler checks; compare its glyph.
    let control = scripted(&e, &data, &out.join("control"), "4,ENTER", false);
    let k = &control.display.script.as_ref().unwrap().keys;
    let four = verify::history_line(&frame(&k[0].after.png), &frame(&k[1].after.png))
        .unwrap()
        .1;
    assert!(result.same_as(&four));
}

#[test]
fn m6_in_editor_correction_rechecks_the_new_source() {
    let Some(e) = env() else { return };
    let out = tempfile::Builder::new()
        .prefix("correction-")
        .tempdir_in(&e.out)
        .unwrap()
        .keep();
    let data = out.join("data");
    copy_tree(initialized(&e), &data).unwrap();
    let source = out.join("BROKEN.ppl");
    std::fs::write(&source, BROKEN).unwrap();
    import::import(&ImportRequest {
        program: source,
        data_dir: data.clone(),
        name: None,
        replace: false,
        as_source: false,
    })
    .unwrap();
    // Check positions the cursor before ';'. The () key inserts a pair;
    // BACK removes its opening half, leaving precisely the missing ')'.
    let r = scripted(
        &e,
        &data,
        &out,
        "SHIFT,1,TAP:26:229,TAP:293:229,ENTER,0x4E,BACK,TAP:293:229,ENTER,HOME,ESC,IDLE",
        true,
    );
    let corrected = BROKEN.replace("parenthesis\";", "parenthesis\");");
    let trace = r.compiler.as_ref().unwrap();
    assert_eq!(trace.checks.len(), 2);
    assert_eq!(
        checkprog::validate(trace, BROKEN, 3).unwrap().return_value,
        Some(0)
    );
    assert_eq!(
        checkprog::validate(trace, &corrected, 7)
            .unwrap()
            .return_value,
        Some(1)
    );
    assert!(
        checkprog::validate(trace, BROKEN, 7).is_err(),
        "old source cannot claim the new result"
    );
    let saved = std::fs::read(data.join("C/DATA/BROKEN.hpprgm")).unwrap();
    assert_eq!(
        hpprgm::parse(&saved).unwrap().source().unwrap().source,
        corrected
    );
}
