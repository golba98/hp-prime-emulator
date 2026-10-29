//! Append-only interpretation history. It never edits a physical observation.
use crate::{CaseEvidence, EvidenceV2, RunManifest, sha256_hex};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub program: String,
    pub program_sha256: String,
    pub raw_artifact_sha256: String,
    pub original_transcript_sha256: String,
}

impl Binding {
    pub fn from_evidence(ev: &EvidenceV2) -> Result<Self, String> {
        Ok(Self {
            program: ev.program.clone(),
            program_sha256: ev.program_sha256.clone(),
            raw_artifact_sha256: ev
                .raw_artifact_sha256
                .clone()
                .ok_or("missing artifact hash")?,
            original_transcript_sha256: sha256_hex(ev.raw_text.as_bytes()),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Interpretation {
    Disputed,
    ConfirmedCorrection {
        corrected_transcript: String,
        corrected_transcript_sha256: String,
        user_confirmed: bool,
        confirmation_record: String,
        confirmed_at_utc: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Revision {
    pub id: String,
    pub binding: Binding,
    pub case_ids: Vec<String>,
    pub reason: String,
    pub evidence_links: Vec<String>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    pub interpretation: Interpretation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceObservation {
    pub id: String,
    pub program: String,
    pub authored_source_sha256: String,
    pub observed_source_path: PathBuf,
    pub observed_transport_sha256: String,
    pub observed_utf8_sha256: String,
    /// Exact declared transformation, never a general trim/whitespace rule.
    pub added_prefix_before_opening_comment: String,
    pub single_final_device_nul: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub schema_version: u32,
    #[serde(default)]
    pub revisions: Vec<Revision>,
    #[serde(default)]
    pub source_observations: Vec<SourceObservation>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revisions: Vec::new(),
            source_observations: Vec::new(),
        }
    }
}

impl Registry {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let value: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unknown provenance registry schema".into());
        }
        let mut previous: BTreeMap<&str, &Revision> = BTreeMap::new();
        let mut active: BTreeMap<(String, String), String> = BTreeMap::new();
        for revision in &self.revisions {
            if revision.id.is_empty()
                || revision.reason.is_empty()
                || revision.evidence_links.is_empty()
                || revision.evidence_links.iter().any(String::is_empty)
                || revision.case_ids.is_empty()
                || revision.case_ids.iter().any(String::is_empty)
                || revision.case_ids.iter().collect::<BTreeSet<_>>().len()
                    != revision.case_ids.len()
                || previous.contains_key(revision.id.as_str())
            {
                return Err("invalid or duplicate provenance revision".into());
            }
            for id in &revision.supersedes {
                let old = previous
                    .get(id.as_str())
                    .ok_or("supersession must name an earlier revision")?;
                if old.binding != revision.binding
                    || old.case_ids.iter().any(|c| !revision.case_ids.contains(c))
                {
                    return Err("supersession differs in origin or omits an earlier case".into());
                }
            }
            for case in &revision.case_ids {
                let key = (revision.binding.program.clone(), case.clone());
                if let Some(id) = active.get(&key) {
                    if !revision.supersedes.contains(id) {
                        return Err(
                            "overlapping interpretations require explicit supersession".into()
                        );
                    }
                }
                active.insert(key, revision.id.clone());
            }
            if let Interpretation::ConfirmedCorrection {
                corrected_transcript,
                corrected_transcript_sha256,
                user_confirmed,
                confirmation_record,
                confirmed_at_utc,
            } = &revision.interpretation
            {
                if !*user_confirmed
                    || confirmation_record.is_empty()
                    || confirmed_at_utc.is_empty()
                    || sha256_hex(corrected_transcript.as_bytes()) != *corrected_transcript_sha256
                {
                    return Err(
                        "transcript correction lacks confirmed, hash-bound provenance".into(),
                    );
                }
            }
            previous.insert(&revision.id, revision);
        }
        let mut source_ids = BTreeSet::new();
        let mut programs = BTreeSet::new();
        for source in &self.source_observations {
            if source.id.is_empty()
                || !source_ids.insert(&source.id)
                || !programs.insert(&source.program)
            {
                return Err("duplicate or invalid observed-source proof".into());
            }
        }
        Ok(())
    }
}

/// Append one record while retaining every preceding record verbatim as data.
/// A create-new synced sidecar remains the immutable source of that revision.
pub fn append_revision(path: &Path, revision: &Revision) -> Result<(), String> {
    let parent = path.parent().ok_or("registry needs a parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let lock_path = parent.join("provenance.lock");
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|e| format!("provenance registry is locked or unavailable: {e}"))?;
    struct Lock(PathBuf, Option<std::fs::File>);
    impl Drop for Lock {
        fn drop(&mut self) {
            if let Some(file) = self.1.take() {
                let _ = file.sync_all();
                drop(file);
            }
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _lock = Lock(lock_path, Some(lock));
    let mut registry = Registry::load(path)?;
    registry.revisions.push(revision.clone());
    registry.validate()?;
    std::fs::create_dir_all(parent.join("provenance-records")).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(revision).map_err(|e| e.to_string())?;
    let record = parent
        .join("provenance-records")
        .join(format!("{}.json", sha256_hex(&bytes)));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(record)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    // Never truncate the registry on a failed write. The previous index remains
    // valid; a complete immutable sidecar is retained if index publication fails.
    let tmp = parent.join(format!(
        ".provenance-{}-{}.json",
        std::process::id(),
        sha256_hex(&bytes)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&registry).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

pub struct Applied {
    pub results: BTreeMap<String, CaseEvidence>,
    pub disputed: BTreeSet<String>,
    pub revision_ids: BTreeMap<String, String>,
    pub history: Vec<Revision>,
    pub source: Option<SourceObservation>,
}

pub fn apply(
    registry: &Registry,
    ev: &EvidenceV2,
    manifest: &RunManifest,
    base: &Path,
) -> Result<Applied, String> {
    registry.validate()?;
    let binding = Binding::from_evidence(ev)?;
    let mut result = Applied {
        results: ev.results.clone(),
        disputed: BTreeSet::new(),
        revision_ids: BTreeMap::new(),
        history: Vec::new(),
        source: None,
    };
    for revision in registry
        .revisions
        .iter()
        .filter(|r| r.binding.program == ev.program)
    {
        if revision.binding != binding {
            return Err(format!("provenance binding differs for {}", revision.id));
        }
        if revision
            .case_ids
            .iter()
            .any(|id| !ev.results.contains_key(id))
        {
            return Err("revision names an unobserved case".into());
        }
        match &revision.interpretation {
            Interpretation::Disputed => {
                result.disputed.extend(revision.case_ids.clone());
            }
            Interpretation::ConfirmedCorrection {
                corrected_transcript,
                ..
            } => {
                if ev.method != crate::METHOD_SCREENSHOT {
                    return Err(
                        "machine-readable notes cannot be corrected as screenshot transcriptions"
                            .into(),
                    );
                }
                let parsed = crate::parse_result_lines(corrected_transcript, &manifest.short_ids)?;
                crate::check_run_against_manifest(&parsed, manifest)?;
                if parsed.settings != ev.settings || parsed.setting_changes != ev.setting_changes {
                    return Err("transcript correction changes settings".into());
                }
                for (id, original) in &ev.results {
                    if !revision.case_ids.contains(id) && parsed.results.get(id) != Some(original) {
                        return Err(format!(
                            "transcript correction changes undeclared case {id}"
                        ));
                    }
                }
                for id in &revision.case_ids {
                    result.results.insert(
                        id.clone(),
                        parsed
                            .results
                            .get(id)
                            .ok_or("corrected case missing")?
                            .clone(),
                    );
                    result.disputed.remove(id);
                }
            }
        }
        for id in &revision.case_ids {
            result.revision_ids.insert(id.clone(), revision.id.clone());
        }
        result.history.push(revision.clone());
    }
    if let Some(source) = registry
        .source_observations
        .iter()
        .find(|s| s.program == ev.program)
    {
        if source.authored_source_sha256 != manifest.sha256 || !manifest.source.starts_with("//") {
            return Err(
                "observed-source proof differs from authored comment-leading source".into(),
            );
        }
        let prefix = &source.added_prefix_before_opening_comment;
        if prefix.is_empty()
            || prefix.len() > 64
            || !prefix
                .bytes()
                .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            return Err("observed-source prefix must be exact declared ASCII whitespace".into());
        }
        let raw =
            std::fs::read(base.join(&source.observed_source_path)).map_err(|e| e.to_string())?;
        if sha256_hex(&raw) != source.observed_transport_sha256 || raw.len() % 2 != 0 {
            return Err("observed-source transport hash or UTF-16 length differs".into());
        }
        let units: Vec<_> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut text = String::from_utf16(&units).map_err(|e| e.to_string())?;
        if source.single_final_device_nul {
            text = text
                .strip_suffix('\0')
                .ok_or("observed source lacks declared device NUL")?
                .to_owned();
        }
        if sha256_hex(text.as_bytes()) != source.observed_utf8_sha256
            || text != format!("{prefix}{}", manifest.source)
        {
            return Err("observed-source bytes differ beyond the declared comment prefix".into());
        }
        result.source = Some(source.clone());
    }
    Ok(result)
}
