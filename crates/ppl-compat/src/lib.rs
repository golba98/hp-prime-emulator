//! Differential compatibility testing.
//!
//! A *suite* (TOML) lists cases; each case is an expression or a function
//! body returning a value. The same suite is
//!
//! 1. run in the emulator, case by case, and
//! 2. compiled into one calculator program (`ZZT_…`) that evaluates every
//!    case inside `IFERR` and stores `TYPE`/`STRING` of each result in the
//!    note `ZZT_R`, which the host reads back over USB.
//!
//! Results recorded from hardware are saved as *evidence* (JSON, with
//! firmware build and provenance). A case is VERIFIED only when an evidence
//! record from a physical calculator matches the emulator.
//!
//! The v2 pipeline (new API, alongside the original one above) adds:
//!
//! * suite groups and per-case isolation / declared side effects
//!   ([`Group`], [`Case::isolate`], [`Case::mutates`], [`select_cases`]);
//! * a static side-effect audit of every case ([`audit`]);
//! * a generator with refusal rules, a settings snapshot and a hash tag,
//!   writing to the terminal or to a new note ([`generate`]);
//! * run manifests and a name ledger so names are never reused
//!   ([`manifest`]);
//! * per-run evidence files keeping raw text, merged per case ([`evidence`]).

pub mod audit;
pub mod campaign;
pub mod cleanup;
pub mod corpus;
pub mod diag_parity;
pub mod evidence;
pub mod generate;
pub mod import;
pub mod manifest;
mod sha256;

pub use audit::{BuiltinEffect, CaseAudit, audit_case, audit_program, classify_builtin};
pub use campaign::{
    CampaignState, CaptureOutcome, DEFAULT_CAMPAIGNS_DIR, GroupOutcome, GroupState, Stage,
    capture_note_text,
};
pub use cleanup::{
    ALWAYS_PROTECTED, CleanupBatch, CleanupPlan, CleanupTarget, DEFAULT_CLEANUP_DIR, ObjType,
    Ownership, SkippedObject, TargetState, adopt_probe_object, generate_cleanup_helper,
    load_batches, plan_cleanup, save_batches, verify_helper,
};
pub use corpus::{CATEGORIES, GeneratedCorpus, SeedRng, generate_corpus};
pub use evidence::{
    EvidenceV2, METHOD_NOTE_FETCH, METHOD_SCREENSHOT, MergedCase, check_run_against_manifest,
    compare_v2, load_suite_evidence, merge_evidence, normalise_display, save_evidence_v2,
    status_v2,
};
pub use generate::{
    CaseEvidence, Channel, GeneratedProgram, MAX_CASES_PER_PROGRAM, ParsedRun,
    emulate_print_program, generate_device_program, parse_result_lines, valid_program_name,
    verify_program_tag,
};
pub use import::{FileClass, ImportReport, ImportedFile, import_archive};
pub use manifest::{
    DEFAULT_RUNS_DIR, DeviceRecord, LedgerEntry, RunManifest, RunStatus, ledger_append,
    ledger_contains, ledger_entries, load_manifest, save_manifest, serial_fingerprint, unused_name,
};
pub use sha256::sha256_hex;

#[cfg(test)]
mod tests_v2;

/// Default directory of hardware evidence (relative to the repo).
pub const DEFAULT_EVIDENCE_DIR: &str = "tests/compat/evidence";

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

pub const RESULT_NOTE: &str = "ZZT_R";
const REC: &str = "@@";
const SEP: char = '|';

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Suite {
    pub suite: String,
    #[serde(default)]
    pub description: String,
    /// Optional `[[group]]` declarations (v2).
    #[serde(default, rename = "group", skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    #[serde(rename = "case")]
    pub cases: Vec<Case>,
}

/// A named subset of a suite, small enough for one device program.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Group {
    pub id: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    /// A single expression; the case returns its value.
    #[serde(default)]
    pub expr: Option<String>,
    /// Statements forming a function body (should `RETURN` a value).
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Known, tracked divergence: the regression test tolerates a mismatch.
    #[serde(default)]
    pub known_divergence: Option<String>,
    /// Group id (v2).
    #[serde(default)]
    pub group: Option<String>,
    /// Must run alone in its device program (required for any `mutates`;
    /// also used for cases that risk an error or a parse failure).
    #[serde(default)]
    pub isolate: bool,
    /// Declared persistent side effects, by audit name (e.g. `["HAngle"]`).
    #[serde(default)]
    pub mutates: Vec<String>,
    /// Why this case is risky on hardware.
    #[serde(default)]
    pub risk: Option<String>,
}

impl Case {
    pub(crate) fn function_body(&self) -> String {
        match (&self.expr, &self.body) {
            (Some(e), _) => format!("RETURN {e};"),
            (None, Some(b)) => b.trim().to_string(),
            (None, None) => "RETURN 0;".to_string(),
        }
    }
}

pub fn load_suite(path: &Path) -> Result<Suite, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let suite: Suite = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut groups = std::collections::HashSet::new();
    for g in &suite.groups {
        if !groups.insert(g.id.as_str()) {
            return Err(format!("{}: duplicate group id {}", path.display(), g.id));
        }
    }
    if !suite.groups.is_empty() {
        for c in &suite.cases {
            if let Some(g) = &c.group {
                if !groups.contains(g.as_str()) {
                    return Err(format!(
                        "{}: case {} uses undeclared group {g}",
                        path.display(),
                        c.id
                    ));
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for c in &suite.cases {
        if !seen.insert(&c.id) {
            return Err(format!("{}: duplicate case id {}", path.display(), c.id));
        }
        if c.id.contains(SEP) || c.id.contains(REC) {
            return Err(format!(
                "{}: case id {} contains a reserved character",
                path.display(),
                c.id
            ));
        }
    }
    Ok(suite)
}

/// Cases of `suite` in suite order, optionally restricted to a group and/or
/// to explicit ids. Unknown groups or ids, ids outside the group, and an
/// empty selection are errors.
pub fn select_cases<'a>(
    suite: &'a Suite,
    group: Option<&str>,
    ids: Option<&[String]>,
) -> Result<Vec<&'a Case>, String> {
    if let Some(g) = group {
        let declared = suite.groups.iter().any(|x| x.id == g);
        let used = suite.cases.iter().any(|c| c.group.as_deref() == Some(g));
        if !declared && !used {
            return Err(format!("suite {} has no group {g}", suite.suite));
        }
    }
    if let Some(ids) = ids {
        for id in ids {
            let Some(c) = suite.cases.iter().find(|c| &c.id == id) else {
                return Err(format!("suite {} has no case {id}", suite.suite));
            };
            if let Some(g) = group {
                if c.group.as_deref() != Some(g) {
                    return Err(format!("case {id} is not in group {g}"));
                }
            }
        }
    }
    let out: Vec<&Case> = suite
        .cases
        .iter()
        .filter(|c| group.is_none_or(|g| c.group.as_deref() == Some(g)))
        .filter(|c| ids.is_none_or(|ids| ids.iter().any(|i| i == &c.id)))
        .collect();
    if out.is_empty() {
        return Err("selection is empty".into());
    }
    Ok(out)
}

/// Observed result of one case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// `TYPE()` code and `STRING()` text.
    Value { ty: i64, text: String },
    /// Runtime error (the calculator reports no detail through IFERR).
    Error { detail: String },
    /// The program did not compile.
    CompileError { detail: String },
    /// No record for this case.
    Missing,
}

impl Outcome {
    pub fn summary(&self) -> String {
        match self {
            Outcome::Value { ty, text } => format!("{text} (type {ty})"),
            Outcome::Error { detail } if detail.is_empty() => "error".into(),
            Outcome::Error { detail } => format!("error: {detail}"),
            Outcome::CompileError { detail } => format!("compile error: {detail}"),
            Outcome::Missing => "missing".into(),
        }
    }

    /// Equality for compatibility purposes: errors match errors.
    pub fn matches(&self, other: &Outcome) -> bool {
        match (self, other) {
            (Outcome::Value { ty: a, text: x }, Outcome::Value { ty: b, text: y }) => {
                a == b && x == y
            }
            (Outcome::Error { .. }, Outcome::Error { .. }) => true,
            _ => false,
        }
    }
}

fn case_fn_name(i: usize) -> String {
    format!("ZC{}", i + 1)
}

/// Upper-case alphanumeric program name for a suite, unique per source.
pub fn program_name(suite: &Suite) -> String {
    let src = device_program_with_name(suite, "ZZT_X");
    let mut h: u32 = 0x811C_9DC5;
    for b in src.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    let base: String = suite
        .suite
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(10)
        .collect::<String>()
        .to_uppercase();
    format!("ZZT_{base}_{:04X}", h & 0xFFFF)
}

/// The calculator program for a suite (UTF-16 safe, CRLF-free).
pub fn device_program(suite: &Suite) -> (String, String) {
    let name = program_name(suite);
    let src = device_program_with_name(suite, &name);
    (name, src)
}

fn device_program_with_name(suite: &Suite, name: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "// Generated by `ppl compare` for suite {}. Safe to delete.\n",
        suite.suite
    ));
    for (i, c) in suite.cases.iter().enumerate() {
        out.push_str(&format!(
            "{}()\nBEGIN\n{}\nEND;\n\n",
            case_fn_name(i),
            indent(&c.function_body())
        ));
    }
    // Only IFERR … THEN … END is used (no IFERR … ELSE), and results are
    // recorded through a flag, to depend on as little syntax as possible.
    out.push_str(&format!(
        "EXPORT {name}()\nBEGIN\n  LOCAL r, v, ok;\n  r:=\"{REC}ZZT{SEP}{name}\";\n"
    ));
    for (i, c) in suite.cases.iter().enumerate() {
        out.push_str(&format!(
            "  ok:=1;\n  IFERR v:={f}(); THEN ok:=0; END;\n  IF ok THEN r:=r+\"{REC}{id}{SEP}\"+STRING(TYPE(v))+\"{SEP}\"+STRING(v); ELSE r:=r+\"{REC}{id}{SEP}E{SEP}\"; END;\n",
            f = case_fn_name(i),
            id = c.id
        ));
    }
    out.push_str(&format!("  r:=r+\"{REC}END\";\n  Notes(\"{RESULT_NOTE}\"):=r;\n  PRINT(\"{name}: {} cases done\");\n  RETURN SIZE(r);\nEND;\n", suite.cases.len()));
    out
}

pub(crate) fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs every case in the emulator, each in its own program so a compile
/// problem in one case does not hide the others.
pub fn run_emulator(suite: &Suite) -> BTreeMap<String, Outcome> {
    let mut out = BTreeMap::new();
    for c in &suite.cases {
        out.insert(c.id.clone(), run_case(c));
    }
    out
}

/// The single-case program used by [`run_case`] and [`audit_case`].
pub(crate) fn case_wrapper_source(c: &Case) -> String {
    format!(
        "ZC1()\nBEGIN\n{}\nEND;\n\nEXPORT ZZT_ONE()\nBEGIN\n  RETURN ZC1();\nEND;\n",
        indent(&c.function_body())
    )
}

pub fn run_case(c: &Case) -> Outcome {
    let src = case_wrapper_source(c);
    let file = ppl_syntax::SourceFile::new(format!("<case {}>", c.id), src);
    let (prog, diags) =
        ppl_compiler::compile_source(&file, &ppl_compiler::CompileOptions::default());
    let Some(prog) = prog else {
        let d = diags
            .errors()
            .next()
            .map(|d| d.message.clone())
            .unwrap_or_default();
        return Outcome::CompileError { detail: d };
    };
    let mut m = ppl_vm::Machine::new(
        Rc::new(prog),
        Box::new(ppl_vm::HeadlessHost::new()),
        ppl_vm::Settings::default(),
        ppl_vm::Limits {
            max_steps: Some(5_000_000),
            ..ppl_vm::Limits::default()
        },
    );
    match m.call_function("ZZT_ONE", Vec::new()) {
        Ok(v) => Outcome::Value {
            ty: v.type_code(),
            // The device program records STRING(v), so the emulator side
            // must use the same STRING() transformation (a top-level
            // string is quoted on the hardware).
            text: v.string_form(m.settings.format()),
        },
        Err(e) => Outcome::Error {
            detail: e.to_string(),
        },
    }
}

/// Parses the `ZZT_R` note written by [`device_program`].
pub fn parse_device_note(text: &str) -> Result<(String, BTreeMap<String, Outcome>), String> {
    let start = text
        .find(&format!("{REC}ZZT{SEP}"))
        .ok_or("note does not contain a ZZT result header")?;
    let body = &text[start..];
    let mut program = String::new();
    let mut results = BTreeMap::new();
    let mut complete = false;
    for rec in body.split(REC).filter(|r| !r.is_empty()) {
        if rec.trim_end() == "END"
            || rec.starts_with("END") && rec.trim_end_matches(['\0', '\n', '\r', ' ']) == "END"
        {
            complete = true;
            break;
        }
        let mut parts = rec.splitn(3, SEP);
        let id = parts.next().unwrap_or_default().to_string();
        let second = parts.next().unwrap_or_default();
        if id == "ZZT" {
            program = second.trim().to_string();
            continue;
        }
        let outcome = if second == "E" {
            Outcome::Error {
                detail: String::new(),
            }
        } else {
            let ty = second
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("bad type field in record `{rec}`"))?;
            Outcome::Value {
                ty,
                text: parts.next().unwrap_or_default().to_string(),
            }
        };
        results.insert(id, outcome);
    }
    if !complete {
        return Err("result note is truncated (no END marker)".into());
    }
    Ok((program, results))
}

/// Hardware evidence for one suite run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub suite: String,
    pub program: String,
    pub source_fnv: String,
    pub device_model: String,
    pub usb_pid: String,
    pub firmware_build: Option<u32>,
    pub boot_version: Option<String>,
    /// Fingerprint of the serial number (the serial itself is not stored).
    pub serial_id: Option<String>,
    pub recorded_at_utc: String,
    pub method: String,
    pub results: BTreeMap<String, Outcome>,
}

pub fn source_fingerprint(src: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn load_evidence(path: &Path) -> Result<Evidence, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    /// Emulator matches a recorded hardware result.
    Verified,
    /// Same type, different text (e.g. formatting only).
    Partial,
    /// Emulator differs from hardware.
    Failed,
    /// No hardware result recorded.
    Unverified,
    /// Emulator reports the feature as not implemented.
    Unsupported,
    /// Hardware run did not produce a result for this case.
    Blocked,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Verified => "VERIFIED",
            Status::Partial => "PARTIAL",
            Status::Failed => "FAILED",
            Status::Unverified => "UNVERIFIED",
            Status::Unsupported => "UNSUPPORTED",
            Status::Blocked => "BLOCKED",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseReport {
    pub id: String,
    pub source: String,
    pub emulator: Outcome,
    pub device: Option<Outcome>,
    pub status: Status,
    pub note: Option<String>,
    pub known_divergence: Option<String>,
    /// Raw device record line (v2 evidence only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_raw: Option<String>,
    /// Program whose evidence supplied `device` (v2 evidence only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_program: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub suite: String,
    pub evidence: Option<EvidenceSummary>,
    pub cases: Vec<CaseReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub program: String,
    pub firmware_build: Option<u32>,
    pub device_model: String,
    pub recorded_at_utc: String,
}

pub fn compare(suite: &Suite, emu: &BTreeMap<String, Outcome>, ev: Option<&Evidence>) -> Report {
    let cases = suite
        .cases
        .iter()
        .map(|c| {
            let e = emu.get(&c.id).cloned().unwrap_or(Outcome::Missing);
            let d = ev.map(|ev| ev.results.get(&c.id).cloned().unwrap_or(Outcome::Missing));
            let unsupported = matches!(&e, Outcome::Error { detail } if detail.contains("Unsupported"));
            let status = match &d {
                None if unsupported => Status::Unsupported,
                None => Status::Unverified,
                Some(Outcome::Missing) => Status::Blocked,
                Some(dv) if e.matches(dv) => Status::Verified,
                Some(_) if unsupported => Status::Unsupported,
                Some(Outcome::Value { ty: dt, .. }) if matches!(&e, Outcome::Value { ty, .. } if ty == dt) => Status::Partial,
                Some(_) => Status::Failed,
            };
            CaseReport {
                id: c.id.clone(),
                source: c.expr.clone().unwrap_or_else(|| c.body.clone().unwrap_or_default()),
                emulator: e,
                device: d,
                status,
                note: c.note.clone(),
                known_divergence: c.known_divergence.clone(),
                device_raw: None,
                evidence_program: None,
            }
        })
        .collect();
    Report {
        suite: suite.suite.clone(),
        evidence: ev.map(|e| EvidenceSummary {
            program: e.program.clone(),
            firmware_build: e.firmware_build,
            device_model: e.device_model.clone(),
            recorded_at_utc: e.recorded_at_utc.clone(),
        }),
        cases,
    }
}

impl Report {
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        for c in &self.cases {
            *m.entry(c.status.as_str()).or_insert(0) += 1;
        }
        m
    }

    pub fn markdown(&self) -> String {
        let mut s = format!("# Compatibility report: `{}`\n\n", self.suite);
        match &self.evidence {
            Some(e) => s.push_str(&format!(
                "Hardware evidence: program `{}` on {} (firmware build {}), recorded {}.\n\n",
                e.program,
                e.device_model,
                e.firmware_build
                    .map(|b| b.to_string())
                    .unwrap_or("unknown".into()),
                e.recorded_at_utc
            )),
            None => s.push_str("No hardware evidence recorded: every case is UNVERIFIED.\n\n"),
        }
        let counts: Vec<String> = self
            .counts()
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        s.push_str(&format!("Summary: {}\n\n", counts.join(", ")));
        s.push_str("| case | source | emulator | G1 | status |\n|---|---|---|---|---|\n");
        for c in &self.cases {
            let esc = |t: &str| t.replace('|', "\\|").replace('\n', " ");
            s.push_str(&format!(
                "| `{}` | `{}` | {} | {} | {}{} |\n",
                c.id,
                esc(&c.source),
                esc(&c.emulator.summary()),
                c.device
                    .as_ref()
                    .map(|d| esc(&d.summary()))
                    .unwrap_or_else(|| "—".into()),
                c.status.as_str(),
                c.known_divergence
                    .as_ref()
                    .map(|k| format!(" (known: {})", esc(k)))
                    .unwrap_or_default()
            ));
        }
        s
    }
}

/// UTC timestamp `YYYY-MM-DDTHH:MM:SSZ` without external crates.
pub fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite() -> Suite {
        toml::from_str(
            r#"
            suite = "demo"
            [[case]]
            id = "add"
            expr = "1+2"
            [[case]]
            id = "div0"
            expr = "1/0"
            [[case]]
            id = "body"
            body = """
            LOCAL L:={1,2};
            RETURN L;
            """
            "#,
        )
        .unwrap()
    }

    #[test]
    fn emulator_outcomes() {
        let r = run_emulator(&suite());
        assert_eq!(
            r["add"],
            Outcome::Value {
                ty: 0,
                text: "3".into()
            }
        );
        assert!(matches!(r["div0"], Outcome::Error { .. }));
        assert_eq!(
            r["body"],
            Outcome::Value {
                ty: 6,
                text: "{1,2}".into()
            }
        );
    }

    #[test]
    fn device_program_compiles_in_emulator_except_notes() {
        let (name, src) = device_program(&suite());
        assert!(name.starts_with("ZZT_DEMO_"));
        // Notes() is not emulated, so only check that it parses.
        let (_, d) = ppl_syntax::parse(&src);
        assert!(!d.has_errors(), "{:?}", d.items);
    }

    #[test]
    fn note_roundtrip_and_compare() {
        let note = "@@ZZT|ZZT_DEMO_1234@@add|0|3@@div0|E|@@body|6|{1,2}@@END";
        let (prog, res) = parse_device_note(note).unwrap();
        assert_eq!(prog, "ZZT_DEMO_1234");
        let s = suite();
        let ev = Evidence {
            suite: "demo".into(),
            program: prog,
            source_fnv: String::new(),
            device_model: "test".into(),
            usb_pid: "1541".into(),
            firmware_build: Some(15157),
            boot_version: None,
            serial_id: None,
            recorded_at_utc: utc_now(),
            method: "unit test".into(),
            results: res,
        };
        let rep = compare(&s, &run_emulator(&s), Some(&ev));
        assert!(
            rep.cases.iter().all(|c| c.status == Status::Verified),
            "{}",
            rep.markdown()
        );
        assert!(parse_device_note("@@ZZT|X@@a|0|1").is_err());
    }

    #[test]
    fn core_suite_loads_and_runs_without_compile_errors() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/cases/core.toml");
        let s = load_suite(&path).expect("core.toml should load cleanly");
        assert_eq!(s.suite, "core");
        assert!(!s.cases.is_empty(), "core suite should have cases");

        let results = run_emulator(&s);
        assert_eq!(results.len(), s.cases.len());

        let mut compile_errors = Vec::new();
        let mut unexpected_outcomes = Vec::new();

        for c in &s.cases {
            match results.get(&c.id) {
                Some(Outcome::CompileError { detail }) => {
                    compile_errors.push((c.id.clone(), detail.clone()));
                }
                Some(Outcome::Value { .. }) => {}
                Some(Outcome::Error { detail }) => {
                    // Errors are only acceptable for cases flagged as risky
                    // (isolated or with a recorded risk), e.g. 1E499*10.
                    if !(c.isolate || c.risk.is_some()) {
                        unexpected_outcomes.push((c.id.clone(), format!("Error: {detail}")));
                    }
                }
                Some(Outcome::Missing) | None => {
                    unexpected_outcomes.push((c.id.clone(), "missing".to_string()));
                }
            }
        }

        assert!(
            compile_errors.is_empty(),
            "Found compile errors in core suite: {compile_errors:#?}"
        );
        assert!(
            unexpected_outcomes.is_empty(),
            "Found unexpected outcomes in core suite: {unexpected_outcomes:#?}"
        );

        // Also verify the generated physical device program parses with zero syntax errors
        let (prog_name, device_src) = device_program(&s);
        assert!(prog_name.starts_with("ZZT_CORE_"));
        let (_, diags) = ppl_syntax::parse(&device_src);
        assert!(
            !diags.has_errors(),
            "Generated device program has syntax errors: {:?}",
            diags.items
        );
    }
}
