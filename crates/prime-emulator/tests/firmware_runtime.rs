//! M7 genuine firmware runtime tests. No firmware: explicitly logged skip.
use prime_emulator::{
    checkprog::CheckConfig,
    runprog::{self, ExecutionControl, ProgramReport, RunProgramConfig},
    runtime::{ExecutionRequest, ExecutionStatus},
    runtime_value::{FirmwareValue, ReturnRepresentation, parse_arguments},
};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};
struct Env {
    firmware: PathBuf,
    rom: Option<PathBuf>,
    factory: PathBuf,
    initial: Option<PathBuf>,
    out: PathBuf,
}
fn env() -> Option<Env> {
    let Some(firmware) = std::env::var_os("PRIME_TEST_FIRMWARE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    else {
        eprintln!("SKIP M7: PRIME_TEST_FIRMWARE unset");
        return None;
    };
    let factory = PathBuf::from(
        std::env::var_os("PRIME_TEST_FACTORY_A").expect("M7 requires PRIME_TEST_FACTORY_A"),
    );
    assert!(firmware.is_file());
    assert!(factory.join("programs/misc/armfir.dat").is_file());
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    let out = OUT
        .get_or_init(|| {
            let p = std::env::var_os("PRIME_TEST_RUNTIME_OUT")
                .map(PathBuf::from)
                .unwrap_or_else(|| tempfile::tempdir().unwrap().keep());
            std::fs::create_dir_all(&p).unwrap();
            p
        })
        .clone();
    Some(Env {
        firmware,
        rom: std::env::var_os("PRIME_TEST_OS_ROM").map(PathBuf::from),
        factory,
        initial: std::env::var_os("PRIME_TEST_INIT_DATA").map(PathBuf::from),
        out,
    })
}
fn req(entry: &str, args: &str) -> ExecutionRequest {
    ExecutionRequest {
        entry: entry.into(),
        arguments: parse_arguments(args).unwrap(),
    }
}
fn config(e: &Env, name: &str, requests: Vec<ExecutionRequest>) -> RunProgramConfig {
    let program = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/m7")
        .join(format!("{name}.ppl"));
    RunProgramConfig {
        check: CheckConfig {
            program,
            name: None,
            as_source: false,
            firmware: e.firmware.clone(),
            os_rom: e.rom.clone(),
            factory_a: e.factory.clone(),
            out_dir: e.out.clone(),
            init_data: e.initial.clone(),
            settle_ms: 300,
            boot_timeout: Duration::from_secs(60),
            key_timeout: Duration::from_secs(20),
            max_seconds: 180.,
            max_instructions: 8_000_000_000,
        },
        requests,
        runtime_keys: vec![],
        execution_timeout: Duration::from_secs(20),
        control: ExecutionControl::default(),
        trace_runtime: true,
        screenshot: None,
    }
}
fn run(cfg: &RunProgramConfig) -> ProgramReport {
    let r = runprog::run(cfg);
    let p = r
        .work_dir
        .as_ref()
        .unwrap_or(&cfg.check.out_dir)
        .join(format!("m7-{}.json", r.program));
    std::fs::write(&p, serde_json::to_vec_pretty(&r).unwrap()).unwrap();
    println!("M7 {} {:?}: {}", r.program, r.execution_status, p.display());
    r
}
fn completed(r: &ProgramReport) {
    assert_eq!(
        r.execution_status,
        ExecutionStatus::Completed,
        "{}",
        r.evidence
    );
    assert!(r.compilation.source_preserved);
    assert!(r.runtime.compile_verified);
    for i in &r.runtime.invocations {
        assert!(i.body_entered);
        assert!(i.ended_ms.is_some());
        assert_eq!(i.source_sha256, r.source_sha256.as_deref().unwrap());
        assert!(i.unavailable.is_empty(), "{:?}", i.unavailable);
        assert!(!i.calls.is_empty());
        assert_eq!(i.calls[0].arguments.len(), i.requested_arguments.len());
        for (observed, requested) in i.calls[0].arguments.iter().zip(&i.requested_arguments) {
            assert_eq!(
                observed.value,
                real(&requested.value),
                "guest argument differs from requested decimal"
            );
        }
        assert!(i.screenshot.is_some());
    }
}
fn real(n: &str) -> Option<FirmwareValue> {
    Some(FirmwareValue::Real { value: n.into() })
}
#[test]
fn m7_hello_square_and_nested_calls() {
    let Some(e) = env() else { return };
    let h = run(&config(
        &e,
        "HELLO",
        vec![req("HELLO", "[]"), req("HELLO", "[]")],
    ));
    completed(&h);
    for (id, i) in h.runtime.invocations.iter().enumerate() {
        assert_eq!(i.output.len(), 1);
        assert_eq!(i.output[0].invocation, id);
        assert_eq!(i.output[0].text.as_deref(), Some("Hello from HP Prime!"));
    }
    let s = run(&config(
        &e,
        "SQUARE",
        ["[7]", "[0]", "[3]", "[-4]"]
            .iter()
            .map(|a| req("SQUARE", a))
            .collect(),
    ));
    completed(&s);
    for (i, n) in s.runtime.invocations.iter().zip(["49", "0", "9", "16"]) {
        assert_eq!(i.result.as_ref().unwrap().value, real(n));
        assert_eq!(i.calls[0].argument_count, 1);
    }
    let d = run(&config(&e, "DEMO", vec![req("DEMO", "[]")]));
    completed(&d);
    assert_eq!(d.return_value, real("49"));
    assert_eq!(d.output[0].text.as_deref(), Some("SQUARE(7) = 49"));
    assert!(
        d.runtime.invocations[0]
            .calls
            .iter()
            .any(|f| f.name == "SQUARE")
    );
    let f = run(&config(&e, "FACT", vec![req("FACT", "[5]")]));
    completed(&f);
    assert_eq!(f.return_value, real("120"));
    assert!(f.runtime.invocations[0].calls.len() >= 5);
}
#[test]
fn m7_graphics_are_guest_pixels() {
    let Some(e) = env() else { return };
    let r = run(&config(&e, "DRAWTEST", vec![req("DRAWTEST", "[]")]));
    completed(&r);
    let frame = r.runtime.invocations[0].frame.as_ref().unwrap();
    let bytes = &frame.pixels;
    assert_eq!(bytes.len(), 320 * 240);
    let mut blue = 0;
    let mut white = 0;
    for (n, p) in bytes.iter().enumerate() {
        if p & 0xffffff == 0x0000ff {
            blue += 1;
        }
        if (p >> 16) & 255 > 200 && (p >> 8) & 255 > 200 && p & 255 >= 250 {
            white += 1;
            let x = n % 320;
            let y = n / 320;
            assert!((30..200).contains(&x) && (50..85).contains(&y));
        }
    }
    assert!(blue > 75000, "{blue}");
    assert!(white > 100, "{white}");
    assert!(r.runtime.invocations[0].framebuffer_writes.unwrap() > 0);
}
#[test]
fn m7_runtime_errors_and_recovery() {
    let Some(e) = env() else { return };
    for (name, code, message) in [
        ("ERRLIST", 24, "Error: Invalid input"),
        ("ERRTYPE", 47, "Error: Bad argument type"),
        ("ERRDIM", 57, "Error: Invalid dimension"),
        ("ERRZERO", 4, "Error: X/0"),
    ] {
        let r = run(&config(&e, name, vec![req(name, "[]")]));
        assert_eq!(
            r.execution_status,
            ExecutionStatus::RuntimeError,
            "{}",
            r.evidence
        );
        assert!(r.compilation.source_preserved);
        assert_eq!(
            r.compile_status,
            Some(prime_emulator::checkprog::Status::Valid)
        );
        let d = &r.runtime_errors[0];
        assert_eq!(d.raw_code, Some(code));
        assert_eq!(d.message.as_deref(), Some(message));
        assert!(d.raw_context.as_deref().unwrap().contains(name));
        assert_eq!(d.line, None);
    }
    let g = run(&config(
        &e,
        "GLOBALTEST",
        vec![req("INC", "[]"), req("INC", "[]"), req("INC", "[]")],
    ));
    completed(&g);
    for (i, n) in g.runtime.invocations.iter().zip(["1", "2", "3"]) {
        assert_eq!(i.result.as_ref().unwrap().value, real(n));
    }
}
#[test]
fn m7_compile_entry_and_argument_errors() {
    let Some(e) = env() else { return };
    let b = run(&config(&e, "BROKEN", vec![req("BROKEN", "[]")]));
    assert_eq!(b.execution_status, ExecutionStatus::CompileError);
    assert!(b.runtime.invocations.is_empty());
    let missing = run(&config(&e, "HELLO", vec![req("NOENTRY", "[]")]));
    assert_eq!(missing.execution_status, ExecutionStatus::InvalidEntry);
    assert!(missing.runtime.invocations.is_empty());
    for args in ["[]", "[1,2]"] {
        let r = run(&config(&e, "SQUARE", vec![req("SQUARE", args)]));
        assert_eq!(
            r.execution_status,
            ExecutionStatus::InvalidArguments,
            "{}",
            r.evidence
        );
        assert!(r.runtime.invocations.iter().all(|i| !i.body_entered));
    }
}
#[test]
fn m7_wait_timeout_input_and_cancel() {
    let Some(e) = env() else { return };
    let mut cfg = config(&e, "LOOP", vec![req("LOOP", "[]")]);
    cfg.execution_timeout = Duration::from_millis(250);
    let r = run(&cfg);
    assert_eq!(r.execution_status, ExecutionStatus::TimedOut);
    assert!(r.return_value.is_none());
    assert!(r.runtime.invocations[0].ended_ms.is_none());
    let mut cfg = config(&e, "WAITING", vec![req("WAITING", "[]")]);
    cfg.execution_timeout = Duration::from_secs(8);
    cfg.runtime_keys = prime_emulator::keys::parse_keys("ENTER").unwrap();
    let r = run(&cfg);
    completed(&r);
    assert_eq!(r.return_value, real("7"));
    let cfg = config(&e, "LOOP", vec![req("LOOP", "[]")]);
    let c = cfg.control.clone();
    let worker = std::thread::spawn(move || run(&cfg));
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    while c.phase() != 3 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    c.cancel();
    let r = worker.join().unwrap();
    assert_eq!(r.execution_status, ExecutionStatus::Cancelled);
}
#[test]
fn m7_failure_then_success_does_not_reuse_results() {
    let Some(e) = env() else { return };
    let r = run(&config(
        &e,
        "SEQUENCE",
        vec![
            req("FAIL", "[]"),
            req("GOOD", "[]"),
            req("CATCHED", "[]"),
            req("GOOD", "[]"),
        ],
    ));
    assert_eq!(
        r.execution_status,
        ExecutionStatus::RuntimeError,
        "{}",
        r.evidence
    );
    let calls = &r.runtime.invocations;
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].status, Some(ExecutionStatus::RuntimeError));
    for i in [1, 2, 3] {
        assert_eq!(
            calls[i].status,
            Some(ExecutionStatus::Completed),
            "{:?}",
            calls[i].unavailable
        );
        assert!(calls[i].diagnostic.is_none());
    }
    assert_eq!(calls[2].result.as_ref().unwrap().value, real("17"));
    for i in [1, 3] {
        assert_eq!(calls[i].result.as_ref().unwrap().value, real("7"));
        assert!(calls[i].output.iter().all(|o| o.invocation == i));
        assert_eq!(calls[i].output[0].kind, "clear");
        assert!(
            calls[i]
                .output
                .iter()
                .any(|o| o.text.as_deref() == Some("Fresh output"))
        );
        assert!(
            calls[i]
                .output
                .iter()
                .any(|o| o.text.as_deref() == Some("49"))
        );
    }
}
#[test]
fn m7_numeric_input_and_function_name_encoding() {
    let Some(e) = env() else { return };
    let r = run(&config(
        &e,
        "ARGTEST",
        vec![req("ADDTWO", "[2,3]"), req("MiX_sum", "[-4,7]")],
    ));
    completed(&r);
    assert_eq!(
        r.runtime.invocations[0].result.as_ref().unwrap().value,
        real("5")
    );
    assert_eq!(r.return_value, real("3"));
    for i in &r.runtime.invocations {
        assert_eq!(i.export_cache_flags, Some(41));
        assert_eq!(i.calls[0].argument_count, 2);
        assert_eq!(i.calls[0].declared_argument_count, 2);
    }
    let r = run(&config(
        &e,
        "SQUARE",
        ["[0.1]", "[1e20]", "[1e-20]"]
            .iter()
            .map(|a| req("SQUARE", a))
            .collect(),
    ));
    completed(&r);
    for (i, n) in r.runtime.invocations.iter().zip(["0.01", "1e40", "1e-40"]) {
        assert_eq!(i.result.as_ref().unwrap().value, real(n));
    }
}
fn list(items: Vec<FirmwareValue>) -> FirmwareValue {
    FirmwareValue::List { items }
}
fn num(n: &str) -> FirmwareValue {
    FirmwareValue::Real { value: n.into() }
}
/// Formerly reported Unsupported although the guest returned: completion is
/// firmware evidence; the list layout was derived from captured guest bytes.
#[test]
fn m7_list_returns_complete_with_decoded_values() {
    let Some(e) = env() else { return };
    let r = run(&config(&e, "LISTRET", vec![req("LISTRET", "[]")]));
    completed(&r);
    assert_eq!(r.return_value, Some(list(vec![num("1"), num("2")])));
    assert_eq!(r.return_representation, ReturnRepresentation::Decoded);
    assert_eq!(r.return_display.as_deref(), Some("{1, 2}"));
    assert!(r.caveats.is_empty(), "{:?}", r.caveats);
    let cases = [
        ("LIST0", list(vec![])),
        ("LIST3", list(vec![num("1"), num("2"), num("3")])),
        (
            "LISTNEG",
            list(vec![num("-1.5"), num("1e200"), num("0.001")]),
        ),
        (
            "LISTSTR",
            list(vec![FirmwareValue::String { value: "a".into() }, num("2")]),
        ),
        ("LISTNEST", list(vec![list(vec![num("1")]), num("2")])),
    ];
    let r = run(&config(
        &e,
        "LISTS",
        cases.iter().map(|(n, _)| req(n, "[]")).collect(),
    ));
    completed(&r);
    for (i, (name, expected)) in r.runtime.invocations.iter().zip(&cases) {
        assert_eq!(i.status, Some(ExecutionStatus::Completed), "{name}");
        assert_eq!(
            i.result.as_ref().unwrap().value.as_ref(),
            Some(expected),
            "{name}"
        );
    }
}
