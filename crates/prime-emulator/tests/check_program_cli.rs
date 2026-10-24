use std::process::Command;

#[test]
fn unsupported_firmware_report_preserves_name_and_source_verdict_is_unavailable() {
    let t = tempfile::tempdir().unwrap();
    let firmware = t.path().join("synthetic-not-hp.elf");
    std::fs::write(&firmware, b"synthetic firmware, not a compiler").unwrap();
    let report = t.path().join("report.json");
    let command = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_prime-emulator"));
        c.args(["check-program", "--firmware"])
            .arg(&firmware)
            .args(["--factory-a"])
            .arg(t.path())
            .args(["--program"])
            .arg(t.path().join("NoNativeParser.hpprgm"))
            .args(["--name", "MixedCase_Name", "--report"])
            .arg(&report);
        c
    };
    let result = command().output().unwrap();
    assert_eq!(result.status.code(), Some(8));
    let r: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(r["program"], "MixedCase_Name");
    assert_eq!(r["status"], "unsupported");
    assert_eq!(r["engine"], "firmware");
    assert_eq!(r["diagnostics"], serde_json::json!([]));
    assert!(r["observation"].is_null());
    assert!(r["work_dir"].is_null());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&report).unwrap()).unwrap(),
        r
    );
    // A report cannot be written over a directory: classify the command failure.
    std::fs::remove_file(&report).unwrap();
    std::fs::create_dir(&report).unwrap();
    assert_eq!(command().output().unwrap().status.code(), Some(4));
}
