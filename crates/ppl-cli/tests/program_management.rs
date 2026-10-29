//! Every case stops before USB; no test is authorized to mutate hardware.
use std::process::Command;
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn full_fireworks_inspection_is_structured_and_local() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/FIREWORKS.hpprgm"
    );
    let result = cli(&[
        "device",
        "--device",
        "not-a-connected-device",
        "replace",
        "inspect",
        "FIREWORKS",
        source,
        "--json",
    ]);
    assert!(result.status.success());
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    // The fixed fixture has 10,644 UTF-16 code units. The user's editable
    // example may change while developing programs in the IDE.
    assert_eq!(json["sizes"]["utf16_units"], 10644);
    assert_eq!(json["sizes"]["payload_bytes"], 21288);
    assert_eq!(json["sizes"]["hid_reports"], 339);
    assert_eq!(json["sizes"]["supported"], false);
    assert_eq!(json["uploaded"], false);
}
#[test]
fn replacement_size_and_authorization_fail_before_usb() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/FIREWORKS.hpprgm"
    );
    let result = cli(&[
        "device",
        "replace",
        "prepare",
        "FIREWORKS",
        source,
        "--json",
    ]);
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(!result.status.success());
    assert!(json["error"].as_str().unwrap().contains("conservative"));
    let result = cli(&[
        "device",
        "replace",
        "apply",
        "--operation-id",
        "ZZT_D12345",
        "--json",
    ]);
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(json["error"].as_str().unwrap().contains("--yes"));
    assert_eq!(json["uploaded"], false);
}
#[test]
fn combined_check_reports_provenance_and_real_syntax_location() {
    let result = cli(&[
        "check",
        "--all-backends",
        "--json",
        "--code",
        "EXPORT DEMO()\nBEGIN\nLOCAL := ;\nEND;",
    ]);
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(!result.status.success());
    assert_eq!(json["physical_check_executed"], false);
    assert!(
        json["native"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["line"] == 3 && d["source"] == "native-compiler")
    );
}
#[test]
fn malformed_upload_encoding_is_never_lossily_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("bad.hpprgm");
    std::fs::write(&source, [0xff, 0xfe, 0x00]).unwrap();
    let result = cli(&["device", "send", "ZZT_TEST", source.to_str().unwrap()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("odd UTF-16"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("About to create"));
}
