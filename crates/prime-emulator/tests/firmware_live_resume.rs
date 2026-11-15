//! Genuine locally installed G1 execution across a complete helper-process
//! replacement. All storage is disposable; no physical device is opened.
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
fn a_live_ppl_frame_and_pending_wait_resume_in_a_fresh_firmware_process() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        return;
    };
    let evidence = std::env::var_os("PRIME_TEST_EVIDENCE_DIR");
    let temporary = if let Some(evidence) = &evidence {
        std::fs::create_dir_all(evidence).unwrap();
        tempfile::Builder::new()
            .prefix("prime-live-resume-")
            .tempdir_in(evidence)
            .unwrap()
    } else {
        tempfile::Builder::new()
            .prefix("prime-live-resume-")
            .tempdir()
            .unwrap()
    };
    let root = temporary.keep();
    eprintln!("live-resume profile/evidence: {}", root.display());
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
    let timeout = Duration::from_secs(120);
    let mut live = LiveSession::spawn(&options, &profile, &cancel).unwrap();
    let environment = ppl_engine::environment::EnvironmentConfig {
        battery: ppl_engine::environment::BatteryConfig {
            adc: 838,
            power: ppl_engine::environment::BatteryPower::Charging,
        },
        storage_capacity_bytes: 128 * 1024 * 1024,
        board_revision_pins: 3,
    };
    live.send(&Command::ConfigureEnvironment {
        request: 0,
        config: environment.clone(),
    })
    .unwrap();
    let end = Instant::now() + timeout;
    loop {
        if let ppl_engine::session::Event::Environment {
            request: 0,
            success,
            config,
            message,
        } = live.event(end, &cancel).unwrap()
        {
            assert!(success, "{message}");
            assert_eq!(config, environment);
            break;
        }
    }
    let source = "EXPORT M17LIVEINIT() BEGIN A:=12; RETURN A; END;\nEXPORT M17LIVEFRAME() BEGIN LOCAL n:=37; A:=A+100; WAIT(0); A:=A+1; RETURN {A,n,A+n}; END;\nEXPORT M17LIVETIMED() BEGIN LOCAL n:=73; A:=A+1000; WAIT(5); A:=A+1; RETURN {A,n}; END;";
    let hash = ppl_engine::source::hash(source.as_bytes());
    let imported = live
        .request(
            1,
            &Command::StoreProgram {
                request: 1,
                name: "ZZT_LIVERESUME".into(),
                source: source.into(),
                expected_previous_sha256: None,
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(imported.success, "{}", imported.message);
    let initialized = live
        .request(
            2,
            &Command::RunStored {
                request: 2,
                name: "ZZT_LIVERESUME".into(),
                source_sha256: hash.clone(),
                entry: "M17LIVEINIT".into(),
                arguments: String::new(),
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(initialized.success, "{}", initialized.message);
    assert_eq!(
        initialized.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"12"}))
    );
    live.send(&Command::RunStored {
        request: 3,
        name: "ZZT_LIVERESUME".into(),
        source_sha256: hash.clone(),
        entry: "M17LIVEFRAME".into(),
        arguments: String::new(),
    })
    .unwrap();
    live.await_started(3, &hash, "M17LIVEFRAME", timeout, &cancel)
        .unwrap();
    let saved = live
        .request(4, &Command::SaveLive { request: 4 }, timeout, &cancel)
        .unwrap();
    assert!(saved.success, "{}", saved.message);
    // Drop kills and joins this test's own helper, without firmware OFF.
    drop(live);
    let mut restored = LiveSession::spawn_resuming(&options, &profile, &cancel).unwrap();
    let attachment = restored.resumed().unwrap().clone();
    assert!(!attachment.previous_generation);
    assert_eq!(attachment.environment, environment);
    assert_eq!(attachment.next_request, 5);
    assert_eq!(attachment.pending.len(), 1);
    assert_eq!(attachment.pending[0].request, 3);
    assert_eq!(attachment.pending[0].source.as_deref(), Some(source));
    assert_eq!(attachment.pending[0].entry.as_deref(), Some("M17LIVEFRAME"));
    for (request, down) in [(5, true), (6, false)] {
        let reply = restored
            .request(
                request,
                &Command::ScenarioInput {
                    request,
                    run_request: 3,
                    input: CalculatorInput::Key { physical: 30, down },
                },
                timeout,
                &cancel,
            )
            .unwrap();
        assert!(reply.success, "{}", reply.message);
    }
    let result = restored.await_result(3, timeout, &cancel).unwrap();
    assert!(result.success, "{}", result.message);
    let observed = result.execution.unwrap();
    assert_eq!(
        observed.return_value,
        Some(
            serde_json::json!({"type":"list","value":[{"type":"real","value":"113"},{"type":"real","value":"37"},{"type":"real","value":"150"}]})
        )
    );
    std::fs::write(
        root.join("resumed-return.json"),
        serde_json::to_vec_pretty(&observed).unwrap(),
    )
    .unwrap();
    // A second generation captures a real timed PPL frame. Publication and
    // helper downtime both exceed the original wait; restoration must retain
    // its guest-time remainder rather than treating that host time as elapsed.
    restored
        .send(&Command::RunStored {
            request: 7,
            name: "ZZT_LIVERESUME".into(),
            source_sha256: hash.clone(),
            entry: "M17LIVETIMED".into(),
            arguments: String::new(),
        })
        .unwrap();
    restored
        .await_started(7, &hash, "M17LIVETIMED", timeout, &cancel)
        .unwrap();
    let saved = restored
        .request(8, &Command::SaveLive { request: 8 }, timeout, &cancel)
        .unwrap();
    assert!(saved.success, "{}", saved.message);
    // Ordinary transport shutdown, without changing the saved live image.
    restored.send(&Command::Shutdown).unwrap();
    let end = Instant::now() + timeout;
    loop {
        match restored.event(end, &cancel).unwrap() {
            ppl_engine::session::Event::Closed { .. } => break,
            ppl_engine::session::Event::Error { message } => panic!("{message}"),
            _ => {}
        }
    }
    drop(restored);
    let mut timed = LiveSession::spawn_resuming(&options, &profile, &cancel).unwrap();
    let attachment = timed.resumed().unwrap();
    assert_eq!(attachment.next_request, 9);
    assert_eq!(attachment.pending[0].request, 7);
    let resumed_at = Instant::now();
    let result = timed.await_result(7, timeout, &cancel).unwrap();
    assert!(result.success, "{}", result.message);
    assert!(
        resumed_at.elapsed() >= Duration::from_millis(500),
        "Host downtime consumed a saved guest PPL timer"
    );
    assert_eq!(
        result.execution.as_ref().unwrap().return_value,
        Some(
            serde_json::json!({"type":"list","value":[{"type":"real","value":"1114"},{"type":"real","value":"73"}]})
        )
    );
    std::fs::write(
        root.join("timed-return.json"),
        serde_json::to_vec_pretty(&result.execution).unwrap(),
    )
    .unwrap();
    drop(timed);
    // A damaged current payload explicitly selects the intact previous full
    // image, including its older active WAIT and exact request/source binding.
    let pointer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join(".live/current.json")).unwrap())
            .unwrap();
    let damaged = profile
        .join(".live")
        .join(pointer["generation"].as_str().unwrap())
        .join("memory.gz");
    std::fs::write(&damaged, b"deliberately damaged test current image").unwrap();
    let mut fallback = LiveSession::spawn_resuming(&options, &profile, &cancel).unwrap();
    assert!(fallback.resumed().unwrap().previous_generation);
    assert_eq!(fallback.resumed().unwrap().next_request, 5);
    for (request, down) in [(5, true), (6, false)] {
        assert!(
            fallback
                .request(
                    request,
                    &Command::ScenarioInput {
                        request,
                        run_request: 3,
                        input: CalculatorInput::Key { physical: 30, down }
                    },
                    timeout,
                    &cancel
                )
                .unwrap()
                .success
        );
    }
    let result = fallback.await_result(3, timeout, &cancel).unwrap();
    assert!(result.success, "{}", result.message);
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(
            serde_json::json!({"type":"list","value":[{"type":"real","value":"113"},{"type":"real","value":"37"},{"type":"real","value":"150"}]})
        )
    );
    fallback.power_down(timeout, &cancel).unwrap();
    drop(fallback);
    // A different ROM identity is refused without touching any guest file.
    let before = ["A", "C"]
        .map(|drive| prime_emulator::storage::try_snapshot(&profile.join(drive)).unwrap());
    let mut incompatible = options.clone();
    if let Some(rom) = &options.os_rom {
        use std::io::Write;
        let changed = root.join("different-ROM.bin");
        std::fs::copy(rom, &changed).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&changed)
            .unwrap()
            .write_all(b"test identity change")
            .unwrap();
        incompatible.os_rom = Some(changed);
        let error = LiveSession::spawn_resuming(&incompatible, &profile, &cancel)
            .err()
            .expect("An incompatible ROM was accepted");
        assert!(error.contains("compatible live archive"), "{error}");
        let after = ["A", "C"]
            .map(|drive| prime_emulator::storage::try_snapshot(&profile.join(drive)).unwrap());
        assert_eq!(before, after, "Incompatible restore changed guest files");
    }
}
