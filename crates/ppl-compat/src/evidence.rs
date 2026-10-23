//! Hardware evidence, schema 2.
//!
//! One file per device run: `tests/compat/evidence/<suite>/<program>.json`,
//! keeping the raw transcript (or note text) exactly as observed, the
//! settings snapshot from the program header, and device identity. Files
//! are never overwritten ([`save_evidence_v2`] refuses). Legacy single-file
//! v1 evidence (`tests/compat/evidence/<suite>.json`) is read and converted.
//!
//! # Status rules ([`status_v2`])
//!
//! * exact match of `TYPE` code and `STRING` text (or error vs error):
//!   **VERIFIED**;
//! * same type and text equal only after [`normalise_display`]: **PARTIAL**
//!   (never VERIFIED);
//! * the emulator reports the feature as unsupported: **UNSUPPORTED**;
//! * the device produced no record for the case: **BLOCKED**;
//! * anything else: **FAILED** (unlike the legacy [`crate::compare`], a
//!   same-type value with different text is a failure, not PARTIAL);
//! * no evidence: **UNVERIFIED** (or UNSUPPORTED).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::generate::{CaseEvidence, ParsedRun};
use crate::manifest::{DeviceRecord, RunManifest};
use crate::{CaseReport, Evidence, EvidenceSummary, Outcome, Report, Status, Suite};

/// Evidence read from a PRINT-channel screenshot, transcribed and confirmed
/// by the user.
pub const METHOD_SCREENSHOT: &str = "screenshot-transcribed-user-confirmed";
/// Evidence read from a result note fetched over USB.
pub const METHOD_NOTE_FETCH: &str = "note-fetch";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceV2 {
    pub schema: u32,
    pub suite: String,
    pub program: String,
    pub program_sha256: String,
    /// [`METHOD_SCREENSHOT`] or [`METHOD_NOTE_FETCH`] (converted v1 files
    /// keep their original method string).
    pub method: String,
    pub device: DeviceRecord,
    pub recorded_at_utc: String,
    /// Header snapshot: `HAngle`, `HFormat`, `HDigits` as `STRING()` text.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// Path of the screenshot / note file, relative to the evidence dir.
    #[serde(default)]
    pub raw_artifact: Option<String>,
    #[serde(default)]
    pub raw_artifact_sha256: Option<String>,
    /// The full transcript or note text as observed.
    pub raw_text: String,
    pub results: BTreeMap<String, CaseEvidence>,
    #[serde(default)]
    pub setting_changes: Vec<(String, String)>,
    /// Status of each case against the emulator at recording time.
    #[serde(default)]
    pub comparison: BTreeMap<String, Status>,
}

/// Checks that a parsed transcript belongs to the run described by `m`:
/// same program name, header tag equal to the tag in the manifest source,
/// and a manifest whose source still matches its hash.
pub fn check_run_against_manifest(run: &ParsedRun, m: &RunManifest) -> Result<(), String> {
    m.verify_source()?;
    if run.program != m.program {
        return Err(format!(
            "transcript is from program {}, manifest is for {}",
            run.program, m.program
        ));
    }
    let (_, sha8) = crate::generate::verify_program_tag(&m.source)?;
    if run.sha8 != sha8 {
        return Err(format!(
            "transcript tag {} does not match the manifest program tag {sha8}",
            run.sha8
        ));
    }
    Ok(())
}

impl EvidenceV2 {
    /// Builds evidence from a parsed run after checking it against its
    /// manifest. `comparison` is left empty: fill it with
    /// [`EvidenceV2::compare_with`].
    pub fn from_run(
        m: &RunManifest,
        run: &ParsedRun,
        method: &str,
        device: DeviceRecord,
        raw_text: String,
        recorded_at_utc: &str,
    ) -> Result<EvidenceV2, String> {
        check_run_against_manifest(run, m)?;
        Ok(EvidenceV2 {
            schema: 2,
            suite: m.suite.clone(),
            program: m.program.clone(),
            program_sha256: m.sha256.clone(),
            method: method.to_string(),
            device,
            recorded_at_utc: recorded_at_utc.to_string(),
            settings: run.settings.clone(),
            raw_artifact: None,
            raw_artifact_sha256: None,
            raw_text,
            results: run.results.clone(),
            setting_changes: run.setting_changes.clone(),
            comparison: BTreeMap::new(),
        })
    }

    /// Fills `comparison` from emulator outcomes.
    pub fn compare_with(&mut self, emu: &BTreeMap<String, Outcome>) {
        self.comparison = self
            .results
            .iter()
            .map(|(id, ev)| {
                let e = emu.get(id).cloned().unwrap_or(Outcome::Missing);
                (id.clone(), status_v2(&e, Some(&ev.outcome)))
            })
            .collect();
    }

    /// Converts legacy v1 evidence. Raw text was not kept in v1, so `raw`
    /// fields are empty and `program_sha256` is `fnv64:<source_fnv>`.
    pub fn from_v1(v1: &Evidence) -> EvidenceV2 {
        EvidenceV2 {
            schema: 2,
            suite: v1.suite.clone(),
            program: v1.program.clone(),
            program_sha256: format!("fnv64:{}", v1.source_fnv),
            method: v1.method.clone(),
            device: DeviceRecord {
                model: v1.device_model.clone(),
                usb_pid: v1.usb_pid.clone(),
                firmware_build: v1.firmware_build,
                boot_version: v1.boot_version.clone(),
                serial_id: v1.serial_id.clone(),
            },
            recorded_at_utc: v1.recorded_at_utc.clone(),
            settings: BTreeMap::new(),
            raw_artifact: None,
            raw_artifact_sha256: None,
            raw_text: String::new(),
            results: v1
                .results
                .iter()
                .map(|(id, o)| {
                    (
                        id.clone(),
                        CaseEvidence {
                            raw: String::new(),
                            outcome: o.clone(),
                        },
                    )
                })
                .collect(),
            setting_changes: Vec::new(),
            comparison: BTreeMap::new(),
        }
    }
}

/// Writes `evidence_dir/<suite>/<program>.json`; refuses to overwrite.
pub fn save_evidence_v2(evidence_dir: &Path, ev: &EvidenceV2) -> Result<PathBuf, String> {
    if !crate::generate::valid_program_name(&ev.program) {
        return Err(format!("invalid program name `{}`", ev.program));
    }
    if ev.suite.is_empty()
        || !ev
            .suite
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "suite name `{}` is not a safe directory name",
            ev.suite
        ));
    }
    let dir = evidence_dir.join(&ev.suite);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.json", ev.program));
    let json = serde_json::to_string_pretty(ev).map_err(|e| e.to_string())? + "\n";
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            format!(
                "{}: {e} (evidence files are never overwritten)",
                path.display()
            )
        })?;
    std::io::Write::write_all(&mut f, json.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Loads all evidence for a suite: every `evidence_dir/<suite>/*.json`
/// (schema 2) in file-name order, plus the legacy `evidence_dir/<suite>.json`
/// (v1, converted) if present. A missing directory yields no evidence; an
/// unparsable file is an error.
pub fn load_suite_evidence(evidence_dir: &Path, suite: &str) -> Result<Vec<EvidenceV2>, String> {
    let mut out = Vec::new();
    let legacy = evidence_dir.join(format!("{suite}.json"));
    if legacy.is_file() {
        out.push(EvidenceV2::from_v1(&crate::load_evidence(&legacy)?));
    }
    let dir = evidence_dir.join(suite);
    if dir.is_dir() {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        for p in paths {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let ev: EvidenceV2 =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            if ev.schema != 2 {
                return Err(format!("{}: unsupported schema {}", p.display(), ev.schema));
            }
            if ev.suite != suite {
                return Err(format!(
                    "{}: evidence is for suite {}, not {suite}",
                    p.display(),
                    ev.suite
                ));
            }
            out.push(ev);
        }
    }
    Ok(out)
}

/// The latest device result for one case, with its provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedCase {
    pub evidence: CaseEvidence,
    pub program: String,
    pub program_sha256: String,
    pub method: String,
    pub recorded_at_utc: String,
    pub firmware_build: Option<u32>,
    pub device_model: String,
}

/// Per case, the latest result by `recorded_at_utc` (ties: later in the
/// slice wins). A `Missing` record never replaces an observed result.
pub fn merge_evidence(evs: &[EvidenceV2]) -> BTreeMap<String, MergedCase> {
    let mut out: BTreeMap<String, MergedCase> = BTreeMap::new();
    for ev in evs {
        for (id, ce) in &ev.results {
            let cand = MergedCase {
                evidence: ce.clone(),
                program: ev.program.clone(),
                program_sha256: ev.program_sha256.clone(),
                method: ev.method.clone(),
                recorded_at_utc: ev.recorded_at_utc.clone(),
                firmware_build: ev.device.firmware_build,
                device_model: ev.device.model.clone(),
            };
            let replace = match out.get(id) {
                None => true,
                Some(old) => {
                    let cand_missing = cand.evidence.outcome == Outcome::Missing;
                    let old_missing = old.evidence.outcome == Outcome::Missing;
                    match (old_missing, cand_missing) {
                        (false, true) => false,
                        (true, false) => true,
                        _ => cand.recorded_at_utc >= old.recorded_at_utc,
                    }
                }
            };
            if replace {
                out.insert(id.clone(), cand);
            }
        }
    }
    out
}

/// Normalises documented display differences between the calculator and
/// the emulator: `ᴇ` (U+1D07, the Prime's exponent glyph) to `E`, `−`
/// (U+2212 MINUS SIGN) to `-`, and leading/trailing whitespace. Nothing else
/// is normalised. A match that needs this is reported PARTIAL, never
/// VERIFIED.
pub fn normalise_display(s: &str) -> String {
    s.trim().replace('\u{1D07}', "E").replace('\u{2212}', "-")
}

/// Status of one case (see the module documentation).
pub fn status_v2(emu: &Outcome, dev: Option<&Outcome>) -> Status {
    let unsupported = matches!(emu, Outcome::Error { detail } if detail.contains("Unsupported"));
    match dev {
        None if unsupported => Status::Unsupported,
        None => Status::Unverified,
        Some(Outcome::Missing) => Status::Blocked,
        Some(d) if emu.matches(d) => Status::Verified,
        Some(Outcome::Value { ty: dt, text: dx }) => match emu {
            Outcome::Value { ty, text }
                if ty == dt && normalise_display(text) == normalise_display(dx) =>
            {
                Status::Partial
            }
            _ if unsupported => Status::Unsupported,
            _ => Status::Failed,
        },
        Some(_) if unsupported => Status::Unsupported,
        Some(_) => Status::Failed,
    }
}

/// Compares emulator outcomes with merged evidence.
pub fn compare_v2(
    suite: &Suite,
    emu: &BTreeMap<String, Outcome>,
    merged: &BTreeMap<String, MergedCase>,
) -> Report {
    let cases = suite
        .cases
        .iter()
        .map(|c| {
            let e = emu.get(&c.id).cloned().unwrap_or(Outcome::Missing);
            let m = merged.get(&c.id);
            let d = m.map(|m| m.evidence.outcome.clone());
            CaseReport {
                id: c.id.clone(),
                source: c
                    .expr
                    .clone()
                    .unwrap_or_else(|| c.body.clone().unwrap_or_default()),
                status: status_v2(&e, d.as_ref()),
                emulator: e,
                device: d,
                note: c.note.clone(),
                known_divergence: c.known_divergence.clone(),
                device_raw: m.map(|m| m.evidence.raw.clone()),
                evidence_program: m.map(|m| m.program.clone()),
            }
        })
        .collect();
    let latest = merged
        .values()
        .max_by(|a, b| a.recorded_at_utc.cmp(&b.recorded_at_utc));
    let evidence = latest.map(|l| {
        let mut programs: Vec<&str> = merged.values().map(|m| m.program.as_str()).collect();
        programs.sort();
        programs.dedup();
        EvidenceSummary {
            program: programs.join(", "),
            firmware_build: l.firmware_build,
            device_model: l.device_model.clone(),
            recorded_at_utc: l.recorded_at_utc.clone(),
        }
    });
    Report {
        suite: suite.suite.clone(),
        evidence,
        cases,
    }
}
