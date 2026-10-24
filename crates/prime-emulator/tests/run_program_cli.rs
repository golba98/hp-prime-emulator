//! CLI errors are structured even when firmware assets are absent.
use std::process::Command;
#[test]
fn runtime_cli_rejects_malformed_arguments_without_booting() {
    for (args, extra) in [
        ("[true]", vec![]),
        (r#"[{"type":"string","value":"x"}]"#, vec![]),
        ("[]", vec!["--execution-timeout-s", "0"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("report.json");
        let out = Command::new(env!("CARGO_BIN_EXE_prime-emulator"))
            .args([
                "run-program",
                "--program",
                "ABSENT.ppl",
                "--entry",
                "HELLO",
                "--firmware",
                "absent.elf",
                "--factory-a",
                "absent-A",
                "--args",
                args,
                "--report",
            ])
            .arg(&report)
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(10));
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
        assert_eq!(j["execution_status"], "invalid_arguments");
        assert!(j["runtime"]["invocations"].as_array().unwrap().is_empty());
        assert!(j["return_value"].is_null());
    }
}
#[test]
fn missing_source_is_an_infrastructure_error_not_a_runtime_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_prime-emulator"))
        .args([
            "run-program",
            "--program",
            "MISSING.ppl",
            "--entry",
            "HELLO",
            "--firmware",
            "absent.elf",
            "--factory-a",
            "absent-A",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(j["execution_status"], "infrastructure_error");
    assert!(j["evidence"].as_str().unwrap().contains("MISSING.ppl"));
    assert_eq!(j["runtime_errors"], serde_json::json!([]));
}
