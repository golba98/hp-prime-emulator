//! Connection to a physical calculator.
//!
//! Two parts: a presence light that only enumerates USB (it never opens the
//! device, so it cannot disturb the CLI, the mirror or Connectivity Kit), and
//! a single job slot for every operation that does open it, because USB
//! access is exclusive. Jobs open the device, do one thing, and release it.
//! Writes go through `ppl_programs::service`, the same guarded path the CLI
//! uses; this module adds no write of its own.

use ppl_device::control::ManagementAction as Action;
use ppl_device::{DeviceError, FileType, Prime, Probe};
use ppl_programs::{DeviceIdentity, UploadResult, delete_store::Store, replacement::Plan};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// How often the presence light re-enumerates USB.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Screenshot format 0x08: full 320×240 16-bit.
pub const SCREENSHOT_FORMAT: u8 = 8;

/// USB presence, from enumeration only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DeviceStatus {
    /// No enumeration has completed yet.
    #[default]
    Unknown,
    NotConnected,
    Connected {
        model: &'static str,
        count: usize,
    },
    /// USB enumeration itself failed (e.g. no HID access).
    DetectFailed(String),
}

impl DeviceStatus {
    pub fn label(&self) -> String {
        match self {
            DeviceStatus::Unknown => "Calculator: checking…".into(),
            DeviceStatus::NotConnected => "Calculator: not connected".into(),
            DeviceStatus::Connected { count: 1, .. } => "Calculator ready".into(),
            DeviceStatus::Connected { count, .. } => format!("{count} calculators connected"),
            DeviceStatus::DetectFailed(_) => "Calculator: USB unavailable".into(),
        }
    }

    pub fn connected(&self) -> bool {
        matches!(self, DeviceStatus::Connected { .. })
    }
}

/// Who asked for a job, so the result reaches the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Identify,
    Send,
    ListPrograms,
    ReadProgram,
    Screenshot,
}

impl Purpose {
    pub fn busy_label(self) -> &'static str {
        match self {
            Purpose::Identify => "reading calculator status…",
            Purpose::Send => "sending…",
            Purpose::ListPrograms => "listing programs…",
            Purpose::ReadProgram => "reading program…",
            Purpose::Screenshot => "capturing screen…",
        }
    }
}

/// What a finished job reports.
#[derive(Debug)]
pub enum JobOutput {
    Identity(DeviceIdentity),
    /// A new program was created (or the attempt's verified outcome).
    Created(UploadResult),
    /// The name exists; a backed-up replacement is prepared, nothing written.
    ReplacementPrepared(Box<Plan>),
    /// Result of an authorized replacement.
    Replaced(serde_json::Value),
    Inventory {
        /// (name, payload bytes), sorted by name.
        programs: Vec<(String, usize)>,
        note: String,
    },
    Program {
        name: String,
        source: String,
    },
    Screenshot(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    pub message: String,
    pub hint: Option<&'static str>,
}

impl From<DeviceError> for JobError {
    fn from(error: DeviceError) -> Self {
        let hint = ppl_programs::user_hint(&error);
        let message = match &error {
            DeviceError::Refused(m) if m.starts_with("multiple calculators") => {
                "More than one calculator is connected; unplug the others and retry.".into()
            }
            _ => error.to_string(),
        };
        Self { message, hint }
    }
}

pub type JobResult = Result<JobOutput, JobError>;

type Job = Box<dyn FnOnce(&mut Prime) -> JobResult + Send>;

/// The presence light and the job slot.
#[derive(Default)]
pub struct DeviceLink {
    pub status: DeviceStatus,
    status_rx: Option<Receiver<DeviceStatus>>,
    job: Option<(Purpose, Receiver<JobResult>)>,
}

impl DeviceLink {
    /// Starts the presence light. It stops by itself once the link is dropped.
    pub fn start_polling(&mut self, ctx: &egui::Context) {
        if self.status_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut last = None;
            loop {
                let status = match ppl_device::detect() {
                    Ok(found) if found.is_empty() => DeviceStatus::NotConnected,
                    Ok(found) => DeviceStatus::Connected {
                        model: found[0].model,
                        count: found.len(),
                    },
                    Err(e) => DeviceStatus::DetectFailed(e.to_string()),
                };
                if last.as_ref() != Some(&status) {
                    if tx.send(status.clone()).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                    last = Some(status);
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
        self.status_rx = Some(rx);
    }

    /// The running job, if any.
    pub fn busy(&self) -> Option<Purpose> {
        self.job.as_ref().map(|(p, _)| *p)
    }

    /// Runs `job` on a worker with a freshly opened device. Returns false
    /// (and starts nothing) while another job holds the slot.
    pub fn start(
        &mut self,
        purpose: Purpose,
        ctx: &egui::Context,
        job: impl FnOnce(&mut Prime) -> JobResult + Send + 'static,
    ) -> bool {
        if self.job.is_some() {
            return false;
        }
        let job: Job = Box::new(job);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = open().and_then(|mut prime| job(&mut prime));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.job = Some((purpose, rx));
        true
    }

    /// Updates the presence light and returns a finished job, if any.
    pub fn poll(&mut self) -> Option<(Purpose, JobResult)> {
        if let Some(rx) = &self.status_rx {
            while let Ok(status) = rx.try_recv() {
                self.status = status;
            }
        }
        let (purpose, rx) = self.job.as_ref()?;
        let purpose = *purpose;
        match rx.try_recv() {
            Ok(result) => {
                self.job = None;
                Some((purpose, result))
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.job = None;
                Some((
                    purpose,
                    Err(JobError {
                        message: "the device worker stopped unexpectedly".into(),
                        hint: None,
                    }),
                ))
            }
        }
    }

    /// Test hook: a job slot that reports `result` on the next poll.
    #[cfg(test)]
    pub fn inject(&mut self, purpose: Purpose, result: JobResult) {
        let (tx, rx) = mpsc::channel();
        tx.send(result).unwrap();
        self.job = Some((purpose, rx));
    }
}

/// Opens the one connected calculator.
#[cfg(not(test))]
fn open() -> Result<Prime, JobError> {
    Ok(ppl_programs::open_selected(None)?)
}

/// Unit tests never open a physical calculator, whatever is plugged in.
#[cfg(test)]
fn open() -> Result<Prime, JobError> {
    Err(DeviceError::Refused("unit tests never open a physical calculator".into()).into())
}

pub fn identify(prime: &mut Prime) -> JobResult {
    Ok(JobOutput::Identity(DeviceIdentity::read(prime)?))
}

/// Sends `source` as program `name`. A read-only probe decides the path:
/// no reply → guarded create (the preflight still refuses an existing name);
/// present → a backed-up replacement is *prepared* and returned for the user
/// to authorize. Nothing is deleted here.
pub fn send(prime: &mut Prime, name: &str, source: &str, sha256: &str, store: &Store) -> JobResult {
    let identity = DeviceIdentity::read(prime)?;
    match prime.probe_file(FileType::Program, name, ppl_device::PROBE_TIMEOUT)? {
        Probe::Present(found) if found.ty == FileType::Program && found.name == name => {
            let action = Action::PrepareReplacement {
                name: name.into(),
                source: source.into(),
            };
            let value = ppl_programs::service::handle(prime, &identity.serial_id, &action, store)?;
            let plan = serde_json::from_value(value).map_err(ppl_programs::disk)?;
            Ok(JobOutput::ReplacementPrepared(Box::new(plan)))
        }
        Probe::Present(found) if found.name.eq_ignore_ascii_case(name) => Err(JobError {
            message: format!(
                "The calculator has a program named `{}`, which differs only in case. \
                 Use exactly that name to replace it, or pick another name.",
                found.name
            ),
            hint: None,
        }),
        Probe::Present(found) => Err(DeviceError::Inconclusive(format!(
            "probe for `{name}` was answered with unrelated object `{}`",
            found.name
        ))
        .into()),
        Probe::NoReply { .. } => {
            check_source(name, source)?;
            let action = Action::Create {
                name: name.into(),
                source: source.into(),
                confirmation_sha256: sha256.into(),
            };
            let value = ppl_programs::service::handle(prime, &identity.serial_id, &action, store)?;
            let result = serde_json::from_value(value).map_err(ppl_programs::disk)?;
            Ok(JobOutput::Created(result))
        }
    }
}

/// The same source check a replacement runs before it may delete anything,
/// so both send paths accept exactly the same programs.
fn check_source(name: &str, source: &str) -> Result<(), JobError> {
    let source = ppl_engine::source::Source::text(source, name).map_err(ppl_programs::refused)?;
    let checks = ppl_engine::checking::check(
        source,
        ppl_engine::checking::configured_firmware().as_ref(),
        &Default::default(),
    );
    if checks.accepted() {
        Ok(())
    } else {
        Err(JobError {
            message: "The program did not pass the source checks, so nothing was sent.".into(),
            hint: Some("Fix the problems listed in the Problems panel, then send again."),
        })
    }
}

/// Applies a replacement the user authorized for exactly `new_sha256`.
pub fn apply_replacement(
    prime: &mut Prime,
    operation_id: &str,
    new_sha256: &str,
    store: &Store,
) -> JobResult {
    let identity = DeviceIdentity::read(prime)?;
    let action = Action::ApplyReplacement {
        operation_id: operation_id.into(),
        confirmation_new_sha256: new_sha256.into(),
    };
    let value = ppl_programs::service::handle(prime, &identity.serial_id, &action, store)?;
    Ok(JobOutput::Replaced(value))
}

/// Read-only program list. The F9 stream may be incomplete on this firmware.
pub fn list_programs(prime: &mut Prime) -> JobResult {
    let inventory = prime.receive_inventory()?;
    let mut programs: Vec<(String, usize)> = inventory
        .records
        .iter()
        .filter(|f| f.ty == FileType::Program)
        .map(|f| (f.name.clone(), f.data.len()))
        .collect();
    programs.sort_by_key(|p| p.0.to_lowercase());
    Ok(JobOutput::Inventory {
        programs,
        note: inventory.reason,
    })
}

/// Read-only source of one program.
pub fn read_program(prime: &mut Prime, name: &str) -> JobResult {
    let file = prime.read_program_exact(name)?;
    let source = ppl_device::program::utf16(&file.data)?;
    Ok(JobOutput::Program {
        name: file.name,
        source,
    })
}

/// Read-only screenshot as PNG bytes.
pub fn screenshot(prime: &mut Prime) -> JobResult {
    Ok(JobOutput::Screenshot(prime.screenshot(SCREENSHOT_FORMAT)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_test_device_open_is_always_refused() {
        assert!(
            matches!(open(), Err(JobError { message, .. }) if message.contains("unit tests never open a physical calculator"))
        );
    }

    #[test]
    fn errors_carry_shared_hints() {
        let e = JobError::from(DeviceError::NotFound);
        assert!(e.hint.is_some_and(|h| h.contains("USB")));
        let e = JobError::from(DeviceError::Inconclusive("x".into()));
        assert!(e.hint.is_some_and(|h| h.contains("nothing was written")));
        let e = JobError::from(DeviceError::Refused(
            "multiple calculators match; pass --device <serial-id>".into(),
        ));
        assert!(e.message.contains("unplug the others"));
    }

    #[test]
    fn job_slot_is_exclusive_and_reports_once() {
        let mut link = DeviceLink::default();
        link.inject(
            Purpose::Identify,
            Err(JobError {
                message: "m".into(),
                hint: None,
            }),
        );
        assert_eq!(link.busy(), Some(Purpose::Identify));
        let ctx = egui::Context::default();
        assert!(!link.start(Purpose::Send, &ctx, |_| unreachable!()));
        assert!(matches!(link.poll(), Some((Purpose::Identify, Err(_)))));
        assert!(link.busy().is_none());
        assert!(link.poll().is_none());
    }

    #[test]
    fn status_labels() {
        assert_eq!(
            DeviceStatus::NotConnected.label(),
            "Calculator: not connected"
        );
        let one = DeviceStatus::Connected {
            model: "HP Prime G1",
            count: 1,
        };
        assert!(one.connected());
        assert_eq!(one.label(), "Calculator ready");
    }
}
