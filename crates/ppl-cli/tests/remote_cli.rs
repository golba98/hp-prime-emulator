//! These tests validate host-side errors only; they never open a calculator.
use std::process::Command;
fn invalid(args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device"])
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["execution_confirmed"], false);
    assert_eq!(value["uploaded"], false);
    assert_eq!(value["execution_state"], "not_requested");
    assert_eq!(value["error"]["kind"], "invalid_input");
    value
}
#[test]
fn malformed_expressions_are_json_errors_before_usb_access() {
    for expr in ["", "(", "1+", "A:=1", "2;3"] {
        invalid(&["exec", expr]);
    }
}
#[test]
fn invalid_program_and_arguments_never_reach_usb() {
    invalid(&["run", "1BAD", "--remote"]);
    invalid(&["run", "TEST", "--remote", "--args", "5);"]);
}
#[test]
fn remote_usage_is_documented_without_changing_emulator_run() {
    for args in [
        vec!["device", "exec", "--help"],
        vec!["device", "listener", "--help"],
        vec!["device", "run", "--help"],
        vec!["run", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
}

#[test]
fn recovery_identifiers_are_validated_before_usb() {
    invalid(&["status", "--request-id", "../outside"]);
    invalid(&["listener", "abandon-probe", "not-an-id"]);
    invalid(&[
        "listener",
        "fixed-mailbox-probe",
        "--resume",
        "USER_PROGRAM",
    ]);
}
