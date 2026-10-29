//! Audited physical-reference comparisons. No USB access or hardware writes.
//! Equality here covers TYPE/STRING observations only, not complete semantics.
use crate::{CaseReport, Channel, EvidenceV2, Outcome, Report, RunManifest, Status, Suite};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[path = "assessment_provenance.rs"]
pub mod provenance;

#[derive(Debug, Serialize, Deserialize)]
pub struct Audit {
    pub program: String,
    pub accepted: bool,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Observation {
    pub program: String,
    pub settings: BTreeMap<String, String>,
    pub physical: Outcome,
    pub native: Outcome,
    pub status: Status,
    /// A capture can be valid while this interpretation is disputed.
    #[serde(default)]
    pub disputed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_physical: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_observation: Option<provenance::SourceObservation>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub schema_version: u32,
    pub suite_sha256: String,
    pub comparison_scope: String,
    pub audits: Vec<Audit>,
    pub observations: BTreeMap<String, Vec<Observation>>,
    #[serde(default)]
    pub interpretation_history: Vec<provenance::Revision>,
    pub report: Report,
}

/// Fail closed when any source, settings, raw artifact or transcript identity
/// differs. Re-generation deliberately refuses old, changed harness wrappers.
pub fn qualify(
    suite: &Suite,
    ev: &EvidenceV2,
    manifest: &RunManifest,
    artifact: &[u8],
) -> Result<ppl_vm::Settings, String> {
    manifest.verify_source()?;
    if ev.schema != 2
        || ev.suite != suite.suite
        || manifest.suite != suite.suite
        || ev.program != manifest.program
        || ev.program_sha256 != manifest.sha256
    {
        return Err("suite/program/source identity differs".into());
    }
    if ev.device.firmware_build != Some(15157)
        || ev.device.usb_pid != "03f0:1541"
        || ev.device.boot_version.as_deref() != Some("V0.050.640")
        || ev.device.serial_id.as_ref().is_none_or(String::is_empty)
        || manifest.device.as_ref().is_some_and(|d| d != &ev.device)
    {
        return Err("reference device identity is missing or differs from G1 build 15157".into());
    }
    if !matches!(
        ev.method.as_str(),
        crate::METHOD_NOTE_FETCH | crate::METHOD_SCREENSHOT
    ) || ev.raw_artifact_sha256.as_deref() != Some(crate::sha256_hex(artifact).as_str())
    {
        return Err("physical method or raw artifact hash is not established".into());
    }
    let mut cases = Vec::new();
    for id in &manifest.case_ids {
        cases.push(
            suite
                .cases
                .iter()
                .find(|c| c.id == *id)
                .ok_or_else(|| format!("current suite has no case {id}"))?,
        );
    }
    let channel = match manifest.channel.as_str() {
        "print" => Channel::Print,
        "note" => Channel::Note {
            note: manifest.note.clone().ok_or("missing note name")?,
        },
        _ => return Err("unknown evidence channel".into()),
    };
    let generated = crate::generate_device_program(suite, &cases, &manifest.program, &channel)
        .map_err(|e| e.join("; "))?;
    if generated.source != manifest.source || generated.short_ids != manifest.short_ids {
        return Err("current cases or generator differ from captured source".into());
    }
    let parsed = crate::parse_result_lines(&ev.raw_text, &manifest.short_ids)?;
    crate::check_run_against_manifest(&parsed, manifest)?;
    if parsed.results != ev.results
        || parsed.settings != ev.settings
        || parsed.setting_changes != ev.setting_changes
        || !ev.setting_changes.is_empty()
    {
        return Err(
            "transcript/results/settings differ or settings changed during the batch".into(),
        );
    }
    // Note bytes are machine-readable. Screenshots still require the explicitly
    // confirmed transcription; their hash does not itself prove that text.
    if channel.as_str() == "note" {
        if artifact.len() % 2 != 0 {
            return Err("odd UTF-16LE note".into());
        }
        let units: Vec<_> = artifact
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let text = String::from_utf16(&units).map_err(|e| e.to_string())?;
        if text.trim_end_matches('\0') != ev.raw_text.trim_end_matches('\0') {
            return Err("raw note and recorded transcript differ".into());
        }
    }
    let setting = |name: &str| -> Result<i64, String> {
        ev.settings
            .get(name)
            .ok_or_else(|| format!("missing {name}"))?
            .parse()
            .map_err(|_| format!("invalid {name}"))
    };
    let angle = ppl_vm::AngleMode::from_code(setting("HAngle")?).ok_or("unknown angle mode")?;
    let format = setting("HFormat")?;
    let digits = setting("HDigits")?;
    if !(0..=3).contains(&format) || !(0..=11).contains(&digits) {
        return Err("unsupported reference number format".into());
    }
    let mut settings = ppl_vm::Settings {
        angle,
        ..Default::default()
    };
    settings.set_format(format, digits);
    Ok(settings)
}

pub fn assess(
    suite: &Suite,
    evidence: &[EvidenceV2],
    runs: &Path,
    evidence_dir: &Path,
) -> Assessment {
    let mut audits = Vec::new();
    let mut observations: BTreeMap<String, Vec<Observation>> = BTreeMap::new();
    let mut excluded = std::collections::BTreeSet::new();
    let registry = provenance::Registry::load(&evidence_dir.join("provenance.json"));
    let mut interpretation_history = Vec::new();
    for ev in evidence {
        let qualification = (|| {
            let manifest = crate::load_manifest(runs, &ev.program)?;
            let path = Path::new(
                ev.raw_artifact
                    .as_deref()
                    .ok_or("missing raw artifact path")?,
            );
            let artifact = std::fs::read(path)
                .or_else(|_| std::fs::read(evidence_dir.join(path)))
                .map_err(|e| format!("raw artifact unavailable: {e}"))?;
            let settings = qualify(suite, ev, &manifest, &artifact)?;
            let applied = provenance::apply(
                registry.as_ref().map_err(Clone::clone)?,
                ev,
                &manifest,
                evidence_dir,
            )?;
            Ok::<_, String>((settings, applied))
        })();
        match qualification {
            Err(reason) => {
                excluded.extend(ev.results.keys().cloned());
                audits.push(Audit {
                    program: ev.program.clone(),
                    accepted: false,
                    reason,
                });
            }
            Ok((settings, applied)) => {
                for revision in &applied.history {
                    if !interpretation_history
                        .iter()
                        .any(|r: &provenance::Revision| r.id == revision.id)
                    {
                        interpretation_history.push(revision.clone());
                    }
                }
                audits.push(Audit { program:ev.program.clone(), accepted:true,
                    reason:"source, wrapper, device, artifact and complete transcript validated; TYPE/STRING scope".into() });
                for case in &suite.cases {
                    if let Some(reference) = applied.results.get(&case.id) {
                        let native = crate::run_case_with_settings(case, settings.clone());
                        let mut status = crate::status_v2(&native, Some(&reference.outcome));
                        // IFERR records no error identity or recovery effects.
                        if matches!(
                            (&native, &reference.outcome),
                            (Outcome::Error { .. }, Outcome::Error { .. })
                        ) {
                            status = Status::Inconclusive;
                        }
                        let disputed = applied.disputed.contains(&case.id);
                        if disputed {
                            status = Status::Inconclusive;
                        }
                        observations
                            .entry(case.id.clone())
                            .or_default()
                            .push(Observation {
                                program: ev.program.clone(),
                                settings: ev.settings.clone(),
                                physical: reference.outcome.clone(),
                                native,
                                status,
                                disputed,
                                interpretation_id: applied.revision_ids.get(&case.id).cloned(),
                                original_physical: ev
                                    .results
                                    .get(&case.id)
                                    .filter(|old| *old != reference)
                                    .map(|old| old.outcome.clone()),
                                source_observation: applied.source.clone(),
                            });
                    }
                }
            }
        }
    }
    let cases = suite
        .cases
        .iter()
        .map(|case| {
            let obs = observations
                .get(&case.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let emulator = obs
                .first()
                .map(|o| o.native.clone())
                .unwrap_or_else(|| crate::run_case(case));
            let active: Vec<_> = obs.iter().filter(|o| !o.disputed).collect();
            let conflict = active.iter().any(|a| {
                active
                    .iter()
                    .any(|b| a.settings == b.settings && a.physical != b.physical)
            });
            let status = if conflict {
                Status::Unstable
            } else if active.iter().any(|o| o.status == Status::Failed) {
                Status::Failed
            } else if active.iter().any(|o| o.status == Status::Inconclusive) {
                Status::Inconclusive
            } else if active.iter().any(|o| o.status == Status::Partial) {
                Status::Partial
            } else if let Some(o) = active.first() {
                o.status
            } else if !obs.is_empty() || excluded.contains(&case.id) {
                Status::Inconclusive
            } else {
                crate::status_v2(&emulator, None)
            };
            CaseReport {
                id: case.id.clone(),
                source: case
                    .expr
                    .clone()
                    .or_else(|| case.body.clone())
                    .unwrap_or_default(),
                emulator,
                device: obs.first().map(|o| o.physical.clone()),
                status,
                note: case.note.clone(),
                known_divergence: case.known_divergence.clone(),
                device_raw: None,
                evidence_program: (!obs.is_empty()).then(|| {
                    obs.iter()
                        .map(|o| o.program.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }),
            }
        })
        .collect();
    let accepted: Vec<_> = evidence
        .iter()
        .filter(|ev| audits.iter().any(|a| a.accepted && a.program == ev.program))
        .collect();
    let summary = accepted.last().map(|ev| crate::EvidenceSummary {
        program: accepted
            .iter()
            .map(|e| e.program.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        firmware_build: ev.device.firmware_build,
        device_model: ev.device.model.clone(),
        recorded_at_utc: ev.recorded_at_utc.clone(),
    });
    Assessment { schema_version:1, suite_sha256:crate::sha256_hex(&serde_json::to_vec(suite).expect("serializable suite")),
        comparison_scope:"TYPE and STRING observations; no claim of full values, side effects or error equivalence".into(),
        audits, observations, interpretation_history, report:Report { suite:suite.suite.clone(), evidence:summary, cases } }
}
