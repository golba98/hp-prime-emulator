//! Application compatibility suite: real `.hpprgm`/PPL programs executed by
//! the original firmware, compared against recorded expectations.
//!
//! Every report field is derived from compared evidence. Local-firmware
//! references are never presented as physical-calculator results: physical
//! parity is `NOT VERIFIED` unless physical evidence is recorded for a case.
use crate::{
    checkprog::{self, CheckConfig},
    framediff,
    keys::{self, Key},
    runprog::{self, ExecutionControl, ProgramReport, RunProgramConfig},
    runtime::{ExecutionRequest, ExecutionStatus},
    runtime_value::{self, FirmwareValue, ReturnRepresentation},
    sha256,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One application case, read from `tests/runtime/cases/<id>.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub category: String,
    pub description: String,
    /// Source file, relative to the case file.
    pub program: PathBuf,
    pub source_sha256: String,
    pub entry: String,
    #[serde(default = "no_arguments")]
    pub args: String,
    /// Key/touch/checkpoint script replayed after function entry.
    #[serde(default)]
    pub runtime_keys: Option<String>,
    #[serde(default = "default_timeout")]
    pub execution_timeout_s: f64,
    /// Invocations of the entry within one boot (state contamination check).
    #[serde(default = "one")]
    pub repeat: usize,
    /// Firmware Check only (Program Editor), no execution: compile parity
    /// for applications the firmware rejects.
    #[serde(default)]
    pub check_only: bool,
    pub initial_state: String,
    pub expect: Expect,
    #[serde(default)]
    pub checkpoint: Vec<CheckpointExpect>,
    #[serde(default)]
    pub known_gaps: Vec<String>,
    /// An open emulator defect this case still reproduces, with evidence.
    /// Reported XFAIL; passing becomes a failure so the case is updated.
    #[serde(default)]
    pub expected_failure: Option<String>,
}
fn no_arguments() -> String {
    "[]".into()
}
fn default_timeout() -> f64 {
    20.0
}
fn one() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub outcome: ExecutionStatus,
    /// `FirmwareValue::display` of the returned value.
    #[serde(default)]
    pub return_display: Option<String>,
    /// Per-invocation returns when `repeat` > 1 and state carries over.
    #[serde(default)]
    pub return_sequence: Option<Vec<String>>,
    /// The host is expected not to decode the returned representation.
    #[serde(default)]
    pub return_undecoded: bool,
    /// Exact PRINT text events, in order.
    #[serde(default)]
    pub output: Option<Vec<String>>,
    #[serde(default)]
    pub error_code: Option<u8>,
    #[serde(default)]
    pub error_message: Option<String>,
    /// Firmware Check diagnostic position `[line, column]` for a rejected
    /// program.
    #[serde(default)]
    pub compile_error_at: Option<[usize; 2]>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointExpect {
    /// `return` (frame at function return) or a `SNAP:`/`DELAY:` label.
    pub label: String,
    /// Reference PNG, relative to the suite root.
    pub reference: PathBuf,
    /// Why the frame cannot be compared exactly (e.g. random animation);
    /// it is then captured but reported `NOT COMPARED`.
    #[serde(default)]
    pub nondeterministic: Option<String>,
    /// Ignored pixels (non-black in the mask PNG), with the reason.
    #[serde(default)]
    pub mask: Option<PathBuf>,
    #[serde(default)]
    pub mask_reason: Option<String>,
}

/// Sidecar recorded next to every reference PNG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// `local-firmware` or `physical-g1`.
    pub provenance: String,
    pub firmware_sha256: Option<String>,
    pub case: String,
    pub label: String,
    pub pixels_sha256: String,
    pub source_sha256: String,
    pub recorded_by: String,
}

#[derive(Debug, Clone)]
pub struct SuiteConfig {
    pub root: PathBuf,
    pub firmware: PathBuf,
    pub os_rom: Option<PathBuf>,
    pub factory_a: PathBuf,
    pub init_data: Option<PathBuf>,
    pub out_dir: PathBuf,
    /// Write references from runs whose outcome matched expectations.
    pub record: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    Match,
    Differ,
    NotTested,
    NotCompared,
    Unsupported,
    Indeterminate,
    NotVerified,
    Recorded,
}
impl Verdict {
    fn text(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Match => "MATCH",
            Self::Differ => "DIFFER",
            Self::NotTested => "NOT TESTED",
            Self::NotCompared => "NOT COMPARED",
            Self::Unsupported => "UNSUPPORTED",
            Self::Indeterminate => "INDETERMINATE",
            Self::NotVerified => "NOT VERIFIED",
            Self::Recorded => "RECORDED",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckpointResult {
    pub label: String,
    pub verdict: Verdict,
    pub actual_png: Option<String>,
    pub reference: String,
    pub comparison: Option<framediff::Comparison>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    pub id: String,
    pub category: String,
    pub file: String,
    pub source_sha256: String,
    pub firmware_sha256: String,
    pub entry: String,
    pub initial_state: String,
    pub input_script: Option<String>,
    pub execution_status: ExecutionStatus,
    pub return_display: Option<String>,
    pub return_representation: ReturnRepresentation,
    pub output: Vec<String>,
    pub runtime_error: Option<String>,
    /// Firmware Check diagnostics, `line:column message`.
    pub compile_diagnostics: Vec<String>,
    pub caveats: Vec<String>,
    pub compilation: Verdict,
    pub import: Verdict,
    pub execution: Verdict,
    pub runtime_errors: Verdict,
    pub display: Verdict,
    pub keyboard: Verdict,
    pub touch: Verdict,
    pub physical: Verdict,
    pub checkpoints: Vec<CheckpointResult>,
    pub differences: Vec<String>,
    pub known_gaps: Vec<String>,
    pub expected_failure: Option<String>,
    pub evidence: String,
    pub work_dir: Option<PathBuf>,
}
impl CaseResult {
    /// Local-firmware execution agreed with every compared expectation.
    pub fn local_pass(&self) -> bool {
        [
            self.compilation,
            self.import,
            self.execution,
            self.runtime_errors,
        ]
        .iter()
        .all(|v| *v == Verdict::Pass || *v == Verdict::Match)
            && [self.display, self.keyboard, self.touch]
                .iter()
                .all(|v| !matches!(v, Verdict::Differ | Verdict::Fail))
    }
    /// Agrees with the case: a pass, or a still-reproduced expected failure.
    pub fn as_expected(&self) -> bool {
        self.local_pass() != self.expected_failure.is_some()
    }
    fn local_text(&self) -> &'static str {
        match (self.local_pass(), self.expected_failure.is_some()) {
            (true, false) => "PASS",
            (false, false) => "FAIL",
            (false, true) => "XFAIL (open defect)",
            (true, true) => "XPASS (update the case)",
        }
    }
    /// The Phase-7 per-application report.
    pub fn text(&self) -> String {
        let mut s = String::new();
        let line = |s: &mut String, k: &str, v: &str| s.push_str(&format!("{k:<19}{v}\n"));
        s.push_str("HP PRIME G1 — APPLICATION COMPATIBILITY\n\n");
        line(&mut s, "Case:", &self.id);
        line(&mut s, "File:", &self.file);
        line(&mut s, "Source SHA-256:", &self.source_sha256);
        line(
            &mut s,
            "Firmware:",
            &format!("G1 2.4.2 (armfir.elf {})", &self.firmware_sha256[..16]),
        );
        line(&mut s, "Entry:", &self.entry);
        line(&mut s, "Initial state:", &self.initial_state);
        line(
            &mut s,
            "Input script:",
            self.input_script.as_deref().unwrap_or("none"),
        );
        line(
            &mut s,
            "Reference:",
            "local firmware (not a physical calculator)",
        );
        s.push('\n');
        line(&mut s, "Compilation:", self.compilation.text());
        for d in &self.compile_diagnostics {
            s.push_str(&format!("  firmware Check:  {d}\n"));
        }
        line(&mut s, "Program Import:", self.import.text());
        line(
            &mut s,
            "Execution:",
            &format!("{} ({:?})", self.execution.text(), self.execution_status),
        );
        line(
            &mut s,
            "Return value:",
            &match (&self.return_display, &self.return_representation) {
                (Some(v), _) => v.clone(),
                (None, ReturnRepresentation::Undecoded { tag, subtype, .. }) => format!(
                    "not decoded by host (tag {:#x}/{:#x})",
                    tag.unwrap_or(0),
                    subtype.unwrap_or(0)
                ),
                _ => "none".into(),
            },
        );
        line(
            &mut s,
            "Runtime Errors:",
            &match &self.runtime_error {
                Some(e) => format!("{} ({e})", self.runtime_errors.text()),
                None => format!("{} (none observed)", self.runtime_errors.text()),
            },
        );
        line(&mut s, "Display:", self.display.text());
        line(&mut s, "Keyboard Input:", self.keyboard.text());
        line(&mut s, "Touch Input:", self.touch.text());
        line(&mut s, "Physical G1 Test:", self.physical.text());
        for c in &self.checkpoints {
            let detail = match (&c.comparison, &c.note) {
                (Some(cmp), _) if !cmp.exact => format!(
                    " — {} px differ, bbox {:?}, mean channel error {:.2}",
                    cmp.changed, cmp.bbox, cmp.mean_channel_error
                ),
                (_, Some(n)) => format!(" — {n}"),
                _ => String::new(),
            };
            s.push_str(&format!(
                "  checkpoint {:<12} {}{detail}\n",
                c.label,
                c.verdict.text()
            ));
        }
        for c in &self.caveats {
            s.push_str(&format!("  caveat: {c}\n"));
        }
        for d in &self.differences {
            s.push_str(&format!("  difference: {d}\n"));
        }
        for g in &self.known_gaps {
            s.push_str(&format!("  known gap: {g}\n"));
        }
        if let Some(x) = &self.expected_failure {
            s.push_str(&format!("  expected failure: {x}\n"));
        }
        s.push_str(&format!(
            "\nOverall:\n  Local firmware execution: {}\n  Physical-device parity:   {}\n",
            self.local_text(),
            self.physical.text()
        ));
        s
    }
}

pub fn load_cases(dir: &Path) -> Result<Vec<(PathBuf, Case)>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    let mut cases = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let case: Case = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if case.repeat == 0 || case.repeat > 8 {
            return Err(format!("{}: repeat must be 1-8", path.display()));
        }
        cases.push((path, case));
    }
    Ok(cases)
}

fn input_kinds(keys: &[Key]) -> (bool, bool) {
    (
        keys.iter().any(|k| k.code != 0),
        keys.iter().any(|k| !k.touch.is_empty()),
    )
}

/// Runs one case in a fresh boot and compares it with its expectations.
pub fn run_case(cfg: &SuiteConfig, case_file: &Path, case: &Case) -> Result<CaseResult, String> {
    let program = case_file
        .parent()
        .unwrap_or(Path::new("."))
        .join(&case.program);
    let bytes = std::fs::read(&program).map_err(|e| format!("{}: {e}", program.display()))?;
    let source_sha256 = sha256::hex(&bytes);
    if source_sha256 != case.source_sha256 {
        return Err(format!(
            "{}: source SHA-256 {source_sha256} differs from the case's {}",
            program.display(),
            case.source_sha256
        ));
    }
    let firmware_sha256 = sha256::hex(&std::fs::read(&cfg.firmware).map_err(|e| e.to_string())?);
    let keys = match &case.runtime_keys {
        Some(k) => keys::parse_keys(k)?,
        None => Vec::new(),
    };
    let arguments = runtime_value::parse_arguments(&case.args)?;
    // Binary containers import as-is; plain-text sources (including text
    // saved under a .hpprgm name) import as source, as the Connectivity Kit
    // and the calculator accept them.
    let as_source = bytes.get(..4) != Some(&crate::hpprgm::MAGIC.to_le_bytes()[..]);
    let out_dir = cfg.out_dir.join("runs");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let run = RunProgramConfig {
        check: CheckConfig {
            program: program.clone(),
            name: None,
            as_source,
            firmware: cfg.firmware.clone(),
            os_rom: cfg.os_rom.clone(),
            factory_a: cfg.factory_a.clone(),
            out_dir,
            init_data: cfg.init_data.clone(),
            settle_ms: 300,
            boot_timeout: Duration::from_secs(60),
            key_timeout: Duration::from_secs(20),
            max_seconds: 300.,
            max_instructions: 8_000_000_000,
        },
        requests: (0..case.repeat)
            .map(|_| ExecutionRequest {
                entry: case.entry.clone(),
                arguments: arguments.clone(),
            })
            .collect(),
        runtime_keys: keys.clone(),
        execution_timeout: Duration::try_from_secs_f64(case.execution_timeout_s)
            .map_err(|e| e.to_string())?,
        control: ExecutionControl::default(),
        // The framebuffer-store watch is diagnostic only and slows redraw-
        // heavy screens (the syntax-coloured editor during Check) past the
        // settle window; frames are captured without it.
        trace_runtime: false,
        screenshot: None,
    };
    let report = if case.check_only {
        let c = checkprog::check(&run.check);
        let status = match c.status {
            checkprog::Status::Invalid => ExecutionStatus::CompileError,
            checkprog::Status::Timeout => ExecutionStatus::TimedOut,
            checkprog::Status::Unsupported => ExecutionStatus::Unsupported,
            checkprog::Status::InfrastructureError => ExecutionStatus::InfrastructureError,
            // Accepted, but a Check-only case never executes.
            checkprog::Status::Valid => ExecutionStatus::Indeterminate,
        };
        let mut r = ProgramReport::rejected(&run.check, status, c.evidence.clone());
        r.compile_status = c.observation.is_some().then_some(c.status);
        r.source_sha256 = c.source_sha256.clone();
        r.work_dir = c.work_dir.clone();
        r.compilation = c;
        r
    } else {
        runprog::run(&run)
    };
    if let Some(dir) = &report.work_dir {
        let _ = std::fs::write(
            dir.join("app-suite-report.json"),
            serde_json::to_vec_pretty(&report).unwrap_or_default(),
        );
    }
    Ok(compare(
        cfg,
        case,
        &program,
        source_sha256,
        firmware_sha256,
        &keys,
        &report,
    ))
}

fn compare(
    cfg: &SuiteConfig,
    case: &Case,
    program: &Path,
    source_sha256: String,
    firmware_sha256: String,
    keys: &[Key],
    r: &ProgramReport,
) -> CaseResult {
    let mut differences = Vec::new();
    let e = &case.expect;
    let compile_diagnostics: Vec<String> = r
        .compilation
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                "{}:{} {}",
                d.line.map_or("?".into(), |v| v.to_string()),
                d.column.map_or("?".into(), |v| v.to_string()),
                d.message.as_deref().unwrap_or("")
            )
        })
        .collect();
    let rejected_at = r
        .compilation
        .diagnostics
        .first()
        .and_then(|d| d.line.zip(d.column))
        .map(|(l, c)| [l, c]);
    let compilation = match r.compile_status {
        Some(checkprog::Status::Valid) if e.outcome != ExecutionStatus::CompileError => {
            Verdict::Pass
        }
        Some(checkprog::Status::Invalid)
            if e.outcome == ExecutionStatus::CompileError
                && e.compile_error_at.is_none_or(|at| Some(at) == rejected_at) =>
        {
            Verdict::Match
        }
        Some(checkprog::Status::Unsupported) => Verdict::Unsupported,
        Some(_) => {
            differences.push(format!(
                "firmware Check {:?} at {rejected_at:?}, expected {:?} at {:?}",
                r.compile_status, e.outcome, e.compile_error_at
            ));
            Verdict::Differ
        }
        None => Verdict::Indeterminate,
    };
    let import = if r.compilation.import.is_some() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    let mut execution = Verdict::Match;
    if r.execution_status != e.outcome {
        differences.push(format!(
            "outcome {:?}, expected {:?}: {}",
            r.execution_status, e.outcome, r.evidence
        ));
        execution = match r.execution_status {
            ExecutionStatus::Unsupported => Verdict::Unsupported,
            ExecutionStatus::Indeterminate => Verdict::Indeterminate,
            _ => Verdict::Differ,
        };
    }
    // Every repeated invocation must agree, not only the last one.
    for (i, inv) in r.runtime.invocations.iter().enumerate() {
        let value = inv.result.as_ref().and_then(|v| v.value.as_ref());
        let display = value
            .filter(|v| !matches!(v, FirmwareValue::Error { .. }))
            .map(FirmwareValue::display);
        let expected = e
            .return_sequence
            .as_ref()
            .map(|s| s.get(i).cloned())
            .unwrap_or_else(|| e.return_display.clone());
        if (e.return_display.is_some() || e.return_sequence.is_some()) && display != expected {
            differences.push(format!(
                "invocation {i}: return {display:?}, expected {expected:?}"
            ));
            execution = Verdict::Differ;
        }
        let undecoded = matches!(
            ReturnRepresentation::of(inv.result.as_ref()),
            ReturnRepresentation::Undecoded { .. }
        );
        if e.return_undecoded != undecoded && e.outcome == ExecutionStatus::Completed {
            differences.push(format!(
                "invocation {i}: return representation decoded={}, expected decoded={}",
                !undecoded, !e.return_undecoded
            ));
            execution = Verdict::Differ;
        }
        if let Some(expected) = &e.output {
            let printed: Vec<String> = inv.output.iter().filter_map(|o| o.text.clone()).collect();
            if &printed != expected {
                differences.push(format!(
                    "invocation {i}: output {printed:?}, expected {expected:?}"
                ));
                execution = Verdict::Differ;
            }
        }
    }
    let diagnostic = r
        .runtime
        .invocations
        .iter()
        .find_map(|c| c.diagnostic.clone());
    let runtime_error = diagnostic.as_ref().map(|d| {
        format!(
            "code {} {}",
            d.raw_code.map_or("?".into(), |c| c.to_string()),
            d.message.as_deref().unwrap_or("")
        )
    });
    let runtime_errors = match (&diagnostic, e.error_code) {
        (None, None) => Verdict::Pass,
        (Some(d), Some(code))
            if d.raw_code == Some(code)
                && e.error_message
                    .as_ref()
                    .is_none_or(|m| d.message.as_ref() == Some(m)) =>
        {
            Verdict::Match
        }
        _ => {
            differences.push(format!(
                "runtime error {runtime_error:?}, expected code {:?} {:?}",
                e.error_code, e.error_message
            ));
            Verdict::Differ
        }
    };
    let last = r.runtime.invocations.last();
    let mut checkpoints = Vec::new();
    for expect in &case.checkpoint {
        let actual_png = last.and_then(|c| {
            if expect.label == "return" {
                c.screenshot.clone()
            } else {
                c.checkpoints
                    .iter()
                    .find(|p| p.label == expect.label)
                    .and_then(|p| p.png.clone())
            }
        });
        checkpoints.push(compare_checkpoint(
            cfg,
            case,
            expect,
            actual_png,
            &firmware_sha256,
            &source_sha256,
            execution == Verdict::Match,
        ));
    }
    let display = if checkpoints.is_empty() {
        Verdict::NotTested
    } else if checkpoints.iter().any(|c| c.verdict == Verdict::Differ) {
        Verdict::Differ
    } else if checkpoints.iter().any(|c| c.verdict == Verdict::Fail) {
        Verdict::Fail
    } else if checkpoints
        .iter()
        .all(|c| c.verdict == Verdict::NotCompared)
    {
        Verdict::NotCompared
    } else if checkpoints.iter().any(|c| c.verdict == Verdict::Recorded) {
        Verdict::Recorded
    } else {
        Verdict::Match
    };
    for c in checkpoints.iter().filter(|c| c.verdict == Verdict::Differ) {
        differences.push(format!("display checkpoint {} differs", c.label));
    }
    let (uses_keys, uses_touch) = input_kinds(keys);
    let input = |used: bool| {
        if !used {
            Verdict::NotTested
        } else if execution == Verdict::Match {
            Verdict::Match
        } else {
            Verdict::Differ
        }
    };
    let physical = physical_verdict(&cfg.root, &case.id);
    CaseResult {
        id: case.id.clone(),
        category: case.category.clone(),
        file: program.display().to_string(),
        source_sha256,
        firmware_sha256,
        entry: case.entry.clone(),
        initial_state: case.initial_state.clone(),
        input_script: case.runtime_keys.clone(),
        execution_status: r.execution_status,
        return_display: r.return_display.clone(),
        return_representation: r.return_representation.clone(),
        output: r.output.iter().filter_map(|o| o.text.clone()).collect(),
        runtime_error,
        compile_diagnostics,
        caveats: r.caveats.clone(),
        compilation,
        import,
        execution,
        runtime_errors,
        display,
        keyboard: input(uses_keys),
        touch: input(uses_touch),
        physical,
        checkpoints,
        differences,
        known_gaps: case.known_gaps.clone(),
        expected_failure: case.expected_failure.clone(),
        evidence: r.evidence.clone(),
        work_dir: r.work_dir.clone(),
    }
}

/// Physical parity needs a recorded physical-G1 observation for the case;
/// none exist yet, and local-firmware evidence never substitutes for one.
fn physical_verdict(root: &Path, id: &str) -> Verdict {
    let path = root
        .join("reference/physical-g1")
        .join(format!("{id}.json"));
    match std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    {
        Some(v) if v["provenance"] == "physical-g1" && v["verdict"] == "MATCH" => Verdict::Match,
        Some(v) if v["provenance"] == "physical-g1" && v["verdict"] == "DIFFER" => Verdict::Differ,
        _ => Verdict::NotVerified,
    }
}

fn compare_checkpoint(
    cfg: &SuiteConfig,
    case: &Case,
    expect: &CheckpointExpect,
    actual_png: Option<String>,
    firmware_sha256: &str,
    source_sha256: &str,
    outcome_matched: bool,
) -> CheckpointResult {
    let reference = cfg.root.join(&expect.reference);
    let mut result = CheckpointResult {
        label: expect.label.clone(),
        verdict: Verdict::Fail,
        actual_png: actual_png.clone(),
        reference: reference.display().to_string(),
        comparison: None,
        note: None,
    };
    let Some(actual_png) = actual_png else {
        result.note = Some("checkpoint was not captured".into());
        return result;
    };
    let actual = match framediff::load(Path::new(&actual_png)) {
        Ok(f) => f,
        Err(e) => {
            result.note = Some(e);
            return result;
        }
    };
    if let Some(reason) = &expect.nondeterministic {
        result.verdict = Verdict::NotCompared;
        result.note = Some(format!("captured; not compared: {reason}"));
        return result;
    }
    if cfg.record {
        if !outcome_matched {
            result.note = Some("not recorded: the run did not match its expectations".into());
            return result;
        }
        let sidecar = Provenance {
            provenance: "local-firmware".into(),
            firmware_sha256: Some(firmware_sha256.into()),
            case: case.id.clone(),
            label: expect.label.clone(),
            pixels_sha256: actual.sha256(),
            source_sha256: source_sha256.into(),
            recorded_by: "prime-emulator app-suite --record".into(),
        };
        let written = reference
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| actual.save_png(&reference))
            .and_then(|_| {
                std::fs::write(
                    reference.with_extension("json"),
                    serde_json::to_vec_pretty(&sidecar).unwrap(),
                )
            });
        match written {
            Ok(()) => result.verdict = Verdict::Recorded,
            Err(e) => result.note = Some(e.to_string()),
        }
        return result;
    }
    let reference_frame = match framediff::load(&reference) {
        Ok(f) => f,
        Err(e) => {
            result.note = Some(format!("no reference: {e}"));
            return result;
        }
    };
    let mask = match &expect.mask {
        Some(m) => match framediff::load_mask(&cfg.root.join(m)) {
            Ok(m) => Some(m),
            Err(e) => {
                result.note = Some(e);
                return result;
            }
        },
        None => None,
    };
    match framediff::compare(&actual, &reference_frame, mask.as_deref()) {
        Ok(c) => {
            result.verdict = if c.exact {
                Verdict::Match
            } else {
                Verdict::Differ
            };
            if !c.exact {
                let diff = cfg
                    .out_dir
                    .join(format!("{}-{}-diff.png", case.id, expect.label));
                let _ = framediff::visualize(&actual, &reference_frame).save_png(&diff);
                result.note = Some(format!("diff image {}", diff.display()));
            } else if let Some(reason) = &expect.mask_reason {
                result.note = Some(format!("masked: {reason}"));
            }
            result.comparison = Some(c);
        }
        Err(e) => result.note = Some(e),
    }
    result
}

/// Runs every case, writing `<id>.json`, `<id>.txt` and `SUMMARY.md`.
pub fn run_suite(cfg: &SuiteConfig, only: Option<&str>) -> Result<Vec<CaseResult>, String> {
    let cases = load_cases(&cfg.root.join("cases"))?;
    std::fs::create_dir_all(&cfg.out_dir).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for (path, case) in cases
        .iter()
        .filter(|(_, c)| only.is_none_or(|o| o.eq_ignore_ascii_case(&c.id)))
    {
        eprintln!("app-suite: {} ({})", case.id, case.category);
        let result = run_case(cfg, path, case)?;
        std::fs::write(
            cfg.out_dir.join(format!("{}.json", case.id)),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(cfg.out_dir.join(format!("{}.txt", case.id)), result.text())
            .map_err(|e| e.to_string())?;
        eprintln!(
            "app-suite: {} {:?} — local {}",
            case.id,
            result.execution_status,
            result.local_text()
        );
        results.push(result);
    }
    std::fs::write(cfg.out_dir.join("SUMMARY.md"), summary(&results)).map_err(|e| e.to_string())?;
    Ok(results)
}

pub fn summary(results: &[CaseResult]) -> String {
    let mut s = String::from(
        "# HP Prime G1 application compatibility (local firmware)\n\n\
         References are local G1 2.4.2 firmware observations; physical parity is \
         reported separately.\n\n\
         | Case | Category | Outcome | Return | Execution | Errors | Display | Keys | Touch | Physical | Local |\n\
         |---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for r in results {
        s.push_str(&format!(
            "| {} | {} | {:?} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.id,
            r.category,
            r.execution_status,
            r.return_display
                .as_deref()
                .map(|v| v.replace('|', "\\|"))
                .unwrap_or_else(|| "—".into()),
            r.execution.text(),
            r.runtime_errors.text(),
            r.display.text(),
            r.keyboard.text(),
            r.touch.text(),
            r.physical.text(),
            r.local_text(),
        ));
    }
    s
}
