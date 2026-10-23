//! Persistent, resumable test campaigns.
//!
//! A campaign walks the groups (device-program batches) of one corpus suite
//! through the hardware stages, remembering per-group progress in a JSON
//! ledger so a campaign survives process exit, reboot and USB disconnects.
//! Groups are advanced one stage at a time and never blindly repeated:
//!
//! ```text
//! Pending → Prepared → Uploaded → Captured → Done
//!                ↘ Blocked (refused; nothing written)
//! ```
//!
//! * `Prepared` — a device program exists and is frozen in a run manifest
//!   (`tests/compat/runs`); nothing has touched the calculator.
//! * `Uploaded` — the guarded preflight passed and the read-back matched
//!   (the user still has to press Run on the calculator: `EC` key injection
//!   is not effective on G1 firmware 15157).
//! * `Captured` — the result note was fetched and saved as evidence.
//! * `Done` — the evidence was compared against the emulator; per-case
//!   match counts are stored on the group.
//!
//! Budgets (`max_uploads`) bound how many groups may ever be uploaded in
//! this campaign, independent of how many were prepared.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Suite, utc_now};

/// Default directory for campaign ledgers (git-ignored).
pub const DEFAULT_CAMPAIGNS_DIR: &str = "artifacts/campaigns";

/// One group's progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupState {
    pub group: String,
    pub case_ids: Vec<String>,
    pub stage: Stage,
    /// Run-manifest program name once prepared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// `blocked`/`done` explanation, mismatch details, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_utc: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Not prepared yet.
    Pending,
    /// Device program generated and frozen in a run manifest.
    Prepared,
    /// Uploaded with confirmed read-back (user must run it by hand).
    Uploaded,
    /// Result note fetched and saved as evidence.
    Captured,
    /// Evidence compared against the emulator.
    Done,
    /// Refused by a safety check; nothing was written to the calculator.
    Blocked,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Pending => "pending",
            Stage::Prepared => "prepared",
            Stage::Uploaded => "uploaded",
            Stage::Captured => "captured",
            Stage::Done => "done",
            Stage::Blocked => "blocked",
        }
    }
}

/// Per-group differential outcome once captured.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupOutcome {
    pub matches: usize,
    /// Hardware matches after display normalisation only (e.g. the G1's
    /// Unicode minus; `Status::Partial`).
    #[serde(default)]
    pub display_equal: usize,
    pub mismatches: usize,
    pub hardware_errors: usize,
    pub emulator_errors: usize,
    pub missing: usize,
    /// Case ids that mismatched.
    pub mismatch_ids: Vec<String>,
}

/// The campaign ledger (one JSON file per campaign).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CampaignState {
    pub id: String,
    pub suite: String,
    pub corpus_path: String,
    pub corpus_sha256: String,
    pub created_at_utc: String,
    /// Hard upper bound on uploads in this campaign.
    pub max_uploads: usize,
    /// Total `F7` writes ever performed in this campaign (monotonic; a
    /// group reset never reduces it — every write is a real flash write).
    #[serde(default)]
    pub uploads_total: usize,
    pub groups: Vec<GroupState>,
    /// Offline emulator pass: case id → outcome summary (value/error text).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub offline: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_utc: Option<String>,
}

impl CampaignState {
    /// Plans a campaign over every group of `suite`.
    pub fn new(
        id: &str,
        suite: &Suite,
        corpus_path: &Path,
        corpus_sha256: &str,
        max_uploads: usize,
    ) -> CampaignState {
        let groups: Vec<GroupState> = suite
            .groups
            .iter()
            .map(|g| {
                let case_ids: Vec<String> = suite
                    .cases
                    .iter()
                    .filter(|c| c.group.as_deref() == Some(&g.id))
                    .map(|c| c.id.clone())
                    .collect();
                GroupState {
                    group: g.id.clone(),
                    case_ids,
                    stage: Stage::Pending,
                    program: None,
                    note: None,
                    updated_at_utc: None,
                }
            })
            .collect();
        CampaignState {
            id: id.to_string(),
            suite: suite.suite.clone(),
            corpus_path: corpus_path.display().to_string(),
            corpus_sha256: corpus_sha256.to_string(),
            created_at_utc: utc_now(),
            max_uploads,
            uploads_total: 0,
            groups,
            offline: BTreeMap::new(),
            finished_at_utc: None,
        }
    }

    /// Writes ever performed (monotonic), used for budget checks.
    pub fn uploads_used(&self) -> usize {
        self.uploads_total.max(
            self.groups
                .iter()
                .filter(|g| matches!(g.stage, Stage::Uploaded | Stage::Captured | Stage::Done))
                .count(),
        )
    }

    /// Records one real upload (called after a confirmed read-back).
    pub fn record_upload(&mut self) {
        self.uploads_total += 1;
    }

    pub fn uploads_left(&self) -> usize {
        self.max_uploads.saturating_sub(self.uploads_used())
    }

    pub fn stage_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut m: BTreeMap<&'static str, usize> = BTreeMap::new();
        for g in &self.groups {
            *m.entry(g.stage.as_str()).or_insert(0) += 1;
        }
        m
    }

    pub fn group(&self, id: &str) -> Option<&GroupState> {
        self.groups.iter().find(|g| g.group == id)
    }

    pub fn group_mut(&mut self, id: &str) -> Option<&mut GroupState> {
        self.groups.iter_mut().find(|g| g.group == id)
    }

    /// The first group in `stage`, in plan order.
    pub fn first_at(&self, stage: Stage) -> Option<&GroupState> {
        self.groups.iter().find(|g| g.stage == stage)
    }

    /// Marks a group's stage and stamps the time.
    pub fn advance(&mut self, group: &str, stage: Stage, note: Option<String>) {
        if let Some(g) = self.group_mut(group) {
            g.stage = stage;
            g.note = note;
            g.updated_at_utc = Some(utc_now());
        }
    }

    /// Stores the offline emulator outcome of one case.
    pub fn record_offline(&mut self, case_id: &str, summary: String) {
        self.offline.insert(case_id.to_string(), summary);
    }

    // -- persistence -----------------------------------------------------

    fn path(dir: &Path, id: &str) -> PathBuf {
        dir.join(format!("{id}.json"))
    }

    pub fn save(&self, dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = Self::path(dir, &self.id);
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn load(dir: &Path, id: &str) -> Result<CampaignState, String> {
        let path = Self::path(dir, id);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("bad campaign file: {e}"))
    }

    /// Human-readable summary for `status`.
    pub fn summary(&self) -> String {
        let counts = self.stage_counts();
        let parts: Vec<String> = counts.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        format!(
            "campaign {} (suite `{}`, {} groups / {} cases)\n  stages: {}\n  uploads: {}/{}{}\n  offline results: {}",
            self.id,
            self.suite,
            self.groups.len(),
            self.groups.iter().map(|g| g.case_ids.len()).sum::<usize>(),
            parts.join(", "),
            self.uploads_used(),
            self.max_uploads,
            if self.uploads_left() == 0 {
                " (budget exhausted)"
            } else {
                ""
            },
            self.offline.len(),
        )
    }
}

/// Result of the device-free part of a capture: the evidence record, the
/// per-group outcome tally and the emulator expectations used for the
/// mismatch report.
pub struct CaptureOutcome {
    pub evidence: crate::EvidenceV2,
    pub tally: GroupOutcome,
    pub emulator: BTreeMap<String, crate::Outcome>,
}

/// Runs the complete device-free capture pipeline for one group.
///
/// Parses the result-note transcript, builds the evidence record, runs the
/// identical cases in the emulator, classifies every case and tallies the
/// group. Device I/O, saving the evidence file and campaign bookkeeping
/// stay with the caller (the CLI), so this is fully unit-testable.
pub fn capture_note_text(
    m: &crate::RunManifest,
    note_text: &str,
    suite: &Suite,
    device: crate::DeviceRecord,
    utc: &str,
) -> Result<CaptureOutcome, String> {
    let parsed = crate::parse_result_lines(note_text, &m.short_ids)?;
    let mut ev = crate::EvidenceV2::from_run(
        m,
        &parsed,
        crate::METHOD_NOTE_FETCH,
        device,
        note_text.to_string(),
        utc,
    )?;
    let emu = crate::run_emulator(suite);
    ev.compare_with(&emu);
    let mut tally = GroupOutcome::default();
    for (cid, st) in &ev.comparison {
        match st.as_str() {
            "VERIFIED" => tally.matches += 1,
            "PARTIAL" => tally.display_equal += 1,
            "FAILED" => {
                tally.mismatches += 1;
                tally.mismatch_ids.push(cid.clone());
            }
            _ => tally.missing += 1,
        }
    }
    for (cid, out) in &emu {
        if m.case_ids.contains(cid) && matches!(out, crate::Outcome::Error { .. }) {
            tally.emulator_errors += 1;
        }
    }
    Ok(CaptureOutcome {
        evidence: ev,
        tally,
        emulator: emu,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{CATEGORIES, generate_corpus};

    fn corpus_suite(seed: u64) -> crate::Suite {
        generate_corpus(40, seed, &[]).unwrap().suite
    }

    #[test]
    fn plans_every_group_with_its_cases() {
        let suite = corpus_suite(3);
        let c = CampaignState::new("c1", &suite, Path::new("x.toml"), "deadbeef", 10);
        assert_eq!(c.groups.len(), suite.groups.len());
        for g in &c.groups {
            let expect: usize = suite
                .cases
                .iter()
                .filter(|x| x.group.as_deref() == Some(&g.group))
                .count();
            assert_eq!(g.case_ids.len(), expect);
            assert_eq!(g.stage, Stage::Pending);
        }
    }

    #[test]
    fn stages_advance_and_persist() {
        let suite = corpus_suite(4);
        let dir = std::env::temp_dir().join(format!("zzt-camp-{}", std::process::id()));
        let mut c = CampaignState::new("c2", &suite, Path::new("x.toml"), "aa", 5);
        let g = c.first_at(Stage::Pending).unwrap().group.clone();
        c.advance(&g, Stage::Prepared, Some("manifest saved".into()));
        c.group_mut(&g).unwrap().program = Some("ZZT_TEST1".into());
        c.record_offline("whatever", "1 (type 0)".into());
        c.save(&dir).unwrap();
        let loaded = CampaignState::load(&dir, "c2").unwrap();
        assert_eq!(loaded.groups[0].stage, Stage::Prepared);
        assert_eq!(loaded.groups[0].program.as_deref(), Some("ZZT_TEST1"));
        assert_eq!(
            loaded.offline.get("whatever").map(String::as_str),
            Some("1 (type 0)")
        );
        assert!(loaded.summary().contains("prepared: 1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upload_budget_is_enforced_by_counting() {
        let suite = corpus_suite(5);
        let mut c = CampaignState::new("c3", &suite, Path::new("x"), "bb", 2);
        assert_eq!(c.uploads_left(), 2);
        let ids: Vec<String> = c.groups.iter().take(3).map(|g| g.group.clone()).collect();
        for (i, gid) in ids.iter().enumerate() {
            // simulate two uploaded groups and one done
            let stage = if i == 2 { Stage::Done } else { Stage::Uploaded };
            c.advance(gid, stage, None);
        }
        assert_eq!(c.uploads_used(), 3);
        assert_eq!(c.uploads_left(), 0);
        assert!(c.summary().contains("budget exhausted"));
    }

    #[test]
    fn first_at_returns_plan_order() {
        let suite = corpus_suite(6);
        let mut c = CampaignState::new("c4", &suite, Path::new("x"), "cc", 9);
        let first = c.groups[0].group.clone();
        c.advance(&first, Stage::Prepared, None);
        assert_eq!(c.first_at(Stage::Prepared).unwrap().group, first);
        assert_ne!(c.first_at(Stage::Pending).unwrap().group, first);
        // resume: the first pending group is the second in plan order
        assert_eq!(c.first_at(Stage::Pending).unwrap().group, c.groups[1].group);
    }

    #[test]
    fn corpus_categories_exist() {
        assert!(CATEGORIES.contains(&"num_basic"));
        assert!(CATEGORIES.contains(&"errors"));
    }

    #[test]
    fn capture_note_text_pipeline_classifies_and_tallies() {
        use crate::{
            Channel, DeviceRecord, Outcome, RunManifest, capture_note_text,
            generate_device_program, run_emulator, select_cases,
        };
        let suite = corpus_suite(9);
        let cases = select_cases(&suite, Some("num_basic_b1"), None).unwrap();
        assert!(!cases.is_empty());
        let ch = Channel::Note {
            note: "ZZT_CAPTR".into(),
        };
        let g = generate_device_program(&suite, &cases, "ZZT_CAPT1", &ch).unwrap();
        let m = RunManifest::prepared(
            &suite.suite,
            Some("num_basic_b1"),
            &g,
            "2026-10-23T00:00:00Z",
        );
        let emu = run_emulator(&suite);

        // Synthesize the transcript exactly as the device program writes
        // it, from the emulator's own results: every case must match.
        let short_of: BTreeMap<&str, &str> = g
            .short_ids
            .iter()
            .map(|(s, id)| (id.as_str(), s.as_str()))
            .collect();
        let mut text = format!("@@ZZT|ZZT_CAPT1|{}|0|0|12", g.sha8);
        for case in &cases {
            let sid = short_of[case.id.as_str()];
            let line = match &emu[&case.id] {
                Outcome::Value { ty, text: v } => format!("{sid}|{ty}|{v}"),
                Outcome::Error { .. } => format!("{sid}|E|"),
                _ => unreachable!("offline corpus cases always ran"),
            };
            text.push('\n');
            text.push_str(&line);
        }
        text.push_str("\n@@END\n");

        let device = DeviceRecord {
            model: "HP Prime G1".into(),
            usb_pid: "03f0:1541".into(),
            firmware_build: Some(15157),
            boot_version: None,
            serial_id: None,
        };
        let cap = capture_note_text(&m, &text, &suite, device, "2026-10-23T00:01:00Z").unwrap();
        assert_eq!(cap.tally.matches, cases.len(), "all cases must verify");
        assert_eq!(cap.tally.mismatches, 0);
        assert_eq!(cap.tally.missing, 0);
        assert!(cap.tally.mismatch_ids.is_empty());

        // One corrupted value: exactly one FAILED, tallied and reported.
        let first_value_line = text.lines().nth(1).unwrap().to_string();
        let corrupted = text.replacen(&first_value_line, "c1|0|999999", 1);
        let cap2 = capture_note_text(
            &m,
            &corrupted,
            &suite,
            DeviceRecord {
                model: "HP Prime G1".into(),
                usb_pid: "03f0:1541".into(),
                firmware_build: Some(15157),
                boot_version: None,
                serial_id: None,
            },
            "t",
        )
        .unwrap();
        assert_eq!(cap2.tally.mismatches, 1, "corrupted case must fail");
        let failed = cases[0].id.clone();
        assert!(cap2.tally.mismatch_ids.contains(&failed));
    }
}
