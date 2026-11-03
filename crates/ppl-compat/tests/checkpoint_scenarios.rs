//! Checkpoint scenarios use disposable local-firmware images, never USB.
use ppl_compat::differential::*;
use ppl_engine::{
    Cancellation,
    checkpoint::CheckpointResult,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::Command,
};
use std::{path::Path, time::Duration};

fn fixture() -> LoadedScenario {
    LoadedScenario::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/differential/m16/checkpoint.toml"),
    )
    .unwrap()
}
fn options() -> Option<Options> {
    let (Some(firmware), Some(init), Some(runner)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
        std::env::var_os("PRIME_TEST_RUNNER"),
    ) else {
        eprintln!("skipping local-firmware checkpoint: PRIME_TEST_FIRMWARE/INIT_DATA/RUNNER unset");
        return None;
    };
    Some(Options {
        runner: Some(runner.into()),
        firmware: Some(firmware.into()),
        init_data: Some(init.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        ..Default::default()
    })
}
#[test]
fn native_checkpoint_is_explicitly_unsupported_and_v1_is_rejected() {
    let fixture = fixture();
    let report = run_native(&fixture, &Cancellation::default());
    assert_eq!(report.status, RunStatus::Unsupported);
    assert!(report.reason.unwrap().contains("checkpoint"));
    let mut old = fixture.scenario.clone();
    old.schema_version = 1;
    assert!(LoadedScenario::from_spec(old, Path::new("."), "old".into()).is_err());
}
#[test]
fn real_firmware_checkpoint_scenario_restarts_from_saved_state() {
    let Some(options) = options() else {
        return;
    };
    let fixture = fixture();
    let result = compare(
        &fixture,
        vec![run_firmware(&fixture, &options, &Cancellation::default())],
    );
    if let Some(path) = std::env::var_os("PRIME_M16_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    assert!(
        result.passed,
        "{:?}; {:?}",
        result.mismatches,
        result.runs.iter().map(|r| &r.reason).collect::<Vec<_>>()
    );
}
#[test]
fn real_firmware_typed_checkpoint_survives_process_kill() {
    let Some(options) = options() else {
        return;
    };
    let profile = tempfile::tempdir().unwrap();
    let original = options.init_data.as_ref().unwrap();
    for drive in ["A", "C"] {
        copy_profile(&original.join(drive), &profile.path().join(drive)).unwrap();
    }
    let fixture = fixture();
    let source = fixture.sources["ZZT_M16CK"].clone();
    let hash = fixture.source_manifest()["ZZT_M16CK"].clone();
    let cancel = Cancellation::default();
    let mut live = LiveSession::spawn(&options, profile.path(), &cancel).unwrap();
    let stored = live
        .request(
            1,
            &Command::StoreProgram {
                request: 1,
                name: "ZZT_M16CK".into(),
                source,
                expected_previous_sha256: None,
            },
            options.timeout,
            &cancel,
        )
        .unwrap();
    assert!(stored.success, "{}", stored.message);
    let run = live
        .request(
            2,
            &Command::RunStored {
                request: 2,
                name: "ZZT_M16CK".into(),
                source_sha256: hash.clone(),
                entry: "M16CKSET".into(),
                arguments: String::new(),
            },
            options.timeout,
            &cancel,
        )
        .unwrap();
    assert!(run.success, "{}", run.message);
    // A normal Result waiter must refuse the differently typed operation promptly.
    let rejected = live
        .request(
            3,
            &Command::Checkpoint { id: 3, wake: false },
            Duration::from_millis(1),
            &cancel,
        )
        .err()
        .unwrap();
    assert!(rejected.contains("LiveSession::checkpoint"));
    let saved = live
        .checkpoint(4, false, Duration::from_secs(60), &cancel)
        .unwrap();
    let CheckpointResult::Saved { receipt } = saved else {
        panic!("checkpoint was not saved: {saved:?}");
    };
    assert!(!receipt.restores_cpu && !receipt.restores_framebuffer);
    // Drop kills this owned helper group immediately, without sending firmware OFF.
    drop(live);
    let mut recovered = LiveSession::spawn(&options, profile.path(), &cancel).unwrap();
    let probe = recovered
        .request(
            5,
            &Command::RunStored {
                request: 5,
                name: "ZZT_M16CK".into(),
                source_sha256: hash.clone(),
                entry: "M16CKGET".into(),
                arguments: String::new(),
            },
            options.timeout,
            &cancel,
        )
        .unwrap();
    assert!(probe.success, "{}", probe.message);
    assert_eq!(
        probe.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"731"}))
    );
    let woke = recovered
        .checkpoint(6, true, Duration::from_secs(60), &cancel)
        .unwrap();
    assert!(matches!(woke, CheckpointResult::Saved { .. }));
    let awake_probe = recovered
        .request(
            7,
            &Command::RunStored {
                request: 7,
                name: "ZZT_M16CK".into(),
                source_sha256: hash,
                entry: "M16CKGET".into(),
                arguments: String::new(),
            },
            options.timeout,
            &cancel,
        )
        .unwrap();
    assert!(awake_probe.success, "{}", awake_probe.message);
    assert_eq!(
        awake_probe.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"731"}))
    );
    recovered
        .power_down(Duration::from_secs(60), &cancel)
        .unwrap();
}
