//! Communication with a physical HP Prime over USB HID.
//!
//! Protocol source: debrouxl/hplp (`libhpcalcs/src/prime_*.c`), the
//! "old"/v1 framing used by G1 calculators with 64-byte reports:
//!
//! * host → calc report: `[0x00 report id][seq][≤63 payload bytes]`; the
//!   last report of a message is short; `seq` counts from 0 and skips 0xFF;
//! * calc → host report: `[seq][63 payload bytes]`; reports starting with
//!   0xFF are out-of-band and skipped;
//! * message: `[cmd]` alone, or `[cmd][0x01][len u32 BE][body]`.
//!
//! **Safety.** Only these commands are implemented:
//!
//! | cmd | name | effect on calculator |
//! |---|---|---|
//! | `FF` | check ready | none |
//! | `FA` | get info | none |
//! | `FC` | screenshot | none |
//! | `F9` | receive backup inventory | reads objects; completeness remains unverified |
//! | `F8` | request file | none (calculator replies with an `F7` file) |
//! | `F7` | send file | creates guarded objects; owned-note updates require [`ownership::OwnedNote`] |
//! | `EB` | delete object | removes exactly one named object (see [`delete`]); F7-shaped message
//! | `EC` | key press | explicit, session-authorized UI control only; execution never uses it |
//!
//! There is no firmware, backup-restore, date/time or memory-write
//! code in this crate.
//!
//! **Absence is never inferred from silence.** A G1 does not answer an `F8`
//! for an object that does not exist, but it also does not answer when it is
//! busy, locked, in another screen, or when the reply is merely late. A
//! new-object creation is therefore only possible through a [`WriteGuard`], which
//! [`Prime::preflight_new_names`] issues after it has shown, twice, that a
//! known-present *control* object answers while every target stays silent.
//! Late replies are drained (see [`Prime::drain`]) before every command and
//! after every timeout so they cannot be mistaken for the next reply.
//! Updating a previously created mailbox note uses a separate connection-bound
//! ownership capability; it cannot authorize program or system-object replacement.

use std::fmt;
use std::time::{Duration, Instant};

use hidapi::HidApi;

use guard::fnv64;

pub mod delete;
/// Offline official extended framing research; never selected by live USB.
pub mod extended;
pub mod files;

#[cfg(test)]
mod mock_tests;

pub use delete::{
    Backup, Completion, DeleteEvent, DeleteGuard, DeleteOptions, DeleteReport, DeleteTarget,
    Deletion, DirectDeleteReport, MAX_TARGETS, Mechanism, NoteWatch, PROTECTED_OBJECTS,
    SETTINGS_ANCHOR_VERIFIED, TargetResult, TargetStatus, authorize_usb_deletion,
    await_deletion_note, delete_via_usb, generate_deletion_helper, prepare_deletion,
    prepare_deletion_with, run_deletion, upload_deletion_helper, verify_deleted,
};
pub use files::{FileType, PrimeFile};

pub mod control;
pub mod guard;
pub mod info;
pub mod inventory;
pub mod keys;
mod late;
pub mod listener;
mod lock;
pub mod ownership;
pub mod program;
pub mod protocol;
pub mod remote;
pub mod run;
pub mod screenshot;
pub mod session;
pub mod transport;

pub use guard::{
    ControlObject, DEFAULT_CONTROL, DEFAULT_SETTINGS_ANCHOR, GUARD_MAX_AGE, MAX_NAME_LEN,
    NamePolicy, PROBE_TIMEOUT, ReadBack, SAFE_PREFIX, SETTINGS_ANCHORS, UploadReport, WriteGuard,
    fresh_test_name, normalize_program_text, result_note_name, validate_object_name,
    validate_program_name, validate_test_name,
};
pub use info::{CalcInfo, Exchange, Probe, TraceEvent};
pub use keys::{
    KEYS, KeyDef, NAVIGATION_KEYS, describe_codes, key_by_name, key_code, key_name,
    parse_key_sequence,
};
pub use protocol::{crc16, utf16_runs};
pub use run::{
    ExecutionMethod, RunConfig, RunEvent, RunOutcome, RunPreflight, preflight as run_preflight,
    run as run_program,
};
pub use session::{DeviceSession, OP_KEYS, PressRecord, now_unix};
pub use transport::{Detected, KNOWN_PIDS, Transport, VID_HP, detect};

pub const CMD_CHECK_READY: u8 = 0xFF;
pub const CMD_GET_INFO: u8 = 0xFA;
pub const CMD_SCREENSHOT: u8 = 0xFC;
pub const CMD_REQ_FILE: u8 = 0xF8;
pub const CMD_FILE: u8 = 0xF7;
pub const CMD_KEY: u8 = 0xEC;
/// Delete one object on the calculator (Connectivity Kit reverse
/// engineering, see `delete.rs` for the evidence).
pub const CMD_DELETE_OBJECT: u8 = 0xEB;

/// The byte a ready G1 sends in reply to `FF` (observed on build 15157).
pub const READY_REPLY: u8 = 0x59;

/// Per-read timeout used by [`Prime::drain`].
const DRAIN_TIMEOUT_MS: i32 = 50;

/// Maximum program payload (UTF-16LE bytes) supported by legacy framing.
/// Hardware evidence: 15,926 B stored and read back exactly.
/// 254 reports * 63 bytes = 16,002 bytes minus file headers (~28 B).
/// Beyond 254 reports, legacy sequence wraps to 00, causing receiver assembly reset.
pub const MAX_LEGACY_PROGRAM_PAYLOAD: usize = 15_928;

/// Maximum program payload (UTF-16LE bytes) supported by extended framing (Mode 1).
/// The wire protocol length field is u32 LE; this is the host defensive allocation limit (16 MiB).
pub const MAX_EXTENDED_PROGRAM_PAYLOAD: usize = 16 * 1024 * 1024;

/// Legacy alias for MAX_LEGACY_PROGRAM_PAYLOAD.
pub const MAX_PROGRAM_PAYLOAD: usize = MAX_LEGACY_PROGRAM_PAYLOAD;

/// Wire transfer protocol variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransferProtocol {
    Legacy,
    Extended,
}

impl TransferProtocol {
    pub fn max_payload_bytes(self) -> usize {
        match self {
            TransferProtocol::Legacy => MAX_LEGACY_PROGRAM_PAYLOAD,
            TransferProtocol::Extended => MAX_EXTENDED_PROGRAM_PAYLOAD,
        }
    }
}

/// Transport protocol mode currently active on the USB HID session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransportMode {
    Legacy,
    Mode1,
}

/// Checks `source` as the UTF-16LE payload against the given protocol maximum.
pub fn check_program_payload_for_protocol(
    _name: &str,
    source: &str,
    protocol: TransferProtocol,
) -> Result<usize> {
    let bytes = source.encode_utf16().count() * 2;
    let max = protocol.max_payload_bytes();
    if bytes > max {
        let policy = match protocol {
            TransferProtocol::Legacy => "conservative G1 legacy upload policy",
            TransferProtocol::Extended => "extended transport maximum policy",
        };
        return Err(DeviceError::Refused(format!(
            "program payload is {bytes} bytes; {policy} allows at most {max} bytes (nothing was sent)."
        )));
    }
    Ok(bytes)
}

/// Checks `source` against the conservative legacy payload limit ([`MAX_LEGACY_PROGRAM_PAYLOAD`]).
pub fn check_program_payload_legacy(name: &str, source: &str) -> Result<usize> {
    check_program_payload_for_protocol(name, source, TransferProtocol::Legacy)
}

/// Size of `source` as the UTF-16LE payload of an `F7` program upload, or
/// [`DeviceError::Refused`] if it exceeds the conservative legacy limit
/// ([`MAX_LEGACY_PROGRAM_PAYLOAD`]).
pub fn check_program_payload(name: &str, source: &str) -> Result<usize> {
    check_program_payload_legacy(name, source)
}
/// How long the post-upload read-back keeps listening to its one `F8`.
pub const READBACK_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound on reports discarded by one [`Prime::drain`] call.
const DRAIN_MAX_REPORTS: usize = 4096;

#[derive(Debug)]
pub enum DeviceError {
    Hid(String),
    NotFound,
    Timeout(&'static str),
    Protocol(String),
    /// A write was refused by the safety guard.
    Refused(String),
    /// Device state could not be established. Callers must retain any
    /// already-submitted mutation as unresolved; this error does not undo it.
    Inconclusive(String),
    Busy(String),
    /// The object type has no verified read/backup/delete support.
    Unsupported(String),
    /// The calculator answered liveness checks but not a file request, twice:
    /// the object is not present.
    Absent(String),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Hid(e) => write!(f, "HID error: {e}"),
            DeviceError::NotFound => write!(f, "no HP Prime found on USB"),
            DeviceError::Timeout(what) => write!(f, "timed out waiting for {what}"),
            DeviceError::Protocol(e) => write!(f, "protocol error: {e}"),
            DeviceError::Refused(e) => write!(f, "refused: {e}"),
            DeviceError::Inconclusive(e) => {
                write!(f, "inconclusive: {e}")
            }
            DeviceError::Busy(e) => write!(f, "device busy: {e}"),
            DeviceError::Unsupported(e) => write!(f, "unsupported object type: {e}"),
            DeviceError::Absent(e) => write!(f, "not found: {e}"),
        }
    }
}

impl std::error::Error for DeviceError {}

impl From<hidapi::HidError> for DeviceError {
    fn from(e: hidapi::HidError) -> Self {
        DeviceError::Hid(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DeviceError>;
pub struct Prime {
    _lock: Option<std::fs::File>,
    owner: std::sync::Arc<()>,
    dev: Box<dyn Transport>,
    pub detected: Detected,
    report_size: usize,
    started: Instant,
    pub log: Vec<Exchange>,
    pub traces: Vec<TraceEvent>,
    pub trace_enabled: bool,
    pub read_timeout: Duration,
    pub mode: TransportMode,
    pub next_message_id: u32,
    retired: Option<String>,
}

impl Prime {
    /// A framing failure retires this connection. A new handle must establish
    /// fresh device identity; continuing on an ambiguous stream is unsafe.
    pub fn require_healthy(&self) -> Result<()> {
        match &self.retired {
            Some(reason) => Err(DeviceError::Inconclusive(format!(
                "connection retired: {reason}; close and reconnect"
            ))),
            None => Ok(()),
        }
    }

    pub fn open_first() -> Result<Prime> {
        let mut devices = detect()?;
        if devices.len() > 1 {
            return Err(DeviceError::Refused(
                "multiple HP Primes connected; select a device explicitly".into(),
            ));
        }
        let d = devices.pop().ok_or(DeviceError::NotFound)?;
        Prime::open(d)
    }

    pub fn open(d: Detected) -> Result<Prime> {
        let lock = lock::acquire(&d.path)?;
        let api = HidApi::new()?;
        let path =
            std::ffi::CString::new(d.path.clone()).map_err(|e| DeviceError::Hid(e.to_string()))?;
        let dev = api.open_path(&path)?;
        let mut prime = Prime::with_transport(d, Box::new(dev));
        prime._lock = Some(lock);
        Ok(prime)
    }

    /// Wraps an already-open transport (used by tests with a mock).
    pub fn with_transport(detected: Detected, t: Box<dyn Transport>) -> Prime {
        Prime {
            _lock: None,
            owner: std::sync::Arc::new(()),
            dev: t,
            report_size: detected.report_size(),
            detected,
            started: Instant::now(),
            log: Vec::new(),
            traces: Vec::new(),
            trace_enabled: false,
            read_timeout: Duration::from_secs(8),
            mode: TransportMode::Legacy,
            next_message_id: 1,
            retired: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_trace(
        &mut self,
        op_type: &str,
        endpoint: &str,
        requested_bytes: usize,
        actual_bytes: usize,
        status: &str,
        cmd_id: Option<u8>,
        resp_id: Option<u8>,
        resp_len: Option<usize>,
        timeout_stage: Option<&str>,
        elapsed: Duration,
        details: &str,
    ) {
        if !self.trace_enabled {
            return;
        }
        self.traces.push(TraceEvent {
            at: self.started.elapsed(),
            op_type: op_type.to_string(),
            endpoint: endpoint.to_string(),
            requested_bytes,
            actual_bytes,
            status: status.to_string(),
            cmd_id,
            resp_id,
            resp_len,
            timeout_stage: timeout_stage.map(|s| s.to_string()),
            elapsed,
            details: details.to_string(),
        });
    }

    fn write_report(&mut self, report: &[u8]) -> Result<()> {
        let t0 = Instant::now();
        self.log.push(Exchange {
            outgoing: true,
            bytes: report.to_vec(),
            at: self.started.elapsed(),
            drained: false,
        });
        let res = self.dev.write(report);
        let elapsed = t0.elapsed();
        match &res {
            Ok(n) if *n == report.len() => {
                // Only the first report of a message starts with the command
                // byte (`[00][seq=0][cmd..]`); the rest is payload.
                let cmd_id = (report.len() > 2 && report[1] == 0).then(|| report[2]);
                self.record_trace(
                    "write",
                    "HID OUT",
                    report.len(),
                    *n,
                    "ok",
                    cmd_id,
                    None,
                    None,
                    None,
                    elapsed,
                    "",
                );
            }
            Ok(n) => {
                self.record_trace(
                    "write",
                    "HID OUT",
                    report.len(),
                    *n,
                    "short",
                    None,
                    None,
                    None,
                    None,
                    elapsed,
                    "short write",
                );
            }
            Err(e) => {
                self.record_trace(
                    "write",
                    "HID OUT",
                    report.len(),
                    0,
                    "error",
                    None,
                    None,
                    None,
                    None,
                    elapsed,
                    &e.to_string(),
                );
            }
        }
        let n = res?;
        if n != report.len() {
            return Err(DeviceError::Hid(format!(
                "short HID write: {n}/{} bytes; not retried",
                report.len()
            )));
        }
        Ok(())
    }

    fn read_report(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; self.report_size];
        let t0 = Instant::now();
        let res = self
            .dev
            .read_timeout(&mut buf, timeout.as_millis().min(i32::MAX as u128) as i32);
        let elapsed = t0.elapsed();
        match &res {
            Ok(0) => {
                self.record_trace(
                    "read",
                    "HID IN",
                    self.report_size,
                    0,
                    "timeout",
                    None,
                    None,
                    None,
                    Some("read_report"),
                    elapsed,
                    "no report",
                );
                Ok(None)
            }
            Ok(n) => {
                buf.truncate(*n);
                let resp_id = if *n > 1 && buf[0] == 0 {
                    Some(buf[1])
                } else {
                    None
                };
                self.record_trace(
                    "read",
                    "HID IN",
                    self.report_size,
                    *n,
                    "ok",
                    None,
                    resp_id,
                    Some(*n),
                    None,
                    elapsed,
                    "",
                );
                self.log.push(Exchange {
                    outgoing: false,
                    bytes: buf.clone(),
                    at: self.started.elapsed(),
                    drained: false,
                });
                Ok(Some(buf))
            }
            Err(e) => {
                self.record_trace(
                    "read",
                    "HID IN",
                    self.report_size,
                    0,
                    "error",
                    None,
                    None,
                    None,
                    None,
                    elapsed,
                    &e.to_string(),
                );
                Err(res.err().unwrap())
            }
        }
    }

    /// Discards input that is already queued or arrives within ~50 ms, so a
    /// late reply to an earlier command cannot be taken as the reply to the
    /// next one. Drained reports are kept in [`Prime::log`] with
    /// `drained: true`. Returns the number of reports discarded. Called at
    /// the start of every command and after every timeout.
    pub fn drain(&mut self) -> usize {
        let mut n = 0;
        let deadline = Instant::now() + Duration::from_millis(250);
        while n < DRAIN_MAX_REPORTS && Instant::now() < deadline {
            let mut buf = vec![0u8; self.report_size];
            match self.dev.read_timeout(&mut buf, DRAIN_TIMEOUT_MS) {
                Ok(0) | Err(_) => break,
                Ok(k) => {
                    buf.truncate(k);
                    self.log.push(Exchange {
                        outgoing: false,
                        bytes: buf,
                        at: self.started.elapsed(),
                        drained: true,
                    });
                    n += 1;
                }
            }
        }
        n
    }

    /// Splits a message into reports (hplp `prime_send_data`).
    pub fn frame(message: &[u8], report_size: usize) -> Vec<Vec<u8>> {
        let chunk = report_size - 1;
        let mut out = Vec::new();
        let mut seq: u8 = 0;
        let full = message.len() / chunk;
        let rem = message.len() % chunk;
        for i in 0..full {
            let mut r = vec![0u8, seq];
            r.extend_from_slice(&message[i * chunk..(i + 1) * chunk]);
            out.push(r);
            seq = seq.wrapping_add(1);
            if seq == 0xFF {
                seq = 0;
            }
        }
        if rem > 0 || message.is_empty() {
            let mut r = vec![0u8, seq];
            r.extend_from_slice(&message[full * chunk..]);
            out.push(r);
        }
        out
    }

    /// Emits one logical message to the calculator using the active transport mode.
    fn send_message(&mut self, message: &[u8]) -> Result<()> {
        self.require_healthy()?;
        match self.mode {
            TransportMode::Legacy => {
                for r in Prime::frame(message, self.report_size) {
                    self.write_report(&r)?;
                }
                Ok(())
            }
            TransportMode::Mode1 => self.send_message_mode1(message),
        }
    }

    /// Reads one reply message, draining the line after any failure.
    fn recv_message(&mut self, cmd: u8, what: &'static str, first: Duration) -> Result<Vec<u8>> {
        let r = match self.mode {
            TransportMode::Legacy => self.recv_message_inner(cmd, what, first),
            TransportMode::Mode1 => self.recv_message_mode1(what, first),
        };
        if let Err(e) = &r {
            if !matches!(e, DeviceError::Timeout(_)) {
                self.retired = Some(e.to_string());
            }
            self.drain();
        }
        r
    }

    /// Reassembles a legacy-framed stream (counting reports `[want, payload]`),
    /// ignoring out-of-band `0xFF` reports and `0xFE` flow-control ACKs in Mode 1.
    pub(crate) fn recv_message_legacy_from(
        &mut self,
        cmd: Option<u8>,
        what: &'static str,
        first: Duration,
        first_rep: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut expected: Option<usize> = None;
        let mut count: u32 = 0;
        let mut next_rep = first_rep;
        let deadline = Instant::now() + first + Duration::from_secs(30);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DeviceError::Timeout(what));
            }
            let timeout = if count == 0 {
                first
            } else {
                Duration::from_secs(2)
            };
            let r = match next_rep.take() {
                Some(r) => r,
                None => match self.read_report(timeout.min(remaining))? {
                    Some(r) => r,
                    None => {
                        if count == 0 {
                            return Err(DeviceError::Timeout(what));
                        }
                        break;
                    }
                },
            };
            if r[0] == 0xFF || (count == 0 && r[0] != 0x00) {
                continue;
            }
            let want = ((count + count / 0xFF) & 0xFF) as u8;
            if r[0] != want {
                return Err(DeviceError::Protocol(format!(
                    "{what}: report out of sequence (got {:02X}, expected {want:02X})",
                    r[0]
                )));
            }
            count += 1;
            data.extend_from_slice(&r[1..]);
            if expected.is_none() {
                if let Some(c) = cmd {
                    if c != CMD_CHECK_READY && data.first() != Some(&c) {
                        return Err(DeviceError::Protocol(format!(
                            "{what}: unexpected reply command"
                        )));
                    }
                }
                expected = Some(match cmd {
                    Some(CMD_CHECK_READY) => 1,
                    _ if data.first() == Some(&crate::transport::READY_REPLY) => 1,
                    _ if data.len() >= 6 && data[1] == 0x01 => {
                        u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize + 6
                    }
                    _ => {
                        return Err(DeviceError::Protocol(format!(
                            "{what}: unexpected reply header {:02X?}",
                            &data[..data.len().min(8)]
                        )));
                    }
                });
                if expected.is_some_and(|size| size > 16 * 1024 * 1024) {
                    return Err(DeviceError::Protocol("reply exceeds 16 MiB limit".into()));
                }
            }
            if r.len() < self.report_size || data.len() >= expected.unwrap() {
                break;
            }
        }
        let exp = expected.unwrap_or(data.len());
        if data.len() < exp {
            return Err(DeviceError::Protocol(format!(
                "{what}: expected {exp} bytes, got {}",
                data.len()
            )));
        }
        data.truncate(exp);
        Ok(data)
    }

    /// hplp `prime_recv_data`.
    fn recv_message_inner(
        &mut self,
        cmd: u8,
        what: &'static str,
        first: Duration,
    ) -> Result<Vec<u8>> {
        self.recv_message_legacy_from(Some(cmd), what, first, None)
    }

    /// `FF`: returns the single reply byte(s).
    pub fn check_ready(&mut self) -> Result<Vec<u8>> {
        self.drain();
        let t0 = Instant::now();
        self.record_trace(
            "check_ready",
            "protocol",
            1,
            0,
            "start",
            Some(CMD_CHECK_READY),
            None,
            None,
            None,
            Duration::ZERO,
            "ping",
        );
        self.send_message(&[CMD_CHECK_READY])?;
        let r = self.recv_message(CMD_CHECK_READY, "check-ready reply", self.read_timeout)?;
        self.record_trace(
            "check_ready",
            "protocol",
            1,
            r.len(),
            "ok",
            Some(CMD_CHECK_READY),
            r.first().copied(),
            Some(r.len()),
            None,
            t0.elapsed(),
            &format!("reply: {:02X?}", r),
        );
        Ok(r)
    }

    /// `FA`: calculator name, firmware version and serial (read-only).
    pub fn info(&mut self) -> Result<CalcInfo> {
        self.drain();
        self.send_message(&[CMD_GET_INFO])?;
        let raw = self.recv_message(CMD_GET_INFO, "info reply", self.read_timeout)?;
        Ok(CalcInfo::parse(&raw))
    }

    /// `FC`: screenshot as PNG bytes. `format` 0x08 = 320×240 16-bit.
    pub fn screenshot(&mut self, format: u8) -> Result<Vec<u8>> {
        self.screenshot_timeout(format, Duration::from_secs(15))
    }

    /// `FC`: screenshot as PNG bytes with custom first-report timeout.
    pub fn screenshot_timeout(&mut self, format: u8, timeout: Duration) -> Result<Vec<u8>> {
        self.drain();
        self.send_message(&[CMD_SCREENSHOT, format])?;
        let raw = self.recv_message(CMD_SCREENSHOT, "screenshot", timeout)?;
        let png_at = raw
            .windows(4)
            .position(|w| w == [0x89, b'P', b'N', b'G'])
            .ok_or_else(|| DeviceError::Protocol("screenshot reply contains no PNG".into()))?;
        Ok(raw[png_at..].to_vec())
    }

    /// `F8`: asks for one object and waits up to `timeout` for the first
    /// reply report.
    ///
    /// * an `F7` file → [`Probe::Present`] (the file's own name and type are
    ///   returned unchanged; callers compare them with what they asked for);
    /// * nothing → [`Probe::NoReply`], which is **not** proof of absence;
    /// * any other reply → [`DeviceError::Protocol`].
    pub fn probe_file(&mut self, ty: FileType, name: &str, timeout: Duration) -> Result<Probe> {
        self.drain();
        let t0 = Instant::now();
        self.record_trace(
            "probe",
            "protocol",
            0,
            0,
            "start",
            Some(CMD_REQ_FILE),
            None,
            None,
            None,
            Duration::ZERO,
            &format!("{}:{}", ty.label(), name),
        );
        let msg = files::encode_file_message(CMD_REQ_FILE, ty, name, &[]);
        self.send_message(&msg)?;
        match self.recv_message(CMD_FILE, "file reply", timeout) {
            Ok(raw) if raw.first() == Some(&CMD_FILE) => {
                let f = files::decode_file_message(&raw)?;
                self.record_trace(
                    "probe",
                    "protocol",
                    0,
                    raw.len(),
                    "present",
                    Some(CMD_REQ_FILE),
                    Some(CMD_FILE),
                    Some(raw.len()),
                    None,
                    t0.elapsed(),
                    &format!("{}:{}: {} B", f.ty.label(), f.name, f.data.len()),
                );
                Ok(Probe::Present(f))
            }
            Ok(raw) => Err(DeviceError::Protocol(format!(
                "F8 `{name}`: reply is not an F7 file (starts {:02X?})",
                &raw[..raw.len().min(8)]
            ))),
            Err(DeviceError::Timeout(_)) => {
                self.record_trace(
                    "probe",
                    "protocol",
                    0,
                    0,
                    "no_reply",
                    Some(CMD_REQ_FILE),
                    None,
                    None,
                    Some("probe_file"),
                    t0.elapsed(),
                    &format!("{}:{} silent", ty.label(), name),
                );
                Ok(Probe::NoReply { waited: timeout })
            }
            Err(e) => Err(e),
        }
    }

    /// Read-only fetch helper for `ppl device get`: [`Prime::probe_file`]
    /// with [`Prime::read_timeout`]. [`Probe::NoReply`] means only "no
    /// reply"; no write path may use this to decide that a name is free —
    /// writes go through [`Prime::preflight_new_names`].
    pub fn request_file(&mut self, ty: FileType, name: &str) -> Result<Probe> {
        self.probe_file(ty, name, self.read_timeout)
    }

    /// Sends the `EB` delete-object message for exactly one named object.
    ///
    /// Wire evidence (Connectivity Kit 2.4.2, `CConKitApp` delete
    /// dispatcher at `0x1400589f0` in `ConnectivityKit.exe`): the CK
    /// builds a standard `F7` file message (type byte, length-prefixed
    /// UTF-16LE name, CRC-16/XMODEM) and overwrites the command byte
    /// with `0xEB`. Types are the usual object codes (`0x05` note,
    /// `0x06` program). The calculator sends no acknowledgment; the
    /// caller must verify the deletion through [`delete::verify_deleted`]
    /// before reporting success. The message is never retried.
    pub(crate) fn delete_object(&mut self, ty: FileType, name: &str) -> Result<()> {
        // Reachable only through a DeleteGuard, which re-validates every
        // target under its own policy; this is the final per-object check.
        validate_object_name(ty, name)?;
        if PROTECTED_OBJECTS
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name))
        {
            return Err(DeviceError::Refused(format!(
                "`{name}` is a protected object and cannot be deleted"
            )));
        }
        let start = self.log.len();
        self.drain();
        self.ensure_quiet(start, &[name.into()])?;
        let msg = files::encode_file_message(CMD_DELETE_OBJECT, ty, name, &[]);
        self.send_message(&msg)
    }

    /// Establishes that none of `targets` exists on the calculator and
    /// returns the only token that allows [`Prime::send_new_program`].
    ///
    /// Static checks (any failure → [`DeviceError::Refused`], no traffic):
    /// every name passes [`validate_test_name`], exactly one target is a
    /// program, no duplicates, the control is not a target.
    ///
    /// Device checks, in order:
    /// 1. `FF` must answer [`READY_REPLY`];
    /// 2. the control must be [`Probe::Present`] with a good CRC (only its
    ///    size and a hash of its data are recorded, never its contents);
    /// 3. every target must give [`Probe::NoReply`] within [`PROBE_TIMEOUT`];
    /// 4. the control must answer again, unchanged;
    /// 5. every target must stay silent a second time.
    ///
    /// Any target that answers → [`DeviceError::Refused`]. Anything else
    /// that goes wrong (no `FF`, silent control, HID or protocol error,
    /// unexpected late data) → [`DeviceError::Inconclusive`].
    pub fn preflight_new_names(
        &mut self,
        targets: &[(FileType, &str)],
        control: Option<&ControlObject>,
    ) -> Result<WriteGuard> {
        self.preflight_new_names_inner(targets, control, false, NamePolicy::Test, &[])
    }

    /// Like [`Prime::preflight_new_names`] for one **ordinary user
    /// program** (any name accepted by [`validate_program_name`]). It covers
    /// the program only (no `NAME`+`R` result note) and needs no control
    /// object. The exact name and its all-upper/all-lower case variants must
    /// each stay silent twice, so an existing program is never overwritten
    /// or shadowed. The returned guard carries [`NamePolicy::User`].
    pub fn preflight_new_user_program(
        &mut self,
        name: &str,
        control: Option<&ControlObject>,
    ) -> Result<WriteGuard> {
        validate_program_name(name)?;
        let mut variants = Vec::new();
        for v in [name.to_ascii_uppercase(), name.to_ascii_lowercase()] {
            if v != name && !variants.contains(&v) {
                variants.push(v);
            }
        }
        self.preflight_new_names_inner(
            &[(FileType::Program, name)],
            control,
            false,
            NamePolicy::User,
            &variants,
        )
    }

    pub(crate) fn preflight_mailbox(
        &mut self,
        request: &remote::Request,
        control: Option<&ControlObject>,
    ) -> Result<WriteGuard> {
        if request.request_note != format!("ZZT_Q{}", request.id)
            || request.receipt_note != format!("ZZT_A{}", request.id)
            || request.result_note != format!("ZZT_R{}", request.id)
            || request.id.len() != 10
            || !request.id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(DeviceError::Refused("invalid mailbox reservation".into()));
        }
        self.preflight_new_names_inner(
            &[
                (FileType::Note, &request.request_note),
                (FileType::Note, &request.receipt_note),
                (FileType::Note, &request.result_note),
            ],
            control,
            true,
            NamePolicy::Test,
            &[],
        )
    }

    fn preflight_new_names_inner(
        &mut self,
        targets: &[(FileType, &str)],
        control: Option<&ControlObject>,
        mailbox: bool,
        policy: NamePolicy,
        variants: &[String],
    ) -> Result<WriteGuard> {
        for (_, n) in targets {
            policy.validate(n)?;
        }
        let programs: Vec<&str> = targets
            .iter()
            .filter(|(t, _)| *t == FileType::Program)
            .map(|(_, n)| *n)
            .collect();
        let notes: Vec<&str> = targets
            .iter()
            .filter(|(t, _)| *t == FileType::Note)
            .map(|(_, n)| *n)
            .collect();
        // Exactly one write per guard: either a program (with its result
        // note), or a single note-only guard (see `send_new_note`).
        if programs.is_empty() && notes.len() != if mailbox { 3 } else { 1 } {
            return Err(DeviceError::Refused(format!(
                "a preflight must cover exactly one program (got {}) or exactly one note (got {})",
                programs.len(),
                notes.len()
            )));
        }
        if !programs.is_empty() && programs.len() != 1 {
            return Err(DeviceError::Refused(format!(
                "a preflight must cover exactly one program (got {})",
                programs.len()
            )));
        }
        for (i, (_, a)) in targets.iter().enumerate() {
            if targets[..i].iter().any(|(_, b)| a.eq_ignore_ascii_case(b)) {
                return Err(DeviceError::Refused(format!("`{a}` is listed twice")));
            }
            if let Some(ctrl) = control {
                if a.eq_ignore_ascii_case(&ctrl.name) {
                    return Err(DeviceError::Refused(format!(
                        "`{a}` is both a target and the control object"
                    )));
                }
            }
        }
        let mut names: Vec<String> = targets.iter().map(|(_, n)| n.to_string()).collect();
        names.extend(variants.iter().cloned());
        let start = self.log.len();
        let mut log = Vec::new();

        // 1. Initial FF check-ready
        let ready = self
            .check_ready()
            .map_err(|e| DeviceError::Inconclusive(format!("check-ready (FF) failed: {e}")))?;
        if ready.first() != Some(&READY_REPLY) {
            return Err(DeviceError::Inconclusive(format!(
                "check-ready (FF) replied {ready:02X?}, expected [{READY_REPLY:02X}]"
            )));
        }
        log.push(format!("FF check-ready: reply {READY_REPLY:02X} (ready)"));
        self.ensure_quiet(start, &names)?;

        // 2. Control probe (optional)
        let mut active_control: Option<(ControlObject, (usize, u64))> = None;
        if let Some(ctrl) = control {
            match self.probe_file(ctrl.ty, &ctrl.name, PROBE_TIMEOUT) {
                Ok(Probe::Present(f)) => {
                    if f.name != ctrl.name || f.ty != ctrl.ty {
                        return Err(DeviceError::Inconclusive(format!(
                            "control probe for {ctrl} was answered with object `{}`",
                            f.name
                        )));
                    }
                    if !f.crc_ok {
                        return Err(DeviceError::Inconclusive(format!(
                            "control {ctrl} reply has a CRC mismatch"
                        )));
                    }
                    let h = fnv64(&f.data);
                    log.push(format!(
                        "control {ctrl}: present, {} bytes, fnv64 {h:016x}, crc ok (probe 1)",
                        f.data.len()
                    ));
                    active_control = Some((ctrl.clone(), (f.data.len(), h)));
                }
                Ok(Probe::NoReply { waited }) => {
                    log.push(format!(
                        "control {ctrl}: no reply within {:.1} s; using device-level transport verification (FF check-ready)",
                        waited.as_secs_f64()
                    ));
                }
                Err(e) => {
                    return Err(DeviceError::Inconclusive(format!(
                        "control probe for {ctrl} failed: {e}"
                    )));
                }
            }
            self.ensure_quiet(start, &names)?;
        } else {
            log.push("no control object specified; using device-level transport verification (FF check-ready)".to_string());
        }

        // 3. Targets, round 1
        for (ty, n) in targets {
            self.probe_absent(*ty, n, 1, &names, &mut log)?;
            self.ensure_quiet(start, &names)?;
        }
        for v in variants {
            self.probe_absent(FileType::Program, v, 1, &names, &mut log)?;
            self.ensure_quiet(start, &names)?;
        }

        // 4. Inter-round verification: active control or FF check-ready
        if let Some((ref ctrl, (expected_len, expected_hash))) = active_control {
            let second = self.probe_control(ctrl, 2, &mut log)?;
            if second != (expected_len, expected_hash) {
                return Err(DeviceError::Inconclusive(format!(
                    "control {ctrl} changed between probes ({expected_len} → {} bytes)",
                    second.0
                )));
            }
        } else {
            let ready = self.check_ready().map_err(|e| {
                DeviceError::Inconclusive(format!("inter-round check-ready (FF) failed: {e}"))
            })?;
            if ready.first() != Some(&READY_REPLY) {
                return Err(DeviceError::Inconclusive(format!(
                    "inter-round check-ready (FF) replied {ready:02X?}, expected [{READY_REPLY:02X}]"
                )));
            }
            log.push(format!(
                "FF check-ready (round 2): reply {READY_REPLY:02X} (ready)"
            ));
        }
        self.ensure_quiet(start, &names)?;

        // 5. Targets, round 2
        for (ty, n) in targets {
            self.probe_absent(*ty, n, 2, &names, &mut log)?;
            self.ensure_quiet(start, &names)?;
        }
        for v in variants {
            self.probe_absent(FileType::Program, v, 2, &names, &mut log)?;
            self.ensure_quiet(start, &names)?;
        }

        // 6. Post-probe check-ready
        let ready = self.check_ready().map_err(|e| {
            DeviceError::Inconclusive(format!("post-probe check-ready (FF) failed: {e}"))
        })?;
        if ready.first() != Some(&READY_REPLY) {
            return Err(DeviceError::Inconclusive(format!(
                "post-probe check-ready (FF) replied {ready:02X?}, expected [{READY_REPLY:02X}]"
            )));
        }

        if let Some((ref ctrl, _)) = active_control {
            log.push(format!(
                "preflight passed: {} silent twice while control {ctrl} answered twice",
                names.join(", ")
            ));
        } else {
            log.push(format!(
                "preflight passed: {} silent twice with device responsive across all rounds",
                names.join(", ")
            ));
        }

        Ok(WriteGuard {
            // `unwrap_or` (not `unwrap_or_else`) eagerly indexed
            // `notes[0]` and panicked for program-only preflights (found
            // by the cleanup PoC, which preflights a program with no
            // note). Validation above guarantees one program or one note.
            program: programs
                .first()
                .or_else(|| notes.first())
                .copied()
                .unwrap_or_default()
                .to_string(),
            targets: targets.iter().map(|(t, n)| (*t, n.to_string())).collect(),
            device_path: self.detected.path.clone(),
            owner: std::sync::Arc::downgrade(&self.owner),
            created: Instant::now(),
            max_age: GUARD_MAX_AGE,
            log,
            policy,
        })
    }

    /// Probes the control object; returns `(size, hash)` of its data.
    fn probe_control(
        &mut self,
        control: &ControlObject,
        round: u32,
        log: &mut Vec<String>,
    ) -> Result<(usize, u64)> {
        match self.probe_file(control.ty, &control.name, PROBE_TIMEOUT) {
            Ok(Probe::Present(f)) => {
                if f.name != control.name || f.ty != control.ty {
                    return Err(DeviceError::Inconclusive(format!(
                        "control probe for {control} was answered with object `{}`",
                        f.name
                    )));
                }
                if !f.crc_ok {
                    return Err(DeviceError::Inconclusive(format!(
                        "control {control} reply has a CRC mismatch"
                    )));
                }
                let h = fnv64(&f.data);
                log.push(format!(
                    "control {control}: present, {} bytes, fnv64 {h:016x}, crc ok (probe {round})",
                    f.data.len()
                ));
                Ok((f.data.len(), h))
            }
            Ok(Probe::NoReply { waited }) => Err(DeviceError::Inconclusive(format!(
                "control {control} did not answer within {:.1} s (probe {round}); without a \
                 known-present object answering, silence proves nothing",
                waited.as_secs_f64()
            ))),
            Err(e) => Err(DeviceError::Inconclusive(format!(
                "control probe {round} for {control} failed: {e}"
            ))),
        }
    }

    /// Probes one target; only [`Probe::NoReply`] passes.
    fn probe_absent(
        &mut self,
        ty: FileType,
        name: &str,
        round: u32,
        names: &[String],
        log: &mut Vec<String>,
    ) -> Result<()> {
        match self.probe_file(ty, name, PROBE_TIMEOUT) {
            Ok(Probe::NoReply { waited }) => {
                log.push(format!(
                    "target {}:{name}: no reply within {:.1} s (round {round})",
                    ty.label(),
                    waited.as_secs_f64()
                ));
                Ok(())
            }
            Ok(Probe::Present(f)) if names.iter().any(|n| n.eq_ignore_ascii_case(&f.name)) => {
                Err(DeviceError::Refused(format!(
                    "`{}` exists on the calculator ({} bytes); it will not be overwritten",
                    f.name,
                    f.data.len()
                )))
            }
            Ok(Probe::Present(f)) => Err(DeviceError::Inconclusive(format!(
                "probe for `{name}` was answered with unrelated object `{}`",
                f.name
            ))),
            Err(e) => Err(DeviceError::Inconclusive(format!(
                "probe {round} for `{name}` failed: {e}"
            ))),
        }
    }

    /// Fails if anything was drained since log index `from`: a late reply
    /// naming a target means it exists; anything else is unexplained.
    fn ensure_quiet(&self, from: usize, names: &[String]) -> Result<()> {
        let drained: Vec<&Exchange> = self.log[from.min(self.log.len())..]
            .iter()
            .filter(|e| e.drained)
            .collect();
        if drained.is_empty() {
            return Ok(());
        }
        for file in crate::late::files(&drained, self.report_size)? {
            if names.iter().any(|n| n == &file.name) {
                return Err(DeviceError::Refused(format!(
                    "`{}` exists on the calculator (a CRC-valid late {} reply); no absence was established",
                    file.name,
                    file.ty.label()
                )));
            }
        }
        Err(DeviceError::Inconclusive(format!(
            "{} unexpected report(s) arrived late (possibly a slow reply to an earlier \
             request); existence cannot be established",
            drained.len()
        )))
    }

    /// Reads the object back with a **single** `F8` and compares it with
    /// `sent`. The host keeps listening for [`READBACK_TIMEOUT`] instead of
    /// re-asking: every new `F8` first drains the input queue, which would
    /// discard a reply the calculator is already sending after a slow
    /// compile or commit. Silence is [`ReadBack::NoReply`], never success.
    fn read_back(&mut self, ty: FileType, name: &str, sent: &str) -> ReadBack {
        match self.probe_file(ty, name, READBACK_TIMEOUT) {
            Ok(Probe::Present(f)) if f.name != name || f.ty != ty => ReadBack::Failed {
                error: format!("read-back returned object `{}`", f.name),
            },
            Ok(Probe::Present(f)) if !f.crc_ok => ReadBack::Failed {
                error: "read-back CRC mismatch".into(),
            },
            Ok(Probe::Present(f)) => {
                let got = normalize_program_text(&f.text());
                if got == sent {
                    ReadBack::Matches
                } else {
                    ReadBack::Differs {
                        sent_chars: sent.chars().count(),
                        got_chars: got.chars().count(),
                        got_text: got,
                    }
                }
            }
            Ok(Probe::NoReply { .. }) => ReadBack::NoReply,
            Err(e) => ReadBack::Failed {
                error: e.to_string(),
            },
        }
    }

    /// Sends a **new** program. The name comes from `guard`, which must
    /// have been issued by [`Prime::preflight_new_names`] for this device
    /// less than [`GUARD_MAX_AGE`] ago. The send is never retried. After the
    /// `F7` the program is read back and compared (see [`ReadBack`]).
    pub fn send_new_program(&mut self, guard: WriteGuard, source: &str) -> Result<UploadReport> {
        if guard.is_expired() {
            return Err(DeviceError::Refused(format!(
                "write guard for `{}` is {:.0} s old (limit {} s); run the preflight again",
                guard.program,
                guard.age().as_secs_f64(),
                guard.max_age.as_secs()
            )));
        }
        if guard.device_path != self.detected.path
            || !guard
                .owner
                .upgrade()
                .is_some_and(|owner| std::sync::Arc::ptr_eq(&owner, &self.owner))
        {
            return Err(DeviceError::Refused(
                "write guard was issued for a different device or connection".into(),
            ));
        }
        guard.policy.validate(&guard.program)?;
        let names: Vec<String> = guard.targets.iter().map(|(_, n)| n.clone()).collect();
        let start = self.log.len();
        self.drain();
        self.ensure_quiet(start, &names)?;

        let name = guard.program.clone();
        let payload_bytes = source.encode_utf16().count() * 2;
        if payload_bytes > MAX_LEGACY_PROGRAM_PAYLOAD {
            if !self.supports_extended_transport().unwrap_or(false) {
                return Err(DeviceError::Refused(format!(
                    "program payload is {payload_bytes} bytes; conservative G1 legacy upload policy allows at most {MAX_LEGACY_PROGRAM_PAYLOAD} bytes (nothing was sent)."
                )));
            }
            self.ensure_mode1()?;
        }
        let protocol = match self.mode {
            TransportMode::Legacy => TransferProtocol::Legacy,
            TransportMode::Mode1 => TransferProtocol::Extended,
        };
        check_program_payload_for_protocol(&name, source, protocol)?;
        let data: Vec<u8> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let msg = files::encode_file_message(CMD_FILE, FileType::Program, &name, &data);
        self.send_message(&msg)?;
        self.drain();

        // Brief settle time to let the physical calculator compile and commit the new program
        std::thread::sleep(Duration::from_millis(50));
        self.drain();

        let readback = self.read_back(FileType::Program, &name, &normalize_program_text(source));
        Ok(UploadReport {
            name,
            bytes_sent: data.len(),
            readback,
            preflight_log: guard.log,
        })
    }

    /// Sends a **new** note (same guard rules as [`Prime::send_new_program`],
    /// but the guard covers exactly one note and no program). The note text
    /// is sent as UTF-16LE and read back for verification.
    pub fn send_new_note(&mut self, guard: WriteGuard, text: &str) -> Result<UploadReport> {
        if guard.is_expired() {
            return Err(DeviceError::Refused(format!(
                "write guard for `{}` is {:.0} s old (limit {} s); run the preflight again",
                guard.program,
                guard.age().as_secs_f64(),
                guard.max_age.as_secs()
            )));
        }
        if guard.device_path != self.detected.path
            || !guard
                .owner
                .upgrade()
                .is_some_and(|owner| std::sync::Arc::ptr_eq(&owner, &self.owner))
        {
            return Err(DeviceError::Refused(
                "write guard was issued for a different device or connection".into(),
            ));
        }
        if guard.policy != NamePolicy::Test {
            return Err(DeviceError::Refused(
                "notes may only be created under the test-object naming policy".into(),
            ));
        }
        validate_test_name(&guard.program)?;
        if guard.targets.iter().any(|(t, _)| *t == FileType::Program) {
            return Err(DeviceError::Refused(
                "a note guard must not cover a program; use send_new_program".into(),
            ));
        }
        let names: Vec<String> = guard.targets.iter().map(|(_, n)| n.clone()).collect();
        let start = self.log.len();
        self.drain();
        self.ensure_quiet(start, &names)?;

        let name = guard.program.clone();
        let data: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let msg = files::encode_file_message(CMD_FILE, FileType::Note, &name, &data);
        self.send_message(&msg)?;
        self.drain();

        // Brief settle time to let the physical calculator commit the note
        std::thread::sleep(Duration::from_millis(50));
        self.drain();

        let readback = self.read_back(FileType::Note, &name, &normalize_program_text(text));
        Ok(UploadReport {
            name,
            bytes_sent: data.len(),
            readback,
            preflight_log: guard.log,
        })
    }

    /// `EC`: key presses. Drives whatever is on the calculator screen.
    ///
    /// No automatic execution path calls this. Diagnostic callers require
    /// an explicit bounded device-control session; transmission proves no UI action.
    pub fn press_keys(&mut self, codes: &[u8]) -> Result<()> {
        if codes.iter().any(|&c| c > 50) {
            return Err(DeviceError::Refused("key codes must be 0–50".into()));
        }
        self.drain();
        let mut msg = vec![CMD_KEY, 0x01];
        msg.extend_from_slice(&(codes.len() as u32).to_be_bytes());
        msg.extend_from_slice(codes);
        self.send_message(&msg)
    }

    /// Renders the exchange log as hex lines for documentation.
    pub fn log_text(&self) -> String {
        let mut out = String::new();
        for e in &self.log {
            let dir = match (e.outgoing, e.drained) {
                (true, _) => "host→calc",
                (false, false) => "calc→host",
                (false, true) => "calc→host (drained)",
            };
            let hex: Vec<String> = e.bytes.iter().map(|b| format!("{b:02X}")).collect();
            out.push_str(&format!(
                "{:>8.3}s {dir} {:3} B  {}\n",
                e.at.as_secs_f64(),
                e.bytes.len(),
                hex.join(" ")
            ));
        }
        out
    }

    /// Formats the structured trace log.
    pub fn trace_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Time (s)  | Op Type     | Endpoint   | Req/Act   | Status   | Cmd/Resp | Elapsed  | Details\n");
        out.push_str("----------+-------------+------------+-----------+----------+----------+----------+-------------------\n");
        for t in &self.traces {
            let cmd_str = match (t.cmd_id, t.resp_id) {
                (Some(c), Some(r)) => format!("{:02X}/{:02X}", c, r),
                (Some(c), None) => format!("{:02X}/--", c),
                (None, Some(r)) => format!("--/{:02X}", r),
                (None, None) => "--/--".to_string(),
            };
            out.push_str(&format!(
                "{:>8.3} | {:<11} | {:<10} | {:>4}/{:<4} | {:<8} | {:<8} | {:>7.3}s | {}\n",
                t.at.as_secs_f64(),
                t.op_type,
                t.endpoint,
                t.requested_bytes,
                t.actual_bytes,
                t.status,
                cmd_str,
                t.elapsed.as_secs_f64(),
                t.details
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_xmodem_check_value() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn framing_matches_hplp() {
        let r = Prime::frame(&[0xFF], 64);
        assert_eq!(r, vec![vec![0x00, 0x00, 0xFF]]);
        let msg: Vec<u8> = (0..130u8).collect();
        let r = Prime::frame(&msg, 64);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].len(), 65);
        assert_eq!(r[1][1], 1);
        assert_eq!(r[2].len(), 2 + 130 - 126);
        // exactly one full chunk: no trailing empty report
        assert_eq!(Prime::frame(&[0u8; 63], 64).len(), 1);
    }

    #[test]
    fn info_reply_from_g1_build_15157() {
        // Captured from a G1 (03f0:1541); serial digits replaced.
        let mut raw = vec![0xFA, 0x01, 0, 0, 0, 0x6C];
        let mut body = vec![0u8; 108];
        body[65] = 0x30;
        body[68..76].copy_from_slice(&[0x24, 0x20, 0x01, 0x39, 0x35, 0x3B, 0, 0]);
        body[76..86].copy_from_slice(b"V0.050.640");
        body[92..102].copy_from_slice(b"0000000000");
        raw.extend(body);
        let info = CalcInfo::parse(&raw);
        assert_eq!(info.build, Some(15157));
        assert_eq!(info.version.as_deref(), Some("V0.050.640"));
        assert_eq!(info.serial.as_deref(), Some("0000000000"));
    }

    #[test]
    fn utf16_text_extraction() {
        let mut b = vec![0u8, 1, 2];
        b.extend("2.1.14730".encode_utf16().flat_map(u16::to_le_bytes));
        b.extend([0, 0]);
        let runs = utf16_runs(&b, 3);
        assert!(runs.iter().any(|s| s.contains("14730")), "{runs:?}");
    }
}
