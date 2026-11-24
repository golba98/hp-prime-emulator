//! Read-only setup checks: no firmware execution or calculator access.
use std::process::Command;
fn command(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ppl"));
    cmd.arg("doctor")
        .arg("--json")
        .env("PPL_CONFIG", root.join("absent-config.toml"));
    for variable in [
        "PPL_FIRMWARE_RUNNER",
        "PPL_FIRMWARE",
        "PPL_OS_ROM",
        "PPL_FACTORY_A",
        "PPL_FIRMWARE_INIT_DATA",
    ] {
        cmd.env_remove(variable);
    }
    cmd
}
#[test]
fn firmware_doctor_reports_independent_failures_as_one_json_document() {
    let root = tempfile::tempdir().unwrap();
    let output = command(root.path())
        .args([
            "--engine",
            "firmware",
            "--firmware",
            "missing.elf",
            "--factory-a",
            "missing-factory",
            "--firmware-runner",
            "missing-helper",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for component in ["firmware", "factory_resources", "helper"] {
        assert!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["component"] == component && check["status"] == "failed"),
            "{component}: {report}"
        );
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn native_doctor_works_with_missing_firmware_and_invalid_optional_config() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("bad-config.toml");
    std::fs::write(&config, "this is not valid TOML").unwrap();
    let output = command(root.path())
        .env("PPL_CONFIG", &config)
        .args(["--engine", "native", "--firmware", "missing.elf"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["engine"], "native");
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["status"] != "failed")
    );
    assert_eq!(
        std::fs::read_to_string(config).unwrap(),
        "this is not valid TOML"
    );
}
