//! Orchestrated execution of an uploaded program on a physical Prime.
//!
//! `EC` remote key submissions produced no observed transition on the
//! reference G1; input delivery remains unverified (firmware 2.2.15157; see
//! `docs/reverse-engineering/usb-protocol.md` §3.7), so the only supported
//! execution method today is [`ExecutionMethod::ManualRun`]: the controller
//! automates everything except the single `Run` keypress on the calculator.
//! It performs a read-only preflight, polls for the program's result note,
//! retrieves and validates it, and reports the outcome. Nothing on the
//! calculator is written, deleted or pressed.
//!
//! Polling is built on [`Prime::probe_file`], so every silent poll naturally
//! waits one `poll` interval on both real and mock transports, and the
//! deadline is expressed as a poll count rather than wall-clock time. This
//! keeps the controller deterministic under the scripted mock clock.

use std::time::Duration;

use crate::guard::PROBE_TIMEOUT;
use crate::{ControlObject, DeviceError, FileType, Prime, Probe};

/// How the program is started on the calculator.
///
/// A remote-key variant is reserved for a future firmware or protocol
/// revision where `EC` is proven effective; it is deliberately not
/// constructible from the CLI until that evidence exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionMethod {
    /// The user presses `Run` once in the Program catalog; the controller
    /// handles preflight, completion detection and capture.
    ManualRun,
}

/// Everything the controller needs to run one uploaded test program.
#[derive(Clone, Debug)]
pub struct RunConfig {
    /// Program name on the calculator (must already exist).
    pub program: String,
    /// Result note the program writes when it completes.
    pub note: String,
    /// Known-present control object for the preflight.
    pub control: ControlObject,
    /// Total time to wait for the note (rounded down to whole polls).
    pub timeout: Duration,
    /// Wait per silent poll.
    pub poll: Duration,
    /// Extra wait after the note first appears, before retrieval, in case
    /// the program is still appending to it.
    pub settle: Duration,
    /// The result note must start with this text (e.g. `@@ZZT|NAME|`);
    /// `None` disables the identity check.
    pub expect_header: Option<String>,
}

impl RunConfig {
    pub fn new(program: &str, control: ControlObject) -> RunConfig {
        RunConfig {
            program: program.into(),
            note: crate::result_note_name(program),
            control,
            timeout: Duration::from_secs(600),
            poll: Duration::from_secs(2),
            settle: Duration::from_secs(2),
            expect_header: Some(format!("@@ZZT|{program}|")),
        }
    }

    fn max_polls(&self) -> u32 {
        let t = self.timeout.as_millis();
        let p = self.poll.as_millis();
        (t / p.max(1)).max(1) as u32
    }
}

/// Progress events for callers that want to print status.
#[derive(Clone, Debug)]
pub enum RunEvent {
    /// Preflight passed; polling for the note may start.
    PreflightDone,
    /// One silent poll completed.
    Poll { attempt: u32, of: u32 },
    /// The note appeared; waiting out the settle interval.
    NoteSeen,
    /// The note content was retrieved.
    Retrieved,
}

/// Terminal outcome of [`run`].
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    /// The note appeared and was retrieved and (when requested) validated.
    Captured {
        /// Normalized note text.
        text: String,
        polls: u32,
    },
    /// The program did not produce its note within the timeout.
    TimedOut { note: String, polls: u32 },
    /// The note appeared but does not carry the expected header.
    HeaderMismatch { note: String, expected: String },
}

/// Why a run could not start. No traffic past the failing check.
#[derive(Debug)]
pub enum RunPreflight {
    /// The program is not on the calculator.
    MissingProgram(String),
    /// A result note already exists (stale evidence from an earlier run):
    /// the run is refused rather than mixing evidence.
    StaleNote(String),
    /// The control object stopped answering.
    ControlLost(String),
    /// Transport or protocol failure.
    Device(DeviceError),
}

impl std::fmt::Display for RunPreflight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunPreflight::MissingProgram(n) => {
                write!(f, "program `{n}` is not on the calculator")
            }
            RunPreflight::StaleNote(n) => {
                write!(f, "result note `{n}` already exists (stale evidence)")
            }
            RunPreflight::ControlLost(n) => {
                write!(f, "control `{n}` stopped answering")
            }
            RunPreflight::Device(e) => write!(f, "{e}"),
        }
    }
}

/// Read-only preflight: the control answers, the program is present and the
/// result note is silent. Like the write preflight, note silence is checked
/// twice with the control re-answered in between, so silence cannot be
/// mistaken for a dead device.
pub fn preflight(p: &mut Prime, cfg: &RunConfig) -> Result<(), RunPreflight> {
    if !probe_present(p, cfg.control.ty, &cfg.control.name)? {
        return Err(RunPreflight::ControlLost(cfg.control.name.clone()));
    }
    if !probe_present(p, FileType::Program, &cfg.program)? {
        return Err(RunPreflight::MissingProgram(cfg.program.clone()));
    }
    for round in 1..=2 {
        if probe_present(p, FileType::Note, &cfg.note)? {
            return Err(RunPreflight::StaleNote(cfg.note.clone()));
        }
        if round == 1 && !probe_present(p, cfg.control.ty, &cfg.control.name)? {
            return Err(RunPreflight::ControlLost(cfg.control.name.clone()));
        }
    }
    Ok(())
}

fn probe_present(p: &mut Prime, ty: FileType, name: &str) -> Result<bool, RunPreflight> {
    match p.probe_file(ty, name, PROBE_TIMEOUT) {
        Ok(Probe::Present(_)) => Ok(true),
        Ok(Probe::NoReply { .. }) => Ok(false),
        Err(e) => Err(RunPreflight::Device(e)),
    }
}

/// Runs one uploaded program end to end (except the manual `Run` press).
///
/// The caller prints its own "press Run now" instruction when it receives
/// [`RunEvent::PreflightDone`]; the note is polled from then on.
pub fn run<F>(p: &mut Prime, cfg: &RunConfig, mut on_event: F) -> Result<RunOutcome, RunPreflight>
where
    F: FnMut(RunEvent),
{
    preflight(p, cfg)?;
    on_event(RunEvent::PreflightDone);

    let max = cfg.max_polls();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match p.probe_file(FileType::Note, &cfg.note, cfg.poll) {
            Ok(Probe::Present(_)) => {
                on_event(RunEvent::NoteSeen);
                // Give the program time to finish appending before reading.
                if let Err(e) = p.probe_file(FileType::Note, &cfg.note, cfg.settle) {
                    return Err(RunPreflight::Device(e));
                }
                let text = match p.probe_file(FileType::Note, &cfg.note, PROBE_TIMEOUT) {
                    Ok(Probe::Present(f)) => crate::normalize_program_text(&f.text()),
                    Ok(Probe::NoReply { .. }) => {
                        return Err(RunPreflight::Device(DeviceError::Protocol(format!(
                            "result note `{}` went silent after appearing",
                            cfg.note
                        ))));
                    }
                    Err(e) => return Err(RunPreflight::Device(e)),
                };
                on_event(RunEvent::Retrieved);
                if let Some(header) = &cfg.expect_header {
                    if !text.starts_with(header.as_str()) {
                        return Ok(RunOutcome::HeaderMismatch {
                            note: cfg.note.clone(),
                            expected: header.clone(),
                        });
                    }
                }
                return Ok(RunOutcome::Captured {
                    text,
                    polls: attempt,
                });
            }
            Ok(Probe::NoReply { .. }) => {}
            Err(e) => return Err(RunPreflight::Device(e)),
        }
        if attempt >= max {
            return Ok(RunOutcome::TimedOut {
                note: cfg.note.clone(),
                polls: attempt,
            });
        }
        on_event(RunEvent::Poll { attempt, of: max });
    }
}
