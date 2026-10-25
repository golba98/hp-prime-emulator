//! Integration tests for `ppl device mirror`.
use std::process::Command;

#[test]
fn device_mirror_help_is_discoverable_and_documents_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "mirror", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Live physical calculator screen mirroring"));
    assert!(stdout.contains("--scale"));
    assert!(stdout.contains("--fps"));
    assert!(stdout.contains("--fullscreen"));
    assert!(stdout.contains("--snapshot"));
    assert!(stdout.contains("--filter"));
    assert!(stdout.contains("--quality"));
    assert!(stdout.contains("--capture"));
    assert!(stdout.contains("incremental"));
    assert!(stdout.contains("--diagnostics"));
}

#[test]
fn device_help_lists_mirror_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("mirror"));
    assert!(stdout.contains("Live physical calculator screen mirroring window"));
}

#[test]
fn existing_device_screenshot_still_works_and_documents_format() {
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "screenshot", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Save the calculator screen as PNG"));
    assert!(stdout.contains("--format"));
}

#[test]
fn mirror_with_nonexistent_device_exits_with_error_and_does_not_panic() {
    // When an invalid device serial is given, it safely errors before opening
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "mirror", "--device", "0000000000000000"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:") || stderr.contains("not found"));
}

#[test]
fn invalid_mirror_options_fail_before_usb_is_opened() {
    for (flag, value) in [
        ("--scale", "0"),
        ("--scale", "5"),
        ("--fps", "0"),
        ("--fps", "NaN"),
        ("--fps", "inf"),
        ("--filter", "sharp"),
        ("--quality", "thumbnail"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
            .args(["device", "mirror", flag, value])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{flag} {value}: {:?}", out);
        assert!(!String::from_utf8_lossy(&out.stderr).contains("device busy"));
    }
}

#[test]
fn incremental_refuses_before_any_usb_or_ipc_access() {
    for extra in [
        vec![],
        vec!["--quality", "fast"],
        vec!["--control-socket", "/does-not-exist"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
            .args(["device", "mirror", "--capture", "incremental"])
            .args(extra)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["status"], "unsupported_capability");
        assert_eq!(result["usb_commands_sent"], 0);
    }
}
#[test]
fn experimental_probe_requires_explicit_acknowledgement_and_identity() {
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "device",
            "probe-incremental",
            "--out-dir",
            "/must-not-be-created",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "device",
            "probe-incremental",
            "--acknowledge-experimental-fc04",
            "--out-dir",
            "/must-not-be-created",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["fc04_attempts"], 0);
    assert_eq!(result["status"], "refused");
}

#[test]
fn device_help_lists_verify_incremental_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("verify-incremental"));
    assert!(stdout.contains("visual-change verification"));
}

#[test]
fn verify_incremental_requires_explicit_acknowledgement_and_device() {
    // Missing required --acknowledge-experimental-fc04
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "device",
            "verify-incremental",
            "--out-dir",
            "/must-not-be-created",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));

    // Acknowledged, but missing --device
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args([
            "device",
            "verify-incremental",
            "--acknowledge-experimental-fc04",
            "--out-dir",
            "/must-not-be-created",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["fc04_attempts"], 0);
    assert_eq!(result["status"], "refused");
    assert_eq!(result["operation"], "verify_incremental");
}

#[test]
fn verify_incremental_rejects_out_of_bounds_updates() {
    for bad_n in ["0", "51"] {
        let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
            .args([
                "device",
                "verify-incremental",
                "--acknowledge-experimental-fc04",
                "--out-dir",
                "/must-not-be-created",
                "--updates",
                bad_n,
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "updates {bad_n} should fail");
    }
}
