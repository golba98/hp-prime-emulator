//! Fast original-firmware Check: one long-lived local firmware session that
//! calls the firmware's own compiler directly (`Command::DirectCheck`), with
//! no calculator UI driving per request.
//!
//! The session runs on a scratch copy of the initialized storage, never on a
//! user profile. Before use it must reproduce two stored original-firmware
//! results exactly (the canary); a request that reaches no verdict returns an
//! error, so callers fall back to the fresh checker.
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::session::{self, CheckVerdict, Event, Packet};
use crate::{Cancellation, firmware};

/// Corpus case `v01_hello` (accepted by the original firmware).
const CANARY_ACCEPTED: &str = "EXPORT DPHELLO()\nBEGIN\n RETURN 1;\nEND;\n";
/// Corpus case `e12_unclosed_paren`: rejected, "Error: Syntax Error" at
/// 3:13, UTF-16 offset 35 (tests/diagnostics/reference).
const CANARY_REJECTED: &str = "EXPORT DPPAREN()\nBEGIN\n RETURN (1+2;\nEND;\n";

struct Inner {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<Event>,
    next: u64,
}

pub struct DirectChecker {
    inner: Mutex<Inner>,
    _profile: tempfile::TempDir,
    /// Launch to the canary's end: the cold start.
    pub cold_start: Duration,
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = target.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

impl DirectChecker {
    /// Boots the firmware and runs the canary. Needs the runner, firmware
    /// and initialized data (`PPL_FIRMWARE_INIT_DATA`).
    pub fn start(o: &firmware::Options) -> Result<DirectChecker, String> {
        let started = Instant::now();
        let runner = firmware::discover_runner(o.runner.as_deref())?;
        let fw = o.firmware.as_ref().ok_or("firmware is not configured")?;
        let init = o
            .init_data
            .as_ref()
            .ok_or("the direct checker needs initialized data (PPL_FIRMWARE_INIT_DATA)")?;
        let profile = tempfile::Builder::new()
            .prefix("ppl-direct-check-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        copy_tree(init, profile.path())?;
        let log =
            std::fs::File::create(profile.path().join("session.log")).map_err(|e| e.to_string())?;
        let mut command = Command::new(runner);
        command
            .arg("session")
            .arg("--firmware")
            .arg(fw)
            .arg("--data-dir")
            .arg(profile.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        if let Some(rom) = &o.os_rom {
            command.arg("--os-rom").arg(rom);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let stdin = child.stdin.take().ok_or("no session stdin")?;
        let mut stdout = child.stdout.take().ok_or("no session stdout")?;
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            // Frames are drained and discarded; only events matter here.
            while let Ok(packet) = session::read_packet(&mut stdout) {
                if let Packet::Event(e) = packet {
                    if tx.send(e).is_err() {
                        break;
                    }
                }
            }
        });
        let mut checker = DirectChecker {
            inner: Mutex::new(Inner {
                child,
                stdin,
                events,
                next: 0,
            }),
            _profile: profile,
            cold_start: Duration::ZERO,
        };
        let never = Cancellation::default();
        let accepted = checker.check(CANARY_ACCEPTED, Duration::from_secs(240), &never)?;
        let rejected = checker.check(CANARY_REJECTED, Duration::from_secs(30), &never)?;
        let d = rejected.diagnostics.first();
        let canary_ok = accepted.accepted
            && accepted.diagnostics.is_empty()
            && !rejected.accepted
            && rejected.diagnostics.len() == 1
            && d.is_some_and(|d| {
                d.source == "guest-firmware"
                    && d.message.as_deref() == Some("Error: Syntax Error")
                    && (d.line, d.column) == (Some(3), Some(13))
                    && d.raw_position
                        .as_ref()
                        .and_then(|p| p["offset_utf16"].as_u64())
                        == Some(35)
            });
        if !canary_ok {
            return Err("the direct firmware Check did not reproduce the stored reference".into());
        }
        checker.cold_start = started.elapsed();
        Ok(checker)
    }

    /// Checks `source` with the original firmware compiler. `Err` means no
    /// verdict (never acceptance or rejection).
    pub fn check(
        &self,
        source: &str,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Result<CheckVerdict, String> {
        let mut inner = self.inner.lock().unwrap();
        inner.next += 1;
        let request = inner.next;
        session::write_command(
            &mut inner.stdin,
            &session::Command::DirectCheck {
                request,
                source: source.to_owned(),
            },
        )
        .map_err(|e| format!("direct checker: {e}"))?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.cancelled() {
                let _ = session::write_command(&mut inner.stdin, &session::Command::Stop);
                return Err("cancelled".into());
            }
            if Instant::now() > deadline {
                return Err("the direct firmware Check timed out".into());
            }
            match inner.events.recv_timeout(Duration::from_millis(20)) {
                Ok(Event::Hello { version }) if version != session::VERSION => {
                    return Err("incompatible local firmware helper; rebuild the workspace".into());
                }
                Ok(Event::Result {
                    request: r,
                    message,
                    check,
                    ..
                }) if r == request => {
                    let sha = crate::source::hash(source.as_bytes());
                    return match check {
                        Some(v) if v.source_sha256 == sha => Ok(v),
                        Some(_) => Err("verdict for a different source".into()),
                        None => Err(message),
                    };
                }
                Ok(Event::Closed { message }) => return Err(format!("session closed: {message}")),
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("the direct checker session ended".into());
                }
            }
        }
    }
}

impl Drop for DirectChecker {
    fn drop(&mut self) {
        let inner = self.inner.get_mut().unwrap();
        let _ = session::write_command(&mut inner.stdin, &session::Command::Shutdown);
        let _ = inner.stdin.flush();
        let deadline = Instant::now() + Duration::from_secs(3);
        while inner.child.try_wait().ok().flatten().is_none() {
            if Instant::now() > deadline {
                let _ = inner.child.kill();
                let _ = inner.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
