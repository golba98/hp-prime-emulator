//! Isolated, UI-driven firmware compilation. No dependency on the native
//! PPL syntax/compiler crates: source syntax is decided exclusively by HP.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::calcverify::{DEFAULT_SETUP_KEYS, copy_tree};
use crate::compiler::{self, CheckObservation, CompilerTrace, RawPosition};
use crate::import::{ImportReport, ImportRequest};
use crate::runner::{self, Outcome, RunConfig};
use crate::script::ScriptConfig;
use crate::services::UnsupportedPolicy;
use crate::trace::TraceConfig;
use crate::{hpprgm, import, keys, sha256};

pub const CHECK_KEYS: &str = "SHIFT,1,TAP:26:229,TAP:293:229,ENTER,HOME,ESC,IDLE";
pub const CHECK_STEP: usize = 3;

#[derive(Debug, Clone)]
pub struct CheckConfig {
    pub program: PathBuf,
    pub name: Option<String>,
    pub as_source: bool,
    pub firmware: PathBuf,
    pub os_rom: Option<PathBuf>,
    pub factory_a: PathBuf,
    /// Parent for a unique run directory; existing contents are preserved.
    pub out_dir: PathBuf,
    /// Optional program-free, initialized storage, copied before use.
    pub init_data: Option<PathBuf>,
    pub settle_ms: u64,
    pub boot_timeout: Duration,
    pub key_timeout: Duration,
    pub max_seconds: f64,
    pub max_instructions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Valid,
    Invalid,
    InfrastructureError,
    Timeout,
    Unsupported,
}

impl Status {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Valid => 0,
            Self::Invalid => 7,
            Self::InfrastructureError => 4,
            Self::Timeout => 3,
            Self::Unsupported => 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: String,
    pub code: Option<String>,
    pub message: Option<String>,
    pub line: Option<usize>,
    /// One-based Unicode-scalar column, not a Rust byte column.
    pub column: Option<usize>,
    pub raw_position: Option<RawPosition>,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub engine: &'static str,
    pub program: String,
    pub status: Status,
    pub evidence: String,
    pub firmware_sha256: Option<String>,
    pub source_sha256: Option<String>,
    pub import: Option<ImportReport>,
    pub diagnostics: Vec<Diagnostic>,
    pub observation: Option<CheckObservation>,
    pub work_dir: Option<PathBuf>,
    pub source_preserved: bool,
    pub exports_before: Vec<(String, u32)>,
    pub exports_after: Vec<(String, u32)>,
    pub changed_sections: Vec<u32>,
    pub timings: Timings,
}

#[derive(Debug, Default, Serialize)]
pub struct Timings {
    pub initialize_s: f64,
    pub import_s: f64,
    pub boot_s: Option<f64>,
    pub discovery_s: Option<f64>,
    pub compiler_ms: Option<f64>,
    pub diagnostic_ms: Option<f64>,
    pub guest_session_s: f64,
    pub instructions: u64,
    pub instructions_per_second: Option<f64>,
    pub total_s: f64,
}

/// Validate raw guest facts before deriving any source verdict. A stale,
/// missing, mismatched, partial or truncated observation is indeterminate.
pub fn validate<'a>(
    trace: &'a CompilerTrace,
    source: &str,
    step: usize,
) -> Result<&'a CheckObservation, String> {
    if trace.dropped != 0 {
        return Err("compiler observations were dropped".into());
    }
    let candidates: Vec<_> = trace
        .checks
        .iter()
        .filter(|c| c.kind.is_none() && c.step == Some(step))
        .collect();
    if candidates.len() != 1 {
        return Err("expected exactly one Check invocation at this step".into());
    }
    let c = candidates[0];
    if !c.errors.is_empty() {
        return Err(format!("guest observation errors: {:?}", c.errors));
    }
    if !c.compiler_entered || c.cpsr & 0x20 != 0 || c.program_va.is_none() || c.result_va.is_none()
    {
        return Err("compiler entry, ARM state or context was not verified".into());
    }
    if c.sources.len() != 1
        || c.parser_returns.len() != 1
        || c.sources[0].source_index != 0
        || c.sources[0].sha256 != sha256::hex(source.as_bytes())
        || c.sources[0].utf16_units != source.encode_utf16().count()
    {
        return Err("compiler did not observe exactly the imported PPL source".into());
    }
    if !matches!(c.return_value, Some(0 | 1)) || c.message.as_ref().is_none_or(|s| s.is_empty()) {
        return Err("missing or unrecognized compiler return/dialog".into());
    }
    let (Some(returned), Some(message)) = (c.returned_ms, c.message_ms) else {
        return Err("missing compiler completion timing".into());
    };
    if !c.started_ms.is_finite()
        || !returned.is_finite()
        || !message.is_finite()
        || c.started_ms > returned
        || returned > message
    {
        return Err("invalid compiler observation ordering".into());
    }
    if c.return_value == Some(1) {
        if c.raw_position.is_some()
            || c.dialog_resource != Some(0x410)
            || !matches!(c.parser_returns[0], 0 | 1)
        {
            return Err("inconsistent successful Check evidence".into());
        }
    } else if c.dialog_resource != Some(0x3416) || c.raw_position.is_none() {
        return Err("unrecognized failed Check presentation".into());
    }
    Ok(c)
}

/// The diagnostic for a rejected, validated Check of `source`. The position
/// is mapped from the guest's raw UTF-16 offset; when it cannot be mapped it
/// stays unavailable rather than being estimated.
pub fn firmware_diagnostic(source: &str, c: &CheckObservation) -> Diagnostic {
    let mapped = c
        .raw_position
        .as_ref()
        .and_then(|p| compiler::position(source, p));
    Diagnostic {
        severity: "error".into(),
        code: None,
        message: c.message.clone(),
        line: mapped.map(|p| p.0),
        column: mapped.map(|p| p.1),
        raw_position: c.raw_position.clone(),
        source: "guest-firmware".into(),
    }
}

pub fn outcome_status(outcome: Outcome) -> Option<Status> {
    match outcome {
        Outcome::ScriptComplete | Outcome::ProgramComplete => None,
        Outcome::LimitReached | Outcome::ScriptFailed => Some(Status::Timeout),
        Outcome::Fault | Outcome::UnsupportedStop => Some(Status::InfrastructureError),
        Outcome::Heartbeat => Some(Status::Unsupported),
    }
}

pub(crate) fn pending(cfg: &CheckConfig) -> CheckReport {
    CheckReport {
        schema_version: 1,
        request_id: None,
        engine: "firmware",
        program: cfg
            .name
            .clone()
            .or_else(|| {
                cfg.program
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default(),
        status: Status::InfrastructureError,
        evidence: String::new(),
        firmware_sha256: None,
        source_sha256: None,
        import: None,
        diagnostics: Vec::new(),
        observation: None,
        work_dir: None,
        source_preserved: false,
        exports_before: Vec::new(),
        exports_after: Vec::new(),
        changed_sections: Vec::new(),
        timings: Timings::default(),
    }
}

pub fn check(cfg: &CheckConfig) -> CheckReport {
    let started = Instant::now();
    let mut r = pending(cfg);
    if let Err(e) = perform(cfg, &mut r) {
        r.evidence = e;
    }
    r.timings.total_s = started.elapsed().as_secs_f64();
    r
}

fn write_report(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

pub(crate) fn session_config(
    cfg: &CheckConfig,
    work: &Path,
    label: &str,
    script: &str,
    observe: bool,
) -> Result<RunConfig, String> {
    let dir = work.join(label);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(RunConfig {
        firmware_path: cfg.firmware.clone(),
        os_rom_path: cfg.os_rom.clone(),
        data_dir: work.join("data"),
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        max_instructions: cfg.max_instructions,
        max_seconds: cfg.max_seconds,
        trace: TraceConfig {
            compiler: observe,
            fs_trace: true,
            verbosity: 0,
            ..Default::default()
        },
        script: Some(ScriptConfig {
            keys: keys::parse_keys(script)?,
            settle_ms: cfg.settle_ms,
            boot_timeout: cfg.boot_timeout,
            key_timeout: cfg.key_timeout,
            screenshot_dir: Some(dir.join("shots")),
            ..Default::default()
        }),
        ui_timers: true,
        storage_diff: true,
        ..Default::default()
    })
}

fn session(
    cfg: &CheckConfig,
    work: &Path,
    label: &str,
    script: &str,
    observe: bool,
) -> Result<runner::RunSummary, String> {
    let dir = work.join(label);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = runner::run(&session_config(cfg, work, label, script, observe)?)
        .map_err(|e| e.to_string())?;
    write_report(&dir.join("report.json"), &result.report)?;
    Ok(result)
}

fn require_complete(r: &mut CheckReport, result: &runner::RunSummary) -> Result<(), String> {
    if let Some(status) = outcome_status(result.outcome) {
        r.status = status;
        return Err(format!(
            "{}: {}",
            result.report.outcome, result.report.evidence
        ));
    }
    if result.report.fault.is_some() || !result.report.outcomes.firmware_execution {
        return Err("guest execution failed".into());
    }
    Ok(())
}

fn create_work_dir(parent: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    tempfile::Builder::new()
        .prefix("check-")
        .tempdir_in(parent)
        .map(tempfile::TempDir::keep)
        .map_err(|e| e.to_string())
}

pub(crate) struct Prepared {
    pub work: PathBuf,
    pub source: String,
    pub before: Vec<u8>,
}

fn perform(cfg: &CheckConfig, r: &mut CheckReport) -> Result<(), String> {
    let prepared = prepare(cfg, r)?;
    let result = session(cfg, &prepared.work, "check", CHECK_KEYS, true)?;
    finish(r, &prepared, &result)
}

pub(crate) fn prepare(cfg: &CheckConfig, r: &mut CheckReport) -> Result<Prepared, String> {
    if !cfg.max_seconds.is_finite()
        || cfg.max_seconds <= 0.0
        || cfg.max_instructions == 0
        || cfg.boot_timeout.is_zero()
        || cfg.key_timeout.is_zero()
        || cfg.settle_ms == 0
    {
        return Err("execution and settling limits must be positive and finite".into());
    }
    let fw = std::fs::read(&cfg.firmware).map_err(|e| e.to_string())?;
    let hash = sha256::hex(&fw);
    r.firmware_sha256 = Some(hash.clone());
    if hash != compiler::FIRMWARE_SHA256 {
        r.status = Status::Unsupported;
        return Err("no verified compiler-observation profile for this firmware SHA-256".into());
    }
    let work = create_work_dir(&cfg.out_dir)?;
    r.work_dir = Some(work.clone());
    let init_start = Instant::now();
    if let Some(init) = &cfg.init_data {
        if !init.join("C/DATA").is_dir() {
            return Err("init-data has no initialized C/DATA".into());
        }
        for entry in std::fs::read_dir(init.join("C/DATA")).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("hpprgm"))
            {
                return Err(
                    "init-data must be program-free (it is never cleaned or modified)".into(),
                );
            }
        }
        copy_tree(init, &work.join("data")).map_err(|e| e.to_string())?;
    } else {
        if !cfg.factory_a.join("programs/misc/armfir.dat").is_file() {
            return Err("factory-a lacks programs/misc/armfir.dat".into());
        }
        copy_tree(&cfg.factory_a, &work.join("data/A")).map_err(|e| e.to_string())?;
        let initialized = session(
            cfg,
            &work,
            "init",
            &format!("{DEFAULT_SETUP_KEYS},IDLE"),
            false,
        )?;
        require_complete(r, &initialized)?;
    }
    r.timings.initialize_s = init_start.elapsed().as_secs_f64();
    let import_start = Instant::now();
    let imported = import::import(&ImportRequest {
        program: cfg.program.clone(),
        data_dir: work.join("data"),
        name: cfg.name.clone(),
        as_source: cfg.as_source,
        replace: false,
    })
    .map_err(|e| e.to_string())?;
    r.program = imported.name.clone();
    r.source_sha256 = Some(imported.source_sha256.clone());
    let before = std::fs::read(&imported.host_path).map_err(|e| e.to_string())?;
    let original = hpprgm::parse(&before).map_err(|e| e.to_string())?;
    let source = original.source().map_err(|e| e.to_string())?.source;
    r.exports_before = original.exports();
    std::fs::write(work.join("before.hpprgm"), &before).map_err(|e| e.to_string())?;
    r.import = Some(imported);
    r.timings.import_s = import_start.elapsed().as_secs_f64();
    Ok(Prepared {
        work,
        source,
        before,
    })
}

pub(crate) fn finish(
    r: &mut CheckReport,
    prepared: &Prepared,
    result: &runner::RunSummary,
) -> Result<(), String> {
    let Prepared {
        work,
        source,
        before,
    } = prepared;
    let run = &result.report;
    r.timings.guest_session_s = run.elapsed_secs;
    r.timings.instructions = run.instructions;
    r.timings.instructions_per_second =
        (run.elapsed_secs > 0.0).then(|| run.instructions as f64 / run.elapsed_secs);
    r.timings.boot_s = run
        .display
        .script
        .as_ref()
        .and_then(|s| s.boot_settled_ms)
        .or_else(|| run.runtime.as_ref().and_then(|r| r.boot_settled_ms))
        .map(|ms| ms as f64 / 1000.0);
    r.timings.discovery_s = run
        .fs_events
        .iter()
        .filter(|e| e["step"].is_null() && e["op"] == "fread")
        .filter_map(|e| e["t"].as_f64())
        .reduce(f64::max);
    // Preserve whatever the guest wrote, even when the invocation failed.
    let imported = r.import.as_ref().unwrap();
    let after = std::fs::read(&imported.host_path).map_err(|e| e.to_string())?;
    std::fs::write(work.join("after.hpprgm"), &after).map_err(|e| e.to_string())?;
    let saved = hpprgm::parse(&after).map_err(|e| e.to_string())?;
    r.source_preserved = saved.source().map_err(|e| e.to_string())?.source == *source;
    r.exports_after = saved.exports();
    let canonical_before =
        hpprgm::parse(&hpprgm::canonical_padding(before).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let canonical_after =
        hpprgm::parse(&hpprgm::canonical_padding(&after).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut types: Vec<_> = canonical_before
        .objects
        .iter()
        .chain(&canonical_after.objects)
        .map(|o| o.ty)
        .collect();
    types.sort_unstable();
    types.dedup();
    r.changed_sections = types
        .into_iter()
        .filter(|ty| {
            let one = |c: &hpprgm::Container| {
                hpprgm::Container {
                    objects: c.objects.iter().filter(|o| o.ty == *ty).cloned().collect(),
                }
                .to_bytes()
            };
            one(&canonical_before) != one(&canonical_after)
        })
        .collect();
    if !run.runtime.as_ref().is_some_and(|s| s.compile_verified) {
        require_complete(r, result)?;
    }
    if !r.source_preserved {
        return Err("firmware changed the imported source".into());
    }
    r.status = Status::Unsupported;
    if run.fs_events_dropped != 0 {
        return Err("filesystem evidence was truncated".into());
    }
    let enumerated: Vec<_> = run
        .fs_events
        .iter()
        .filter(|e| e["op"] == "wfindnext")
        .filter_map(|e| e["name"].as_str())
        .filter(|n| n.to_ascii_lowercase().ends_with(".hpprgm"))
        .collect();
    if enumerated != [format!("{}.hpprgm", r.program)] {
        return Err(format!(
            "program discovery identity is ambiguous: {enumerated:?}"
        ));
    }
    if run
        .unsupported_hits
        .iter()
        .any(|h| ![0x102cb, 0x10293].contains(&h.id))
    {
        return Err("unverified OS service was reached; inspect check/report.json".into());
    }
    let trace = run
        .compiler
        .as_ref()
        .ok_or("no guest compiler observations")?;
    let c = validate(trace, source, CHECK_STEP)?;
    r.observation = Some(c.clone());
    r.timings.compiler_ms = c.returned_ms.map(|t| t - c.started_ms);
    r.timings.diagnostic_ms = c.message_ms.zip(c.returned_ms).map(|(m, t)| m - t);
    if c.return_value == Some(1) {
        r.status = Status::Valid;
    } else {
        r.status = Status::Invalid;
        r.diagnostics.push(firmware_diagnostic(source, c));
    }
    r.evidence = "verified UI Check, compiler entry, exact parser source, compiler return and guest dialog; source preserved".into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_directories_are_unique_and_never_clean_the_parent() {
        let parent = tempfile::tempdir().unwrap();
        let sentinel = parent.path().join("existing-data");
        std::fs::write(&sentinel, b"preserve").unwrap();
        let first = create_work_dir(parent.path()).unwrap();
        std::fs::write(first.join("source"), b"first revision").unwrap();
        let second = create_work_dir(parent.path()).unwrap();
        assert_ne!(first, second);
        assert!(!second.join("source").exists());
        assert_eq!(
            std::fs::read(first.join("source")).unwrap(),
            b"first revision"
        );
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve");
    }
}
