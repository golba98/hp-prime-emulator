//! Actual firmware text delivery across a helper replacement. No physical USB.
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::{Command, Event},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn idle(live: &mut LiveSession, next: &mut u64, cancel: &Cancellation) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < end, "firmware input did not settle");
        let report = live
            .inspect(*next, Duration::from_secs(10), cancel)
            .unwrap();
        *next += 1;
        let r = &report.resources;
        if r["typing_pending"] == 0
            && r["queued_keys"] == 0
            && r["input_pending"] == 0
            && r["touch_pending"] == 0
            && r["pump_parked"] == true
            && r["main_ui_idle"] == true
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
fn key(live: &mut LiveSession, code: u16) {
    for down in [true, false] {
        live.send(&Command::Key { code, down }).unwrap();
    }
}
fn tap(live: &mut LiveSession, x: u16) {
    live.send(&Command::Touch {
        status: 1,
        x,
        y: 230,
    })
    .unwrap();
    for _ in 0..18 {
        std::thread::sleep(Duration::from_millis(16));
        live.send(&Command::Touch {
            status: 2,
            x,
            y: 230,
        })
        .unwrap();
    }
    live.send(&Command::Touch {
        status: 8,
        x,
        y: 230,
    })
    .unwrap();
}
#[test]
fn queued_literal_text_survives_live_save_and_process_kill_exactly_once() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        return;
    };
    let temporary = if let Some(evidence) = std::env::var_os("PRIME_TEST_EVIDENCE_DIR") {
        std::fs::create_dir_all(&evidence).unwrap();
        tempfile::Builder::new()
            .prefix("firmware-text-")
            .tempdir_in(evidence)
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let root = temporary.keep();
    eprintln!("text/resume evidence: {}", root.display());
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
    let text = "314159265";
    // Preceding real cursor transitions keep this literal packet queued until
    // capture. Inspect the saved producer to require this tested interleaving.
    for _ in 0..40 {
        key(&mut live, 0x02);
    }
    live.send(&Command::Text {
        text: text.into(),
        paste: false,
    })
    .unwrap();
    let saved = live
        .request(
            1,
            &Command::SaveLive { request: 1 },
            Duration::from_secs(120),
            &cancel,
        )
        .unwrap();
    assert!(saved.success, "{}", saved.message);
    let pointer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join(".live/current.json")).unwrap())
            .unwrap();
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            profile
                .join(".live")
                .join(pointer["generation"].as_str().unwrap())
                .join("state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let bytes: Vec<u8> = serde_json::from_value(state["producer"].clone()).unwrap();
    assert_eq!(&bytes[..8], b"HPSESS01");
    let producer: serde_json::Value = serde_json::from_slice(&bytes[80..]).unwrap();
    assert!(
        producer["device"]["typing"]["queue"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["Insert"]["text"] == text),
        "save must actually contain undelivered text"
    );
    drop(live); // Kill only this test's helper, without OFF or graceful exit.
    let mut live = LiveSession::spawn_resuming(&options, &profile, &cancel).unwrap();
    assert!(live.resumed().is_some());
    let mut next = 2;
    idle(&mut live, &mut next, &cancel);
    key(&mut live, 0x8b);
    key(&mut live, 0xb4);
    idle(&mut live, &mut next, &cancel);
    tap(&mut live, 185);
    idle(&mut live, &mut next, &cancel);
    tap(&mut live, 294);
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        match live.event(end, &cancel).unwrap() {
            Event::Clipboard {
                text: actual,
                generation,
            } => {
                assert_eq!(
                    actual, text,
                    "restored delivery must neither lose nor duplicate input"
                );
                assert!(generation > 0);
                std::fs::write(root.join("copied.txt"), actual).unwrap();
                break;
            }
            Event::Error { message } => panic!("{message}"),
            _ => {}
        }
    }
}
