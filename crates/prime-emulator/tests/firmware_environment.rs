//! Genuine local firmware environment changes, over the production session API.
//! Copied profiles only; never enumerates or opens physical USB.
use ppl_engine::{
    Cancellation,
    environment::{BatteryConfig, BatteryPower, EnvironmentConfig},
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::{CalculatorInput, Command, Event},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
fn genuine_battery_environment_changes_lcd_and_keeps_a_waiting_program_running() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        return;
    };
    let guard = tempfile::tempdir().unwrap();
    let profile = if let Some(base) = std::env::var_os("PRIME_TEST_EVIDENCE_DIR") {
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("firmware-environment-")
            .tempdir_in(base)
            .unwrap()
            .keep()
    } else {
        guard.path().join("profile")
    };
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
    let timeout = Duration::from_secs(60);
    let mut session = LiveSession::spawn(&options, &profile, &cancel).unwrap();
    session.inspect(1, timeout, &cancel).unwrap();
    session.send(&Command::FrameRequest).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    let before = session.frame().expect("actual initial LCD frame");
    let config = EnvironmentConfig {
        battery: BatteryConfig {
            adc: 812,
            power: BatteryPower::Charging,
        },
        ..Default::default()
    };
    session
        .send(&Command::ConfigureEnvironment {
            request: 100,
            config: config.clone(),
        })
        .unwrap();
    let end = Instant::now() + timeout;
    loop {
        if let Event::Environment {
            request,
            success,
            config: actual,
            message,
        } = session.event(end, &cancel).unwrap()
        {
            assert_eq!(request, 100);
            assert!(success, "{message}");
            assert_eq!(actual, config);
            break;
        }
    }
    let end = Instant::now() + Duration::from_secs(10);
    let mut changed = false;
    while Instant::now() < end {
        let inspection = session.inspect(2, timeout, &cancel).unwrap();
        assert_eq!(
            inspection.resources["environment"],
            serde_json::to_value(&config).unwrap()
        );
        session.send(&Command::FrameRequest).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        if let Some(frame) = session.frame() {
            changed =
                (0..18).any(|y| (312..320).any(|x| frame[y * 320 + x] != before[y * 320 + x]));
            if changed {
                break;
            }
        }
    }
    assert!(
        changed,
        "the genuine LCD battery indicator must change, not only the environment report"
    );
    let source = "EXPORT ENVPPL() BEGIN WAIT(0); RETURN 73; END;";
    let hash = ppl_engine::source::hash(source.as_bytes());
    assert!(
        session
            .request(
                3,
                &Command::StoreProgram {
                    request: 3,
                    name: "ZZT_ENV".into(),
                    source: source.into(),
                    expected_previous_sha256: None
                },
                timeout,
                &cancel
            )
            .unwrap()
            .success
    );
    session
        .send(&Command::RunStored {
            request: 4,
            name: "ZZT_ENV".into(),
            source_sha256: hash.clone(),
            entry: "ENVPPL".into(),
            arguments: String::new(),
        })
        .unwrap();
    session
        .await_started(4, &hash, "ENVPPL", timeout, &cancel)
        .unwrap();
    let mut charged = config;
    charged.battery.power = BatteryPower::Charged;
    session
        .send(&Command::ConfigureEnvironment {
            request: 101,
            config: charged,
        })
        .unwrap();
    let end = Instant::now() + timeout;
    loop {
        match session.event(end, &cancel).unwrap() {
            Event::Environment {
                request,
                success,
                message,
                ..
            } => {
                assert_eq!(request, 101);
                assert!(success, "{message}");
                break;
            }
            Event::Result { request: 4, .. } => {
                panic!("environment change unexpectedly completed the waiting program")
            }
            _ => {}
        }
    }
    for (request, down) in [(5, true), (6, false)] {
        assert!(
            session
                .request(
                    request,
                    &Command::ScenarioInput {
                        request,
                        run_request: 4,
                        input: CalculatorInput::Key { physical: 30, down }
                    },
                    timeout,
                    &cancel
                )
                .unwrap()
                .success
        );
    }
    let result = session.await_result(4, timeout, &cancel).unwrap();
    assert!(result.success);
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"73"}))
    );
    session.power_down(timeout, &cancel).unwrap();
    println!(
        "genuine-battery-environment: changed LCD and resumed PPL73; profile {}",
        profile.display()
    );
}

#[test]
fn genuine_firmware_receives_virtual_usb_ready_request_and_produces_its_own_reply() {
    use ppl_device::{transport::Transport, virtual_device::VirtualTransport};
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        return;
    };
    let guard = tempfile::tempdir().unwrap();
    let profile = guard.path().join("profile");
    for drive in ["A", "C"] {
        copy_profile(&Path::new(&initial).join(drive), &profile.join(drive)).unwrap();
    }
    let options = Options {
        firmware: Some(firmware.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        runner: Some(env!("CARGO_BIN_EXE_prime-emulator").into()),
        ..Default::default()
    };
    let mut wire = VirtualTransport::connect(&options, &profile).unwrap();
    // After the constructor's genuine protocol handshake, a new raw HID
    // request must elicit another reply from the original firmware itself.
    assert_eq!(wire.write(&[0, 0, 0xff]).unwrap(), 3);
    let mut reply = [0; 64];
    assert_eq!(wire.read_timeout(&mut reply, 15000).unwrap(), 64);
    assert_eq!(&reply[..2], &[0, 0x59]);
    println!("genuine virtual USB raw ready reply: 64 bytes, type 00/59");
}

#[test]
fn genuine_virtual_usb_client_reads_info_screen_and_uploads_verified_program() {
    use ppl_device::{FileType, Prime, Probe, ReadBack};
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        return;
    };
    let guard = tempfile::tempdir().unwrap();
    let profile = if let Some(base) = std::env::var_os("PRIME_TEST_EVIDENCE_DIR") {
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("firmware-usb-")
            .tempdir_in(base)
            .unwrap()
            .keep()
    } else {
        guard.path().join("profile")
    };
    for drive in ["A", "C"] {
        copy_profile(&Path::new(&initial).join(drive), &profile.join(drive)).unwrap();
    }
    let options = Options {
        firmware: Some(firmware.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        runner: Some(env!("CARGO_BIN_EXE_prime-emulator").into()),
        ..Default::default()
    };
    let source = "EXPORT ZZT_USB() BEGIN RETURN 1234567+9; END;";
    {
        let mut prime = Prime::open_virtual(&options, &profile).unwrap();
        assert_eq!(prime.check_ready().unwrap()[0], 0x59);
        let info = prime.info().unwrap();
        assert_eq!(info.raw[0], 0xfa);
        assert!(
            info.raw.len() > 40,
            "firmware info must include real structure"
        );
        println!(
            "genuine USB info: build={:?}, version={:?}",
            info.build, info.version
        );
        let screen = prime
            .screenshot_timeout(8, std::time::Duration::from_secs(60))
            .unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&screen));
        let mut reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (320, 240));
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert!(
            pixels.windows(2).any(|w| w[0] != w[1]),
            "actual LCD cannot be a constant image"
        );
        let permit = prime
            .preflight_new_names(&[(FileType::Program, "ZZT_USB")], None)
            .unwrap();
        let uploaded = prime.send_new_program(permit, source).unwrap();
        assert!(
            matches!(uploaded.readback, ReadBack::Matches),
            "{uploaded:?}"
        );
        let Probe::Present(file) = prime.request_file(FileType::Program, "ZZT_USB").unwrap() else {
            panic!("uploaded program must answer F8");
        };
        assert!(file.crc_ok);
        assert_eq!(file.text(), source);
        // Existing names are refused through the same guarded transfer client.
        assert!(
            prime
                .preflight_new_names(&[(FileType::Program, "ZZT_USB")], None)
                .is_err()
        );
    }
    // The USB-installed program must survive an ordinary OFF and execute in
    // a fresh firmware process, not merely round-trip through a host cache.
    let cancel = Cancellation::default();
    let timeout = Duration::from_secs(60);
    let mut session = LiveSession::spawn(&options, &profile, &cancel).unwrap();
    let hash = ppl_engine::source::hash(source.as_bytes());
    session
        .send(&Command::RunStored {
            request: 1,
            name: "ZZT_USB".into(),
            source_sha256: hash.clone(),
            entry: "ZZT_USB".into(),
            arguments: String::new(),
        })
        .unwrap();
    session
        .await_started(1, &hash, "ZZT_USB", timeout, &cancel)
        .unwrap();
    let result = session.await_result(1, timeout, &cancel).unwrap();
    assert!(result.success, "{}", result.message);
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"1234576"}))
    );
    session.power_down(timeout, &cancel).unwrap();
    println!(
        "genuine virtual USB info/screenshot/guarded upload/readback/reboot/PPL1234576: {}",
        profile.display()
    );
}
