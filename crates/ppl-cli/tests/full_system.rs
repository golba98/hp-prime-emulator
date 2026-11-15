use std::process::Command;
#[test]
fn research_discovery_is_separate_from_ppl_and_does_not_create_data() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["full-system", "info", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["backend"], "full_system");
    assert_eq!(report["availability"], "not_configured");
    assert!(
        report["unsupported"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("ppl_run"))
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[test]
fn research_rejects_unknown_case_and_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "full-system",
            "run",
            "--case",
            "ppl",
            "--build-root",
            "missing",
            "--output",
            "output",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!temp.path().join("output").exists());
}
#[test]
fn missing_build_is_structured_infrastructure_failure() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "full-system",
            "run",
            "--case",
            "smoke",
            "--build-root",
            "missing",
            "--output",
            "output",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "infrastructure_error");
    assert_eq!(report["kernel_boot_verified"], false);
    assert_eq!(report["physical_hardware_verified"], false);
    assert!(!temp.path().join("output").exists());
}
