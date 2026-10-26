//! Same-session source replacement using documented Programs and AFilesB APIs.
//! The host writes an app attachment; the guest loader performs the source assignment.
use crate::{
    checkprog, hpprgm, import, keys,
    memory::GuestMemory,
    runtime::{ExecutionRequest, ExecutionStatus},
    runtime_value::{self, FirmwareValue},
    script::{ScriptConfig, ScriptDriver, Tick},
    services::SharedState,
    trace::TraceHub,
};
use ppl_engine::session::Event;
use std::{
    path::Path,
    time::{Duration, Instant},
};
pub const LOADER: &str = "EXPORT ZZTIDELOAD()\nBEGIN\n LOCAL b:=Function.AFilesB(\"ZZT_IDE_SOURCE\",0,Function.AFilesB(\"ZZT_IDE_SOURCE\"));\n LOCAL s:=\"\";\n LOCAL i;\n FOR i FROM 1 TO SIZE(b) STEP 2 DO\n  s:=s+CHAR(b(i)+256*b(i+1));\n END;\n Programs(\"ZZT_IDE\"):=s;\n RETURN SIZE(s);\nEND;\n";
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    CheckLoader,
    Load,
    CheckTarget,
    Invoke,
    Execute,
}
pub struct Job {
    pub request: u64,
    source: String,
    entry: Option<String>,
    arguments: String,
    run: bool,
    stage: Stage,
    script: Option<ScriptDriver>,
    check_step: usize,
    catalog_open: bool,
    catalog_start: usize,
    started: Instant,
    execution_started: Option<Instant>,
    reported_running: bool,
}
pub struct Request {
    pub request: u64,
    pub source: String,
    pub entry: Option<String>,
    pub arguments: String,
    pub run: bool,
}
pub struct Jobs {
    pub active: Option<Job>,
    loader_context: Option<u32>,
    catalog: Vec<String>,
    seen_fs: usize,
}
impl Jobs {
    pub fn new() -> Self {
        Self {
            active: None,
            loader_context: None,
            catalog: Vec::new(),
            seen_fs: 0,
        }
    }
    pub fn accepts_input(&self) -> bool {
        self.active.as_ref().is_none_or(|j| j.reported_running)
    }
    fn refresh_catalog(&mut self, shared: &SharedState, trace: &TraceHub) {
        let all = import::list_programs(&shared.data_dir);
        if self.catalog.is_empty() {
            self.catalog = all.clone();
        }
        self.catalog
            .retain(|n| all.iter().any(|a| a.eq_ignore_ascii_case(n)));
        for name in all {
            if !self.catalog.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                self.catalog.push(name);
            }
        }
        let (events, _) = trace.fs_log();
        for e in events.iter().skip(self.seen_fs) {
            if e["op"] == "wfopen" && e["mode"].as_str().is_some_and(|m| m.starts_with('w')) {
                if let Some(path) = e["path"]
                    .as_str()
                    .filter(|p| p.to_ascii_lowercase().ends_with(".hpprgm"))
                {
                    if let Some(name) = path
                        .rsplit('\\')
                        .next()
                        .and_then(|n| n.strip_suffix(".hpprgm"))
                    {
                        if let Some(pos) = self
                            .catalog
                            .iter()
                            .position(|n| n.eq_ignore_ascii_case(name))
                        {
                            let name = self.catalog.remove(pos);
                            self.catalog.insert(0, name);
                        }
                    }
                }
            }
        }
        self.seen_fs = events.len();
    }
    pub fn start(
        &mut self,
        request: Request,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<(), String> {
        let Request {
            request,
            source,
            entry,
            arguments,
            run,
        } = request;
        if trace.runtime.as_ref().is_some_and(|o| {
            let (started, ended) = o.progress();
            started && !ended
        }) {
            return Err("The previous firmware program is still stopping".into());
        }
        if self.active.is_some() {
            return Err("The firmware is busy; stop the current operation first".into());
        }
        hpprgm::build_program(&source).map_err(|e| e.to_string())?;
        let loader = read_program(&shared.data_dir, "ZZT_LOADER")?;
        if loader.source().map_err(|e| e.to_string())?.source != LOADER {
            return Err(
                "The reserved ZZT_LOADER program was modified; use a new IDE calculator profile"
                    .into(),
            );
        }
        let attachment = shared
            .data_dir
            .join("C/DATA/&Function.hpappdir/ZZT_IDE_SOURCE");
        std::fs::create_dir_all(attachment.parent().unwrap()).map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
        std::fs::write(attachment.with_extension("pending"), bytes).map_err(|e| e.to_string())?;
        std::fs::rename(attachment.with_extension("pending"), attachment)
            .map_err(|e| e.to_string())?;
        self.refresh_catalog(shared, trace);
        let stage = if self.loader_context.is_some() {
            Stage::Load
        } else {
            Stage::CheckLoader
        };
        let mut job = Job {
            request,
            source,
            entry,
            arguments,
            run,
            stage,
            script: None,
            check_step: 0,
            catalog_open: false,
            catalog_start: 0,
            started: Instant::now(),
            execution_started: None,
            reported_running: false,
        };
        if stage == Stage::CheckLoader {
            self.check_script(&mut job, "ZZT_LOADER", trace)?;
        } else {
            self.load_script(&mut job, trace)?;
        }
        self.active = Some(job);
        Ok(())
    }
    fn check_script(&self, job: &mut Job, _name: &str, trace: &TraceHub) -> Result<(), String> {
        job.catalog_start = trace.fs_log().0.len();
        let mut sequence = keys::parse_keys("HOME,ESC,SHIFT,1")?;
        for _ in 0..self.catalog.len().div_ceil(7) {
            sequence.extend(keys::parse_keys("SWIPE:160:55:160:210:10")?);
        }
        job.catalog_open = true;
        job.script = Some(script(sequence));
        Ok(())
    }
    fn open_editor(
        &mut self,
        job: &mut Job,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<(), String> {
        let (events, dropped) = trace.fs_log();
        if dropped != 0 {
            return Err(
                "Catalog observation overflowed; restart the local session before another IDE run"
                    .into(),
            );
        }
        let mut names = Vec::new();
        for event in events.iter().skip(job.catalog_start) {
            if event["op"] == "wfopen" && event["mode"] == "rb+" {
                if let Some(name) = event["path"]
                    .as_str()
                    .and_then(|p| p.rsplit('\\').next())
                    .and_then(|p| p.strip_suffix(".hpprgm"))
                {
                    if !names.iter().any(|n: &String| n.eq_ignore_ascii_case(name)) {
                        names.push(name.to_string());
                    }
                }
            }
        }
        if names.len() != import::list_programs(&shared.data_dir).len() {
            return Err("The current firmware catalog order could not be verified".into());
        }
        self.catalog = names;
        let name = if job.stage == Stage::CheckLoader {
            "ZZT_LOADER"
        } else {
            "ZZT_IDE"
        };
        let row = self
            .catalog
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .ok_or("Managed program not found in the live catalog")?;
        let mut sequence = Vec::new();
        let mut offset = 0;
        while row - offset >= 8 {
            let next = (offset + 7).min(self.catalog.len().saturating_sub(8));
            if next == offset {
                return Err("Cannot bring the managed program into view".into());
            }
            sequence.extend(keys::parse_keys("SWIPE:160:199:160:52:60")?);
            offset = next;
        }
        // A catalog row touch opens its editor directly (the Edit softkey is for keyboard selection).
        sequence.extend(keys::parse_keys(&format!(
            "TAP:100:{}",
            52 + 21 * (row - offset)
        ))?);
        job.check_step = sequence.len();
        sequence.extend(keys::parse_keys("TAP:293:229,ENTER,HOME,ESC,IDLE")?);
        trace.compiler.as_ref().unwrap().reset();
        job.script = Some(script(sequence));
        Ok(())
    }
    fn load_script(&self, job: &mut Job, trace: &TraceHub) -> Result<(), String> {
        let observer = trace.runtime.as_ref().unwrap();
        observer.reset();
        observer.compiled();
        observer.arm(
            self.loader_context.ok_or("Loader has not been compiled")?,
            &crate::sha256::hex(LOADER.as_bytes()),
            &ExecutionRequest {
                entry: "ZZTIDELOAD".into(),
                arguments: vec![],
            },
            trace.elapsed_secs() * 1000.0,
        )?;
        observer.cache_flags(1);
        job.script = Some(script(runtime_value::invocation_keys("ZZTIDELOAD", &[])?));
        Ok(())
    }
    pub fn cancel(&mut self) -> Option<u64> {
        self.active.take().map(|j| j.request)
    }
    pub fn tick(
        &mut self,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Vec<Event> {
        let Some(mut job) = self.active.take() else {
            return vec![];
        };
        let result = self.advance(&mut job, mem, shared, trace);
        match result {
            Ok(None) => {
                let events = if matches!(job.stage, Stage::Invoke | Stage::Execute)
                    && !job.reported_running
                    && trace.runtime.as_ref().unwrap().progress().0
                {
                    job.reported_running = true;
                    job.execution_started = Some(Instant::now());
                    vec![Event::Phase {
                        phase: "running".into(),
                    }]
                } else {
                    vec![]
                };
                self.active = Some(job);
                events
            }
            Ok(Some(message)) => vec![
                Event::Phase {
                    phase: "ready".into(),
                },
                Event::Result {
                    request: job.request,
                    success: true,
                    message,
                },
            ],
            Err(message) => {
                save_report(shared, trace, &job, false, &message);
                vec![
                    Event::Phase {
                        phase: "ready".into(),
                    },
                    Event::Result {
                        request: job.request,
                        success: false,
                        message,
                    },
                ]
            }
        }
    }
    fn advance(
        &mut self,
        job: &mut Job,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<Option<String>, String> {
        if !job.reported_running && job.started.elapsed() > Duration::from_secs(180) {
            return Err("Firmware operation timed out; the live session was retained".into());
        }
        if let Some(script) = &mut job.script {
            trace.set_script_step(script.current_step());
            match script.tick(mem, shared) {
                Tick::Continue => return Ok(None),
                Tick::Failed(e) => return Err(e),
                Tick::Done => job.script = None,
            }
        }
        if job.catalog_open {
            job.catalog_open = false;
            self.open_editor(job, shared, trace)?;
            return Ok(None);
        }
        match job.stage {
            Stage::CheckLoader => {
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                let c = checkprog::validate(&observations, LOADER, job.check_step)?;
                if c.return_value != Some(1) {
                    return Err("The genuine firmware rejected the IDE source loader".into());
                }
                self.loader_context = c.program_va;
                job.stage = Stage::Load;
                self.load_script(job, trace)?;
            }
            Stage::Load => {
                let runtime = trace.runtime.as_ref().unwrap().snapshot();
                let c = runtime
                    .invocations
                    .last()
                    .ok_or("Loader invocation was not observed")?;
                if c.ended_ms.is_none() {
                    return Ok(None);
                }
                let status = trace.runtime.as_ref().unwrap().finish_observed();
                if status != ExecutionStatus::Completed || !c.body_entered {
                    return Err(format!("Source loader failed: {status:?}"));
                }
                job.stage = Stage::CheckTarget;
                self.check_script(job, "ZZT_IDE", trace)?;
            }
            Stage::CheckTarget => {
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                let c = checkprog::validate(&observations, &job.source, job.check_step)?;
                let container = read_program(&shared.data_dir, "ZZT_IDE")?;
                if container.source().map_err(|e| e.to_string())?.source != job.source {
                    return Err("The firmware did not preserve this exact source revision".into());
                }
                if c.return_value != Some(1) {
                    return Err(c
                        .message
                        .clone()
                        .unwrap_or("The firmware rejected the program".into()));
                }
                if !job.run {
                    let message = "G1 firmware compiled the exact current source".to_string();
                    save_report(shared, trace, job, true, &message);
                    return Ok(Some(message));
                }
                let exports = container.exports();
                let candidates: Vec<_> = exports.iter().filter(|(_, f)| f & 1 != 0).collect();
                let entry = job
                    .entry
                    .clone()
                    .or_else(|| (candidates.len() == 1).then(|| candidates[0].0.clone()))
                    .ok_or("Select a firmware entry point explicitly in Run Configuration")?;
                let flags = exports
                    .iter()
                    .find(|(n, f)| n == &entry && f & 1 != 0)
                    .map(|(_, f)| *f)
                    .ok_or("The selected entry is not exported by this source")?;
                let arguments =
                    runtime_value::parse_arguments(&format!("[{}]", job.arguments.trim()))
                        .map_err(|e| {
                            format!("Firmware arguments must be real numeric literals: {e}")
                        })?;
                let request = ExecutionRequest {
                    entry: entry.clone(),
                    arguments,
                };
                let observer = trace.runtime.as_ref().unwrap();
                observer.reset();
                observer.compiled();
                observer.arm(
                    c.program_va.ok_or("Compiler context missing")?,
                    &crate::sha256::hex(job.source.as_bytes()),
                    &request,
                    trace.elapsed_secs() * 1000.0,
                )?;
                observer.cache_flags(flags);
                job.stage = Stage::Invoke;
                job.script = Some(script(runtime_value::invocation_keys(
                    &entry,
                    &request.arguments,
                )?));
            }
            Stage::Invoke => {
                job.stage = Stage::Execute;
            }
            Stage::Execute => {
                if !trace.runtime.as_ref().unwrap().progress().1 {
                    return Ok(None);
                }
                let runtime = trace.runtime.as_ref().unwrap().snapshot();
                let c = runtime
                    .invocations
                    .last()
                    .ok_or("No genuine firmware invocation")?;
                if c.ended_ms.is_none() {
                    return Ok(None);
                }
                if !c.body_entered || c.truncated || !c.unavailable.is_empty() {
                    return Err("Incomplete firmware runtime evidence".into());
                }
                let status = trace.runtime.as_ref().unwrap().finish_observed();
                if status != ExecutionStatus::Completed {
                    return Err(format!(
                        "Firmware runtime: {status:?}; {}",
                        c.raw_error_context
                            .as_deref()
                            .unwrap_or("see calculator display")
                    ));
                }
                let value = c
                    .result
                    .as_ref()
                    .and_then(|v| v.value.as_ref())
                    .ok_or("Firmware return value was not captured")?;
                let value = match value {
                    FirmwareValue::Real { value } | FirmwareValue::String { value } => {
                        value.clone()
                    }
                    other => format!("{other:?}"),
                };
                let message = format!("G1 firmware returned {value}");
                save_report(shared, trace, job, true, &message);
                return Ok(Some(message));
            }
        }
        Ok(None)
    }
}
fn read_program(data: &Path, name: &str) -> Result<hpprgm::Container, String> {
    read_program_path(&data.join(format!("C/DATA/{name}.hpprgm")))
}
fn read_program_path(path: &Path) -> Result<hpprgm::Container, String> {
    hpprgm::parse(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn script(keys: Vec<keys::Key>) -> ScriptDriver {
    ScriptDriver::new(ScriptConfig {
        keys,
        settle_ms: 150,
        idle_observe_ms: 0,
        ..Default::default()
    })
}
fn save_report(shared: &SharedState, trace: &TraceHub, job: &Job, success: bool, message: &str) {
    let report = serde_json::json!({"request":job.request,"source_sha256":crate::sha256::hex(job.source.as_bytes()),"success":success,"message":message,"compiler":trace.compiler.as_ref().map(|o|o.snapshot()),"runtime":trace.runtime.as_ref().map(|o|o.snapshot())});
    let dir = shared.data_dir.join("ide-reports");
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
            let _ = std::fs::write(dir.join(format!("request-{}.json", job.request)), bytes);
        }
    }
}
