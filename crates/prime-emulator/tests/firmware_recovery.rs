//! Optional proprietary-firmware crash test. Uses only a disposable profile;
//! no physical calculator access. Run with PRIME_TEST_FIRMWARE/INIT_DATA/OS_ROM.
use ppl_engine::session::{self, Command, Event, Lifecycle, Packet};
use std::{
    io::Write,
    path::Path,
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Session {
    child: Child,
    input: ChildStdin,
    events: mpsc::Receiver<Event>,
}
impl Session {
    fn start(profile: &Path) -> Self {
        let mut cmd = Process::new(env!("CARGO_BIN_EXE_prime-emulator"));
        cmd.args(["session", "--firmware"])
            .arg(std::env::var_os("PRIME_TEST_FIRMWARE").unwrap())
            .arg("--data-dir")
            .arg(profile)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(rom) = std::env::var_os("PRIME_TEST_OS_ROM") {
            cmd.arg("--os-rom").arg(rom);
        }
        let mut child = cmd.spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(packet) = session::read_packet(&mut output) {
                if let Packet::Event(e) = packet {
                    let _ = tx.send(e);
                }
            }
        });
        Self {
            child,
            input,
            events,
        }
    }
    fn send(&mut self, command: Command) {
        session::write_command(&mut self.input, &command).unwrap();
        self.input.flush().unwrap();
    }
    fn wait(&self, label: &str, want: impl Fn(&Event) -> bool) -> Event {
        let until = std::time::Instant::now() + Duration::from_secs(120);
        loop {
            let event = self
                .events
                .recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
                .unwrap_or_else(|e| panic!("waiting for {label}: {e}"));
            if let Event::Error { message } = &event {
                panic!("firmware error: {message}");
            }
            if let Event::Status { message } = &event {
                eprintln!("firmware status: {message}");
                assert!(
                    !message.starts_with("Recovery checkpoint failed"),
                    "{message}"
                );
            }
            if want(&event) {
                return event;
            }
        }
    }
    fn ready(&self) {
        self.wait("ready", |e| {
            matches!(
                e,
                Event::Lifecycle {
                    state: Lifecycle::Ready
                }
            )
        });
    }
    fn run(&mut self, request: u64, body: &str, expected: &str) {
        self.send(Command::Run {
            request,
            source: format!("EXPORT RECOVERTEST() BEGIN {body} END;"),
            entry: None,
            arguments: String::new(),
        });
        match self.wait(
            "result",
            |e| matches!(e, Event::Result {request:r,..} if *r == request),
        ) {
            Event::Result {
                success, message, ..
            } => assert!(success && message.contains(expected), "{message}"),
            _ => unreachable!(),
        }
    }
    fn kill(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn recovery_scenario(live_off: bool) {
    let (Some(_), Some(init)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        eprintln!("skipping: proprietary firmware test assets not configured");
        return;
    };
    let profile = tempfile::tempdir().unwrap();
    for drive in ["A", "C"] {
        copy(&Path::new(&init).join(drive), &profile.path().join(drive));
    }
    let mut first = Session::start(profile.path());
    first.ready();
    first.run(1, "A:=123; RETURN A;", "returned 123");
    // Test both a live OFF/ON generation and the clean-shutdown baseline.
    let mut first = if live_off {
        for code in [0x8B, 0x83] {
            first.send(Command::Key { code, down: true });
            first.send(Command::Key { code, down: false });
        }
        first.wait("live OFF checkpoint", |e| matches!(e, Event::Status {message} if message == "Calculator recovery checkpoint saved"));
        first.send(Command::Key {
            code: 0x83,
            down: true,
        });
        first.send(Command::Key {
            code: 0x83,
            down: false,
        });
        first.ready();
        first
    } else {
        first.send(Command::PowerDown);
        first.wait("clean OFF", |e| matches!(e, Event::Closed { .. }));
        first.child.wait().unwrap();
        assert!(!profile.path().join(".recovery/unclean").exists());
        let restarted = Session::start(profile.path());
        restarted.ready();
        restarted
    };
    first.run(2, "A:=456; RETURN A;", "returned 456");
    first.kill();
    assert!(profile.path().join(".recovery/unclean").exists());
    let mut restarted = Session::start(profile.path());
    restarted.wait("recovery notification", |e| matches!(e, Event::Status {message} if message.starts_with("Recovered calculator data")));
    restarted.ready();
    restarted.run(3, "RETURN A;", "returned 123");
    assert!(
        std::fs::read_dir(profile.path().join(".recovery"))
            .unwrap()
            .any(|e| e.unwrap().path().join("archive/C").is_dir())
    );
    restarted.send(Command::PowerDown);
    restarted.wait("closed", |e| matches!(e, Event::Closed { .. }));
    restarted.child.wait().unwrap();
    assert!(!profile.path().join(".recovery/unclean").exists());
}

#[test]
fn killed_session_restores_last_off_and_archives_interrupted_storage() {
    recovery_scenario(false);
}
#[test]
fn live_off_checkpoint_recovers_after_wake_edit_and_kill() {
    recovery_scenario(true);
}
