//! Optional unknown-observer-profile integration; no physical device access.
use ppl_engine::session::{self, Command, Event, Packet};
use std::{
    io::Write,
    path::Path,
    process::{Command as Process, Stdio},
    sync::mpsc,
    time::Duration,
};
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}
#[test]
fn unobserved_firmware_boots_but_cannot_claim_a_managed_check() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_CALCULATOR_FIRMWARE"),
        std::env::var_os("PRIME_TEST_CALCULATOR_INIT"),
    ) else {
        eprintln!("skipping: calculator-only firmware assets not configured");
        return;
    };
    let profile = tempfile::tempdir().unwrap();
    copy(Path::new(&initial), profile.path());
    let mut command = Process::new(env!("CARGO_BIN_EXE_prime-emulator"));
    command
        .args(["session", "--firmware"])
        .arg(firmware)
        .arg("--data-dir")
        .arg(profile.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(rom) = std::env::var_os("PRIME_TEST_CALCULATOR_ROM") {
        command.arg("--os-rom").arg(rom);
    }
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(p) = session::read_packet(&mut stdout) {
            if let Packet::Event(e) = p {
                let _ = tx.send(e);
            }
        }
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        let mut capability = false;
        loop {
            match rx
                .recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
                .unwrap()
            {
                Event::Capabilities {
                    managed_programs,
                    reason,
                } => {
                    assert!(!managed_programs);
                    assert!(reason.is_some());
                    capability = true;
                }
                Event::Lifecycle {
                    state: session::Lifecycle::Ready,
                } => break,
                Event::Closed { message } => panic!("premature close: {message}"),
                _ => {}
            }
        }
        assert!(capability, "capabilities must arrive before Ready");
        session::write_command(
            &mut stdin,
            &Command::Compile {
                request: 42,
                source: "EXPORT TEST() BEGIN RETURN 42; END;".into(),
            },
        )
        .unwrap();
        stdin.flush().unwrap();
        loop {
            if let Event::Result {
                request: 42,
                success,
                check,
                message,
            } = rx.recv_timeout(Duration::from_secs(10)).unwrap()
            {
                assert!(!success);
                assert!(check.is_none());
                assert!(message.contains("no verified"));
                break;
            }
        }
    }));
    let _ = child.kill();
    let _ = child.wait();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
