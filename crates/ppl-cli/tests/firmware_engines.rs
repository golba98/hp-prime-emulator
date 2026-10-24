#![cfg(target_os = "linux")]
//! Genuine M8 CLI tests. Explicit assets fail on bad configuration; unset skips.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
struct Env {
    fw: PathBuf,
    rom: Option<PathBuf>,
    factory: PathBuf,
    init: Option<PathBuf>,
    runner: PathBuf,
    out: PathBuf,
}
fn env() -> Option<Env> {
    let Some(fw) = std::env::var_os("PRIME_TEST_FIRMWARE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    else {
        eprintln!("SKIP M8: PRIME_TEST_FIRMWARE unset");
        return None;
    };
    let factory = PathBuf::from(
        std::env::var_os("PRIME_TEST_FACTORY_A").expect("M8 requires PRIME_TEST_FACTORY_A"),
    );
    let runner = std::env::var_os("PRIME_TEST_RUNNER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_BIN_EXE_ppl")).with_file_name("prime-emulator")
        });
    assert!(fw.is_file() && factory.join("programs/misc/armfir.dat").is_file() && runner.is_file());
    let out = std::env::var_os("PRIME_TEST_UNIFIED_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| tempfile::tempdir().unwrap().keep());
    std::fs::create_dir_all(&out).unwrap();
    Some(Env {
        fw,
        rom: std::env::var_os("PRIME_TEST_OS_ROM").map(PathBuf::from),
        factory,
        init: std::env::var_os("PRIME_TEST_INIT_DATA").map(PathBuf::from),
        runner,
        out,
    })
}
fn fixture(n: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if n == "BSTVisualizer" {
        root.join("tests/fixtures/m8/BSTVisualizer.ppl")
    } else {
        root.join("../prime-emulator/tests/fixtures/m7")
            .join(format!("{n}.ppl"))
    }
}
fn command(e: &Env, dir: &Path, op: &str, path: &Path, extra: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ppl"));
    c.arg(op)
        .arg(path)
        .args(["--engine", "firmware", "--json", "--firmware"])
        .arg(&e.fw)
        .arg("--factory-a")
        .arg(&e.factory)
        .arg("--firmware-runner")
        .arg(&e.runner)
        .arg("--artifacts-dir")
        .arg(dir.join("work"))
        .arg("--report")
        .arg(dir.join("result.json"));
    if let Some(rom) = &e.rom {
        c.arg("--os-rom").arg(rom);
    }
    if let Some(init) = &e.init {
        c.arg("--init-data").arg(init);
    }
    c.args(extra);
    c
}
fn invoke_path(e: &Env, op: &str, path: &Path, extra: &[&str], expected: i32) -> (Value, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("m8-")
        .tempdir_in(&e.out)
        .unwrap()
        .keep();
    let mut c = command(e, &dir, op, path, extra);
    let args = format!("{:?}", c);
    let out = c.output().unwrap();
    std::fs::write(dir.join("stdout.json"), &out.stdout).unwrap();
    std::fs::write(dir.join("stderr.log"), &out.stderr).unwrap();
    std::fs::write(
        dir.join("command.txt"),
        format!("{args}\nexit={:?}\n", out.status.code()),
    )
    .unwrap();
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    println!(
        "M8 {op} {} {} {}",
        path.display(),
        out.status,
        dir.display()
    );
    assert_eq!(out.status.code(), Some(expected), "{}", r);
    assert_eq!(r["engine"], "firmware");
    (r, dir)
}
fn invoke(e: &Env, op: &str, name: &str, extra: &[&str], expected: i32) -> (Value, PathBuf) {
    invoke_path(e, op, &fixture(name), extra, expected)
}
fn real(r: &Value, n: &str) {
    assert_eq!(r["return_value"]["type"], "real");
    assert_eq!(r["return_value"]["value"], n);
    let c = &r["backend"]["report"]["runtime"]["invocations"][0];
    assert_eq!(c["body_entered"], true);
    assert!(!c["ended_ms"].is_null());
}
#[test]
fn m8_checks_source_identity_and_arguments() {
    let Some(e) = env() else { return };
    let (r, _) = invoke(&e, "check", "HELLO", &[], 0);
    assert_eq!(r["compilation_status"], "valid");
    let (r, _) = invoke(&e, "check", "BROKEN", &[], 1);
    assert_eq!(r["diagnostics"][0]["message"], "Error: Syntax Error");
    assert_eq!(r["diagnostics"][0]["line"], 3);
    assert_eq!(r["diagnostics"][0]["column"], 30);
    let (r, _) = invoke(&e, "run", "BROKEN", &[], 1);
    assert_eq!(
        r["backend"]["report"]["runtime"]["invocations"],
        serde_json::json!([])
    );
    let (r, _) = invoke(&e, "run", "HELLO", &[], 0);
    assert_eq!(r["entry"], "HELLO");
    assert_eq!(r["output"][0]["text"], "Hello from HP Prime!");
    for (a, n) in [("[7]", "49"), ("[3]", "9"), ("[0]", "0"), ("[-4]", "16")] {
        let (r, _) = invoke(&e, "run", "SQUARE", &["--entry", "SQUARE", "--args", a], 0);
        real(&r, n);
        assert_eq!(
            r["source"]["engine_source_sha256"],
            r["backend"]["report"]["source_sha256"]
        );
    }
    let (r, _) = invoke(&e, "run", "SQUARE", &["--args", "[1,2]"], 7);
    assert_eq!(r["diagnostics"][0]["message"], "Error: Syntax Error");
}
#[test]
fn m8_graphics_errors_and_entry_selection() {
    let Some(e) = env() else { return };
    let image = e.out.join(format!("draw-{}.png", std::process::id()));
    let (r, _) = invoke(
        &e,
        "run",
        "DRAWTEST",
        &["--png", image.to_str().unwrap()],
        0,
    );
    real(&r, "104");
    assert!(image.is_file());
    let image = ppl_graphics::Grob::read_png(&std::fs::read(image).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (320, 240));
    assert!(image.pixels().iter().filter(|&&p| p == 0x0000ff).count() > 75000);
    assert!(
        image
            .pixels()
            .iter()
            .filter(|&&p| (p >> 16) & 255 > 200 && (p >> 8) & 255 > 200 && p & 255 >= 250)
            .count()
            > 100
    );
    for (name, message) in [
        ("ERRLIST", "Error: Invalid input"),
        ("ERRTYPE", "Error: Bad argument type"),
        ("ERRDIM", "Error: Invalid dimension"),
        ("ERRZERO", "Error: X/0"),
    ] {
        let (r, _) = invoke(&e, "run", name, &[], 3);
        assert_eq!(r["runtime_errors"][0]["message"], message);
        assert!(r["runtime_errors"][0]["line"].is_null());
    }
    let (r, _) = invoke(&e, "run", "ARGTEST", &[], 6);
    assert!(r["error"]["message"].as_str().unwrap().contains("--entry"));
    let (r, _) = invoke(
        &e,
        "run",
        "ARGTEST",
        &["--entry", "MiX_sum", "--args", "[-4,7]"],
        0,
    );
    real(&r, "3");
    for (entry, n) in [(None, "333"), (Some("BST_KeyProbe"), "222")] {
        let args = entry.map(|n| vec!["--entry", n]).unwrap_or_default();
        let (r, _) = invoke(&e, "run", "BSTVisualizer", &args, 0);
        real(&r, n);
    }
    invoke(&e, "run", "BSTVisualizer", &["--entry", "MISSING"], 6);
}
#[test]
fn m8_unicode_container_and_fresh_boot() {
    let Some(mut e) = env() else { return };
    let dir = tempfile::Builder::new()
        .prefix("spaces é ")
        .tempdir_in(&e.out)
        .unwrap()
        .keep();
    let input = dir.join("é HELLO.hpprgm");
    let text = std::fs::read_to_string(fixture("HELLO")).unwrap();
    let container = ppl_syntax::hpprgm::build_program(&text).unwrap().to_bytes();
    std::fs::write(&input, &container).unwrap();
    e.init = None;
    let (r, _) = invoke_path(&e, "run", &input, &[], 0);
    assert_eq!(r["entry"], "HELLO");
    assert_eq!(r["source"]["representation"], "hpprgm-container");
    assert_eq!(std::fs::read(input).unwrap(), container);
    assert_eq!(r["output"][0]["text"], "Hello from HP Prime!");
}
#[test]
fn m8_timeouts_cancellation_and_no_orphans() {
    let Some(e) = env() else { return };
    let (r, _) = invoke(&e, "run", "LOOP", &["--max-time", "0.25"], 4);
    assert_eq!(r["status"], "timed_out");
    assert!(r["return_value"].is_null());
    let dir = tempfile::Builder::new()
        .prefix("cancel-")
        .tempdir_in(&e.out)
        .unwrap()
        .keep();
    let mut cmd = command(
        &e,
        &dir,
        "run",
        &fixture("LOOP"),
        &["--runtime-keys", "IDLE"],
    );
    let stdout = std::fs::File::create(dir.join("stdout.json")).unwrap();
    cmd.stdout(Stdio::from(stdout)).stderr(Stdio::from(
        std::fs::File::create(dir.join("stderr.log")).unwrap(),
    ));
    let started = Instant::now();
    let mut child = cmd.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(100);
    loop {
        if checkpoint(&dir.join("work")) {
            break;
        }
        assert!(Instant::now() < deadline, "execution checkpoint missing");
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    let children =
        std::fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id()))
            .unwrap();
    let cancel = Instant::now();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            assert_eq!(s.code(), Some(130));
            break;
        }
        assert!(cancel.elapsed() < Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(10));
    }
    for pid in children.split_whitespace() {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "orphan helper {pid}"
        );
    }
    let r: Value =
        serde_json::from_slice(&std::fs::read(dir.join("stdout.json")).unwrap()).unwrap();
    assert_eq!(r["status"], "cancelled");
    std::fs::write(dir.join("timings.json"),serde_json::to_vec_pretty(&serde_json::json!({"total_s":started.elapsed().as_secs_f64(),"cancellation_s":cancel.elapsed().as_secs_f64()})).unwrap()).unwrap();
    println!("M8 cancellation artifacts {}", dir.display());
}
fn checkpoint(p: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(p) else {
        return false;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|s| {
            s.to_string_lossy().starts_with("runtime-entry-")
                && s.to_string_lossy().ends_with(".json")
        }) {
            if let Ok(bytes) = std::fs::read(&p) {
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    if value["body_entered"] == true {
                        return true;
                    }
                }
            }
        }
        if p.is_dir() && checkpoint(&p) {
            return true;
        }
    }
    false
}
#[test]
fn m8_independent_boot_reliability() {
    let Some(e) = env() else { return };
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..3 {
        for name in ["HELLO", "SQUARE", "DRAWTEST", "ERRTYPE"] {
            let args = if name == "SQUARE" {
                vec!["--args", "[7]"]
            } else {
                vec![]
            };
            let (r, _) = invoke(
                &e,
                "run",
                name,
                &args,
                if name == "ERRTYPE" { 3 } else { 0 },
            );
            assert!(ids.insert(r["request_id"].as_str().unwrap().to_owned()));
            if name == "SQUARE" {
                real(&r, "49");
            }
            if name == "HELLO" {
                assert_eq!(r["output"][0]["text"], "Hello from HP Prime!");
            }
        }
    }
    assert_eq!(ids.len(), 12);
}
