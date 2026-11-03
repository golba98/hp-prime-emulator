//! Optional real firmware live inspection. Only copied test storage, never USB.
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::{CalculatorInput, Command},
};
use std::{path::Path, time::Duration};

#[test]
fn live_inspection_exposes_native_waits_and_keeps_a_running_program_usable() {
    let (Some(fw), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    for drive in ["A", "C"] {
        copy_profile(&Path::new(&initial).join(drive), &profile.join(drive)).unwrap();
    }
    let opts = Options {
        firmware: Some(fw.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        runner: Some(env!("CARGO_BIN_EXE_prime-emulator").into()),
        ..Default::default()
    };
    let cancel = Cancellation::default();
    let deadline = Duration::from_secs(60);
    let mut live = LiveSession::spawn(&opts, &profile, &cancel).unwrap();
    let idle = live.inspect(1, deadline, &cancel).unwrap();
    assert!(!idle.readiness.live_restore_ready);
    assert!(!idle.readiness.blockers.is_empty());
    assert!(idle.resources["main_wait_generation"].as_u64().unwrap() > 0);
    assert!(
        idle.owners
            .iter()
            .any(|o| o.service_name.as_deref() == Some("GetEvent"))
    );
    let source = "EXPORT M17INSPWAIT() BEGIN WAIT(0); RETURN 7; END;";
    let hash = ppl_engine::source::hash(source.as_bytes());
    assert!(
        live.request(
            2,
            &Command::StoreProgram {
                request: 2,
                name: "ZZT_M17INSPECT".into(),
                source: source.into(),
                expected_previous_sha256: None
            },
            deadline,
            &cancel
        )
        .unwrap()
        .success
    );
    live.send(&Command::RunStored {
        request: 3,
        name: "ZZT_M17INSPECT".into(),
        source_sha256: hash.clone(),
        entry: "M17INSPWAIT".into(),
        arguments: String::new(),
    })
    .unwrap();
    live.await_started(3, &hash, "M17INSPWAIT", deadline, &cancel)
        .unwrap();
    let busy = live.inspect(4, deadline, &cancel).unwrap();
    assert!(!busy.readiness.live_restore_ready);
    assert!(
        busy.readiness
            .blockers
            .iter()
            .any(|s| s == "managed_program_operation_active")
    );
    for (request, down) in [(5, true), (6, false)] {
        assert!(
            live.request(
                request,
                &Command::ScenarioInput {
                    request,
                    run_request: 3,
                    input: CalculatorInput::Key { physical: 30, down }
                },
                deadline,
                &cancel
            )
            .unwrap()
            .success
        );
    }
    let result = live.await_result(3, deadline, &cancel).unwrap();
    assert!(result.success);
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"7"}))
    );
    live.power_down(deadline, &cancel).unwrap();
}
