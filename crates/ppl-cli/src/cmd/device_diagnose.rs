//! Genuine firmware checks and durable agent-driven attempts. No physical
//! execution or source rewriting is hidden behind these orchestration commands.
use super::{
    device::DeviceOpts,
    device_control::{base, emit, failure, unsupported},
};
use clap::{Args, ValueEnum};
use ppl_engine::{Cancellation, Operation, Status, source::Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum DiagnoseEngine {
    Firmware,
    Physical,
}
#[derive(Debug, Args, Default)]
pub struct FirmwareOptions {
    #[arg(long)]
    firmware: Option<PathBuf>,
    #[arg(long)]
    os_rom: Option<PathBuf>,
    #[arg(long)]
    factory_a: Option<PathBuf>,
    #[arg(long)]
    init_data: Option<PathBuf>,
    #[arg(long)]
    firmware_runner: Option<PathBuf>,
    #[arg(long,default_value_t=360,value_parser=clap::value_parser!(u64).range(1..=3600))]
    timeout_secs: u64,
}
#[derive(Debug, Args)]
pub struct FileOptions {
    pub file: PathBuf,
}
#[derive(Debug, Args)]
pub struct DiagnoseOptions {
    pub file: PathBuf,
    #[arg(long, value_enum, default_value = "firmware")]
    pub engine: DiagnoseEngine,
    #[command(flatten)]
    pub firmware: FirmwareOptions,
    #[arg(long)]
    pub artifacts_dir: Option<PathBuf>,
}
#[derive(Debug, Args)]
pub struct DebugOptions {
    pub file: PathBuf,
    #[arg(long)]
    pub resume: Option<String>,
    #[arg(long,value_parser=clap::value_parser!(u32).range(1..=100))]
    pub max_attempts: Option<u32>,
    #[arg(long, default_value = "artifacts/device-debug")]
    pub artifacts_dir: PathBuf,
    #[command(flatten)]
    pub firmware: FirmwareOptions,
}
fn env_path(explicit: &Option<PathBuf>, name: &str) -> Option<PathBuf> {
    explicit.clone().or_else(|| {
        std::env::var_os(name)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    })
}
fn options(o: &FirmwareOptions, artifacts: Option<PathBuf>) -> ppl_engine::firmware::Options {
    ppl_engine::firmware::Options {
        runner: env_path(&o.firmware_runner, "PPL_FIRMWARE_RUNNER"),
        firmware: env_path(&o.firmware, "PPL_FIRMWARE"),
        os_rom: env_path(&o.os_rom, "PPL_OS_ROM"),
        factory_a: env_path(&o.factory_a, "PPL_FACTORY_A"),
        init_data: env_path(&o.init_data, "PPL_FIRMWARE_INIT_DATA"),
        timeout: Duration::from_secs(if o.timeout_secs == 0 {
            360
        } else {
            o.timeout_secs
        }),
        artifacts_dir: artifacts,
        ..Default::default()
    }
}
pub(super) fn firmware_available() -> bool {
    let opts = options(&FirmwareOptions::default(), None);
    // Availability is local only. A successful discovery never proves G1 acceptance.
    ppl_engine::firmware::discover_runner(opts.runner.as_deref()).is_ok()
        && opts.firmware.as_ref().is_some_and(|p| p.is_file())
        && opts.os_rom.as_ref().is_some_and(|p| p.is_file())
        && opts
            .factory_a
            .as_ref()
            .is_some_and(|p| p.join("programs/misc/armfir.dat").is_file())
}
fn check(source: Source, o: &FirmwareOptions, artifacts: Option<PathBuf>) -> Value {
    let mut out = base("diagnose");
    out["engine"] = json!("firmware_emulator");
    out["safe_next_actions"] = json!(["edit_local_source", "diagnose --engine firmware"]);
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    if let Err(e) = ctrlc::set_handler(move || signal.cancel()) {
        failure(&mut out, "infrastructure_error", 5, e);
        return out;
    }
    let report = ppl_engine::firmware::execute(
        source,
        Operation::Check,
        &options(o, artifacts),
        &cancellation,
    );
    let (status, code) = match report.status {
        Status::Success => ("valid", 0),
        Status::InvalidSource => ("syntax_error", 1),
        Status::TimedOut => ("timeout", 7),
        Status::Cancelled => ("cancelled", 130),
        Status::MissingFirmware => ("missing_firmware", 5),
        Status::MissingRunner => ("missing_runner", 5),
        Status::MissingResources => ("missing_resources", 5),
        Status::Unsupported => ("unsupported_capability", 6),
        _ => ("infrastructure_error", 5),
    };
    out["source"] = json!(report.source);
    out["diagnostics"] = json!(report.diagnostics);
    out["evidence_paths"] = json!(report.artifacts);
    out["compilation_status"] = json!(report.compilation_status);
    out["status"] = json!(status);
    if code != 0 {
        failure(
            &mut out,
            status,
            code,
            report
                .error
                .as_ref()
                .map(|e| e.message.as_str())
                .unwrap_or("genuine firmware rejected source"),
        );
    }
    out["firmware_report"] = json!(report);
    out
}
pub fn diagnose(opts: &DeviceOpts, o: &DiagnoseOptions) -> u8 {
    if matches!(o.engine, DiagnoseEngine::Physical) {
        return run_file(
            opts,
            &FileOptions {
                file: o.file.clone(),
            },
        );
    }
    let started = Instant::now();
    let mut out = match Source::file(&o.file) {
        Ok(source) => check(source, &o.firmware, o.artifacts_dir.clone()),
        Err(e) => {
            let mut out = base("diagnose");
            out["engine"] = json!("firmware_emulator");
            failure(&mut out, "invalid_input", 2, e);
            out
        }
    };
    out["timings"] = json!({"total_ms":started.elapsed().as_secs_f64()*1000.});
    emit(&out, opts.json)
}
pub fn run_file(opts: &DeviceOpts, o: &FileOptions) -> u8 {
    let mut out = unsupported("run_file");
    match Source::file(&o.file) {
        Ok(source) => {
            out["source"] = json!(source.identity);
            out["preflight_errors"] = staging_errors(&source);
            out["compilation_status"] = json!("not_attempted_no_verified_physical_backend");
        }
        Err(e) => failure(&mut out, "invalid_input", 2, e),
    }
    emit(&out, opts.json)
}
fn staging_errors(source: &Source) -> Value {
    let mut errors = vec![];
    if let Err(e) = super::remote_input::source(&source.text) {
        errors.push(e);
    }
    if let Err(e) = ppl_device::check_program_payload(&source.stem, &source.text) {
        errors.push(e.to_string());
    }
    json!(errors)
}

#[derive(Debug, Serialize, Deserialize)]
struct Session {
    schema_version: u32,
    id: String,
    max_attempts: u32,
    device_selector: Option<String>,
    attempts: Vec<Attempt>,
}
#[derive(Debug, Serialize, Deserialize)]
struct Attempt {
    id: String,
    input_sha256: String,
    state: String,
    source: PathBuf,
    report: PathBuf,
}
fn validate_session(
    session: &Session,
    id: &str,
    selector: Option<&str>,
    limit: Option<u32>,
) -> Result<(), String> {
    if session.schema_version != 1
        || session.id != id
        || session.max_attempts == 0
        || session.max_attempts > 100
    {
        return Err("invalid debug session identity/schema/limit".into());
    }
    if selector != session.device_selector.as_deref() {
        return Err("resume requires the original --device selector".into());
    }
    if limit.is_some_and(|n| n != session.max_attempts) {
        return Err("a resumed session cannot change its attempt limit".into());
    }
    if session.attempts.len() >= session.max_attempts as usize {
        return Err("debug attempt limit reached".into());
    }
    if session
        .attempts
        .iter()
        .any(|a| !matches!(a.state.as_str(), "checking" | "terminal"))
    {
        return Err("prior physical operation has an unresolved outcome; do not retry".into());
    }
    Ok(())
}
fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension("json.pending");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    f.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(path.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn debug(opts: &DeviceOpts, o: &DebugOptions) -> u8 {
    let mut out = base("debug");
    let result = (|| -> Result<Value, String> {
        use std::io::Write;
        let source = Source::file(&o.file)?;
        let id = o
            .resume
            .clone()
            .unwrap_or_else(|| format!("debug-{}", ppl_device::fresh_test_name()));
        if id.len() > 40
            || !id.starts_with("debug-ZZT_")
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("invalid debug session ID".into());
        }
        super::device_control::private_directory(&o.artifacts_dir, true)
            .map_err(|e| e.to_string())?;
        let dir = o.artifacts_dir.join(&id);
        if o.resume.is_none() {
            super::device_control::private_directory(&dir, false).map_err(|e| e.to_string())?;
        }
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("session.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "another process owns this debug session")?;
        let journal = dir.join("session.json");
        let mut session = if o.resume.is_some() {
            let bytes = std::fs::read(&journal).map_err(|e| e.to_string())?;
            if bytes.len() > 1024 * 1024 {
                return Err("oversized debug journal".into());
            }
            serde_json::from_slice::<Session>(&bytes).map_err(|e| e.to_string())?
        } else {
            Session {
                schema_version: 1,
                id: id.clone(),
                max_attempts: o.max_attempts.unwrap_or(5),
                device_selector: opts.device.clone(),
                attempts: vec![],
            }
        };
        validate_session(&session, &id, opts.device.as_deref(), o.max_attempts)?;
        if session.attempts.last().is_some_and(|a| {
            a.input_sha256 == source.identity.input_sha256 && a.state == "terminal"
        }) {
            return Err("source is unchanged since the previous terminal attempt; inspect its report rather than repeating it".into());
        }
        let index = session.attempts.len() + 1;
        let attempt_id = format!("attempt-{index:03}");
        let attempt_dir = dir.join(&attempt_id);
        super::device_control::private_directory(&attempt_dir, false).map_err(|e| e.to_string())?;
        let snapshot = attempt_dir.join("source.original");
        let report_path = attempt_dir.join("report.json");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&snapshot)
            .map_err(|e| e.to_string())?;
        file.write_all(&source.bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        session.attempts.push(Attempt {
            id: attempt_id.clone(),
            input_sha256: source.identity.input_sha256.clone(),
            state: "checking".into(),
            source: snapshot.clone(),
            report: report_path.clone(),
        });
        save(&journal, &session)?;
        let mut report = check(
            source.clone(),
            &o.firmware,
            Some(attempt_dir.join("firmware")),
        );
        let local = report.clone();
        if report["status"] == "valid" {
            report = unsupported("debug");
            report["source"] = json!(source.identity);
            report["preflight_errors"] = staging_errors(&source);
            report["firmware_check"] = local;
        }
        report["operation"] = json!("debug");
        report["session_id"] = json!(id);
        report["attempt_id"] = json!(attempt_id);
        report["attempts_used"] = json!(index);
        report["max_attempts"] = json!(session.max_attempts);
        report["agent_action"] = json!(if report["status"] == "syntax_error" {
            "edit_local_source_then_resume"
        } else if report["status"] == "unsupported_capability" {
            "physical_backend_verification_required"
        } else {
            "inspect_report"
        });
        report["source_snapshot"] = json!(snapshot);
        report["evidence_paths"]
            .as_array_mut()
            .unwrap()
            .extend([json!(report_path), json!(journal)]);
        save(&report_path, &report)?;
        session.attempts.last_mut().unwrap().state = "terminal".into();
        save(&journal, &session)?;
        Ok(report)
    })();
    match result {
        Ok(report) => out = report,
        Err(e) => failure(&mut out, "refused", 2, e),
    }
    emit(&out, opts.json)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            schema_version: 1,
            id: "debug-ZZT_A".into(),
            max_attempts: 2,
            device_selector: Some("hashed".into()),
            attempts: vec![],
        }
    }
    #[test]
    fn resume_cannot_widen_limits_change_device_or_retry_unknown_execution() {
        let mut s = session();
        assert!(validate_session(&s, &s.id, Some("hashed"), None).is_ok());
        assert!(validate_session(&s, &s.id, Some("other"), None).is_err());
        assert!(validate_session(&s, &s.id, Some("hashed"), Some(3)).is_err());
        s.attempts.push(Attempt {
            id: "a".into(),
            input_sha256: "x".into(),
            state: "physical_submitting".into(),
            source: PathBuf::new(),
            report: PathBuf::new(),
        });
        assert!(validate_session(&s, &s.id, Some("hashed"), None).is_err());
        s.attempts[0].state = "terminal".into();
        s.attempts.push(Attempt {
            id: "b".into(),
            input_sha256: "y".into(),
            state: "terminal".into(),
            source: PathBuf::new(),
            report: PathBuf::new(),
        });
        assert!(validate_session(&s, &s.id, Some("hashed"), None).is_err());
    }
}
