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

/// Built-in settings objects usable as a liveness anchor (type 0x00). They
/// exist on every calculator, so no control program has to be installed.
pub const SETTINGS_ANCHORS: &[&str] = &["calc.settings", "cas.settings"];

/// The default control-free anchor.
pub const DEFAULT_SETTINGS_ANCHOR: &str = "settings:calc.settings";

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

/// Which naming rules a [`WriteGuard`] was issued under. A guard keeps its
/// policy, so a user-program guard cannot be spent as a disposable test
/// object (or the other way round).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamePolicy {
    /// Disposable tool-owned objects: [`validate_test_name`].
    Test,
    /// Ordinary user programs: [`validate_program_name`].
    User,
}

/// Names that would shadow PPL keywords or system identifiers. Compared
/// case-insensitively. Deliberately conservative.
const RESERVED_NAMES: &[&str] = &[
    "ANS", "BEGIN", "BREAK", "CASE", "CONST", "CONTINUE", "DEFAULT", "DO", "DOWNTO", "ELSE",
    "ELSIF", "END", "EXPORT", "FOR", "FROM", "HOME", "IF", "IFERR", "KILL", "LOCAL", "REPEAT",
    "RETURN", "STEP", "THEN", "THETA", "TO", "UNTIL", "VAR", "WHILE", "ZZT_CTRL",
];

/// Checks the name of an ordinary user program: 1..=[`MAX_NAME_LEN`]
/// characters of `[A-Za-z0-9_]`, starting with a letter, case preserved.
/// No `ZZT_` prefix is required. Single letters and letter+digit names
/// (`A`, `L1`, `G0`, …) are refused because they are system variables, as
/// are the names in [`RESERVED_NAMES`].
pub fn validate_program_name(name: &str) -> Result<()> {
    let reason = if name.is_empty() {
        Some("it is empty".to_string())
    } else if name.len() > MAX_NAME_LEN {
        Some(format!("it is longer than {MAX_NAME_LEN} characters"))
    } else if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some("only A-Z, a-z, 0-9 and _ are allowed".to_string())
    } else if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        Some("it must start with a letter".to_string())
    } else if name.len() == 1
        || (name.len() == 2 && name.as_bytes()[1].is_ascii_digit())
        || RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(name))
    {
        Some("it is a reserved calculator name".to_string())
    } else {
        None
    };
    match reason {
        None => Ok(()),
        Some(r) => Err(DeviceError::Refused(format!(
            "`{name}` is not a valid program name: {r}"
        ))),
    }
}

/// Checks the name of an existing calculator object that the user asks to
/// delete. Program and note names follow the same character rules as
/// [`validate_program_name`]; no `ZZT_` prefix is required. Any other type
/// has no verified read/backup/delete path and is
/// [`DeviceError::Unsupported`]. Names are object identifiers, never paths.
pub fn validate_object_name(ty: FileType, name: &str) -> Result<()> {
    match ty {
        FileType::Program | FileType::Note => {}
        other => {
            return Err(DeviceError::Unsupported(format!(
                "{} objects have no verified deletion support",
                other.label()
            )));
        }
    }
    let reason = if name.is_empty() {
        Some("it is empty".to_string())
    } else if name.len() > MAX_NAME_LEN {
        Some(format!("it is longer than {MAX_NAME_LEN} characters"))
    } else if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some("only A-Z, a-z, 0-9 and _ are allowed".to_string())
    } else if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        Some("it must start with a letter".to_string())
    } else if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        Some("it is a reserved calculator name".to_string())
    } else {
        None
    };
    match reason {
        None => Ok(()),
        Some(r) => Err(DeviceError::Refused(format!(
            "`{name}` is not a valid {} name: {r}",
            ty.label()
        ))),
    }
}

impl NamePolicy {
    pub(crate) fn validate(self, name: &str) -> Result<()> {
        match self {
            NamePolicy::Test => validate_test_name(name),
            NamePolicy::User => validate_program_name(name),
        }
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
        if ty == FileType::Settings {
            // Built-in settings objects are the only non-program anchors.
            return if SETTINGS_ANCHORS.contains(&name) {
                Ok(ControlObject {
                    ty,
                    name: name.to_string(),
                })
            } else {
                Err(format!(
                    "control `{s}`: settings controls must be one of {SETTINGS_ANCHORS:?}"
                ))
            };
        }
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
    pub(crate) policy: NamePolicy,
}

impl WriteGuard {
    /// The naming policy this guard was issued under.
    pub fn policy(&self) -> NamePolicy {
        self.policy
    }

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
