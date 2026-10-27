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
        vec!["program", "../PERSONAL"],
        vec!["program", "PERSONAL", "HELLO"],
        vec!["program", "PERSONAL", "ZZT_ONE"],
        vec!["program", "PERSONAL", "--manual-run"],
        vec!["note", "PERSONAL", "--prepare-only"],
        vec!["program", "zzt_ctrl"],
        vec!["note", "ZZT_CTRL"],
        vec!["program", "ZZT_ONE", "ZZT_ONE"],
        vec!["program", "ZZT_ONE", "ZZT_TWO"],
        vec!["program", "ZZT_ONE", "--manual-run"],
        vec!["program", "ZZT_ONE", "--prepare-only"],
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
#[test]
fn ordinary_names_are_not_rejected_for_lacking_the_test_prefix() {
    // Never reaches USB: the ordinary-object path refuses manual helper flags
    // before opening a device, and the error must not blame the name.
    let result = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "device",
            "delete",
            "program",
            "FIREWORKS",
            "--manual-run",
            "--json",
        ])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let error = value["error"].as_str().unwrap();
    assert!(!error.contains("test object name"), "{error}");
    assert!(error.contains("unavailable for this target"), "{error}");
}

#[test]
fn help_describes_ordinary_deletion_and_the_test_object_split() {
    for action in ["program", "note"] {
        let result = Command::new(env!("CARGO_BIN_EXE_ppl"))
            .args(["device", "delete", action, "--help"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&result.stdout);
        assert!(text.contains("exact name"), "{text}");
        assert!(text.contains("ZZT_"), "{text}");
        assert!(!text.contains("tool-owned ZZT_"), "{text}");
    }
    let top = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "delete", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&top.stdout);
    assert!(
        text.contains("FIREWORKS") && text.contains("unsupported"),
        "{text}"
    );
}

#[test]
fn yes_flag_is_scoped_to_exactly_one_valid_program_before_any_usb_access() {
    for args in [
        vec!["program", "A_ONE", "B_TWO", "--yes"],
        vec!["program", "../FIREWORKS", "--yes"],
        vec!["program", "ZZT_CTRL", "--yes"],
        vec!["program", "--yes"],
        vec!["note", "MyNote", "--yes"],
    ] {
        invalid(&args, 5);
    }
    invalid(&["program", "FIREWORKS", "--yes", "--manual-run"], 2);
    invalid(&["program", "FIREWORKS", "--yes", "--prepare-only"], 2);
    invalid(&["program", "FIREWORKS", "--yes", "--dry-run"], 2);
    invalid(&["program", "FIREWORKS", "--manual-run", "--dry-run"], 2);
}

#[test]
fn help_documents_the_one_command_flow() {
    let result = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "delete", "program", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("--yes"), "{text}");
    assert!(text.contains("--dry-run"), "{text}");
    assert!(
        text.contains("--yes` deletes exactly that one program"),
        "{text}"
    );
}
