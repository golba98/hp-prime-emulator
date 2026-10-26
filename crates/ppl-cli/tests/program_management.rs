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
        "/../../examples/FIREWORKS.hpprgm"
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
    assert_eq!(json["sizes"]["payload_bytes"], 20048);
    assert_eq!(json["sizes"]["hid_reports"], 319);
    assert_eq!(json["sizes"]["supported"], true);
    assert_eq!(
        json["sizes"]["limit_kind"],
        "verified_extended_mode1_transport_limit_16mib"
    );
    assert_eq!(json["uploaded"], false);
}
#[test]
fn replacement_size_and_authorization_fail_before_usb() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("oversize.hpprgm");
    let size = (ppl_device::MAX_EXTENDED_PROGRAM_PAYLOAD / 2) + 10;
    let mut f = std::fs::File::create(&file).unwrap();
    use std::io::Write;
    let chunk = vec![b'A'; 64 * 1024];
    let mut written = 0;
    while written < size {
        let to_write = (size - written).min(chunk.len());
        f.write_all(&chunk[..to_write]).unwrap();
        written += to_write;
    }
    drop(f);

    let result = cli(&[
        "device",
        "replace",
        "prepare",
        "OVERSIZE",
        file.to_str().unwrap(),
        "--json",
    ]);
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(!result.status.success());
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("exceeds maximum limit")
    );
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
