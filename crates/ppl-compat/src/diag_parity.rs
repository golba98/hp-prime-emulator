//! Diagnostic parity: the editor's native analyzer against authentic HP
//! Prime Check results.
//!
//! A *corpus* (`tests/diagnostics/`) holds small PPL sources and a
//! manifest. *Reference* files hold what an authoritative backend reported
//! for each source (the original firmware under the local emulator, or a
//! physical calculator), captured separately and never derived from the
//! native analyzer. A *baseline* records the known outcome of every case,
//! so a test can fail on any new disagreement and on any fix that has not
//! been acknowledged.
//!
//! The native side is exactly the GUI's live analysis:
//! `compile_source(SourceFile::new(title, text), CompileOptions::default())`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::Severity;
use ppl_syntax::source::SourceFile;
use serde::{Deserialize, Serialize};

use crate::sha256::sha256_hex;

/// Corpus directory (relative to the repo).
pub const DEFAULT_CORPUS_DIR: &str = "tests/diagnostics";
/// Reference captured from the original G1 firmware under the local emulator.
pub const LOCAL_FIRMWARE_REFERENCE: &str = "reference/local-firmware-g1-2.4.2.toml";
/// Reference transcribed from a physical calculator (manual procedure).
pub const PHYSICAL_REFERENCE: &str = "reference/physical-g1.toml";
/// Known outcome of every case.
pub const BASELINE_FILE: &str = "parity-baseline.toml";

/// One corpus entry from `cases.toml`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseSpec {
    pub id: String,
    /// Source path relative to the corpus directory.
    pub file: String,
    pub category: String,
    /// What the case probes. A proposal only: the reference decides.
    pub intent: String,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "case")]
    cases: Vec<CaseSpec>,
}

/// A corpus case with its source text and hash.
#[derive(Clone, Debug)]
pub struct CorpusCase {
    pub spec: CaseSpec,
    pub text: String,
    /// SHA-256 of `text` (LF line endings, as the backends receive it).
    pub source_sha256: String,
}

#[derive(Clone, Debug)]
pub struct Corpus {
    pub root: PathBuf,
    pub cases: Vec<CorpusCase>,
}

/// Loads `cases.toml` and every source it names. Sources must be UTF-8 with
/// LF line endings, so the hash is the hash of what each backend checks.
pub fn load_corpus(root: &Path) -> Result<Corpus, String> {
    let manifest_path = root.join("cases.toml");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
    let manifest: Manifest =
        toml::from_str(&raw).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut cases = Vec::new();
    for spec in manifest.cases {
        let name_like = spec.id.starts_with(|c: char| c.is_ascii_alphabetic())
            && spec
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !name_like {
            return Err(format!(
                "case id `{}` must be ASCII letters, digits and `_`, starting with a letter",
                spec.id
            ));
        }
        if !seen.insert(spec.id.clone()) {
            return Err(format!("duplicate case id `{}`", spec.id));
        }
        let path = root.join(&spec.file);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if text.contains('\r') {
            return Err(format!("{}: corpus sources must use LF", path.display()));
        }
        cases.push(CorpusCase {
            source_sha256: sha256_hex(text.as_bytes()),
            text,
            spec,
        });
    }
    Ok(Corpus {
        root: root.to_path_buf(),
        cases,
    })
}

/// The verdict an authoritative backend gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Accepted,
    Rejected,
    /// The backend did not reach a verdict; never treated as either.
    InfrastructureError,
}

/// One case as reported by the reference backend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceResult {
    pub id: String,
    pub source_sha256: String,
    pub verdict: Verdict,
    /// Backend status string (e.g. `success`, `invalid_source`, `timed_out`).
    pub status: String,
    /// Message exactly as the backend displayed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 1-based line, when the backend reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based column in Unicode scalars, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// 0-based UTF-16 offset the firmware recorded, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utf16_offset: Option<u32>,
    /// Retained evidence (backend report) for this result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

/// A reference file: provenance plus one result per case.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceFile {
    pub schema_version: u32,
    /// `local-firmware` or `physical-device`.
    pub evidence: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_command: Option<String>,
    #[serde(default, rename = "result")]
    pub results: Vec<ReferenceResult>,
}

impl ReferenceFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        toml::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&ReferenceResult> {
        self.results.iter().find(|r| r.id == id)
    }
}

/// A native error, located as the editor shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NativeError {
    pub code: String,
    pub class: String,
    pub message: String,
    pub line: u32,
    pub column: u32,
}

/// What the GUI's live analysis reports for one source.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NativeResult {
    pub accepted: bool,
    /// The first error in source order (the firmware reports only its first).
    pub first_error: Option<NativeError>,
    /// Further errors, which no firmware reference can confirm.
    pub additional_errors: usize,
    pub warnings: usize,
}

/// Runs the same analysis as the editor's Problems panel.
pub fn analyze_native(title: &str, text: &str) -> NativeResult {
    let file = SourceFile::new(title, text);
    let (_, diags) = compile_source(&file, &CompileOptions::default());
    let mut errors = diags.items.iter().filter(|d| d.severity == Severity::Error);
    let first_error = errors.next().map(|d| {
        let lc = file.line_col(d.span.start);
        NativeError {
            code: d.code.to_string(),
            class: d.class().label().to_string(),
            message: d.message.clone(),
            line: lc.line,
            column: lc.col,
        }
    });
    NativeResult {
        accepted: first_error.is_none(),
        first_error,
        additional_errors: errors.count(),
        warnings: diags
            .items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count(),
    }
}

/// How the native analyzer compares with the reference for one case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Same verdict; when rejected, the first error is at the same line and column.
    Match,
    /// Both reject, at a different line or column.
    LocationDiffers,
    /// The native analyzer rejects a program the reference accepts.
    FalseRejection,
    /// The native analyzer accepts a program the reference rejects.
    MissedError,
    /// No usable reference result (missing, or the backend reached no verdict).
    NoReference,
    /// The reference was captured from different source text.
    ReferenceStale,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Match => "match",
            Outcome::LocationDiffers => "location-differs",
            Outcome::FalseRejection => "false-rejection",
            Outcome::MissedError => "missed-error",
            Outcome::NoReference => "no-reference",
            Outcome::ReferenceStale => "reference-stale",
        }
    }

    /// The verdicts agree (the location may still differ).
    pub fn verdict_agrees(self) -> bool {
        matches!(self, Outcome::Match | Outcome::LocationDiffers)
    }
}

/// The comparison of one case.
#[derive(Clone, Debug, Serialize)]
pub struct CaseReport {
    pub id: String,
    pub category: String,
    pub reference: Option<ReferenceResult>,
    pub native: NativeResult,
    pub outcome: Outcome,
    /// Native minus reference, when both report a location.
    pub line_delta: Option<i64>,
    pub column_delta: Option<i64>,
    /// Messages are not claimed equal: the firmware's text is generic.
    pub message_parity: &'static str,
}

/// Compares one case. Pure: the reference and native result are inputs.
pub fn compare_case(
    case: &CorpusCase,
    reference: Option<&ReferenceResult>,
    native: NativeResult,
) -> CaseReport {
    let deltas = |r: &ReferenceResult, n: &NativeResult| {
        let e = n.first_error.as_ref()?;
        let line = r.line.map(|l| e.line as i64 - l as i64);
        let col = r.column.map(|c| e.column as i64 - c as i64);
        Some((line, col))
    };
    let (outcome, line_delta, column_delta) = match reference {
        None => (Outcome::NoReference, None, None),
        Some(r) if r.source_sha256 != case.source_sha256 => (Outcome::ReferenceStale, None, None),
        Some(r) => match (r.verdict, native.accepted) {
            (Verdict::InfrastructureError, _) => (Outcome::NoReference, None, None),
            (Verdict::Accepted, true) => (Outcome::Match, None, None),
            (Verdict::Accepted, false) => (Outcome::FalseRejection, None, None),
            (Verdict::Rejected, true) => (Outcome::MissedError, None, None),
            (Verdict::Rejected, false) => {
                let (line, col) = deltas(r, &native).unwrap_or((None, None));
                let same = line == Some(0) && col == Some(0);
                let outcome = if same {
                    Outcome::Match
                } else {
                    Outcome::LocationDiffers
                };
                (outcome, line, col)
            }
        },
    };
    let message_parity = match outcome {
        Outcome::Match | Outcome::LocationDiffers if !native.accepted => {
            "not-comparable (firmware message is generic)"
        }
        Outcome::Match => "n/a (no error)",
        _ => "n/a",
    };
    CaseReport {
        id: case.spec.id.clone(),
        category: case.spec.category.clone(),
        reference: reference.cloned(),
        native,
        outcome,
        line_delta,
        column_delta,
        message_parity,
    }
}

/// The comparison of a whole corpus against one reference file.
#[derive(Clone, Debug, Serialize)]
pub struct ParityReport {
    pub reference_evidence: String,
    pub reference_description: String,
    pub firmware_sha256: Option<String>,
    pub cases: Vec<CaseReport>,
}

/// Compares every corpus case with `reference`, using the native analyzer.
pub fn run_parity(corpus: &Corpus, reference: &ReferenceFile) -> ParityReport {
    let cases = corpus
        .cases
        .iter()
        .map(|c| {
            let native = analyze_native(&c.spec.file, &c.text);
            compare_case(c, reference.get(&c.spec.id), native)
        })
        .collect();
    ParityReport {
        reference_evidence: reference.evidence.clone(),
        reference_description: reference.description.clone(),
        firmware_sha256: reference.firmware_sha256.clone(),
        cases,
    }
}

impl ParityReport {
    pub fn counts(&self) -> BTreeMap<Outcome, usize> {
        let mut m = BTreeMap::new();
        for c in &self.cases {
            *m.entry(c.outcome).or_insert(0) += 1;
        }
        m
    }

    /// A Markdown table with a summary line.
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "Reference: {} ({})\n\n",
            self.reference_description, self.reference_evidence
        ));
        let summary: Vec<String> = self
            .counts()
            .iter()
            .map(|(o, n)| format!("{}: {n}", o.as_str()))
            .collect();
        let agree = self
            .cases
            .iter()
            .filter(|c| c.outcome.verdict_agrees())
            .count();
        let (same, rejected) = self.location_agreement();
        s.push_str(&format!(
            "Verdict agreement: {agree}/{} — {}\n\
             Location agreement: {same}/{rejected} rejected cases at the same line and column\n\n",
            self.cases.len(),
            summary.join(", ")
        ));
        s.push_str("| Case | Category | Reference | Native (GUI analyzer) | Outcome |\n");
        s.push_str("|---|---|---|---|---|\n");
        for c in &self.cases {
            let reference = match &c.reference {
                None => "—".to_string(),
                Some(r) => match r.verdict {
                    Verdict::Accepted => "accepted".into(),
                    Verdict::InfrastructureError => format!("no verdict ({})", r.status),
                    Verdict::Rejected => format!(
                        "rejected {} \"{}\"",
                        location(r.line, r.column),
                        r.message.as_deref().unwrap_or("(no message)")
                    ),
                },
            };
            let native = match &c.native.first_error {
                None if c.native.warnings > 0 => {
                    format!("accepted ({} warning(s))", c.native.warnings)
                }
                None => "accepted".into(),
                Some(e) => {
                    let mut t = format!(
                        "rejected {}:{} `{}` {}",
                        e.line, e.column, e.code, e.message
                    );
                    if c.native.additional_errors > 0 {
                        t.push_str(&format!(" (+{} more)", c.native.additional_errors));
                    }
                    t
                }
            };
            let mut outcome = c.outcome.as_str().to_string();
            if c.outcome == Outcome::LocationDiffers {
                if let (Some(l), Some(col)) = (c.line_delta, c.column_delta) {
                    outcome.push_str(&format!(" (Δline {l:+}, Δcol {col:+})"));
                }
            }
            s.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                c.id,
                c.category,
                escape(&reference),
                escape(&native),
                outcome
            ));
        }
        s
    }
}

fn location(line: Option<u32>, column: Option<u32>) -> String {
    match (line, column) {
        (Some(l), Some(c)) => format!("{l}:{c}"),
        (Some(l), None) => format!("{l}:?"),
        _ => "(location unavailable)".into(),
    }
}

fn escape(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// The expected outcome of one case.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub id: String,
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Baseline {
    #[serde(default, rename = "expect")]
    pub entries: Vec<BaselineEntry>,
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        toml::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Every difference between `report` and `baseline`: new disagreements,
/// unacknowledged fixes, and cases missing on either side.
pub fn baseline_differences(report: &ParityReport, baseline: &Baseline) -> Vec<String> {
    let mut out = Vec::new();
    for c in &report.cases {
        match baseline.entries.iter().find(|e| e.id == c.id) {
            None => out.push(format!(
                "{}: not in the baseline (measured {})",
                c.id,
                c.outcome.as_str()
            )),
            Some(e) if e.outcome != c.outcome => out.push(format!(
                "{}: baseline says {}, measured {}",
                c.id,
                e.outcome.as_str(),
                c.outcome.as_str()
            )),
            Some(_) => {}
        }
    }
    for e in &baseline.entries {
        if !report.cases.iter().any(|c| c.id == e.id) {
            out.push(format!("{}: in the baseline but not in the corpus", e.id));
        }
    }
    out
}

/// SHA-256 (hex) of `data`, as used for corpus and firmware identities.
pub fn sha256_of(data: &[u8]) -> String {
    sha256_hex(data)
}

/// Every case whose native verdict disagrees with the reference, whatever
/// the baseline records. A baseline edit cannot hide these.
pub fn verdict_disagreements(report: &ParityReport) -> Vec<String> {
    report
        .cases
        .iter()
        .filter(|c| matches!(c.outcome, Outcome::FalseRejection | Outcome::MissedError))
        .map(|c| format!("{}: {}", c.id, c.outcome.as_str()))
        .collect()
}

impl ParityReport {
    /// Rejected-by-reference cases whose native first error is at the
    /// reference's line and column, out of all rejected-by-reference cases.
    pub fn location_agreement(&self) -> (usize, usize) {
        let rejected: Vec<_> = self
            .cases
            .iter()
            .filter(|c| {
                c.reference
                    .as_ref()
                    .is_some_and(|r| r.verdict == Verdict::Rejected)
            })
            .collect();
        let same = rejected
            .iter()
            .filter(|c| c.outcome == Outcome::Match)
            .count();
        (same, rejected.len())
    }
}

/// Differences between two reference files for the same corpus: the
/// verdict, status, message and position of every shared case, and ids
/// present in only one file. Provenance fields (`artifact`, capture time)
/// are not compared.
pub fn reference_differences(a: &ReferenceFile, b: &ReferenceFile) -> Vec<String> {
    let mut out = Vec::new();
    for x in &a.results {
        let Some(y) = b.get(&x.id) else {
            out.push(format!("{}: only in the first file", x.id));
            continue;
        };
        let mut fields = Vec::new();
        if x.source_sha256 != y.source_sha256 {
            fields.push("source_sha256".to_string());
        }
        if x.verdict != y.verdict {
            fields.push(format!("verdict {:?} ≠ {:?}", x.verdict, y.verdict));
        }
        if x.status != y.status {
            fields.push(format!("status {} ≠ {}", x.status, y.status));
        }
        if x.message != y.message {
            fields.push(format!("message {:?} ≠ {:?}", x.message, y.message));
        }
        if (x.line, x.column, x.utf16_offset) != (y.line, y.column, y.utf16_offset) {
            fields.push(format!(
                "position {:?}:{:?}@{:?} ≠ {:?}:{:?}@{:?}",
                x.line, x.column, x.utf16_offset, y.line, y.column, y.utf16_offset
            ));
        }
        if !fields.is_empty() {
            out.push(format!("{}: {}", x.id, fields.join("; ")));
        }
    }
    for y in &b.results {
        if a.get(&y.id).is_none() {
            out.push(format!("{}: only in the second file", y.id));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(text: &str) -> CorpusCase {
        CorpusCase {
            spec: CaseSpec {
                id: "t".into(),
                file: "t.ppl".into(),
                category: "test".into(),
                intent: String::new(),
            },
            source_sha256: sha256_hex(text.as_bytes()),
            text: text.into(),
        }
    }

    fn reference(c: &CorpusCase, verdict: Verdict, at: Option<(u32, u32)>) -> ReferenceResult {
        ReferenceResult {
            id: c.spec.id.clone(),
            source_sha256: c.source_sha256.clone(),
            verdict,
            status: String::new(),
            message: at.map(|_| "Error: Syntax Error".into()),
            line: at.map(|a| a.0),
            column: at.map(|a| a.1),
            utf16_offset: None,
            artifact: None,
        }
    }

    const VALID: &str = "EXPORT DEMO()\nBEGIN\n RETURN 1;\nEND;\n";
    const BROKEN: &str = "EXPORT DEMO()\nBEGIN\nLOCAL := ;\nEND;\n";

    #[test]
    fn native_result_matches_editor_positions() {
        let n = analyze_native("t.ppl", BROKEN);
        assert!(!n.accepted);
        let e = n.first_error.unwrap();
        assert_eq!((e.line, e.column), (3, 7));
        assert_eq!(e.class, "Syntax");
    }

    #[test]
    fn outcomes_cover_every_verdict_pair() {
        let v = case(VALID);
        let b = case(BROKEN);
        let o = |c: &CorpusCase, r: ReferenceResult| {
            compare_case(c, Some(&r), analyze_native("t", &c.text)).outcome
        };
        assert_eq!(
            o(&v, reference(&v, Verdict::Accepted, None)),
            Outcome::Match
        );
        assert_eq!(
            o(&v, reference(&v, Verdict::Rejected, Some((3, 2)))),
            Outcome::MissedError
        );
        assert_eq!(
            o(&b, reference(&b, Verdict::Accepted, None)),
            Outcome::FalseRejection
        );
        assert_eq!(
            o(&b, reference(&b, Verdict::Rejected, Some((3, 7)))),
            Outcome::Match
        );
        let r = compare_case(
            &b,
            Some(&reference(&b, Verdict::Rejected, Some((3, 8)))),
            analyze_native("t", BROKEN),
        );
        assert_eq!(r.outcome, Outcome::LocationDiffers);
        assert_eq!((r.line_delta, r.column_delta), (Some(0), Some(-1)));
        assert!(r.message_parity.starts_with("not-comparable"));
    }

    #[test]
    fn missing_stale_or_failed_references_are_not_verdicts() {
        let v = case(VALID);
        let native = || analyze_native("t", VALID);
        assert_eq!(
            compare_case(&v, None, native()).outcome,
            Outcome::NoReference
        );
        let mut stale = reference(&v, Verdict::Accepted, None);
        stale.source_sha256 = "00".into();
        assert_eq!(
            compare_case(&v, Some(&stale), native()).outcome,
            Outcome::ReferenceStale
        );
        let failed = reference(&v, Verdict::InfrastructureError, None);
        assert_eq!(
            compare_case(&v, Some(&failed), native()).outcome,
            Outcome::NoReference
        );
    }

    #[test]
    fn baseline_reports_regressions_and_unacknowledged_fixes() {
        let v = case(VALID);
        let report = ParityReport {
            reference_evidence: "local-firmware".into(),
            reference_description: String::new(),
            firmware_sha256: None,
            cases: vec![compare_case(
                &v,
                Some(&reference(&v, Verdict::Accepted, None)),
                analyze_native("t", VALID),
            )],
        };
        let baseline = |o| Baseline {
            entries: vec![BaselineEntry {
                id: "t".into(),
                outcome: o,
                note: String::new(),
            }],
        };
        assert!(baseline_differences(&report, &baseline(Outcome::Match)).is_empty());
        assert_eq!(
            baseline_differences(&report, &baseline(Outcome::MissedError)).len(),
            1
        );
        assert_eq!(baseline_differences(&report, &Baseline::default()).len(), 1);
    }

    #[test]
    fn reference_file_round_trips_through_toml() {
        let v = case(VALID);
        let file = ReferenceFile {
            schema_version: 1,
            evidence: "local-firmware".into(),
            description: "test".into(),
            firmware_sha256: Some("ab".into()),
            captured_at: None,
            capture_command: None,
            results: vec![reference(&v, Verdict::Rejected, Some((3, 9)))],
        };
        let back: ReferenceFile = toml::from_str(&file.to_toml().unwrap()).unwrap();
        assert_eq!(back.results, file.results);
    }

    fn file(results: Vec<ReferenceResult>) -> ReferenceFile {
        ReferenceFile {
            schema_version: 1,
            evidence: "local-firmware".into(),
            description: String::new(),
            firmware_sha256: None,
            captured_at: None,
            capture_command: None,
            results,
        }
    }

    #[test]
    fn reference_diffs_compare_results_not_provenance() {
        let b = case(BROKEN);
        let mut x = reference(&b, Verdict::Rejected, Some((3, 7)));
        x.utf16_offset = Some(20);
        let mut y = x.clone();
        y.artifact = Some("elsewhere/report.json".into());
        assert!(reference_differences(&file(vec![x.clone()]), &file(vec![y.clone()])).is_empty());
        y.utf16_offset = Some(21);
        let d = reference_differences(&file(vec![x.clone()]), &file(vec![y]));
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].contains("position"), "{d:?}");
        let mut z = x.clone();
        z.id = "other".into();
        let d = reference_differences(&file(vec![x]), &file(vec![z]));
        assert_eq!(d.len(), 2, "{d:?}");
    }

    #[test]
    fn verdict_disagreements_ignore_the_baseline() {
        let v = case(VALID);
        let reference = file(vec![reference(&v, Verdict::Rejected, Some((3, 2)))]);
        let corpus = Corpus {
            root: PathBuf::new(),
            cases: vec![v],
        };
        let report = run_parity(&corpus, &reference);
        // A baseline that records the disagreement does not excuse it.
        let baseline = Baseline {
            entries: vec![BaselineEntry {
                id: "t".into(),
                outcome: Outcome::MissedError,
                note: "known".into(),
            }],
        };
        assert!(baseline_differences(&report, &baseline).is_empty());
        assert_eq!(verdict_disagreements(&report), vec!["t: missed-error"]);
        assert_eq!(report.location_agreement(), (0, 1));
    }
}
