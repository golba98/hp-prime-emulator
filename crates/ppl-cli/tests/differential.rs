//! CLI tests never open a calculator or start firmware.
use std::{path::Path, process::Command};
fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/arithmetic.toml")
}
#[test]
fn native_scenario_is_structured_and_checks_expectations() {
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .arg("differential")
        .arg(fixture())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["passed"], true);
    assert!(report["assertions"].as_u64().unwrap() > 0);
}
#[test]
fn requested_missing_full_system_backend_fails_explicitly() {
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .arg("differential")
        .arg(fixture())
        .args(["--backend", "full-system"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["passed"], false);
    assert_eq!(report["runs"][0]["status"], "unavailable");
    let reason = report["runs"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("PPL full-system execution is unavailable"));
    assert!(reason.contains("does not boot the calculator kernel or run PPL scenarios"));
}
#[test]
fn legacy_replay_cannot_silently_ignore_another_backend_request() {
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "differential",
            "--backend",
            "full-system",
            "--replay-suite",
            "missing-suite.toml",
            "--replay-evidence",
            "missing.json",
            "--replay-manifest",
            "missing-manifest.json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be combined with replay"));
}
