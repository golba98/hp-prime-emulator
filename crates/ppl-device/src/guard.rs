//! Guarded object naming and the write-permission guard.
//!
//! The emulator may only create objects whose names pass these checks,
//! and only while holding a [`WriteGuard`] issued by the preflight.

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::files::FileType;
use super::{DeviceError, Result};

/// Object names the emulator may create on the calculator.
pub const SAFE_PREFIX: &str = "ZZT_";

/// Longest object name the emulator will create.
pub const MAX_NAME_LEN: usize = 16;

/// Default control object for [`Prime::preflight_new_names`].
pub const DEFAULT_CONTROL: &str = "program:ZZT_CTRL";

/// How long a preflight or read-back `F8` waits for the first reply report.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// A [`WriteGuard`] older than this is refused by [`Prime::send_new_program`].
pub const GUARD_MAX_AGE: Duration = Duration::from_secs(300);

/// Checks a name the emulator wants to create: `ZZT_` prefix, at most
/// [`MAX_NAME_LEN`] characters of `[A-Za-z0-9_]`, starting with a letter.
pub fn validate_test_name(name: &str) -> Result<()> {
    let reason = if !name.starts_with(SAFE_PREFIX) {
        Some(format!("it must start with {SAFE_PREFIX}"))
    } else if name.len() == SAFE_PREFIX.len() {
        Some(format!(
            "it needs at least one character after {SAFE_PREFIX}"
        ))
    } else if name.len() > MAX_NAME_LEN {
        Some(format!("it is longer than {MAX_NAME_LEN} characters"))
    } else if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some("only A-Z, a-z, 0-9 and _ are allowed".to_string())
    } else if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        Some("it must start with a letter".to_string())
    } else {
        None
    };
    match reason {
        None => Ok(()),
        Some(r) => Err(DeviceError::Refused(format!(
            "`{name}` is not an allowed test object name: {r}"
        ))),
    }
}

/// A fresh program name: `ZZT_` followed by 6 upper-case hex digits drawn
/// from OS randomness. If the OS generator fails, the digits come from a
/// hash of the clock and process id instead (the preflight, not the name,
/// is what protects existing objects).
pub fn fresh_test_name() -> String {
    let mut b = [0u8; 3];
    if getrandom::fill(&mut b).is_err() {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut seed = t.to_le_bytes().to_vec();
        seed.extend(std::process::id().to_le_bytes());
        b.copy_from_slice(&fnv64(&seed).to_le_bytes()[..3]);
    }
    format!("{SAFE_PREFIX}{:02X}{:02X}{:02X}", b[0], b[1], b[2])
}

/// Name of the note a generated test program writes its results to.
pub fn result_note_name(program: &str) -> String {
    format!("{program}R")
}

/// Normalises program text for read-back comparison: CRLF → LF, then
/// trailing NULs and whitespace removed.
pub fn normalize_program_text(s: &str) -> String {
    s.replace("\r\n", "\n")
        .trim_end_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string()
}

/// FNV-1a 64 (not cryptographic).
pub(crate) fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A known-present object used by the preflight to show that the
/// calculator is answering `F8` requests at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlObject {
    pub ty: FileType,
    pub name: String,
}

impl ControlObject {
    /// Parses `type:name`, e.g. `program:ZZT_CTRL`.
    pub fn parse(s: &str) -> std::result::Result<ControlObject, String> {
        let (t, n) = s.split_once(':').ok_or_else(|| {
            format!("control `{s}`: expected `type:name`, e.g. {DEFAULT_CONTROL}")
        })?;
        let ty = FileType::parse(t.trim())
            .ok_or_else(|| format!("control `{s}`: unknown object type `{}`", t.trim()))?;
        let name = n.trim();
        let ok = !name.is_empty()
            && name.len() <= MAX_NAME_LEN
            && name.starts_with(|c: char| c.is_ascii_alphabetic())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(format!(
                "control `{s}`: name must be 1–{MAX_NAME_LEN} characters of [A-Za-z0-9_] starting with a letter"
            ));
        }
        Ok(ControlObject {
            ty,
            name: name.to_string(),
        })
    }
}

impl FromStr for ControlObject {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        ControlObject::parse(s)
    }
}

impl fmt::Display for ControlObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ty.label(), self.name)
    }
}

/// Permission to create one program, issued only by
/// [`Prime::preflight_new_names`] and consumed by
/// [`Prime::send_new_program`]. Not `Clone`; expires after
/// [`GUARD_MAX_AGE`].
#[derive(Debug)]
pub struct WriteGuard {
    pub(crate) program: String,
    pub(crate) targets: Vec<(FileType, String)>,
    pub(crate) device_path: String,
    pub(crate) owner: std::sync::Weak<()>,
    pub(crate) created: Instant,
    pub(crate) max_age: Duration,
    pub(crate) log: Vec<String>,
}

impl WriteGuard {
    /// The program this guard allows to be created.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Every name the preflight showed to be silent (program first).
    pub fn names(&self) -> Vec<&str> {
        self.targets.iter().map(|(_, n)| n.as_str()).collect()
    }

    /// Every `(type, name)` the preflight checked.
    pub fn targets(&self) -> &[(FileType, String)] {
        &self.targets
    }

    /// Human-readable preflight log.
    pub fn log(&self) -> &[String] {
        &self.log
    }

    pub fn age(&self) -> Duration {
        self.created.elapsed()
    }

    pub fn is_expired(&self) -> bool {
        self.age() > self.max_age
    }
}

/// How the program read back after an upload compared with what was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadBack {
    /// Read-back text equals the sent text (after [`normalize_program_text`]).
    Matches,
    Differs {
        sent_chars: usize,
        got_chars: usize,
        got_text: String,
    },
    /// The read-back `F8` got no reply; the upload is unverified.
    NoReply,
    /// The read-back failed (HID/protocol error, CRC mismatch, wrong name).
    Failed { error: String },
}

/// Outcome of [`Prime::send_new_program`].
#[derive(Clone, Debug)]
pub struct UploadReport {
    pub name: String,
    /// Length of the program data (UTF-16LE source) in the `F7` message.
    pub bytes_sent: usize,
    pub readback: ReadBack,
    /// The preflight log of the consumed guard.
    pub preflight_log: Vec<String>,
}
