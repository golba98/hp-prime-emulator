//! Run manifests and the name ledger.
//!
//! A *run manifest* (`<runs dir>/<program>.json`) records everything about
//! one generated program before anything reaches the calculator: the exact
//! source and its hash, which cases it contains, and later the upload and
//! device details. The *ledger* (`<runs dir>/ledger.jsonl`, one JSON object
//! per line, append-only) records every program and note name ever
//! prepared, so a rerun never reuses a name that may still exist on the
//! calculator.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::generate::{GeneratedProgram, valid_program_name};
use crate::sha256::sha256_hex;

/// Default directory for manifests and the ledger (relative to the repo).
pub const DEFAULT_RUNS_DIR: &str = "tests/compat/runs";
/// File name of the ledger inside the runs directory.
pub const LEDGER_FILE: &str = "ledger.jsonl";
/// Salt prefixed to serial numbers before hashing.
pub const SERIAL_SALT: &str = "ppl-hp-prime-serial-v1:";

/// Identity of the calculator a run was made on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub model: String,
    /// `vvvv:pppp`, e.g. `03f0:1541`.
    pub usb_pid: String,
    pub firmware_build: Option<u32>,
    pub boot_version: Option<String>,
    /// [`serial_fingerprint`] of the serial number, never the serial itself.
    pub serial_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Generated and saved; nothing sent to the device.
    Prepared,
    /// Uploaded and confirmed by read-back.
    Uploaded,
    /// Upload sent but read-back did not confirm it.
    UploadUnverified,
    /// A preflight check refused the run (see `blocked_reason`).
    Blocked,
    /// Evidence from this run has been recorded.
    EvidenceRecorded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema: u32,
    pub suite: String,
    pub group: Option<String>,
    pub program: String,
    pub note: Option<String>,
    /// `print` or `note`.
    pub channel: String,
    pub case_ids: Vec<String>,
    pub short_ids: Vec<(String, String)>,
    pub source: String,
    pub sha256: String,
    pub created_at_utc: String,
    #[serde(default)]
    pub uploaded_at_utc: Option<String>,
    #[serde(default)]
    pub preflight_log: Vec<String>,
    #[serde(default)]
    pub device: Option<DeviceRecord>,
    pub status: RunStatus,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// SHA-256 of the note content when this run created a *plain* note
    /// (no `@@ZZT|` header) — lets cleanup verify note identity before
    /// deletion. Result notes are verified by their header instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_sha256: Option<String>,
}

impl RunManifest {
    /// A `Prepared` manifest for a generated program.
    pub fn prepared(
        suite: &str,
        group: Option<&str>,
        g: &GeneratedProgram,
        created_at_utc: &str,
    ) -> RunManifest {
        RunManifest {
            schema: 1,
            suite: suite.to_string(),
            group: group.map(str::to_string),
            program: g.program.clone(),
            note: g.channel.note().map(str::to_string),
            channel: g.channel.as_str().to_string(),
            case_ids: g.short_ids.iter().map(|(_, c)| c.clone()).collect(),
            short_ids: g.short_ids.clone(),
            source: g.source.clone(),
            sha256: g.sha256.clone(),
            created_at_utc: created_at_utc.to_string(),
            uploaded_at_utc: None,
            preflight_log: Vec::new(),
            device: None,
            status: RunStatus::Prepared,
            blocked_reason: None,
            note_sha256: None,
        }
    }

    /// Checks that `source` still hashes to `sha256` and carries a valid tag.
    pub fn verify_source(&self) -> Result<(), String> {
        let got = sha256_hex(self.source.as_bytes());
        if got != self.sha256 {
            return Err(format!(
                "manifest {}: source hash {got} does not match recorded {}",
                self.program, self.sha256
            ));
        }
        let (prog, _) = crate::generate::verify_program_tag(&self.source)?;
        if prog != self.program {
            return Err(format!(
                "manifest {}: source header names program {prog}",
                self.program
            ));
        }
        Ok(())
    }
}

fn manifest_path(dir: &Path, program: &str) -> Result<PathBuf, String> {
    if !valid_program_name(program) {
        return Err(format!("invalid program name `{program}`"));
    }
    Ok(dir.join(format!("{program}.json")))
}

/// Writes `dir/<program>.json` (creating `dir`), replacing an older version
/// of the same manifest (status updates rewrite it).
pub fn save_manifest(dir: &Path, m: &RunManifest) -> Result<PathBuf, String> {
    let path = manifest_path(dir, &m.program)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(&path, json + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

pub fn load_manifest(dir: &Path, program: &str) -> Result<RunManifest, String> {
    let path = manifest_path(dir, program)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// One ledger line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// A program or note name that was prepared for the calculator.
    pub name: String,
    /// The program it belongs to.
    pub program: String,
    pub utc: String,
}

fn read_ledger(dir: &Path) -> Result<Vec<LedgerEntry>, String> {
    let path = dir.join(LEDGER_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))
        })
        .collect()
}

/// Whether `name` was ever recorded in the ledger. A missing ledger is
/// empty; an unreadable or corrupt one is an error (fail safe).
pub fn ledger_contains(dir: &Path, name: &str) -> Result<bool, String> {
    Ok(read_ledger(dir)?.iter().any(|e| e.name == name))
}

/// Every ledger entry, oldest first. A missing ledger is empty; an
/// unreadable or corrupt one is an error (fail safe).
pub fn ledger_entries(dir: &Path) -> Result<Vec<LedgerEntry>, String> {
    read_ledger(dir)
}

/// Appends one line per name.
pub fn ledger_append(dir: &Path, names: &[String], program: &str, utc: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(LEDGER_FILE);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = String::new();
    for name in names {
        let line = serde_json::to_string(&LedgerEntry {
            name: name.clone(),
            program: program.to_string(),
            utc: utc.to_string(),
        })
        .map_err(|e| e.to_string())?;
        buf.push_str(&line);
        buf.push('\n');
    }
    f.write_all(buf.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The first name `ZZT_<stem><n>` (n = 1, 2, …) not in the ledger and
/// valid under [`valid_program_name`]. `stem` is upper-cased and stripped
/// of characters outside `[A-Z0-9_]`.
pub fn unused_name(dir: &Path, stem: &str) -> Result<String, String> {
    let ledger = read_ledger(dir)?;
    let stem: String = stem
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
        .collect();
    for n in 1..100_000u32 {
        let suffix = n.to_string();
        let keep = 12usize.saturating_sub(suffix.len()).min(stem.len());
        let name = format!("ZZT_{}{suffix}", &stem[..keep]);
        if valid_program_name(&name) && !ledger.iter().any(|e| e.name == name) {
            return Ok(name);
        }
    }
    Err(format!("no unused name for stem `{stem}`"))
}

/// Salted SHA-256 fingerprint of a serial number: first 16 hex digits of
/// `SHA-256("ppl-hp-prime-serial-v1:" + serial)`. Lets evidence from the same
/// unit be grouped without storing the serial.
pub fn serial_fingerprint(serial: &str) -> String {
    sha256_hex(format!("{SERIAL_SALT}{serial}").as_bytes())[..16].to_string()
}
