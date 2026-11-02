//! Optional proprietary-firmware crash test. Uses only a disposable profile;
//! no physical calculator access. Run with PRIME_TEST_FIRMWARE/INIT_DATA/OS_ROM.
use ppl_engine::checkpoint::CheckpointResult;
use ppl_engine::session::{self, Command, Event, Lifecycle, Packet};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct Session {
    child: Child,
    input: ChildStdin,
    events: mpsc::Receiver<Event>,
    profile: PathBuf,
    last_frame: Arc<Mutex<Option<prime_emulator::display::Frame>>>,
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
        let last_frame = Arc::new(Mutex::new(None));
        let frame_out = last_frame.clone();
        std::thread::spawn(move || {
            while let Ok(packet) = session::read_packet(&mut output) {
                match packet {
                    Packet::Event(e) => {
                        let _ = tx.send(e);
                    }
                    Packet::Frame { pixels, .. } => {
                        *frame_out.lock().unwrap() = Some(prime_emulator::display::Frame {
                            width: 320,
                            height: 240,
                            pixels,
                        });
                    }
                }
            }
        });
        Self {
            child,
            input,
            events,
            profile: profile.to_owned(),
            last_frame,
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
    fn checkpoint(&mut self, id: u64, wake: bool) -> ppl_engine::checkpoint::CheckpointReceipt {
        self.send(Command::Checkpoint { id, wake });
        match self.wait(
            "explicit checkpoint receipt",
            |e| matches!(e, Event::Checkpoint {id:r,..} if *r == id),
        ) {
            Event::Checkpoint {
                result: CheckpointResult::Saved { receipt },
                ..
            } => receipt,
            other => {
                self.capture_checkpoint_failure(id, &other);
                panic!("checkpoint did not save: {other:?}");
            }
        }
    }
    fn capture_checkpoint_failure(&mut self, id: u64, event: &Event) {
        if std::env::var_os("PRIME_TEST_EVIDENCE").is_none() {
            return;
        }
        let _ = std::fs::write(
            self.profile.join(format!("failed-checkpoint-{id}.json")),
            serde_json::to_vec_pretty(event).unwrap(),
        );
        let request = u64::MAX - id;
        if session::write_command(&mut self.input, &Command::Inspect { request })
            .and_then(|_| self.input.flush())
            .is_ok()
        {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while let Ok(event) = self
                .events
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                if let Event::Inspection {
                    request: seen,
                    report,
                } = &event
                    && *seen == request
                {
                    let _ = std::fs::write(
                        self.profile.join(format!("failed-inspection-{id}.json")),
                        serde_json::to_vec_pretty(report).unwrap(),
                    );
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
            }
        }
        if let Ok(frame) = self.last_frame.try_lock()
            && let Some(frame) = frame.as_ref()
        {
            let _ = frame.save_png(&self.profile.join(format!("failed-screen-{id}.png")));
        }
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

#[test]
fn explicit_checkpoint_restores_collections_after_cas_in_fresh_processes() {
    collection_recovery_scenario(true);
}

#[test]
fn normal_off_restores_collections_after_cas_in_fresh_processes() {
    collection_recovery_scenario(false);
}

fn collection_recovery_scenario(explicit_second_save: bool) {
    let (Some(_), Some(init)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        eprintln!("skipping: proprietary firmware test assets not configured");
        return;
    };
    struct Profile {
        dir: tempfile::TempDir,
        preserve_failure: bool,
    }
    impl Profile {
        fn path(&self) -> &Path {
            self.dir.path()
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            if self.preserve_failure && std::thread::panicking() {
                self.dir.disable_cleanup(true);
                eprintln!(
                    "preserved failed firmware profile: {}",
                    self.dir.path().display()
                );
            }
        }
    }
    let profile = if let Some(base) = std::env::var_os("PRIME_TEST_EVIDENCE") {
        std::fs::create_dir_all(&base).unwrap();
        Profile {
            dir: tempfile::Builder::new()
                .prefix("post-cas-")
                .tempdir_in(base)
                .unwrap(),
            preserve_failure: true,
        }
    } else {
        Profile {
            dir: tempfile::tempdir().unwrap(),
            preserve_failure: false,
        }
    };
    for drive in ["A", "C"] {
        copy(&Path::new(&init).join(drive), &profile.path().join(drive));
    }
    let mut first = Session::start(profile.path());
    first.ready();
    first.run(
        10,
        "A:=CAS(\"2+3\"); L1:={1,2,3}; M1:=[[1,2],[3,4]]; RETURN A+L1(2)+M1(2,1);",
        "returned 10",
    );
    let receipt = first.checkpoint(11, false);
    assert!(!receipt.restores_cpu && !receipt.restores_framebuffer);
    let generation = profile.path().join(".recovery").join(&receipt.generation);
    assert_eq!(
        prime_emulator::sha256::hex(&std::fs::read(generation.join("manifest.json")).unwrap()),
        receipt.manifest_sha256
    );
    assert!(profile.path().join(".recovery/unclean").exists());
    first.kill();

    let mut second = Session::start(profile.path());
    second.wait("fresh-process recovery", |e| matches!(e, Event::Status {message} if message.starts_with("Recovered calculator data")));
    second.ready();
    second.run(12, "RETURN A+L1(2)+M1(2,1);", "returned 10");
    second.run(13, "A:=900; RETURN A;", "returned 900");
    let mut second = if explicit_second_save {
        let second_receipt = second.checkpoint(14, true);
        assert_ne!(receipt.generation, second_receipt.generation);
        second.ready();
        second
    } else {
        second.send(Command::PowerDown);
        second.wait("baseline normal OFF", |e| matches!(e, Event::Closed { .. }));
        second.child.wait().unwrap();
        let mut rebooted = Session::start(profile.path());
        rebooted.ready();
        rebooted.run(14, "RETURN A+L1(2)+M1(2,1);", "returned 905");
        rebooted
    };
    second.run(15, "A:=901; RETURN A;", "returned 901");
    second.kill();

    let mut third = Session::start(profile.path());
    third.wait("second fresh-process recovery", |e| matches!(e, Event::Status {message} if message.starts_with("Recovered calculator data")));
    third.ready();
    third.run(16, "RETURN A+L1(2)+M1(2,1);", "returned 905");
    third.send(Command::PowerDown);
    third.wait("normal OFF remains authoritative", |e| {
        matches!(e, Event::Closed { .. })
    });
    third.child.wait().unwrap();
    assert!(!profile.path().join(".recovery/unclean").exists());
}

#[test]
fn bootstrap_error_is_delivered_before_process_exits() {
    // No proprietary assets required. This short path used to exit while
    // its detached writer still had the terminal event queued.
    for _ in 0..8 {
        let profile = tempfile::tempdir().unwrap();
        let mut child = Process::new(env!("CARGO_BIN_EXE_prime-emulator"))
            .args(["session", "--firmware"])
            .arg(profile.path().join("missing.elf"))
            .arg("--data-dir")
            .arg(profile.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut output = child.stdout.take().unwrap();
        let mut closed = false;
        while let Ok(packet) = session::read_packet(&mut output) {
            if matches!(packet, Packet::Event(Event::Closed { .. })) {
                closed = true;
            }
        }
        assert!(!child.wait().unwrap().success());
        assert!(closed, "terminal bootstrap error was lost at process exit");
    }
}
