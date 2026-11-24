//! Original compiler and live runtime return exact Home numbers losslessly.
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::Command,
};
use std::{path::Path, time::Duration};

#[test]
fn original_firmware_returns_static_pi_coefficients_and_exact_numeric_lists() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        eprintln!("numeric firmware controls require PRIME_TEST_FIRMWARE/INIT_DATA");
        return;
    };
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/firmware-numeric-return-controls.json"
    ))
    .unwrap();
    assert_eq!(
        prime_emulator::sha256::hex(&std::fs::read(&firmware).unwrap()),
        controls["firmware_sha256"]
    );
    let temporary = if let Some(evidence) = std::env::var_os("PRIME_TEST_EVIDENCE_DIR") {
        std::fs::create_dir_all(&evidence).unwrap();
        tempfile::Builder::new()
            .prefix("firmware-numeric-")
            .tempdir_in(evidence)
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let root = temporary.keep();
    eprintln!("numeric evidence: {}", root.display());
    let profile = root.join("profile");
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
    for (index, case) in controls["cases"].as_array().unwrap().iter().enumerate() {
        let request = 900 + index as u64;
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            prime_emulator::sha256::hex(source.as_bytes()),
            case["source_sha256"]
        );
        let name = format!("ZZT_numeric_{index}");
        let stored = live
            .request(
                request + 100,
                &Command::StoreProgram {
                    request: request + 100,
                    name: name.clone(),
                    source: source.into(),
                    expected_previous_sha256: None,
                },
                Duration::from_secs(90),
                &cancel,
            )
            .unwrap();
        assert!(stored.success, "{}", stored.message);
        assert!(stored.check.unwrap().accepted);
        let response = live
            .request(
                request,
                &Command::RunStored {
                    request,
                    name,
                    source_sha256: case["source_sha256"].as_str().unwrap().into(),
                    entry: case["entry"].as_str().unwrap().into(),
                    arguments: String::new(),
                },
                Duration::from_secs(90),
                &cancel,
            )
            .unwrap();
        assert!(response.success, "{}: {}", case["name"], response.message);
        let execution = response.execution.unwrap();
        assert!(execution.body_entered && execution.completed);
        assert!(
            execution.runtime["fault"].is_null(),
            "{}",
            execution.runtime
        );
        for invocation in execution.runtime["invocations"].as_array().unwrap() {
            assert!(
                invocation["caveats"].as_array().unwrap().is_empty(),
                "{invocation}"
            );
            assert_eq!(invocation["result"]["representation"]["complete"], true);
            assert_eq!(invocation["result"]["representation"]["coherent"], true);
        }
        assert_eq!(
            execution.return_representation,
            serde_json::json!({"kind":"decoded"})
        );
        assert_eq!(
            execution.return_value.unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
        std::fs::write(
            root.join(format!("{}-execution.json", case["name"].as_str().unwrap())),
            serde_json::to_vec_pretty(&execution.runtime).unwrap(),
        )
        .unwrap();
    }
}
