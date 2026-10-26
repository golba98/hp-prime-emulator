//! Headless program execution using the existing firmware session pipeline.
use crate::runtime::{ExecutionRequest, ExecutionStatus, RuntimeTrace};
use crate::runtime_value::FirmwareValue;
use crate::{
    checkprog::{self, CheckConfig, CheckReport},
    hpprgm,
    keys::{self, Key},
    memory::GuestMemory,
    runner::{self, Outcome, RunConfig},
    script::{ScriptConfig, ScriptDriver, Tick},
    services::SharedState,
    sha256,
    trace::TraceHub,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct ExecutionControl {
    cancel: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
}
impl ExecutionControl {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    /// 0 preparing, 1 checking, 2 invoking, 3 executing, 4 finishing, 5 stopped.
    pub fn phase(&self) -> u8 {
        self.phase.load(Ordering::Acquire)
    }
}
#[derive(Debug, Clone)]
pub struct RunProgramConfig {
    pub check: CheckConfig,
    pub requests: Vec<ExecutionRequest>,
    pub runtime_keys: Vec<Key>,
    pub execution_timeout: Duration,
    pub control: ExecutionControl,
    pub trace_runtime: bool,
    pub screenshot: Option<PathBuf>,
}
#[derive(Debug, Serialize)]
pub struct ProgramReport {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub engine: &'static str,
    pub program: String,
    pub source_sha256: Option<String>,
    pub execution_status: ExecutionStatus,
    pub evidence: String,
    pub compile_status: Option<checkprog::Status>,
    pub compilation: CheckReport,
    pub runtime: RuntimeTrace,
    pub return_value: Option<FirmwareValue>,
    pub output: Vec<crate::runtime::OutputEvent>,
    pub runtime_errors: Vec<crate::runtime::RuntimeDiagnostic>,
    pub work_dir: Option<PathBuf>,
    pub raw_report: Option<PathBuf>,
    pub total_s: f64,
    pub guest_session_s: Option<f64>,
    pub guest_instructions: Option<u64>,
    pub screenshot_s: Option<f64>,
}
impl ProgramReport {
    pub fn rejected(check: &CheckConfig, status: ExecutionStatus, why: impl Into<String>) -> Self {
        let compilation = checkprog::pending(check);
        Self {
            schema_version: 1,
            request_id: None,
            engine: "firmware",
            program: compilation.program.clone(),
            source_sha256: None,
            execution_status: status,
            evidence: why.into(),
            compile_status: None,
            compilation,
            runtime: RuntimeTrace::default(),
            return_value: None,
            output: Vec::new(),
            runtime_errors: Vec::new(),
            work_dir: None,
            raw_report: None,
            total_s: 0.0,
            guest_session_s: None,
            guest_instructions: None,
            screenshot_s: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub program: String,
    pub source: String,
    pub program_path: PathBuf,
    pub auto_entry_stem: Option<String>,
    pub requests: Vec<ExecutionRequest>,
    pub runtime_keys: Vec<Key>,
    pub execution_timeout: Duration,
    pub control: ExecutionControl,
    pub script: ScriptConfig,
    pub out_dir: PathBuf,
    pub max_seconds: f64,
}

enum Phase {
    Checking(ScriptDriver),
    Invoking(ScriptDriver),
    Executing(Option<ScriptDriver>),
    Finishing(ScriptDriver),
    Stopped,
}
pub struct Driver {
    plan: Plan,
    phase: Phase,
    index: usize,
    started: Instant,
    execution_started: Option<Instant>,
    program_context: Option<u32>,
    declared_arity: usize,
    source_hash: String,
    first_failure: Option<ExecutionStatus>,
    published_entry: Option<usize>,
    step: Option<usize>,
}
impl Driver {
    pub fn new(plan: Plan) -> Self {
        let phase = Phase::Checking(ScriptDriver::new(plan.script.clone()));
        let source_hash = sha256::hex(plan.source.as_bytes());
        Self {
            plan,
            phase,
            index: 0,
            started: Instant::now(),
            execution_started: None,
            program_context: None,
            declared_arity: 0,
            source_hash,
            first_failure: None,
            published_entry: None,
            step: None,
        }
    }
    pub fn current_step(&self) -> Option<usize> {
        self.step
    }
    fn stop(&mut self, trace: &TraceHub, status: ExecutionStatus, why: impl Into<String>) -> Tick {
        trace.runtime.as_ref().unwrap().terminal(status, why);
        self.plan.control.phase.store(5, Ordering::Release);
        self.phase = Phase::Stopped;
        Tick::Done
    }
    fn script(&self, keys: Vec<Key>, label: &str) -> ScriptDriver {
        let mut cfg = self.plan.script.clone();
        cfg.keys = keys;
        cfg.screenshot_dir = Some(self.plan.out_dir.join(format!("{label}-{:02}", self.index)));
        ScriptDriver::new(cfg)
    }
    fn invoke(&mut self, trace: &TraceHub) -> Result<(), (ExecutionStatus, String)> {
        let bytes = std::fs::read(&self.plan.program_path)
            .map_err(|e| (ExecutionStatus::InfrastructureError, e.to_string()))?;
        let container = hpprgm::parse(&bytes)
            .map_err(|e| (ExecutionStatus::InfrastructureError, e.to_string()))?;
        if let Some(stem) = &self.plan.auto_entry_stem {
            let names: Vec<_> = container
                .exports()
                .into_iter()
                .filter(|(_, f)| f & 1 != 0)
                .map(|(n, _)| n)
                .collect();
            let matching: Vec<_> = names
                .iter()
                .filter(|n| n.eq_ignore_ascii_case(stem))
                .collect();
            let selected = if matching.len() == 1 {
                Some(matching[0].clone())
            } else if names.len() == 1 {
                Some(names[0].clone())
            } else {
                None
            };
            self.plan.requests[self.index].entry = selected.ok_or_else(|| {
                (
                    ExecutionStatus::InvalidEntry,
                    format!(
                        "select --entry explicitly; available exports: {}",
                        names.join(", ")
                    ),
                )
            })?;
        }
        let request = &self.plan.requests[self.index];
        let found = container
            .exports()
            .into_iter()
            .find(|(name, flags)| name == &request.entry && flags & 1 != 0);
        if found.is_none() {
            return Err((
                ExecutionStatus::InvalidEntry,
                format!(
                    "no matching exported entry {} in the checked firmware cache",
                    request.entry
                ),
            ));
        }
        let flags = found.unwrap().1;
        self.declared_arity = ((flags >> 4) & 31) as usize;
        let keys = crate::runtime_value::invocation_keys(&request.entry, &request.arguments)
            .map_err(|e| (ExecutionStatus::Unsupported, e))?;
        trace
            .runtime
            .as_ref()
            .unwrap()
            .arm(
                self.program_context.unwrap(),
                &self.source_hash,
                request,
                trace.elapsed_secs() * 1000.0,
            )
            .map_err(|e| (ExecutionStatus::Unsupported, e))?;
        trace.runtime.as_ref().unwrap().cache_flags(flags);
        self.plan.control.phase.store(2, Ordering::Release);
        self.execution_started = None;
        self.phase = Phase::Invoking(self.script(keys, "invoke"));
        Ok(())
    }
    pub fn tick(&mut self, mem: &GuestMemory, shared: &SharedState, trace: &TraceHub) -> Tick {
        let observer = trace.runtime.as_ref().unwrap();
        if matches!(self.phase, Phase::Stopped) {
            return Tick::Done;
        }
        if self.plan.control.cancelled() {
            observer.finish_invocation(ExecutionStatus::Cancelled);
            return self.stop(
                trace,
                ExecutionStatus::Cancelled,
                "execution cancelled by caller",
            );
        }
        if self.started.elapsed().as_secs_f64() >= self.plan.max_seconds {
            observer.finish_invocation(ExecutionStatus::TimedOut);
            return self.stop(
                trace,
                ExecutionStatus::TimedOut,
                "guest session wall deadline reached",
            );
        }
        if let Some(status) = observer.terminal_status() {
            return self.stop(trace, status, "runtime observer stopped the guest");
        }
        if self
            .execution_started
            .is_some_and(|t| t.elapsed() >= self.plan.execution_timeout)
        {
            observer.finish_invocation(ExecutionStatus::TimedOut);
            return self.stop(
                trace,
                ExecutionStatus::TimedOut,
                "PPL execution deadline reached",
            );
        }
        if self.published_entry != Some(self.index) {
            if let Some(evidence) = observer.body_entry_evidence() {
                if evidence["invocation"].as_u64() == Some(self.index as u64) {
                    let path = self
                        .plan
                        .out_dir
                        .join(format!("runtime-entry-{:02}.json", self.index));
                    if let Err(e) = save_json(&path, &evidence) {
                        return self.stop(trace, ExecutionStatus::InfrastructureError, e);
                    }
                    self.published_entry = Some(self.index);
                }
            }
        }
        match &mut self.phase {
            Phase::Checking(driver) => {
                self.plan.control.phase.store(1, Ordering::Release);
                let tick = driver.tick(mem, shared);
                self.step = driver.current_step();
                match tick {
                    Tick::Continue => {}
                    Tick::Failed(e) => {
                        return self.stop(
                            trace,
                            ExecutionStatus::TimedOut,
                            format!("checking UI: {e}"),
                        );
                    }
                    Tick::Done => {
                        observer.boot_ms(driver.report().boot_settled_ms);
                        let log = trace.compiler.as_ref().unwrap().snapshot();
                        let c = match checkprog::validate(
                            &log,
                            &self.plan.source,
                            checkprog::CHECK_STEP,
                        ) {
                            Ok(c) => c,
                            Err(e) => return self.stop(trace, ExecutionStatus::Unsupported, e),
                        };
                        if c.return_value != Some(1) {
                            return self.stop(
                                trace,
                                ExecutionStatus::CompileError,
                                "original firmware rejected the source; runtime was not armed",
                            );
                        }
                        let (events, dropped) = trace.fs_log();
                        let names: Vec<_> = events
                            .iter()
                            .filter(|e| e["op"] == "wfindnext")
                            .filter_map(|e| e["name"].as_str())
                            .filter(|n| n.to_ascii_lowercase().ends_with(".hpprgm"))
                            .collect();
                        if dropped != 0 || names != [format!("{}.hpprgm", self.plan.program)] {
                            return self.stop(
                                trace,
                                ExecutionStatus::Unsupported,
                                "program discovery was ambiguous or truncated",
                            );
                        }
                        self.program_context = c.program_va;
                        observer.compiled();
                        if let Err((status, e)) = self.invoke(trace) {
                            return self.stop(trace, status, e);
                        }
                    }
                }
            }
            Phase::Invoking(driver) => {
                let tick = driver.tick(mem, shared);
                self.step = driver.current_step();
                let (started, ended) = observer.progress();
                if started {
                    self.execution_started = Some(Instant::now());
                    self.plan.control.phase.store(3, Ordering::Release);
                    if ended {
                        self.begin_finish(trace);
                    } else {
                        let input = (!self.plan.runtime_keys.is_empty())
                            .then(|| self.script(self.plan.runtime_keys.clone(), "input"));
                        self.phase = Phase::Executing(input);
                    }
                } else {
                    match tick {
                        Tick::Failed(e) => {
                            return self.stop(
                                trace,
                                ExecutionStatus::TimedOut,
                                format!("invocation UI: {e}"),
                            );
                        }
                        Tick::Done => {
                            return self.stop(
                                trace,
                                observer.invocation_rejected(mem, shared, self.declared_arity != self.plan.requests[self.index].arguments.len()),
                                "Home evaluation finished before a user-function frame; see guest invocation diagnostic",
                            );
                        }
                        _ => {}
                    }
                }
            }
            Phase::Executing(input) => {
                if observer.progress().1 {
                    self.begin_finish(trace);
                } else if let Some(driver) = input {
                    let tick = driver.tick(mem, shared);
                    self.step = driver.current_step();
                    match tick {
                        Tick::Done => *input = None,
                        Tick::Failed(e) => {
                            return self.stop(
                                trace,
                                ExecutionStatus::TimedOut,
                                format!("runtime input: {e}"),
                            );
                        }
                        _ => {}
                    }
                }
            }
            Phase::Finishing(driver) => {
                let tick = driver.tick(mem, shared);
                self.step = driver.current_step();
                match tick {
                    Tick::Failed(e) => {
                        return self.stop(
                            trace,
                            ExecutionStatus::TimedOut,
                            format!("runtime presentation: {e}"),
                        );
                    }
                    Tick::Done => {
                        observer.capture_error_presentation(mem, shared);
                        self.index += 1;
                        self.execution_started = None;
                        if self.index == self.plan.requests.len() {
                            return self.stop(trace,self.first_failure.unwrap_or(ExecutionStatus::Completed),"genuine compiler, runtime entry, frame teardown and returned guest value observed");
                        }
                        if let Err((status, e)) = self.invoke(trace) {
                            return self.stop(trace, status, e);
                        }
                    }
                    _ => {}
                }
            }
            Phase::Stopped => return Tick::Done,
        }
        Tick::Continue
    }
    fn begin_finish(&mut self, trace: &TraceHub) {
        let observer = trace.runtime.as_ref().unwrap();
        let status = observer.finish_observed();
        if status != ExecutionStatus::Completed && self.first_failure.is_none() {
            self.first_failure = Some(status);
        }
        self.execution_started = None;
        self.plan.control.phase.store(4, Ordering::Release);
        self.phase = Phase::Finishing(self.script(keys::parse_keys("IDLE").unwrap(), "present"));
    }
}

pub fn run(cfg: &RunProgramConfig) -> ProgramReport {
    run_selected(cfg, None)
}

/// Resolve a single omitted entry from the newly compiled guest export cache.
/// Explicit-entry callers retain the existing `run` contract.
pub fn run_auto_entry(cfg: &RunProgramConfig, preferred_stem: &str) -> ProgramReport {
    run_selected(cfg, Some(preferred_stem.to_owned()))
}
fn run_selected(cfg: &RunProgramConfig, auto_entry_stem: Option<String>) -> ProgramReport {
    let started = Instant::now();
    let mut r = ProgramReport::rejected(&cfg.check, ExecutionStatus::InfrastructureError, "");
    let result = perform(cfg, &mut r, auto_entry_stem);
    if let Err(e) = result {
        if r.execution_status == ExecutionStatus::Completed {
            r.execution_status = ExecutionStatus::InfrastructureError;
        }
        r.evidence = e;
    }
    r.total_s = started.elapsed().as_secs_f64();
    r
}
fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
fn perform(
    cfg: &RunProgramConfig,
    r: &mut ProgramReport,
    auto_entry_stem: Option<String>,
) -> Result<(), String> {
    if cfg.requests.is_empty()
        || cfg.requests.len() > crate::runtime::MAX_INVOCATIONS
        || cfg.execution_timeout.is_zero()
    {
        r.execution_status = ExecutionStatus::InvalidArguments;
        return Err("require 1..32 requests and a positive execution timeout".into());
    }
    if cfg.control.cancelled() {
        r.execution_status = ExecutionStatus::Cancelled;
        return Err("cancelled before initialization".into());
    }
    if auto_entry_stem.is_some() && (cfg.requests.len() != 1 || !cfg.requests[0].entry.is_empty()) {
        r.execution_status = ExecutionStatus::InvalidArguments;
        return Err("automatic selection requires one request with an omitted entry".into());
    }
    for req in cfg.requests.iter().filter(|_| auto_entry_stem.is_none()) {
        crate::runtime_value::invocation_keys(&req.entry, &req.arguments).inspect_err(|_| {
            r.execution_status = ExecutionStatus::InvalidArguments;
        })?;
    }
    // Reuse M5 container/source validation before spending time on a guest boot.
    // This deliberately does not parse or compile PPL syntax.
    crate::import::prepare(&crate::import::ImportRequest {
        program: cfg.check.program.clone(),
        data_dir: cfg.check.out_dir.clone(),
        name: cfg.check.name.clone(),
        as_source: cfg.check.as_source,
        replace: false,
    })
    .map_err(|e| e.to_string())?;
    let p = checkprog::prepare(&cfg.check, &mut r.compilation).inspect_err(|_| {
        if r.compilation.status == checkprog::Status::Unsupported {
            r.execution_status = ExecutionStatus::Unsupported;
        }
    })?;
    r.program = r.compilation.program.clone();
    r.source_sha256 = r.compilation.source_sha256.clone();
    r.work_dir = Some(p.work.clone());
    let mut config: RunConfig =
        checkprog::session_config(&cfg.check, &p.work, "execute", checkprog::CHECK_KEYS, true)?;
    config.trace.runtime = true;
    config.lcd_watch = cfg.trace_runtime;
    config.program = Some(Plan {
        program: r.program.clone(),
        source: p.source.clone(),
        program_path: PathBuf::from(&r.compilation.import.as_ref().unwrap().host_path),
        auto_entry_stem,
        requests: cfg.requests.clone(),
        runtime_keys: cfg.runtime_keys.clone(),
        execution_timeout: cfg.execution_timeout,
        control: cfg.control.clone(),
        script: config.script.clone().unwrap(),
        out_dir: p.work.join("execute"),
        max_seconds: cfg.check.max_seconds,
    });
    let result = runner::run(&config).map_err(|e| e.to_string())?;
    let path = p.work.join("execute/report.json");
    save_json(&path, &result.report)?;
    r.raw_report = Some(path);
    r.guest_session_s = Some(result.report.elapsed_secs);
    r.guest_instructions = Some(result.report.instructions);
    r.runtime = result.report.runtime.clone().unwrap_or_default();
    if let Err(e) = checkprog::finish(&mut r.compilation, &p, &result) {
        r.evidence = format!("compiler/session validation: {e}");
    }
    if r.compilation.observation.is_some() {
        r.compile_status = Some(r.compilation.status);
    }
    r.execution_status = match result.outcome {
        Outcome::Fault => ExecutionStatus::GuestExecutionFault,
        Outcome::LimitReached | Outcome::ScriptFailed => {
            r.runtime.terminal.unwrap_or(ExecutionStatus::TimedOut)
        }
        Outcome::UnsupportedStop => ExecutionStatus::Unsupported,
        _ => r.runtime.terminal.unwrap_or(ExecutionStatus::Unsupported),
    };
    if result
        .report
        .unsupported_hits
        .iter()
        .any(|h| ![0x10293, 0x102cb].contains(&h.id))
        && matches!(
            r.execution_status,
            ExecutionStatus::Completed
                | ExecutionStatus::CompileError
                | ExecutionStatus::RuntimeError
                | ExecutionStatus::InvalidArguments
        )
    {
        r.execution_status = ExecutionStatus::Unsupported;
        r.evidence = "an unverified OS service was reached".into();
    }
    if r.runtime.compile_verified
        && (!r.compilation.source_preserved || r.compilation.status != checkprog::Status::Valid)
    {
        r.execution_status = ExecutionStatus::Unsupported;
        r.evidence = "final checked-source/compilation validation failed".into();
    }
    if r.evidence.is_empty() {
        r.evidence = r
            .runtime
            .reason
            .clone()
            .unwrap_or_else(|| result.report.evidence.clone());
    }
    let screenshot_start = Instant::now();
    for invocation in &mut r.runtime.invocations {
        if let Some(frame) = &invocation.frame {
            let path = p.work.join(format!("invocation-{:02}.png", invocation.id));
            frame.save_png(&path).map_err(|e| {
                r.execution_status = ExecutionStatus::InfrastructureError;
                e.to_string()
            })?;
            invocation.screenshot_sha256 =
                Some(sha256::hex(&std::fs::read(&path).map_err(|e| {
                    r.execution_status = ExecutionStatus::InfrastructureError;
                    e.to_string()
                })?));
            invocation.screenshot = Some(path.to_string_lossy().into_owned());
        }
    }
    if let Some(c) = r.runtime.invocations.last() {
        r.return_value = c
            .result
            .as_ref()
            .and_then(|v| v.value.clone())
            .filter(|v| !matches!(v, FirmwareValue::Error { .. }));
        r.output = c.output.clone();
        if let Some(path) = &cfg.screenshot
            && let Some(frame) = &c.frame
        {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| {
                    r.execution_status = ExecutionStatus::InfrastructureError;
                    e.to_string()
                })?;
            }
            frame.save_png(path).map_err(|e| {
                r.execution_status = ExecutionStatus::InfrastructureError;
                e.to_string()
            })?;
        }
    }
    r.runtime_errors = r
        .runtime
        .invocations
        .iter()
        .filter(|c| c.status == Some(ExecutionStatus::RuntimeError))
        .filter_map(|c| c.diagnostic.clone())
        .collect();
    r.screenshot_s = Some(screenshot_start.elapsed().as_secs_f64());
    r.compilation.timings.total_s = started_compilation_time(&r.compilation);
    Ok(())
}
fn started_compilation_time(c: &CheckReport) -> f64 {
    c.timings.initialize_s + c.timings.import_s + c.timings.guest_session_s
}
