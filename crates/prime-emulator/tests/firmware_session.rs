//! Optional real-firmware test of the live session protocol (M12).
//!
//! Requires the proprietary HP firmware, which is never bundled:
//! `PRIME_TEST_FIRMWARE` (armfir.elf) and `PRIME_TEST_INIT_DATA` (initialized
//! storage with `A/` and `C/`, copied before use); optional
//! `PRIME_TEST_OS_ROM`. Skips otherwise. Local-firmware evidence only.

use ppl_engine::session::{self, Command, Event, Lifecycle, Packet};
use std::{
    io::Write,
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    events: mpsc::Receiver<Event>,
}
impl Session {
    fn send(&mut self, command: &Command) {
        session::write_command(&mut self.stdin, command).unwrap();
        self.stdin.flush().unwrap();
    }
    /// The first event `want` accepts, failing after `limit`.
    fn wait(&self, limit: Duration, what: &str, mut want: impl FnMut(&Event) -> bool) -> Event {
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) if want(&event) => return event,
                Ok(_) => {}
                Err(_) => panic!("no {what} within {limit:?}"),
            }
        }
    }
}

#[test]
fn live_session_reports_lifecycle_unsupported_services_and_imports() {
    let (Some(firmware), Some(init)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        eprintln!("skipping: PRIME_TEST_FIRMWARE / PRIME_TEST_INIT_DATA not set");
        return;
    };
    let profile = tempfile::tempdir().unwrap();
    for drive in ["A", "C"] {
        copy_tree(
            &std::path::Path::new(&init).join(drive),
            &profile.path().join(drive),
        );
    }
    // A profile from before the chunked loader: the session upgrades it.
    std::fs::create_dir_all(profile.path().join("C/DATA")).unwrap();
    std::fs::write(
        profile.path().join("C/DATA/ZZT_LOADER.hpprgm"),
        prime_emulator::hpprgm::build_program(prime_emulator::session::LOADER_V1)
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    let mut command = Process::new(env!("CARGO_BIN_EXE_prime-emulator"));
    command
        .arg("session")
        .arg("--firmware")
        .arg(&firmware)
        .arg("--data-dir")
        .arg(profile.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(rom) = std::env::var_os("PRIME_TEST_OS_ROM") {
        command.arg("--os-rom").arg(rom);
    }
    let mut child = command.spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, events) = mpsc::channel();
    let (frames_tx, frames) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(packet) = session::read_packet(&mut stdout) {
            match packet {
                Packet::Event(e) => {
                    let _ = tx.send(e);
                }
                Packet::Frame { pixels, .. } => {
                    let _ = frames_tx.send(pixels.len());
                }
            }
        }
    });
    let mut s = Session {
        child,
        stdin,
        events,
    };
    let minute = Duration::from_secs(60);
    match s.wait(minute, "hello", |e| matches!(e, Event::Hello { .. })) {
        Event::Hello { version } => assert_eq!(version, session::VERSION),
        _ => unreachable!(),
    }
    s.wait(minute, "booting", |e| {
        matches!(
            e,
            Event::Lifecycle {
                state: Lifecycle::Booting
            }
        )
    });
    // Unsupported services are named before Ready (the boot calls them).
    let mut unsupported = Vec::new();
    s.wait(3 * minute, "ready", |e| {
        if let Event::Unsupported { id, name, .. } = e {
            unsupported.push((*id, name.clone()));
        }
        matches!(
            e,
            Event::Lifecycle {
                state: Lifecycle::Ready
            }
        )
    });
    eprintln!("unsupported services during boot: {unsupported:?}");
    assert_eq!(frames.recv_timeout(minute).unwrap(), session::FRAME_PIXELS);

    let source = "EXPORT M12SESS()\nBEGIN\n RETURN 9;\nEND;\n";
    s.send(&Command::Import {
        request: 1,
        name: "M12SESS".into(),
        source: source.into(),
    });
    match s.wait(4 * minute, "import result", |e| {
        matches!(e, Event::Result { request: 1, .. })
    }) {
        Event::Result {
            success, message, ..
        } => assert!(success, "{message}"),
        _ => unreachable!(),
    }
    let stored = std::fs::read(profile.path().join("C/DATA/M12SESS.hpprgm")).unwrap();
    let stored = prime_emulator::hpprgm::parse(&stored).unwrap();
    assert_eq!(stored.source().unwrap().source, source);
    // An existing name is refused before anything is written.
    s.send(&Command::Import {
        request: 2,
        name: "m12sess".into(),
        source: "EXPORT X() BEGIN END;".into(),
    });
    match s.wait(minute, "refusal", |e| {
        matches!(e, Event::Result { request: 2, .. })
    }) {
        Event::Result {
            success, message, ..
        } => {
            assert!(!success);
            assert!(message.contains("already has a program"), "{message}");
        }
        _ => unreachable!(),
    }
    let again = std::fs::read(profile.path().join("C/DATA/M12SESS.hpprgm")).unwrap();
    assert_eq!(
        prime_emulator::hpprgm::parse(&again)
            .unwrap()
            .source()
            .unwrap()
            .source,
        source
    );

    let loader = std::fs::read(profile.path().join("C/DATA/ZZT_LOADER.hpprgm")).unwrap();
    assert_eq!(
        prime_emulator::hpprgm::parse(&loader)
            .unwrap()
            .source()
            .unwrap()
            .source,
        prime_emulator::session::LOADER
    );
    // A program whose UTF-16 form exceeds one firmware byte list (10 KB).
    // Plain-text source under the .hpprgm extension (as the M10 case uses it).
    let big = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/runtime/programs/FIREWORKS_G1.hpprgm"
    ))
    .unwrap()
    .replace("\r\n", "\n");
    assert!(big.len() > 8000);
    s.send(&Command::Import {
        request: 3,
        name: "M12BIG".into(),
        source: big.clone(),
    });
    match s.wait(6 * minute, "large import", |e| {
        matches!(e, Event::Result { request: 3, .. })
    }) {
        Event::Result {
            success, message, ..
        } => assert!(success, "{message}"),
        _ => unreachable!(),
    }
    let stored = std::fs::read(profile.path().join("C/DATA/M12BIG.hpprgm")).unwrap();
    assert_eq!(
        prime_emulator::hpprgm::parse(&stored)
            .unwrap()
            .source()
            .unwrap()
            .source,
        big
    );

    // The firmware's own OFF saves its state, then the session ends.
    let settings = profile.path().join("C/DATA/cas.hpsettings");
    assert!(!settings.exists(), "nothing saved before OFF");
    s.send(&Command::PowerDown);
    s.wait(minute, "sleeping", |e| {
        matches!(
            e,
            Event::Lifecycle {
                state: Lifecycle::Sleeping
            }
        )
    });
    match s.wait(minute, "power-down status", |e| {
        matches!(e, Event::Status { .. })
    }) {
        Event::Status { message } => {
            assert!(
                message.contains("saved its state and switched off"),
                "{message}"
            )
        }
        _ => unreachable!(),
    }
    s.wait(minute, "closed", |e| matches!(e, Event::Closed { .. }));
    assert!(
        settings.is_file(),
        "the firmware's OFF wrote its CAS settings"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "session did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
}
