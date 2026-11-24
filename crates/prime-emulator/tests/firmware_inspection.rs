//! Optional real firmware live inspection. Only copied test storage, never USB.
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::{CalculatorInput, Command},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
fn live_inspection_pauses_explicit_waits_and_keeps_a_running_program_usable() {
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
    // Ready means the UI responds; timer handling may still put the main CPU
    // between event waits at this exact slice boundary. Establish the specific
    // production wait that this test subsequently inspects, rather than
    // assuming an arbitrary paused instruction must be an OSWaitForEvent.
    let end = Instant::now() + Duration::from_secs(5);
    let mut inspection_request = 100;
    let idle = loop {
        let idle = live.inspect(inspection_request, deadline, &cancel).unwrap();
        assert!(idle.readiness.owners_paused, "{:?}", idle.readiness);
        assert!(idle.readiness.memory_gate_acquired, "{:?}", idle.readiness);
        assert_eq!(idle.readiness.parked_native_callbacks, 0);
        let pending = idle.resources["pending_services"].as_object().unwrap();
        if pending.values().any(|p| p["wait"]["kind"] == "event")
            && pending.values().any(|p| p["wait"]["kind"] == "input")
        {
            break idle;
        }
        assert!(
            Instant::now() < end,
            "Firmware did not reach its real UI event wait: {idle:?}"
        );
        inspection_request += 1;
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(idle.readiness.live_restore_ready, "{:?}", idle.readiness);
    assert_eq!(
        idle.resources["producer_component"]["captured"], true,
        "{:?}",
        idle.resources["producer_component"]
    );
    assert_eq!(
        idle.resources["managed_component"]["captured"], true,
        "{:?}",
        idle.resources["managed_component"]
    );
    assert!(idle.readiness.owners_paused, "{:?}", idle.readiness);
    assert!(idle.readiness.memory_gate_acquired, "{:?}", idle.readiness);
    assert_eq!(idle.readiness.parked_native_callbacks, 0);
    assert_eq!(
        idle.resources["kernel_component"]["captured"], true,
        "{:?}",
        idle.resources["kernel_component"]
    );
    assert!(
        idle.resources["synchronization_error"].is_null(),
        "{:?}",
        idle.resources["synchronization_error"]
    );
    let synchronization = idle.resources["synchronization_image"].clone();
    let synchronization: prime_emulator::services::sync::image::SyncImage =
        serde_json::from_value(synchronization).unwrap();
    assert!(!synchronization.resources.is_empty());
    assert!(synchronization.resources.iter().any(|r| matches!(
        r.object,
        prime_emulator::services::sync::image::ObjectImage::Event { .. }
    )));
    let images = idle.resources["owner_images"].as_object().unwrap();
    for owner in idle
        .owners
        .iter()
        .filter(|o| o.phase != ppl_engine::inspection::OwnerPhase::Exited)
    {
        let image = &images[&owner.thread_id.to_string()];
        assert_eq!(image["epoch"], owner.acknowledged_epoch);
        assert!(image["error"].is_null(), "{image:?}");
        assert_eq!(image["image"]["cpu"]["banks"].as_array().unwrap().len(), 6);
        assert!(image["image"]["cpu"]["cp15"].as_array().unwrap().len() >= 10);
    }
    let pending = idle.resources["pending_services"].as_object().unwrap();
    assert!(
        pending.values().any(|p| p["wait"]["kind"] == "event"),
        "{pending:?}"
    );
    assert!(
        pending.values().any(|p| p["wait"]["kind"] == "input"),
        "{pending:?}"
    );
    assert!(idle.readiness.blockers.is_empty());
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
    assert_eq!(
        busy.resources["producer_component"]["captured"], true,
        "{:?}",
        busy.resources["producer_component"]
    );
    assert_eq!(
        busy.resources["managed_component"]["captured"], true,
        "{:?}",
        busy.resources["managed_component"]
    );
    assert_eq!(
        busy.resources["kernel_component"]["captured"], true,
        "{:?}",
        busy.resources["kernel_component"]
    );
    assert!(busy.readiness.live_restore_ready, "{:?}", busy.readiness);
    assert!(busy.readiness.blockers.is_empty());
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
