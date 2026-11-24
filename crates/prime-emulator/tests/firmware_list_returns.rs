//! Actual ephemeral returns, independently checked against stored/STRING values.
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::Command,
};
use std::{path::Path, time::Duration};
#[test]
fn original_ephemeral_list_flags_retain_every_typed_element_and_exact_projection() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        eprintln!("Original list controls require PRIME_TEST_FIRMWARE/INIT_DATA");
        return;
    };
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/firmware-list-flag-controls.json"
    ))
    .unwrap();
    assert_eq!(
        prime_emulator::sha256::hex(&std::fs::read(&firmware).unwrap()),
        fixture["firmware_sha256"]
    );
    let temporary = if let Some(root) = std::env::var_os("PRIME_TEST_EVIDENCE_DIR") {
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("firmware-list-")
            .tempdir_in(root)
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let root = temporary.keep();
    let profile = root.join("profile");
    eprintln!("List evidence: {}", root.display());
    for drive in ["A", "C"] {
        copy_profile(&Path::new(&initial).join(drive), &profile.join(drive)).unwrap();
    }
    let options = Options {
        firmware: Some(firmware.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        runner: Some(env!("CARGO_BIN_EXE_prime-emulator").into()),
        ..Default::default()
    };
    let cancel = Cancellation::default();
    let mut live = LiveSession::spawn(&options, &profile, &cancel).unwrap();
    // Reproduce the original two-program allocation sequence; the first
    // return initializes settings and retires its temporary program objects.
    let reset = live
        .request(
            1800,
            &Command::Run {
                request: 1800,
                source: fixture["reset_source"].as_str().unwrap().into(),
                entry: None,
                arguments: String::new(),
            },
            Duration::from_secs(90),
            &cancel,
        )
        .unwrap();
    assert!(reset.success, "{}", reset.message);
    let mut cases = vec![
        (
            fixture["source"].as_str().unwrap(),
            fixture["source_sha256"].clone(),
            "GROUPADMIT",
            fixture["expected"].clone(),
        ),
        (
            fixture["projection_source"].as_str().unwrap(),
            fixture["projection_source_sha256"].clone(),
            "GROUPADMIT",
            serde_json::json!({"type":"string","value":fixture["projection_expected"]}),
        ),
    ];
    for case in fixture["additional_cases"].as_array().unwrap() {
        cases.push((
            case["source"].as_str().unwrap(),
            case["source_sha256"].clone(),
            case["entry"].as_str().unwrap(),
            case["expected"].clone(),
        ));
    }
    for (index, (source, hash, entry, expected)) in cases.into_iter().enumerate() {
        assert_eq!(prime_emulator::sha256::hex(source.as_bytes()), hash);
        let request = 900 + index as u64;
        let response = live
            .request(
                request,
                &Command::Run {
                    request,
                    source: source.into(),
                    entry: None,
                    arguments: String::new(),
                },
                Duration::from_secs(90),
                &cancel,
            )
            .unwrap();
        assert!(response.success, "{}", response.message);
        let path = profile.join(format!("ide-reports/request-{request}.json"));
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let invocation = report["runtime"]["invocations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|invocation| invocation["entry"] == entry)
            .unwrap();
        assert_eq!(invocation["source_sha256"], hash);
        assert!(invocation["caveats"].as_array().unwrap().is_empty());
        assert!(report["runtime"]["fault"].is_null());
        assert_eq!(invocation["status"], "completed");
        assert_eq!(invocation["body_entered"], true);
        assert_eq!(invocation["result"]["value"], expected);
        assert!(invocation["capture_error"].is_null());
        assert!(invocation["result"]["unavailable"].is_null());
        let graph = &invocation["result"]["representation"];
        assert_eq!(graph["coherent"], true);
        assert_eq!(graph["complete"], true);
        if index == 0 {
            assert!(
                graph["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|node| node["bytes"][2] == 0x16
                        && !node["children"].as_array().unwrap().is_empty()),
                "Control did not capture actual managed list tables"
            );
        }
        std::fs::write(
            root.join(format!("request-{request}.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
