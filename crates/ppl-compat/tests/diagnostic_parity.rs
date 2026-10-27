//! Diagnostic parity ratchet: the editor's native analyzer against the
//! original-firmware Check reference for every corpus case.
//!
//! Fails on any new disagreement and on any improvement that the baseline
//! does not yet record. Runs offline: the reference was captured from the
//! firmware beforehand (`ppl diag-parity capture`).

use std::path::PathBuf;

use ppl_compat::diag_parity::{
    BASELINE_FILE, Baseline, LOCAL_FIRMWARE_REFERENCE, Outcome, PHYSICAL_REFERENCE, ReferenceFile,
    Verdict, baseline_differences, load_corpus, run_parity, verdict_disagreements,
};

// Floors kept here, not in the TOML files, so that editing the baseline or
// re-capturing the reference cannot hide a regression: lowering one is a
// visible code change.

/// The original G1 2.4.2 `armfir.elf` every reference must come from.
const FIRMWARE_SHA256: &str = "ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39";
/// The corpus may grow but never shrink below this.
const MIN_CASES: usize = 75;
/// Cases whose native verdict and first-error position both match.
const MIN_MATCHES: usize = 59;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/diagnostics")
}

#[test]
fn every_case_has_a_current_firmware_reference() {
    let dir = corpus_dir();
    let corpus = load_corpus(&dir).unwrap();
    let reference = ReferenceFile::load(&dir.join(LOCAL_FIRMWARE_REFERENCE)).unwrap();
    assert_eq!(reference.evidence, "local-firmware");
    assert_eq!(reference.firmware_sha256.as_deref(), Some(FIRMWARE_SHA256));
    assert!(
        corpus.cases.len() >= MIN_CASES,
        "{} cases",
        corpus.cases.len()
    );
    for case in &corpus.cases {
        let r = reference
            .get(&case.spec.id)
            .unwrap_or_else(|| panic!("{}: no firmware reference", case.spec.id));
        assert_eq!(
            r.source_sha256, case.source_sha256,
            "{}: source changed since capture; re-capture the reference",
            case.spec.id
        );
        assert_ne!(
            r.verdict,
            Verdict::InfrastructureError,
            "{}: the firmware gave no verdict",
            case.spec.id
        );
    }
    assert_eq!(reference.results.len(), corpus.cases.len());
}

#[test]
fn physical_reference_holds_only_physical_observations() {
    let reference = ReferenceFile::load(&corpus_dir().join(PHYSICAL_REFERENCE)).unwrap();
    assert_eq!(reference.evidence, "physical-device");
    for r in &reference.results {
        assert!(
            r.artifact.is_some(),
            "{}: a physical observation needs its evidence",
            r.id
        );
    }
}

#[test]
fn native_diagnostics_match_the_parity_baseline() {
    let dir = corpus_dir();
    let corpus = load_corpus(&dir).unwrap();
    let reference = ReferenceFile::load(&dir.join(LOCAL_FIRMWARE_REFERENCE)).unwrap();
    let baseline = Baseline::load(&dir.join(BASELINE_FILE)).unwrap();
    let report = run_parity(&corpus, &reference);
    println!("{}", report.to_markdown());
    let differences = baseline_differences(&report, &baseline);
    assert!(
        differences.is_empty(),
        "update tests/diagnostics/{BASELINE_FILE} deliberately:\n{}",
        differences.join("\n")
    );
    // Valid programs the calculator accepts must never be flagged without
    // an acknowledged entry.
    for c in &report.cases {
        if c.outcome == Outcome::FalseRejection {
            let e = baseline.entries.iter().find(|e| e.id == c.id).unwrap();
            assert!(
                !e.note.is_empty(),
                "{}: explain the known false rejection",
                c.id
            );
        }
    }
}

#[test]
fn native_verdicts_agree_with_the_firmware_whatever_the_baseline_says() {
    let dir = corpus_dir();
    let corpus = load_corpus(&dir).unwrap();
    let reference = ReferenceFile::load(&dir.join(LOCAL_FIRMWARE_REFERENCE)).unwrap();
    let report = run_parity(&corpus, &reference);
    let disagreements = verdict_disagreements(&report);
    assert!(
        disagreements.is_empty(),
        "native and firmware verdicts disagree:\n{}",
        disagreements.join("\n")
    );
    let matches = report
        .cases
        .iter()
        .filter(|c| c.outcome == Outcome::Match)
        .count();
    assert!(
        matches >= MIN_MATCHES,
        "only {matches} cases match verdict and position (floor {MIN_MATCHES})"
    );
    assert!(
        report.cases.iter().all(|c| c.outcome.verdict_agrees()),
        "every case needs a usable reference"
    );
}
