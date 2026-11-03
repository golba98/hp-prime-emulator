//! Replay of qualified immutable physical TYPE/STRING evidence. Never USB I/O.
use crate::{EvidenceV2, Outcome, RunManifest, Suite};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

pub struct PhysicalReplay {
    source_sha256: String,
    capture_sha256: String,
    firmware_build: u32,
    results: BTreeMap<String, Outcome>,
    settings: ppl_vm::Settings,
    original: BTreeMap<String, Outcome>,
    disputed: std::collections::BTreeSet<String>,
    history: Vec<crate::assessment::provenance::Revision>,
    source_observation: Option<crate::assessment::provenance::SourceObservation>,
}
/// Validates the same source, original raw artifact, transcript and settings
/// contracts as the existing physical assessment. No self-declared backend tag
/// or local-firmware report can become physical evidence through this adapter.
pub fn load_physical_replay(
    suite: &Suite,
    evidence_path: &Path,
    manifest_path: &Path,
    evidence_dir: &Path,
) -> Result<PhysicalReplay, String> {
    let evidence: EvidenceV2 =
        serde_json::from_slice(&std::fs::read(evidence_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let manifest: RunManifest =
        serde_json::from_slice(&std::fs::read(manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let artifact_path = evidence
        .raw_artifact
        .as_ref()
        .ok_or("capture has no raw artifact")?;
    let raw = std::fs::read(artifact_path)
        .or_else(|_| std::fs::read(evidence_dir.join(artifact_path)))
        .map_err(|e| format!("physical raw artifact unavailable: {e}"))?;
    let settings = crate::assessment::qualify(suite, &evidence, &manifest, &raw)?;
    let registry =
        crate::assessment::provenance::Registry::load(&evidence_dir.join("provenance.json"))?;
    let applied =
        crate::assessment::provenance::apply(&registry, &evidence, &manifest, evidence_dir)?;
    Ok(PhysicalReplay {
        source_sha256: manifest.sha256,
        capture_sha256: crate::sha256_hex(&raw),
        firmware_build: evidence.device.firmware_build.unwrap(),
        original: evidence
            .results
            .iter()
            .map(|(id, c)| (id.clone(), c.outcome.clone()))
            .collect(),
        disputed: applied.disputed,
        history: applied.history,
        source_observation: applied.source,
        results: applied
            .results
            .into_iter()
            .map(|(id, c)| (id, c.outcome))
            .collect(),
        settings,
    })
}
#[derive(Debug, Serialize)]
pub struct LegacyCaseComparison {
    pub case: String,
    pub physical: Outcome,
    pub native: Outcome,
    pub matched: bool,
    pub scope: String,
    pub disputed: bool,
    pub original_physical: Option<Outcome>,
}
#[derive(Debug, Serialize)]
pub struct LegacyReplayReport {
    pub schema_version: u32,
    pub evidence_origin: String,
    pub scope: String,
    pub source_sha256: String,
    pub capture_sha256: String,
    pub firmware_build: u32,
    pub passed: bool,
    pub cases: Vec<LegacyCaseComparison>,
    pub interpretation_history: Vec<crate::assessment::provenance::Revision>,
    pub source_observation: Option<crate::assessment::provenance::SourceObservation>,
}
/// Strict values; IFERR-only error records cannot establish error-detail parity.
/// This API does not modify legacy evidence or legacy status classifications.
pub fn replay_legacy_cases(suite: &Suite, capture: &PhysicalReplay) -> LegacyReplayReport {
    let cases = capture
        .results
        .iter()
        .map(|(id, physical)| {
            let native = suite
                .cases
                .iter()
                .find(|c| c.id == *id)
                .map(|c| crate::run_case_with_settings(c, capture.settings.clone()))
                .unwrap_or(Outcome::Missing);
            let disputed = capture.disputed.contains(id);
            let matched = !disputed && matches_record(&native, physical);
            LegacyCaseComparison {
                case: id.clone(),
                physical: physical.clone(),
                native,
                matched,
                disputed,
                original_physical: capture.original.get(id).filter(|o| *o != physical).cloned(),
                scope: if disputed {
                    "disputed physical interpretation"
                } else if matches!(physical, Outcome::Error { .. }) {
                    "error category recorded; detail unobserved"
                } else {
                    "exact TYPE/STRING"
                }
                .into(),
            }
        })
        .collect::<Vec<_>>();
    LegacyReplayReport {
        schema_version: 1,
        evidence_origin: "qualified-physical-capture".into(),
        scope: "legacy TYPE/STRING only; no inferred variables, graphics or persistent state"
            .into(),
        source_sha256: capture.source_sha256.clone(),
        capture_sha256: capture.capture_sha256.clone(),
        firmware_build: capture.firmware_build,
        passed: !cases.is_empty() && cases.iter().all(|c| c.matched),
        cases,
        interpretation_history: capture.history.clone(),
        source_observation: capture.source_observation.clone(),
    }
}

fn matches_record(native: &Outcome, physical: &Outcome) -> bool {
    matches!((native,physical),(Outcome::Value{ty:a,text:x},Outcome::Value{ty:b,text:y}) if a==b&&x==y)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iferr_only_records_do_not_prove_error_detail_parity() {
        assert!(!matches_record(
            &Outcome::Error {
                detail: "DivisionByZero".into()
            },
            &Outcome::Error {
                detail: String::new()
            }
        ));
        assert!(!matches_record(
            &Outcome::Value {
                ty: 0,
                text: "1".into()
            },
            &Outcome::Value {
                ty: 2,
                text: "1".into()
            }
        ));
    }
}
