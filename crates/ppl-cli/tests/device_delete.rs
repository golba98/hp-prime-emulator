//! All cases fail before opening USB. Successful hardware workflows are opt-in.
use std::process::Command;
fn invalid(args: &[&str], code: i32) {
    let result = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "delete"])
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["deletion_confirmed"], false);
    assert_eq!(value["execution_confirmed"], false);
    assert_eq!(value["backup_created"], false);
    assert!(value["error"].is_string());
}
#[test]
fn unsafe_missing_and_duplicate_targets_are_json_errors_before_usb() {
    for args in [
        vec!["program"],
        vec!["program", "PERSONAL"],
        vec!["note", "ZZT_CTRL"],
        vec!["program", "ZZT_ONE", "ZZT_ONE"],
        vec!["verify", "settings", "ZZT_TEST"],
        vec!["verify", "program", "ZZT_TEST", "--helper", "../bad"],
    ] {
        invalid(&args, 5);
    }
}
#[test]
fn recovery_paths_cannot_escape_the_operation_store() {
    invalid(&["verify", "--operation-id", "../outside"], 5);
    invalid(&["cleanup", "--operation-id", "../outside"], 5);
}
#[test]
fn conflicting_or_zero_deadline_flags_are_machine_readable() {
    invalid(
        &["program", "ZZT_TEST", "--prepare-only", "--manual-run"],
        2,
    );
    invalid(&["note", "ZZT_TEST", "--poll-secs", "0"], 2);
    invalid(&["program", "ZZT_TEST", "--timeout-secs", "0"], 2);
}
#[test]
fn existing_manual_and_new_recovery_interfaces_have_help() {
    for action in ["program", "note", "verify", "cleanup"] {
        let result = Command::new(env!("CARGO_BIN_EXE_ppl"))
            .args(["device", "delete", action, "--help"])
            .output()
            .unwrap();
        assert!(result.status.success());
    }
}
