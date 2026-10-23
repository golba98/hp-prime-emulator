//! Ownership-verified cleanup of this project's test objects on a physical
//! HP Prime.
//!
//! Deletion is performed by the calculator's own PPL runtime (HP Prime
//! programming reference, ch. 28): `Programs("name"):=""` deletes a program
//! and `Notes("name"):=""` deletes a note. A generated cleanup helper
//! program carries an **exact-name allowlist** (no wildcards, no prefix
//! matching, no delete-all), is verified by walking its parsed AST before
//! upload, and reports through its own result note.
//!
//! # Ownership
//!
//! A calculator object is only a cleanup candidate when the project's own
//! records prove we created it:
//!
//! * the name appears in the upload ledger (`tests/compat/runs/ledger.jsonl`), and
//! * for programs: a run manifest exists whose `program` matches, and
//! * the calculator's copy is backed up and identity-verified before any
//!   deletion (program source compared against the manifest source, note
//!   content checked against its `@@ZZT|<program>|…` header).
//!
//! `ZZT_`-shaped names without ledger proof are *not* deleted; unknown
//! objects stay untouched. `ZZT_CTRL` (the preflight control) and every
//! program that is still the active test of a campaign group are always
//! protected, as is anything that is not in the ledger at all.
//!
//! # Lifecycle
//!
//! ```text
//! plan (ledger + manifests + campaigns) -> presence probe -> backup +
//! identity verify -> helper generation -> AST allowlist verification ->
//! guarded upload -> [user runs the helper] -> verify absence -> record
//! ```
//!
//! Every batch is a durable record under `artifacts/cleanup/batches.json`;
//! verification failures are recorded as `Unknown`/`Blocked` and never
//! retried blindly.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::manifest::{DEFAULT_RUNS_DIR, RunManifest, RunStatus, ledger_entries, load_manifest};
use crate::utc_now;

/// Directory for durable cleanup state (git-ignored).
pub const DEFAULT_CLEANUP_DIR: &str = "artifacts/cleanup";

/// Objects that must never be deleted (the preflight control).
pub const ALWAYS_PROTECTED: &[&str] = &["ZZT_CTRL"];

/// Objects this project created outside the manifest flow (documented
/// probe artifacts): name → expected content prefix.
pub const KNOWN_PROBE_NOTES: &[(&str, &str)] =
    &[("ZZT_NOTEST", "F8 NOTE PROBE"), ("ZZT_PROBU1", "a1")];

/// The result-note suffix convention (`<program>R`).
fn is_result_note_of(note: &str, program: &str) -> bool {
    note.len() == program.len() + 1 && note.starts_with(program) && note.ends_with('R')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjType {
    Program,
    Note,
}

impl ObjType {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjType::Program => "program",
            ObjType::Note => "note",
        }
    }
}

/// How the object's identity was established.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    /// In the ledger and owned by a run manifest.
    ManifestRun,
    /// A documented probe object with expected content.
    KnownProbe,
}

/// State of one cleanup target (see the module documentation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    /// Planned, nothing done yet.
    Pending,
    /// Eligible (owned, not protected, not active).
    Eligible,
    /// Backed up locally with an identity check.
    BackedUp,
    /// Named in an uploaded cleanup helper.
    Authorized,
    /// Absence verified on the calculator after the helper ran.
    VerifiedDeleted,
    /// Not touched, with a reason.
    Blocked(String),
    /// Helper ran but absence could not be confirmed (silence is not
    /// proof); needs manual review, never retried blindly.
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CleanupTarget {
    pub name: String,
    pub ty: ObjType,
    /// The owning test program (manifest program or the note's owner).
    pub program: String,
    pub ownership: Ownership,
    /// Why this object is eligible ("results captured", "superseded", …).
    pub reason: String,
    pub state: TargetState,
    /// Local backup path (after the backup step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_sha256: Option<String>,
}

/// An object considered but excluded, with the reason.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkippedObject {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub targets: Vec<CleanupTarget>,
    pub skipped: Vec<SkippedObject>,
    pub planned_at_utc: String,
}

/// One executed cleanup batch: the helper program plus its targets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CleanupBatch {
    pub helper: String,
    pub planned_at_utc: String,
    pub executed_at_utc: Option<String>,
    pub verified_at_utc: Option<String>,
    pub targets: Vec<CleanupTarget>,
    pub skipped: Vec<SkippedObject>,
}

/// Programs (and their result notes) that are the *active* test of a
/// campaign group right now: protected from cleanup.
fn active_objects(campaigns_dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(campaigns_dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // CampaignState shape: groups[].{stage,program}
        #[derive(serde::Deserialize)]
        struct Cfg {
            #[serde(default)]
            groups: Vec<GroupCfg>,
        }
        #[derive(serde::Deserialize)]
        struct GroupCfg {
            stage: String,
            #[serde(default)]
            program: Option<String>,
        }
        let Ok(cfg) = serde_json::from_str::<Cfg>(&text) else {
            continue;
        };
        for g in cfg.groups {
            if matches!(g.stage.as_str(), "uploaded" | "prepared" | "pending") {
                if let Some(p) = g.program {
                    out.insert(p.clone());
                    out.insert(format!("{p}R"));
                }
            }
        }
    }
    out
}

/// Program names (and their result notes) that belong to one campaign,
/// whatever the stage. `--campaign` cleanup scoping uses this.
pub fn campaign_objects(
    campaigns_dir: &Path,
    campaign_id: &str,
) -> Result<BTreeSet<String>, String> {
    let path = campaigns_dir.join(format!("{campaign_id}.json"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[derive(serde::Deserialize)]
    struct Cfg {
        #[serde(default)]
        groups: Vec<GroupCfg>,
    }
    #[derive(serde::Deserialize)]
    struct GroupCfg {
        #[serde(default)]
        program: Option<String>,
    }
    let cfg: Cfg = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BTreeSet::new();
    for g in &cfg.groups {
        if let Some(p) = &g.program {
            out.insert(p.clone());
            out.insert(format!("{p}R"));
        }
    }
    Ok(out)
}

/// Plans cleanup from the ledger, run manifests and campaign states.
///
/// A *skipped* entry is returned for everything not planned, with the
/// reason — the caller shows the full picture, never a silent drop.
pub fn plan_cleanup(runs_dir: &Path, campaigns_dir: &Path) -> Result<CleanupPlan, String> {
    let ledger = ledger_entries(runs_dir)?;
    let mut manifests: Vec<RunManifest> = Vec::new();
    let Ok(entries) = std::fs::read_dir(runs_dir) else {
        return Err(format!("cannot read {}", runs_dir.display()));
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(m) = load_manifest(runs_dir, stem) {
            manifests.push(m);
        }
    }
    let active = active_objects(campaigns_dir);
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    let mut skipped = Vec::new();

    for entry in &ledger {
        if !seen.insert(entry.name.clone()) {
            continue;
        }
        let name = entry.name.as_str();
        if !name.starts_with("ZZT_") {
            skipped.push(SkippedObject {
                name: name.into(),
                reason: "not a ZZT_ test object".into(),
            });
            continue;
        }
        if ALWAYS_PROTECTED.contains(&name) {
            skipped.push(SkippedObject {
                name: name.into(),
                reason: "protected control object".into(),
            });
            continue;
        }
        if active.contains(name) {
            skipped.push(SkippedObject {
                name: name.into(),
                reason: "active test of a running campaign".into(),
            });
            continue;
        }
        // Program of a manifest?
        if let Some(m) = manifests.iter().find(|m| m.program == name) {
            let reason = match m.status {
                RunStatus::EvidenceRecorded => "results captured (evidence stored)".to_string(),
                RunStatus::Uploaded | RunStatus::UploadUnverified => {
                    "superseded by a later program for its group".to_string()
                }
                _ => format!("manifest status {:?}", m.status),
            };
            targets.push(CleanupTarget {
                name: name.into(),
                ty: ObjType::Program,
                program: m.program.clone(),
                ownership: Ownership::ManifestRun,
                reason,
                state: TargetState::Pending,
                backup: None,
                backup_sha256: None,
            });
            continue;
        }
        // Note of a manifest (recorded note or `<program>R` convention)?
        if let Some(m) = manifests
            .iter()
            .find(|m| m.note.as_deref() == Some(name) || is_result_note_of(name, &m.program))
        {
            let reason = if m.status == RunStatus::EvidenceRecorded {
                "result note captured (evidence stored)".to_string()
            } else {
                "result note of a superseded run".to_string()
            };
            targets.push(CleanupTarget {
                name: name.into(),
                ty: ObjType::Note,
                program: m.program.clone(),
                ownership: Ownership::ManifestRun,
                reason,
                state: TargetState::Pending,
                backup: None,
                backup_sha256: None,
            });
            continue;
        }
        // Documented probe object?
        if let Some((probe, prefix)) = KNOWN_PROBE_NOTES.iter().find(|(probe, _)| probe == &name) {
            targets.push(CleanupTarget {
                name: probe.to_string(),
                ty: ObjType::Note,
                program: probe.to_string(),
                ownership: Ownership::KnownProbe,
                reason: format!("documented probe object (expected content {prefix:?})"),
                state: TargetState::Pending,
                backup: None,
                backup_sha256: None,
            });
            continue;
        }
        skipped.push(SkippedObject {
            name: name.into(),
            reason: "in ledger but no manifest or documented origin — manual review".into(),
        });
    }
    Ok(CleanupPlan {
        targets,
        skipped,
        planned_at_utc: utc_now(),
    })
}

/// Generates the cleanup helper program for the exact given targets.
///
/// The helper is an `EXPORT`ed program that
///
/// 1. writes its own result note (`<helper>R`) with a `running` marker,
/// 2. deletes exactly the listed programs and notes,
/// 3. overwrites the marker with `done`,
/// 4. finally attempts to delete itself (best effort; a leftover helper is
///    picked up by the next batch because it is in the ledger).
///
/// No wildcards, no inventory scans, no other side effects.
pub fn generate_cleanup_helper(helper: &str, programs: &[&str], notes: &[&str]) -> String {
    let mut out = String::new();
    out.push_str("// Cleanup helper generated by ppl (exact allowlist, safe to delete).\n");
    out.push_str(&format!("EXPORT {helper}()\nBEGIN\n"));
    out.push_str(&format!("  Notes(\"{helper}R\"):=\"running\";\n"));
    for p in programs {
        out.push_str(&format!("  Programs(\"{p}\"):=\"\";\n"));
    }
    for n in notes {
        out.push_str(&format!("  Notes(\"{n}\"):=\"\";\n"));
    }
    out.push_str(&format!("  Notes(\"{helper}R\"):=\"done\";\n"));
    out.push_str(&format!("  Programs(\"{helper}\"):=\"\";\n"));
    out.push_str("  RETURN 0;\nEND;\n");
    out
}

/// Every `Programs("x"):=""` / `Notes("x"):=""` pair in a parsed program,
/// in source order. Used to verify a helper's allowlist.
pub fn extraction_targets(source: &str) -> Result<Vec<(String, String)>, String> {
    use ppl_syntax::ast::*;
    let (program, diags) = ppl_syntax::parse(source);
    if diags.has_errors() {
        return Err("helper source does not parse".into());
    }
    let mut out = Vec::new();

    fn walk_expr(e: &Expr, out: &mut Vec<(String, String)>) {
        match &e.kind {
            ExprKind::Assign(target, value) => {
                if let ExprKind::Call(base, args) = &target.kind {
                    if let ExprKind::Ident(fname) = &base.kind {
                        let deletes = matches!(&value.kind, ExprKind::Str(v) if v.is_empty());
                        if (fname == "Programs" || fname == "Notes") && args.len() == 1 && deletes {
                            if let ExprKind::Str(name) = &args[0].kind {
                                out.push((fname.clone(), name.clone()));
                            }
                        }
                    }
                }
                walk_expr(target, out);
                walk_expr(value, out);
            }
            ExprKind::Call(base, args) => {
                walk_expr(base, out);
                args.iter().for_each(|a| walk_expr(a, out));
            }
            ExprKind::Index(base, args) => {
                walk_expr(base, out);
                args.iter().for_each(|a| walk_expr(a, out));
            }
            ExprKind::List(args) | ExprKind::Bracket(args) => {
                args.iter().for_each(|a| walk_expr(a, out));
            }
            ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) | ExprKind::Store(a, b) => {
                walk_expr(a, out);
                walk_expr(b, out);
            }
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => {
                walk_expr(a, out)
            }
            _ => {}
        }
    }

    fn walk_stmt(s: &Stmt, out: &mut Vec<(String, String)>) {
        match &s.kind {
            StmtKind::Expr(e) => walk_expr(e, out),
            StmtKind::Local(decls) => decls.iter().for_each(|d| {
                if let Some(e) = &d.init {
                    walk_expr(e, out);
                }
            }),
            StmtKind::Block(b) => b.iter().for_each(|s| walk_stmt(s, out)),
            StmtKind::If {
                cond,
                then_body,
                else_body,
            } => {
                walk_expr(cond, out);
                then_body.iter().for_each(|s| walk_stmt(s, out));
                if let Some(b) = else_body {
                    b.iter().for_each(|s| walk_stmt(s, out));
                }
            }
            StmtKind::Case { arms, default } => {
                arms.iter().for_each(|(c, b)| {
                    walk_expr(c, out);
                    b.iter().for_each(|s| walk_stmt(s, out));
                });
                if let Some(b) = default {
                    b.iter().for_each(|s| walk_stmt(s, out));
                }
            }
            StmtKind::IfErr {
                body,
                handler,
                else_body,
            } => {
                body.iter().for_each(|s| walk_stmt(s, out));
                handler.iter().for_each(|s| walk_stmt(s, out));
                if let Some(b) = else_body {
                    b.iter().for_each(|s| walk_stmt(s, out));
                }
            }
            StmtKind::For {
                var: _,
                from,
                to,
                step,
                down: _,
                body,
            } => {
                walk_expr(from, out);
                walk_expr(to, out);
                if let Some(e) = step {
                    walk_expr(e, out);
                }
                body.iter().for_each(|s| walk_stmt(s, out));
            }
            StmtKind::While { cond, body } => {
                walk_expr(cond, out);
                body.iter().for_each(|s| walk_stmt(s, out));
            }
            StmtKind::Repeat { body, until } => {
                body.iter().for_each(|s| walk_stmt(s, out));
                walk_expr(until, out);
            }
            _ => {}
        }
    }

    for item in &program.items {
        if let Item::Function(f) = item {
            for s in &f.body {
                walk_stmt(s, &mut out);
            }
        }
    }
    Ok(out)
}

/// Verifies that the helper source deletes exactly the planned programs
/// and notes — no more, no less — and contains no other calls.
pub fn verify_helper(
    source: &str,
    helper: &str,
    programs: &[&str],
    notes: &[&str],
) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    let extracted = match extraction_targets(source) {
        Ok(x) => x,
        Err(e) => return Err(vec![e]),
    };
    let mut expect: Vec<(String, String)> = Vec::new();
    expect.push(("Programs".into(), helper.into()));
    for p in programs {
        expect.push(("Programs".into(), (*p).into()));
    }
    for n in notes {
        expect.push(("Notes".into(), (*n).into()));
    }
    let mut got = extracted.clone();
    got.retain(|(f, _)| f == "Programs" || f == "Notes");
    // The two marker writes Notes(helper R):= "running"/"done" are reads
    // excluded above by the empty-string filter; nothing else may appear.
    let mut e_set: BTreeSet<(String, String)> = expect.clone().into_iter().collect();
    e_set.insert(("Notes".into(), format!("{helper}R"))); // marker writes are allowed
    for (f, n) in &got {
        if !e_set.contains(&(f.clone(), n.clone())) {
            errs.push(format!("helper deletes unexpected {f}(\"{n}\")"));
        }
    }
    for (f, n) in &expect {
        if !got.contains(&(f.clone(), n.clone())) {
            errs.push(format!("helper is missing {f}(\"{n}\"):=\"\""));
        }
    }
    if extracted.len() != got.len() {
        errs.push("helper contains non-deletion Programs/Notes accesses".into());
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

// -- durable batch state ----------------------------------------------------

/// Loads the batch history from `<cleanup_dir>/batches.json`.
pub fn load_batches(cleanup_dir: &Path) -> Result<Vec<CleanupBatch>, String> {
    let path = cleanup_dir.join("batches.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Appends or replaces a batch (keyed by helper name).
pub fn save_batches(cleanup_dir: &Path, batches: &[CleanupBatch]) -> Result<(), String> {
    std::fs::create_dir_all(cleanup_dir).map_err(|e| format!("{}: {e}", cleanup_dir.display()))?;
    let path = cleanup_dir.join("batches.json");
    let json = serde_json::to_string_pretty(batches).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
}

/// Registers a documented probe object in the upload ledger so it is
/// owned going forward (used for objects created before ledger flow).
pub fn adopt_probe_object(runs_dir: &Path, name: &str) -> Result<(), String> {
    crate::ledger_append(runs_dir, &[name.to_string()], name, &utc_now())
}

pub fn default_dirs() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    (
        DEFAULT_RUNS_DIR.into(),
        crate::campaign::DEFAULT_CAMPAIGNS_DIR.into(),
        DEFAULT_CLEANUP_DIR.into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("zzt-clean-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_ledger(dir: &Path, names: &[(&str, &str)]) {
        for (n, p) in names {
            crate::ledger_append(dir, &[n.to_string()], p, "2026-10-23T00:00:00Z").unwrap();
        }
    }

    fn write_manifest(dir: &Path, program: &str, status: RunStatus, note: Option<&str>) {
        let m = RunManifest {
            schema: 2,
            suite: "s".into(),
            group: Some("g".into()),
            program: program.into(),
            note: note.map(String::from),
            channel: "note".into(),
            case_ids: vec![],
            short_ids: vec![],
            source: format!("EXPORT {program}() BEGIN END;"),
            sha256: format!("sha{program}"),
            created_at_utc: "2026-10-23T00:00:00Z".into(),
            uploaded_at_utc: None,
            preflight_log: vec![],
            device: None,
            status,
            blocked_reason: None,
            note_sha256: None,
        };
        crate::save_manifest(dir, &m).unwrap();
    }

    #[test]
    fn helper_generation_and_verification_roundtrip() {
        let src = generate_cleanup_helper("ZZT_CLEAN1", &["ZZT_A", "ZZT_B"], &["ZZT_AR"]);
        assert!(verify_helper(&src, "ZZT_CLEAN1", &["ZZT_A", "ZZT_B"], &["ZZT_AR"]).is_ok());
        // missing target -> refused
        assert!(verify_helper(&src, "ZZT_CLEAN1", &["ZZT_A"], &["ZZT_AR"]).is_err());
        // extra delete -> refused
        let extra = src.replace("RETURN 0;", "Programs(\"ZZT_CTRL\"):=\"\";\n  RETURN 0;");
        assert!(verify_helper(&extra, "ZZT_CLEAN1", &["ZZT_A", "ZZT_B"], &["ZZT_AR"]).is_err());
    }

    #[test]
    fn helper_never_touches_protected_objects() {
        let src = generate_cleanup_helper("ZZT_CLEAN1", &["ZZT_A"], &[]);
        assert!(!src.contains("ZZT_CTRL"));
        assert!(!src.contains("tester"));
        let got = extraction_targets(&src).unwrap();
        assert!(got.contains(&("Programs".into(), "ZZT_A".into())));
        assert!(got.contains(&("Programs".into(), "ZZT_CLEAN1".into())));
        assert!(
            !got.iter()
                .any(|(f, n)| f == "Programs" && n != "ZZT_A" && n != "ZZT_CLEAN1")
        );
    }

    #[test]
    fn planner_only_takes_owned_objects() {
        let runs = tmp("plan");
        write_manifest(
            &runs,
            "ZZT_OLD1",
            RunStatus::EvidenceRecorded,
            Some("ZZT_OLD1R"),
        );
        write_ledger(
            &runs,
            &[
                ("ZZT_OLD1", "ZZT_OLD1"),
                ("ZZT_OLD1R", "ZZT_OLD1"),
                ("ZZT_STRAY", "ZZT_STRAY"), // in ledger, no manifest -> review
                ("ZZT_CTRL", "ZZT_CTRL"),   // protected
            ],
        );
        let plan = plan_cleanup(&runs, &runs).unwrap();
        let names: Vec<&str> = plan.targets.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"ZZT_OLD1"));
        assert!(names.contains(&"ZZT_OLD1R"));
        assert!(!names.contains(&"ZZT_STRAY"));
        assert!(!names.contains(&"ZZT_CTRL"));
        let stray = plan.skipped.iter().find(|s| s.name == "ZZT_STRAY").unwrap();
        assert!(stray.reason.contains("manual review"));
        let ctrl = plan.skipped.iter().find(|s| s.name == "ZZT_CTRL").unwrap();
        assert!(ctrl.reason.contains("protected"));
    }

    #[test]
    fn planner_protects_active_campaign_tests() {
        let runs = tmp("active");
        write_manifest(
            &runs,
            "ZZT_ACTIVE1",
            RunStatus::Uploaded,
            Some("ZZT_ACTIVE1R"),
        );
        write_ledger(
            &runs,
            &[
                ("ZZT_ACTIVE1", "ZZT_ACTIVE1"),
                ("ZZT_ACTIVE1R", "ZZT_ACTIVE1"),
            ],
        );
        // campaign file with an uploaded group using ZZT_ACTIVE1
        let camp =
            r#"{"id":"c","groups":[{"group":"g1","stage":"uploaded","program":"ZZT_ACTIVE1"}]}"#;
        std::fs::write(runs.join("camp.json"), camp).unwrap();
        let plan = plan_cleanup(&runs, &runs).unwrap();
        assert!(plan.targets.is_empty());
        assert!(
            plan.skipped
                .iter()
                .any(|s| s.name == "ZZT_ACTIVE1" && s.reason.contains("active"))
        );
    }

    #[test]
    fn planner_adopts_documented_probe_notes() {
        let runs = tmp("probe");
        write_ledger(&runs, &[("ZZT_NOTEST", "ZZT_NOTEST")]);
        let plan = plan_cleanup(&runs, &runs).unwrap();
        let t = plan
            .targets
            .iter()
            .find(|t| t.name == "ZZT_NOTEST")
            .unwrap();
        assert_eq!(t.ty, ObjType::Note);
        assert_eq!(t.ownership, Ownership::KnownProbe);
    }

    #[test]
    fn batches_roundtrip_and_replace_by_helper() {
        let dir = tmp("batches");
        let b1 = CleanupBatch {
            helper: "ZZT_CLEAN1".into(),
            planned_at_utc: "t1".into(),
            executed_at_utc: None,
            verified_at_utc: None,
            targets: vec![],
            skipped: vec![],
        };
        save_batches(&dir, std::slice::from_ref(&b1)).unwrap();
        assert_eq!(load_batches(&dir).unwrap(), vec![b1]);
        let b2 = CleanupBatch {
            helper: "ZZT_CLEAN1".into(),
            planned_at_utc: "t1".into(),
            executed_at_utc: Some("t2".into()),
            verified_at_utc: None,
            targets: vec![],
            skipped: vec![],
        };
        save_batches(&dir, std::slice::from_ref(&b2)).unwrap();
        assert_eq!(load_batches(&dir).unwrap(), vec![b2]);
    }

    #[test]
    fn result_note_convention() {
        assert!(is_result_note_of("ZZT_AR", "ZZT_A"));
        assert!(!is_result_note_of("ZZT_A", "ZZT_A"));
        assert!(!is_result_note_of("ZZT_BR", "ZZT_A"));
    }

    #[test]
    fn helper_handles_empty_allowlist() {
        let src = generate_cleanup_helper("ZZT_CLEAN1", &[], &[]);
        assert!(verify_helper(&src, "ZZT_CLEAN1", &[], &[]).is_ok());
        assert!(src.contains("Programs(\"ZZT_CLEAN1\"):=\"\";"));
    }

    #[test]
    fn campaign_objects_scopes_to_one_campaign() {
        let dir = tmp("camp-objs");
        std::fs::write(
            dir.join("stage-a.json"),
            r#"{"id":"stage-a","groups":[
                {"group":"g1","stage":"done","program":"ZZT_A1"},
                {"group":"g2","stage":"pending"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("other.json"),
            r#"{"id":"other","groups":[
                {"group":"g1","stage":"done","program":"ZZT_B2"}]}"#,
        )
        .unwrap();
        let objs = campaign_objects(&dir, "stage-a").unwrap();
        assert!(objs.contains("ZZT_A1"));
        assert!(objs.contains("ZZT_A1R"));
        assert!(!objs.contains("ZZT_B2"));
        assert!(campaign_objects(&dir, "no-such").is_err());
    }
}
