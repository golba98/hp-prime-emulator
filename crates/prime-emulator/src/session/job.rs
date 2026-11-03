//! Same-session source replacement using documented Programs and AFiles APIs.
//! The host writes an app attachment; the guest loader performs the source assignment.
use crate::{
    checkprog, direct, hpprgm, import, keys,
    memory::GuestMemory,
    runtime::{ExecutionRequest, ExecutionStatus},
    runtime_value::{self, ReturnRepresentation},
    script::{ScriptConfig, ScriptDriver, Tick},
    services::SharedState,
    trace::TraceHub,
};
use ppl_engine::session::{CheckVerdict, Event};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
/// Sustained idle firmware UI that closes an interrupted invocation.
const INTERRUPT_SETTLE: Duration = Duration::from_millis(500);
/// The calculator's ON key, which breaks a running program.
pub const ON_KEY: u16 = 0x83;
/// ON presses per interruption, at most one per [`BREAK_RETRY`].
const BREAK_PRESSES: u32 = 5;
const BREAK_RETRY: Duration = Duration::from_secs(1);
/// How long ON is held per press (a short human press).
const BREAK_HOLD: Duration = Duration::from_millis(200);
/// How long a Run issued during an interruption waits for it to finish.
const DEFERRED_LIMIT: Duration = Duration::from_secs(10);
/// Idle share over [`INTERRUPT_SETTLE`]: the firmware's own timers (every
/// 183 ms) wake the UI pump, so idleness is never continuous.
const INTERRUPT_IDLE_SHARE: f64 = 0.8;
/// Reserved source loader. AFiles decodes a serialized string in firmware,
/// avoiding the interpreted per-character binary conversion. Programs():=
/// remains responsible for compilation and storing the resulting program.
pub const LOADER: &str = "EXPORT ZZTIDELOAD()\nBEGIN\n LOCAL s:=Function.AFiles(\"ZZT_IDE_SOURCE\");\n Programs(\"ZZT_IDE\"):=s;\n RETURN SIZE(s);\nEND;\n";
/// Previous binary-chunk loader, recognized exactly during profile migration.
pub const LOADER_V2: &str = "EXPORT ZZTIDELOAD()\nBEGIN\n LOCAL n:=Function.AFilesB(\"ZZT_IDE_SOURCE\");\n LOCAL s:=\"\";\n LOCAL o,b,c,i;\n FOR o FROM 0 TO n-1 STEP 4096 DO\n  b:=Function.AFilesB(\"ZZT_IDE_SOURCE\",o,MIN(4096,n-o));\n  c:={};\n  FOR i FROM 1 TO SIZE(b)/2 DO\n   c(i):=b(2*i-1)+256*b(2*i);\n  END;\n  s:=s+CHAR(c);\n END;\n Programs(\"ZZT_IDE\"):=s;\n RETURN SIZE(s);\nEND;\n";
/// The first loader (M10), which IDE profiles may still hold; the session
/// replaces exactly this text with [`LOADER`] before boot.
pub const LOADER_V1: &str = "EXPORT ZZTIDELOAD()\nBEGIN\n LOCAL b:=Function.AFilesB(\"ZZT_IDE_SOURCE\",0,Function.AFilesB(\"ZZT_IDE_SOURCE\"));\n LOCAL s:=\"\";\n LOCAL i;\n FOR i FROM 1 TO SIZE(b) STEP 2 DO\n  s:=s+CHAR(b(i)+256*b(i+1));\n END;\n Programs(\"ZZT_IDE\"):=s;\n RETURN SIZE(s);\nEND;\n";
/// Reserved importer: stores ZZT_IDE's source as a new program whose name
/// the host passes in an app attachment (`Programs(name):=`).
pub const IMPORTER: &str = "EXPORT ZZTIDEIMPORT()\nBEGIN\n LOCAL b:=Function.AFilesB(\"ZZT_IDE_NAME\",0,Function.AFilesB(\"ZZT_IDE_NAME\"));\n LOCAL n:=\"\";\n LOCAL i;\n FOR i FROM 1 TO SIZE(b) STEP 2 DO\n  n:=n+CHAR(b(i)+256*b(i+1));\n END;\n Programs(n):=Programs(\"ZZT_IDE\");\n RETURN SIZE(n);\nEND;\n";
/// Named loader avoids the scratch ZZT_IDE module and duplicate exports.
pub const NAMED_LOADER: &str = "EXPORT ZZTNAMEDLOAD()\nBEGIN\n LOCAL b:=Function.AFilesB(\"ZZT_IDE_NAME\",0,Function.AFilesB(\"ZZT_IDE_NAME\"));\n LOCAL n:=\"\";\n LOCAL i;\n FOR i FROM 1 TO SIZE(b) STEP 2 DO\n  n:=n+CHAR(b(i)+256*b(i+1));\n END;\n LOCAL s:=Function.AFiles(\"ZZT_IDE_SOURCE\");\n Programs(n):=s;\n RETURN SIZE(n);\nEND;\n";
/// Share of idle samples over a sliding [`INTERRUPT_SETTLE`] window: the
/// firmware UI is idle when its pump is parked in GetEvent with no input
/// queued and the main thread waiting.
#[derive(Default)]
pub struct IdleMeter {
    samples: std::collections::VecDeque<(Instant, bool)>,
}
impl IdleMeter {
    pub fn idle_now(shared: &SharedState) -> bool {
        shared.getevent_parked()
            && shared.main_waiting()
            && shared.input_events_pending() == 0
            && shared.touch_pending() == 0
    }
    /// Records a sample. True when the UI is idle now and for
    /// [`INTERRUPT_IDLE_SHARE`] of a window covering [`INTERRUPT_SETTLE`].
    pub fn settled(&mut self, shared: &SharedState) -> bool {
        let now = Instant::now();
        let idle = Self::idle_now(shared);
        self.samples.push_back((now, idle));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > INTERRUPT_SETTLE)
        {
            self.samples.pop_front();
        }
        let covered = self
            .samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) >= INTERRUPT_SETTLE.mul_f64(0.8));
        let share = self.samples.iter().filter(|(_, i)| *i).count() as f64
            / self.samples.len().max(1) as f64;
        idle && covered && share >= INTERRUPT_IDLE_SHARE
    }
    pub fn clear(&mut self) {
        self.samples.clear();
    }
    /// Current strict window for failure diagnostics, never an admission override.
    pub fn diagnostic(&self) -> serde_json::Value {
        let idle = self.samples.iter().filter(|(_, idle)| *idle).count();
        // Diagnostic zero-order estimate only: coordinator samples do not prove
        // the owner stayed in this state between observations. Never admit a
        // checkpoint using this estimate or extrapolate beyond the last sample.
        let mut observed = Duration::ZERO;
        let mut estimated_idle = Duration::ZERO;
        let mut maximum_gap = Duration::ZERO;
        for ((start, was_idle), (end, _)) in self.samples.iter().zip(self.samples.iter().skip(1)) {
            let interval = end.saturating_duration_since(*start);
            observed += interval;
            maximum_gap = maximum_gap.max(interval);
            if *was_idle {
                estimated_idle += interval;
            }
        }
        serde_json::json!({"samples":self.samples.len(),"idle_samples":idle,
        "share":idle as f64/self.samples.len().max(1) as f64,
        "covered_ms":self.samples.front().map(|(t,_)|t.elapsed().as_secs_f64()*1000.0),
        "required_share":INTERRUPT_IDLE_SHARE,"required_window_ms":INTERRUPT_SETTLE.as_millis(),
        "sample_interval_estimate": {
            "admission_input": false,
            "observed_ms": observed.as_secs_f64()*1000.0,
            "idle_ms": estimated_idle.as_secs_f64()*1000.0,
            "maximum_gap_ms": maximum_gap.as_secs_f64()*1000.0,
            "share": if observed.is_zero() { None } else { Some(estimated_idle.as_secs_f64()/observed.as_secs_f64()) }
        }})
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    CheckLoader,
    CheckNamedLoader,
    StoreNamed,
    Load,
    /// Check the reserved importer once, to learn its program context.
    CheckImporter,
    /// The importer stores the loaded source under the requested name.
    Import,
    CheckTarget,
    Prepared,
    Invoke,
    Execute,
    /// A direct compiler call plan is running on the UI thread.
    Direct,
}
pub struct Job {
    pub request: u64,
    source: String,
    stored_name: Option<String>,
    binding: String,
    /// The new program name of an Import request.
    import: Option<String>,
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
    reported_entry: bool,
    /// The validated firmware Check verdict on `source`, once observed.
    check: Option<CheckVerdict>,
    /// Each finished UI script: stage, finish time and per-key timings.
    timeline: Vec<serde_json::Value>,
    /// Check by a direct compiler call (`direct`) instead of the UI.
    direct: bool,
    direct_out: Option<Arc<Mutex<Option<direct::Outcome>>>>,
    /// Guest-heap UTF-16 copy of `source` handed to the firmware.
    direct_text: u32,
    direct_started: Option<Instant>,
}
pub struct Request {
    pub request: u64,
    pub source: String,
    pub entry: Option<String>,
    pub arguments: String,
    pub run: bool,
    /// Store the source as this new program instead of checking it.
    pub import: Option<String>,
}
#[derive(Clone)]
struct PreparedProgram {
    source: String,
    stored_sha: String,
    check: crate::compiler::CheckObservation,
    fs_cursor: usize,
    compiler_cursor: usize,
}
pub struct Jobs {
    pub active: Option<Job>,
    prepared: Option<PreparedProgram>,
    /// A running program was interrupted (Stop / ON) at this time; its
    /// invocation is closed once the firmware is back at an idle UI.
    interrupted: Option<Instant>,
    idle: IdleMeter,
    /// ON presses sent for the current interruption, and the latest one.
    break_presses: (u32, Option<Instant>),
    /// When the held ON key went down; released after [`BREAK_HOLD`].
    on_held: Option<Instant>,
    /// A request that arrived while an interrupted program was still
    /// stopping; it starts once the firmware is back at an idle UI.
    deferred: Option<(Request, Instant)>,
    loader_context: Option<u32>,
    named_loader_context: Option<u32>,
    importer_context: Option<u32>,
    catalog: Vec<String>,
    /// The catalog order last read from the guest's program reads.
    observed: Vec<String>,
    seen_fs: usize,
    /// File-service events already examined for catalog reads made while no
    /// job ran (the user browsing the Program Catalog), the reads of the
    /// current burst, and when the last one was.
    user_seen: usize,
    user_burst: Vec<String>,
    user_last: Option<Instant>,
    /// ZZT_IDE's program context, learned from the firmware's own compile in
    /// `Programs("ZZT_IDE"):=source` during a loader run.
    direct_ctx: Option<u32>,
}
impl Jobs {
    pub fn new() -> Self {
        Self {
            active: None,
            prepared: None,
            interrupted: None,
            idle: IdleMeter::default(),
            break_presses: (0, None),
            on_held: None,
            deferred: None,
            loader_context: None,
            named_loader_context: None,
            importer_context: None,
            catalog: Vec::new(),
            observed: Vec::new(),
            seen_fs: 0,
            user_seen: 0,
            user_burst: Vec::new(),
            user_last: None,
            direct_ctx: None,
        }
    }
    /// External calculator interaction can change the compilation namespace.
    pub fn invalidate_prepared(&mut self) {
        self.prepared = None;
    }
    fn prepared_matches(&self, source: &str, shared: &SharedState, trace: &TraceHub) -> bool {
        let Some(p) = &self.prepared else {
            return false;
        };
        if p.source != source || !trace.fs_enabled() {
            return false;
        }
        let Some(compiler) = trace.compiler.as_ref().map(|o| o.snapshot()) else {
            return false;
        };
        if compiler.dropped != 0 || compiler.checks.len() != p.compiler_cursor {
            return false;
        }
        let (events, _, dropped) = trace.fs_log_since(p.fs_cursor);
        // Conservatively invalidate on any storage mutation; pure reads do not
        // invalidate a verified context. Missing evidence never licenses reuse.
        if dropped != 0
            || events.iter().any(|e| {
                !matches!(
                    e["op"].as_str(),
                    Some("fread" | "fseek" | "ftell" | "fclose" | "find" | "wfopen")
                ) || (e["op"] == "wfopen"
                    && e["mode"]
                        .as_str()
                        .is_none_or(|m| !m.starts_with('r') || m.contains('+')))
            })
        {
            return false;
        }
        std::fs::read(shared.data_dir.join("C/DATA/ZZT_IDE.hpprgm"))
            .is_ok_and(|bytes| crate::sha256::hex(&bytes) == p.stored_sha)
    }
    pub fn running_request(&self, request: u64, trace: &TraceHub) -> bool {
        self.active
            .as_ref()
            .is_some_and(|j| j.request == request && j.run)
            && trace
                .runtime
                .as_ref()
                .is_some_and(|o| o.body_entry_evidence().is_some() && o.unfinished())
    }
    pub fn accepts_input(&self) -> bool {
        self.active.as_ref().is_none_or(|j| j.reported_running)
    }
    /// A Stop is still breaking a running program.
    pub fn interrupting(&self) -> bool {
        self.interrupted.is_some()
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
        let (events, next, _) = trace.fs_log_since(self.seen_fs);
        for e in &events {
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
        self.seen_fs = next;
    }
    /// Learns the catalog order from catalog listings the user opened while
    /// no job ran: the firmware reads every program the first time it shows
    /// its catalog and later only changed ones, so a job opening a catalog
    /// the user already saw reads nothing new.
    fn observe_user_catalog(&mut self, shared: &SharedState, trace: &TraceHub) {
        let (events, next, _) = trace.fs_log_since(self.user_seen);
        if self.active.is_some() {
            self.user_seen = next;
            self.user_burst.clear();
            return;
        }
        for event in &events {
            if let Some(name) = catalog_read(event)
                && !self
                    .user_burst
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&name))
            {
                self.user_burst.push(name);
                self.user_last = Some(Instant::now());
            }
        }
        self.user_seen = next;
        if self
            .user_last
            .is_some_and(|t| t.elapsed() >= USER_CATALOG_QUIET)
        {
            let read = std::mem::take(&mut self.user_burst);
            self.user_last = None;
            let expected = import::list_programs(&shared.data_dir);
            if let Ok(order) = catalog_order(&self.observed, read, &expected) {
                self.observed = order.clone();
                self.catalog = order;
            }
        }
    }
    /// Named assignment and execution do not use the IDE scratch module.
    pub fn store_named(
        &mut self,
        request: u64,
        name: String,
        source: String,
        previous: Option<String>,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<(), String> {
        if self.active.is_some() || trace.runtime.as_ref().is_some_and(|o| o.unfinished()) {
            return Err("Firmware is busy".into());
        }
        if !name.starts_with("ZZT_") {
            return Err("Scripted named programs must use disposable ZZT_ names".into());
        }
        import::validate_name(&name).map_err(|e| e.to_string())?;
        if ["ZZT_IDE", "ZZT_LOADER", "ZZT_IMPORT", "ZZT_NAMED"]
            .contains(&name.to_ascii_uppercase().as_str())
        {
            return Err("Reserved session helper cannot be replaced".into());
        }
        match previous {
            None => {
                if import::list_programs(&shared.data_dir)
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&name))
                {
                    return Err("Named program already exists; creation refused".into());
                }
            }
            Some(expected) => {
                let old = read_program(&shared.data_dir, &name)?
                    .source()
                    .map_err(|e| e.to_string())?
                    .source;
                if crate::sha256::hex(old.as_bytes()) != expected {
                    return Err(
                        "Previous program source identity differs; replacement refused".into(),
                    );
                }
            }
        }
        hpprgm::build_program(&source).map_err(|e| e.to_string())?;
        if read_program(&shared.data_dir, "ZZT_NAMED")?
            .source()
            .map_err(|e| e.to_string())?
            .source
            != NAMED_LOADER
        {
            return Err("Reserved named loader differs".into());
        }
        write_attachment(&shared.data_dir, "ZZT_IDE_NAME", &name)?;
        write_source_attachment(&shared.data_dir, &source)?;
        self.invalidate_prepared();
        self.refresh_catalog(shared, trace);
        let mut job = Self::named_job(request, name, source);
        if let Some(context) = self.named_loader_context {
            job.stage = Stage::StoreNamed;
            invoke_reserved(&mut job, trace, context, NAMED_LOADER, "ZZTNAMEDLOAD")?;
        } else {
            job.stage = Stage::CheckNamedLoader;
            self.check_script(&mut job, "ZZT_NAMED", trace)?;
        }
        self.active = Some(job);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)] // Mirrors the correlated protocol request plus emulator services.
    pub fn run_stored(
        &mut self,
        request: u64,
        name: String,
        source_hash: String,
        entry: String,
        arguments: String,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<(), String> {
        if self.active.is_some() || trace.runtime.as_ref().is_some_and(|o| o.unfinished()) {
            return Err("Firmware is busy".into());
        }
        import::validate_name(&name).map_err(|e| e.to_string())?;
        let stored = read_program(&shared.data_dir, &name)?;
        let source = stored.source().map_err(|e| e.to_string())?.source;
        if crate::sha256::hex(source.as_bytes()) != source_hash {
            return Err("Stored source differs from requested exact revision".into());
        }
        let flags = stored
            .exports()
            .iter()
            .find(|(n, f)| n == &entry && f & 1 != 0)
            .map(|(_, f)| *f)
            .ok_or("Entry is absent from stored firmware exports")?;
        let owners: Vec<_> = import::list_programs(&shared.data_dir)
            .into_iter()
            .filter(|n| {
                read_program(&shared.data_dir, n)
                    .is_ok_and(|p| p.exports().iter().any(|(e, f)| e == &entry && f & 1 != 0))
            })
            .collect();
        if owners.len() != 1 {
            return Err(format!(
                "Export {entry} is ambiguous across programs {owners:?}"
            ));
        }
        let args = runtime_value::parse_arguments(&format!("[{}]", arguments.trim()))?;
        let invocation = ExecutionRequest {
            entry: entry.clone(),
            arguments: args,
        };
        let observer = trace
            .runtime
            .as_ref()
            .ok_or("Runtime observation unavailable")?;
        observer.reset();
        observer.arm_stored(&source_hash, &invocation, trace.elapsed_secs() * 1000.0)?;
        observer.cache_flags(flags);
        let mut job = Self::named_job(request, name, source);
        job.run = true;
        job.entry = Some(entry);
        job.binding = "stored_source_dispatch".into();
        job.stage = Stage::Invoke;
        job.script = Some(invocation_script(runtime_value::invocation_keys(
            &invocation.entry,
            &invocation.arguments,
        )?));
        self.active = Some(job);
        Ok(())
    }
    fn named_job(request: u64, name: String, source: String) -> Job {
        Job {
            request,
            source,
            stored_name: Some(name),
            binding: "exact_compilation".into(),
            import: None,
            entry: None,
            arguments: String::new(),
            run: false,
            stage: Stage::StoreNamed,
            script: None,
            check_step: 0,
            catalog_open: false,
            catalog_start: 0,
            started: Instant::now(),
            execution_started: None,
            reported_running: false,
            reported_entry: false,
            check: None,
            timeline: vec![],
            direct: false,
            direct_out: None,
            direct_text: 0,
            direct_started: None,
        }
    }
    pub fn start(
        &mut self,
        request: Request,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<(), String> {
        if self.interrupted.is_some()
            && self.active.is_none()
            && self.deferred.is_none()
            && trace.runtime.as_ref().is_some_and(|o| o.unfinished())
        {
            self.deferred = Some((request, Instant::now()));
            return Ok(());
        }
        let Request {
            request,
            source,
            entry,
            arguments,
            run,
            import,
        } = request;
        if trace.runtime.as_ref().is_some_and(|o| o.unfinished()) {
            return Err("The previous firmware program is still stopping".into());
        }
        if self.active.is_some() {
            return Err("The firmware is busy; stop the current operation first".into());
        }
        hpprgm::build_program(&source).map_err(|e| e.to_string())?;
        if let Some(name) = &import {
            validate_import(&shared.data_dir, name)?;
            let importer = read_program(&shared.data_dir, "ZZT_IMPORT")?;
            if importer.source().map_err(|e| e.to_string())?.source != IMPORTER {
                return Err(
                    "The reserved ZZT_IMPORT program was modified; use a new IDE calculator profile"
                        .into(),
                );
            }
            write_attachment(&shared.data_dir, "ZZT_IDE_NAME", name)?;
        }
        let loader = read_program(&shared.data_dir, "ZZT_LOADER")?;
        if loader.source().map_err(|e| e.to_string())?.source != LOADER {
            return Err(
                "The reserved ZZT_LOADER program was modified; use a new IDE calculator profile"
                    .into(),
            );
        }
        let cached = run && import.is_none() && self.prepared_matches(&source, shared, trace);
        if !cached {
            self.invalidate_prepared();
            write_source_attachment(&shared.data_dir, &source)?;
            self.refresh_catalog(shared, trace);
        }
        let stage = if cached {
            Stage::Prepared
        } else if self.loader_context.is_some() {
            Stage::Load
        } else {
            Stage::CheckLoader
        };
        let mut job = Job {
            request,
            source,
            stored_name: None,
            binding: "exact_compilation".into(),
            import,
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
            reported_entry: false,
            check: None,
            timeline: Vec::new(),
            direct: false,
            direct_out: None,
            direct_text: 0,
            direct_started: None,
        };
        if stage == Stage::Prepared {
            job.timeline
                .push(serde_json::json!({"stage":"reuse-prepared", "done_ms":0.0}));
        }
        if stage == Stage::CheckLoader {
            self.check_script(&mut job, "ZZT_LOADER", trace)?;
        } else if stage != Stage::Prepared {
            self.load_script(&mut job, trace)?;
        }
        self.active = Some(job);
        Ok(())
    }
    fn check_script(&self, job: &mut Job, _name: &str, trace: &TraceHub) -> Result<(), String> {
        job.catalog_start = trace.fs_log_since(usize::MAX).1;
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
        let mut names: Vec<String> = Vec::new();
        for name in events
            .iter()
            .skip(job.catalog_start)
            .filter_map(catalog_read)
        {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
        let expected = import::list_programs(&shared.data_dir);
        let names = catalog_order(&self.observed, names, &expected)?;
        self.observed = names.clone();
        self.catalog = names;
        let name = match job.stage {
            Stage::CheckLoader => "ZZT_LOADER",
            Stage::CheckImporter => "ZZT_IMPORT",
            Stage::CheckNamedLoader => "ZZT_NAMED",
            _ => "ZZT_IDE",
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
    /// Checks `source` by calling the firmware compiler directly. The first
    /// request also runs the loader once, through the UI, to learn which
    /// program context the firmware assigns ZZT_IDE's source to.
    pub fn start_direct(
        &mut self,
        request: u64,
        source: String,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<(), String> {
        self.invalidate_prepared();
        let Some(ctx) = self.direct_ctx else {
            self.start(
                Request {
                    request,
                    source,
                    entry: None,
                    arguments: String::new(),
                    run: false,
                    import: None,
                },
                shared,
                trace,
            )?;
            self.active.as_mut().unwrap().direct = true;
            return Ok(());
        };
        if self.active.is_some() || shared.guest_calls.busy() {
            return Err("The firmware is busy; stop the current operation first".into());
        }
        hpprgm::build_program(&source).map_err(|e| e.to_string())?;
        let mut job = Job {
            request,
            source,
            stored_name: None,
            binding: "exact_compilation".into(),
            import: None,
            entry: None,
            arguments: String::new(),
            run: false,
            stage: Stage::Direct,
            script: None,
            check_step: 0,
            catalog_open: false,
            catalog_start: 0,
            started: Instant::now(),
            execution_started: None,
            reported_running: false,
            reported_entry: false,
            check: None,
            timeline: Vec::new(),
            direct: true,
            direct_out: None,
            direct_text: 0,
            direct_started: None,
        };
        begin_direct(&mut job, ctx, mem, shared, trace)?;
        self.active = Some(job);
        Ok(())
    }
    fn load_script(&self, job: &mut Job, trace: &TraceHub) -> Result<(), String> {
        // The loader's `Programs():=` compile identifies ZZT_IDE's context.
        let context = self.loader_context.ok_or("Loader has not been compiled")?;
        invoke_reserved(job, trace, context, LOADER, "ZZTIDELOAD")
    }
    /// Starts the importer, or its one-time Check, after the source loaded.
    fn import_script(&self, job: &mut Job, trace: &TraceHub) -> Result<(), String> {
        match self.importer_context {
            Some(context) => {
                job.stage = Stage::Import;
                invoke_reserved(job, trace, context, IMPORTER, "ZZTIDEIMPORT")
            }
            None => {
                job.stage = Stage::CheckImporter;
                self.check_script(job, "ZZT_IMPORT", trace)
            }
        }
    }
    /// Records that Stop interrupted a running program (the caller sends ON).
    /// Returns whether a running program was interrupted; ON is then held
    /// down by the session (and repeated while needed) instead of tapped.
    pub fn interrupt(&mut self, shared: &SharedState, trace: &TraceHub) -> bool {
        if !trace.runtime.as_ref().is_some_and(|o| o.unfinished()) {
            return false;
        }
        self.interrupted = Some(Instant::now());
        self.idle.clear();
        self.break_presses = (0, None);
        self.press_on(shared);
        true
    }
    /// Holds ON down like a finger does: a program checks for a break
    /// between statements, and an instantaneous down/up can fall between
    /// two checks.
    fn press_on(&mut self, shared: &SharedState) {
        if self.on_held.is_none() {
            shared.push_key_transition(ON_KEY, true);
            self.on_held = Some(Instant::now());
            self.break_presses = (self.break_presses.0 + 1, Some(Instant::now()));
        }
    }
    fn release_on(&mut self, shared: &SharedState) {
        if self.on_held.is_some_and(|t| t.elapsed() >= BREAK_HOLD) {
            shared.push_key_transition(ON_KEY, false);
            self.on_held = None;
        }
    }
    /// An interrupted program never reaches the evaluator-return hook. It is
    /// over once the firmware UI pump is parked in GetEvent with no input
    /// queued and the main thread waiting, now and for
    /// [`INTERRUPT_IDLE_SHARE`] of the last [`INTERRUPT_SETTLE`]: the firmware
    /// is back at an idle screen.
    fn settle_interrupt(&mut self, shared: &SharedState, trace: &TraceHub) {
        let Some(at) = self.interrupted else {
            return;
        };
        let observer = trace.runtime.as_ref().unwrap();
        if !observer.unfinished() {
            self.interrupted = None;
            return;
        }
        // The firmware takes ON as a key event; one press can arrive while
        // the program is between checks and be missed. While the main thread
        // stays busy, press ON again, as a user would on the calculator.
        let (presses, last) = self.break_presses;
        if !shared.main_waiting()
            && presses < BREAK_PRESSES
            && last.is_none_or(|t| t.elapsed() >= BREAK_RETRY)
        {
            self.press_on(shared);
        }
        if self.idle.settled(shared) && at.elapsed() >= INTERRUPT_SETTLE {
            observer.finish_invocation(ExecutionStatus::Cancelled);
            self.interrupted = None;
            self.idle.clear();
        }
    }
    /// Starts a deferred request once the interrupted program has ended.
    /// `None` while it waits, or when it started (the job then ticks).
    fn start_deferred(&mut self, shared: &SharedState, trace: &TraceHub) -> Option<Vec<Event>> {
        let (_, since) = self.deferred.as_ref()?;
        let stopped = !trace.runtime.as_ref().unwrap().unfinished();
        if !stopped && since.elapsed() < DEFERRED_LIMIT {
            return None;
        }
        let (request, _) = self.deferred.take().unwrap();
        let id = request.request;
        let failed = |message: String| {
            Some(vec![
                Event::Phase {
                    phase: "ready".into(),
                },
                Event::Result {
                    request: id,
                    success: false,
                    message,
                    check: None,
                },
            ])
        };
        if !stopped {
            return failed(
                "The previous firmware program did not stop; press Stop again, then Run".into(),
            );
        }
        match self.start(request, shared, trace) {
            Ok(()) => None,
            Err(message) => failed(message),
        }
    }
    pub fn cancel(&mut self, mem: &GuestMemory, shared: &SharedState) -> Option<u64> {
        self.invalidate_prepared();
        if self.active.is_none()
            && let Some((request, _)) = self.deferred.take()
        {
            return Some(request.request);
        }
        let job = self.active.take()?;
        if job.stage == Stage::Direct {
            // A call in progress finishes on its own; one not yet started is
            // dropped, and its source buffer is no longer referenced.
            shared.guest_calls.cancel_pending();
            if !shared.guest_calls.busy() && job.direct_text != 0 {
                mem.heap_free(job.direct_text);
            }
        }
        Some(job.request)
    }
    pub fn tick(
        &mut self,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Vec<Event> {
        self.release_on(shared);
        self.settle_interrupt(shared, trace);
        self.observe_user_catalog(shared, trace);
        if self.active.is_none()
            && let Some(events) = self.start_deferred(shared, trace)
        {
            return events;
        }
        let Some(mut job) = self.active.take() else {
            return vec![];
        };
        let previous_stage = job.stage;
        let result = self.advance(&mut job, mem, shared, trace);
        match result {
            Ok(None) => {
                let mut events = if matches!(job.stage, Stage::Invoke | Stage::Execute)
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
                if job.stage != previous_stage
                    && progress_message(job.stage) != progress_message(previous_stage)
                    && !job.reported_running
                {
                    events.push(Event::Status {
                        message: progress_message(job.stage).into(),
                    });
                }
                if job.run
                    && !job.reported_entry
                    && let Some(evidence) = trace.runtime.as_ref().unwrap().body_entry_evidence()
                {
                    job.reported_entry = true;
                    events.push(Event::ExecutionStarted {
                        request: job.request,
                        source_sha256: crate::sha256::hex(job.source.as_bytes()),
                        entry: evidence["entry"].as_str().unwrap_or("").into(),
                    });
                }
                self.active = Some(job);
                events
            }
            Ok(Some(message)) => {
                let mut out = execution_events(&job, trace);
                out.extend(vec![
                    Event::Phase {
                        phase: "ready".into(),
                    },
                    Event::Result {
                        request: job.request,
                        success: true,
                        message,
                        check: job.check,
                    },
                ]);
                out
            }
            Err(message) => {
                save_report(shared, trace, &job, false, &message);
                let mut out = execution_events(&job, trace);
                out.extend(vec![
                    Event::Phase {
                        phase: "ready".into(),
                    },
                    Event::Result {
                        request: job.request,
                        success: false,
                        message,
                        check: job.check,
                    },
                ]);
                out
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
                Tick::Continue
                    if matches!(
                        job.stage,
                        Stage::Load | Stage::Import | Stage::Invoke | Stage::StoreNamed
                    ) && script.invocation_delivered(shared)
                        && trace.runtime.as_ref().unwrap().progress().0 =>
                {
                    job.timeline.push(serde_json::json!({
                        "stage": stage_name(job.stage, job.catalog_open),
                        "done_ms": job.started.elapsed().as_secs_f64() * 1000.0,
                        "reason": "runtime_handoff",
                        "final_key": script.invocation_delivery(),
                        "keys": script.report().keys,
                    }));
                    job.script = None;
                }
                Tick::Continue => return Ok(None),
                // A program that keeps drawing, or a helper that runs long
                // (the loader with a large source), never lets the
                // invocation's ENTER settle; once the guest entered the
                // function, its outcome comes from the runtime observer, not
                // the script.
                Tick::Failed(e)
                    if matches!(
                        job.stage,
                        Stage::Invoke
                            | Stage::Execute
                            | Stage::Load
                            | Stage::Import
                            | Stage::StoreNamed
                    ) && trace.runtime.as_ref().unwrap().progress().0 =>
                {
                    job.timeline.push(serde_json::json!({
                        "stage": stage_name(job.stage, job.catalog_open),
                        "done_ms": job.started.elapsed().as_secs_f64() * 1000.0,
                        "unsettled": e,
                    }));
                    job.script = None;
                }
                // The screen never settles while the calculator itself runs a
                // program (one started on it, e.g. ENTER repeating the last
                // Home entry): say so instead of only reporting the timeout.
                Tick::Failed(e) if !shared.main_waiting() => {
                    return Err(format!(
                        "{e}; the calculator is busy (a program started on the \
                         calculator may be running: press Stop or ON first)"
                    ));
                }
                Tick::Failed(e) => return Err(e),
                Tick::Done => {
                    job.timeline.push(serde_json::json!({
                        "stage": stage_name(job.stage, job.catalog_open),
                        "done_ms": job.started.elapsed().as_secs_f64() * 1000.0,
                        "keys": script.report().keys.iter().map(|k| serde_json::json!({
                            "name": k.name,
                            "queued_ms": k.queued_ms,
                            "first_change_ms": k.first_change_latency_ms,
                            "settled_ms": k.settled_ms,
                        })).collect::<Vec<_>>(),
                    }));
                    job.script = None
                }
            }
        }
        if job.catalog_open {
            job.catalog_open = false;
            self.open_editor(job, shared, trace)?;
            return Ok(None);
        }
        match job.stage {
            Stage::CheckNamedLoader => {
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                let c = checkprog::validate(&observations, NAMED_LOADER, job.check_step)?;
                if c.return_value != Some(1) {
                    return Err("Firmware rejected named program loader".into());
                }
                self.named_loader_context = c.program_va;
                job.stage = Stage::StoreNamed;
                invoke_reserved(
                    job,
                    trace,
                    c.program_va.ok_or("Named loader context missing")?,
                    NAMED_LOADER,
                    "ZZTNAMEDLOAD",
                )?;
            }
            Stage::StoreNamed => {
                let runtime = trace.runtime.as_ref().unwrap().snapshot();
                let c = runtime
                    .invocations
                    .last()
                    .ok_or("Named loader was not observed")?;
                if c.ended_ms.is_none() {
                    return Ok(None);
                }
                if trace.runtime.as_ref().unwrap().finish_observed() != ExecutionStatus::Completed
                    || !c.body_entered
                {
                    return Err("Named source assignment failed".into());
                }
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                let assignment = checkprog::validate_assignment(&observations, &job.source)?;
                let name = job.stored_name.as_ref().ok_or("Named program missing")?;
                let stored = read_program(&shared.data_dir, name)?;
                if stored.source().map_err(|e| e.to_string())?.source != job.source {
                    return Err("Named source identity mismatch".into());
                }
                job.check = Some(CheckVerdict {
                    accepted: true,
                    source_sha256: crate::sha256::hex(job.source.as_bytes()),
                    diagnostics: vec![],
                });
                job.timeline.push(
                    serde_json::json!({"stage":"store-named","context":assignment.program_va}),
                );
                let message = format!("Firmware stored exact program {name}");
                save_report(shared, trace, job, true, &message);
                return Ok(Some(message));
            }
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
                if job.direct {
                    let snapshot = trace.compiler.as_ref().unwrap().snapshot();
                    let ctx = diagnostic_context(&snapshot, &job.source)?;
                    self.direct_ctx = Some(ctx);
                    return begin_direct(job, ctx, mem, shared, trace).map(|()| None);
                }
                if job.import.is_some() {
                    self.import_script(job, trace)?;
                    return Ok(None);
                }
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                if let Ok(compiled) = checkprog::validate_assignment(&observations, &job.source) {
                    // Programs():= has already compiled the exact source with
                    // Check's arguments. Preserve its genuine context instead
                    // of selecting and compiling it again through the catalog.
                    self.prepared = Some(PreparedProgram {
                        source: job.source.clone(),
                        stored_sha: crate::sha256::hex(
                            &std::fs::read(shared.data_dir.join("C/DATA/ZZT_IDE.hpprgm"))
                                .map_err(|e| e.to_string())?,
                        ),
                        check: compiled.clone(),
                        fs_cursor: trace.fs_log_since(usize::MAX).1,
                        compiler_cursor: observations.checks.len(),
                    });
                    job.timeline.push(serde_json::json!({
                        "stage": "compile-source",
                        "method": "firmware-assignment",
                        "done_ms": job.started.elapsed().as_secs_f64() * 1000.0,
                        "compiler_started_ms": compiled.started_ms,
                        "compiler_returned_ms": compiled.returned_ms,
                    }));
                    job.stage = Stage::Prepared;
                } else if let Ok(ctx) = diagnostic_context(&observations, &job.source) {
                    self.direct_ctx = Some(ctx);
                    return begin_direct(job, ctx, mem, shared, trace).map(|()| None);
                } else {
                    job.stage = Stage::CheckTarget;
                    self.check_script(job, "ZZT_IDE", trace)?;
                }
            }
            Stage::CheckImporter => {
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                let c = checkprog::validate(&observations, IMPORTER, job.check_step)?;
                if c.return_value != Some(1) {
                    return Err("The genuine firmware rejected the IDE program importer".into());
                }
                self.importer_context = c.program_va;
                self.import_script(job, trace)?;
            }
            Stage::Import => {
                let runtime = trace.runtime.as_ref().unwrap().snapshot();
                let c = runtime
                    .invocations
                    .last()
                    .ok_or("Importer invocation was not observed")?;
                if c.ended_ms.is_none() {
                    return Ok(None);
                }
                let status = trace.runtime.as_ref().unwrap().finish_observed();
                if status != ExecutionStatus::Completed || !c.body_entered {
                    return Err(format!("Program import failed: {status:?}"));
                }
                let name = job.import.clone().unwrap_or_default();
                let stored = read_program(&shared.data_dir, &name)
                    .map_err(|e| format!("The firmware did not store program {name}: {e}"))?;
                if stored.source().map_err(|e| e.to_string())?.source != job.source {
                    return Err(format!(
                        "The firmware stored {name} with a different source revision"
                    ));
                }
                self.refresh_catalog(shared, trace);
                let message = format!(
                    "G1 firmware stored the current source as program {name} (Program Catalog)"
                );
                save_report(shared, trace, job, true, &message);
                return Ok(Some(message));
            }
            Stage::Direct => {
                let out = job
                    .direct_out
                    .as_ref()
                    .and_then(|o| o.lock().unwrap().take());
                let Some(outcome) = out else {
                    if job
                        .direct_started
                        .is_some_and(|t| t.elapsed() > Duration::from_secs(60))
                    {
                        return Err("Direct firmware compiler call did not complete".into());
                    }
                    return Ok(None);
                };
                mem.heap_free(job.direct_text);
                job.direct_text = 0;
                let ctx = self
                    .direct_ctx
                    .ok_or("Direct check without a program context")?;
                let snapshot = trace.compiler.as_ref().unwrap().snapshot();
                job.timeline.push(serde_json::json!({
                    "stage": "direct",
                    "done_ms": job.started.elapsed().as_secs_f64() * 1000.0,
                    "plan_ms": job.direct_started.map(|t| t.elapsed().as_secs_f64() * 1000.0),
                    "outcome": outcome,
                }));
                let obs = direct::validate(&job.source, ctx, &outcome, &snapshot)?;
                let accepted = obs.return_value == Some(1);
                let diagnostics = if accepted {
                    vec![]
                } else {
                    let d = checkprog::firmware_diagnostic(&job.source, &obs);
                    vec![ppl_engine::firmware::diagnostic(
                        &serde_json::to_value(d).map_err(|e| e.to_string())?,
                    )]
                };
                job.check = Some(CheckVerdict {
                    accepted,
                    source_sha256: crate::sha256::hex(job.source.as_bytes()),
                    diagnostics,
                });
                if !accepted {
                    return Err(obs
                        .message
                        .clone()
                        .unwrap_or("The firmware rejected the program".into()));
                }
                if job.run {
                    return Err("Syntax accepted, but executable preparation was not verified; the program was not run".into());
                }
                let message = "G1 firmware compiled the exact current source (direct)".to_string();
                save_report(shared, trace, job, true, &message);
                return Ok(Some(message));
            }
            Stage::CheckTarget | Stage::Prepared => {
                let c = if job.stage == Stage::Prepared {
                    self.prepared
                        .as_ref()
                        .ok_or("Prepared source was invalidated")?
                        .check
                        .clone()
                } else {
                    let observations = trace.compiler.as_ref().unwrap().snapshot();
                    checkprog::validate(&observations, &job.source, job.check_step)?.clone()
                };
                let c = &c;
                let container = read_program(&shared.data_dir, "ZZT_IDE")?;
                if container.source().map_err(|e| e.to_string())?.source != job.source {
                    return Err("The firmware did not preserve this exact source revision".into());
                }
                // A verdict only after the observation and the stored source
                // both match this job's source; the position comes from the
                // same mapping as the fresh checker's.
                let accepted = c.return_value == Some(1);
                let diagnostics = if accepted {
                    vec![]
                } else {
                    let d = checkprog::firmware_diagnostic(&job.source, c);
                    vec![ppl_engine::firmware::diagnostic(
                        &serde_json::to_value(d).map_err(|e| e.to_string())?,
                    )]
                };
                job.check = Some(CheckVerdict {
                    accepted,
                    source_sha256: crate::sha256::hex(job.source.as_bytes()),
                    diagnostics,
                });
                if c.return_value != Some(1) {
                    return Err(c
                        .message
                        .clone()
                        .unwrap_or("The firmware rejected the program".into()));
                }
                self.prepared = Some(PreparedProgram {
                    source: job.source.clone(),
                    stored_sha: crate::sha256::hex(
                        &std::fs::read(shared.data_dir.join("C/DATA/ZZT_IDE.hpprgm"))
                            .map_err(|e| e.to_string())?,
                    ),
                    check: c.clone(),
                    fs_cursor: trace.fs_log_since(usize::MAX).1,
                    compiler_cursor: trace.compiler.as_ref().unwrap().snapshot().checks.len(),
                });
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
                job.script = Some(invocation_script(runtime_value::invocation_keys(
                    &entry,
                    &request.arguments,
                )?));
            }
            Stage::Invoke => {
                job.stage = Stage::Execute;
            }
            Stage::Execute => {
                let observer = trace.runtime.as_ref().unwrap();
                if let Some(status) = observer.terminal_status() {
                    return Err(format!("Firmware runtime: {status:?}"));
                }
                if !observer.progress().1 {
                    return Ok(None);
                }
                let status = observer.finish_observed();
                let runtime = observer.snapshot();
                let c = runtime
                    .invocations
                    .last()
                    .ok_or("No genuine firmware invocation")?;
                let mut message = match status {
                    ExecutionStatus::Completed => {
                        match (
                            c.result.as_ref().and_then(|v| v.value.as_ref()),
                            ReturnRepresentation::of(c.result.as_ref()),
                        ) {
                            (Some(value), _) => {
                                format!("G1 firmware returned {}", value.display())
                            }
                            (None, ReturnRepresentation::Undecoded { tag, subtype, .. }) => {
                                format!(
                                    "G1 firmware completed; return value not decoded by host \
                                     (tag {:#x}/{:#x})",
                                    tag.unwrap_or(0),
                                    subtype.unwrap_or(0)
                                )
                            }
                            (None, _) => "G1 firmware completed".to_string(),
                        }
                    }
                    ExecutionStatus::RuntimeError => {
                        return Err(format!(
                            "Firmware runtime error: {}; {}",
                            c.diagnostic
                                .as_ref()
                                .and_then(|d| d.message.as_deref())
                                .unwrap_or("see calculator display"),
                            c.raw_error_context
                                .as_deref()
                                .unwrap_or("no error context observed")
                        ));
                    }
                    other => {
                        return Err(format!(
                            "Firmware runtime: {other:?}; {}",
                            c.raw_error_context
                                .as_deref()
                                .unwrap_or("see calculator display")
                        ));
                    }
                };
                for caveat in &c.caveats {
                    message.push_str(&format!("\nCaveat: {caveat}"));
                }
                save_report(shared, trace, job, true, &message);
                return Ok(Some(message));
            }
        }
        Ok(None)
    }
}
/// The on-screen catalog order, from the programs the guest read while
/// showing it. A full read gives the order directly. The catalog re-reads
/// only programs changed since it was last shown and lists them first, in
/// the order it reads them, ahead of the unchanged ones (as
/// `refresh_catalog` assumes for writes). A wrong order taps another row,
/// which `checkprog::validate` rejects because its parser source differs, so
/// it fails without a verdict rather than misattributing one.
fn catalog_order(
    previous: &[String],
    mut read: Vec<String>,
    expected: &[String],
) -> Result<Vec<String>, String> {
    let has = |list: &[String], n: &str| list.iter().any(|x| x.eq_ignore_ascii_case(n));
    if read.len() == expected.len() {
        return Ok(read);
    }
    let previous_complete =
        previous.len() == expected.len() && expected.iter().all(|e| has(previous, e));
    // Nothing re-read: nothing changed since the complete order was seen.
    if read.is_empty() && previous_complete {
        return Ok(previous.to_vec());
    }
    if read.is_empty() || !previous_complete || !read.iter().all(|n| has(previous, n)) {
        return Err(format!(
            "The current firmware catalog order could not be verified \
             (observed {read:?}, expected {} programs)",
            expected.len()
        ));
    }
    let unchanged: Vec<String> = previous
        .iter()
        .filter(|p| !has(&read, p))
        .cloned()
        .collect();
    read.extend(unchanged);
    Ok(read)
}
/// A user catalog listing has ended when the firmware read no program for this long.
const USER_CATALOG_QUIET: Duration = Duration::from_millis(500);
/// The program a catalog listing read (`rb+` open of `C:\DATA\NAME.hpprgm`).
fn catalog_read(event: &serde_json::Value) -> Option<String> {
    if event["op"] != "wfopen" || event["mode"] != "rb+" {
        return None;
    }
    event["path"]
        .as_str()
        .and_then(|p| p.rsplit('\\').next())
        .and_then(|p| p.strip_suffix(".hpprgm"))
        .map(str::to_owned)
}
/// This proves only a context for diagnostics, never prepared exports.
fn diagnostic_context(trace: &crate::compiler::CompilerTrace, source: &str) -> Result<u32, String> {
    if trace.dropped != 0 {
        return Err("Incomplete compiler observations".into());
    }
    let sha = crate::sha256::hex(source.as_bytes());
    let matches = trace
        .checks
        .iter()
        .filter(|c| {
            c.kind.as_deref() == Some("assign")
                && c.compiler_entered
                && c.return_value.is_some()
                && c.returned_ms.is_some()
                && c.errors.is_empty()
                && c.sources.len() == 1
                && c.sources[0].sha256 == sha
                && c.sources[0].utf16_units == source.encode_utf16().count()
        })
        .filter_map(|c| c.program_va.filter(|p| *p != 0))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [ctx] => Ok(*ctx),
        _ => Err("Assignment context was missing or ambiguous".into()),
    }
}
fn progress_message(stage: Stage) -> &'static str {
    match stage {
        Stage::CheckLoader | Stage::CheckNamedLoader => "Preparing the firmware source loader…",
        Stage::StoreNamed => "Storing named firmware program…",
        Stage::Load => "Loading and checking source in firmware…",
        Stage::CheckImporter | Stage::Import => "Saving the program to the calculator catalog…",
        Stage::CheckTarget | Stage::Direct => "Checking source with the firmware compiler…",
        Stage::Prepared | Stage::Invoke => "Starting the checked firmware program…",
        Stage::Execute => "Running in the local G1 firmware",
    }
}
fn stage_name(stage: Stage, catalog: bool) -> &'static str {
    match (stage, catalog) {
        (Stage::CheckNamedLoader, _) => "check-named-loader",
        (Stage::StoreNamed, _) => "store-named",
        (Stage::CheckLoader, true) => "open-catalog-for-loader",
        (Stage::CheckLoader, false) => "check-loader",
        (Stage::Load, _) => "run-loader",
        (Stage::CheckImporter, true) => "open-catalog-for-importer",
        (Stage::CheckImporter, false) => "check-importer",
        (Stage::Import, _) => "run-importer",
        (Stage::CheckTarget, true) => "open-catalog",
        (Stage::CheckTarget, false) => "check-source",
        (Stage::Invoke, _) => "invoke",
        (Stage::Execute, _) => "execute",
        (Stage::Direct, _) => "direct",
        (Stage::Prepared, _) => "reuse-prepared",
    }
}
/// Hands `job.source` to the firmware and starts the direct Check plan.
fn begin_direct(
    job: &mut Job,
    ctx: u32,
    mem: &GuestMemory,
    shared: &SharedState,
    trace: &TraceHub,
) -> Result<(), String> {
    let observer = trace
        .compiler
        .as_ref()
        .ok_or("compiler observation is off")?;
    observer.reset();
    observer.set_direct_return(shared.guest_calls.prepare(mem)?);
    let mut units: Vec<u16> = job.source.encode_utf16().collect();
    units.push(0);
    let text = mem.heap_alloc(units.len() * 2).map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    mem.write_bytes(text, &bytes).map_err(|e| e.to_string())?;
    let out = Arc::new(Mutex::new(None));
    if let Err(e) = shared
        .guest_calls
        .submit(Box::new(direct::Plan::new(ctx, text, out.clone())))
    {
        mem.heap_free(text);
        return Err(e);
    }
    job.stage = Stage::Direct;
    job.direct_out = Some(out);
    job.direct_text = text;
    job.direct_started = Some(Instant::now());
    Ok(())
}
/// Arms the runtime observer for a reserved helper program and types its
/// invocation at Home.
fn invoke_reserved(
    job: &mut Job,
    trace: &TraceHub,
    context: u32,
    source: &str,
    entry: &str,
) -> Result<(), String> {
    trace.compiler.as_ref().unwrap().reset();
    let observer = trace.runtime.as_ref().unwrap();
    observer.reset();
    observer.compiled();
    observer.arm(
        context,
        &crate::sha256::hex(source.as_bytes()),
        &ExecutionRequest {
            entry: entry.into(),
            arguments: vec![],
        },
        trace.elapsed_secs() * 1000.0,
    )?;
    observer.cache_flags(1);
    job.script = Some(invocation_script(runtime_value::invocation_keys(
        entry,
        &[],
    )?));
    Ok(())
}
/// Installs `text` (UTF-16LE) as a `Function` app attachment for the guest.
fn write_attachment(data: &Path, name: &str, text: &str) -> Result<(), String> {
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    write_attachment_bytes(data, name, &bytes)
}
/// G1 AFiles string serialization, reproduced from firmware-written strings.
/// The header is the unshared string object tag, followed by its UTF-16 unit
/// count and terminated text. AFiles performs the actual firmware decoding;
/// the assignment compiler and saved source must independently match the
/// original SHA and text before execution is allowed.
fn source_attachment(text: &str) -> Result<Vec<u8>, String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let count = u32::try_from(units.len()).map_err(|_| "Source exceeds AFiles string length")?;
    let mut bytes = Vec::with_capacity(10 + units.len() * 2);
    bytes.extend(0x0012_ffff_u32.to_le_bytes());
    bytes.extend(count.to_le_bytes());
    bytes.extend(units.into_iter().flat_map(u16::to_le_bytes));
    bytes.extend(0_u16.to_le_bytes());
    Ok(bytes)
}
fn write_source_attachment(data: &Path, source: &str) -> Result<(), String> {
    write_attachment_bytes(data, "ZZT_IDE_SOURCE", &source_attachment(source)?)
}
fn write_attachment_bytes(data: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let attachment = data.join("C/DATA/&Function.hpappdir").join(name);
    std::fs::create_dir_all(attachment.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(attachment.with_extension("pending"), bytes).map_err(|e| e.to_string())?;
    std::fs::rename(attachment.with_extension("pending"), attachment).map_err(|e| e.to_string())
}
/// An import creates a new, ordinary program: never a reserved name, never
/// one the calculator already holds.
pub fn validate_import(data: &Path, name: &str) -> Result<(), String> {
    import::validate_name(name).map_err(|e| e.to_string())?;
    if name.to_ascii_uppercase().starts_with("ZZT_") {
        return Err("ZZT_ names are reserved for the IDE's helper programs".into());
    }
    if import::list_programs(data)
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
    {
        return Err(format!(
            "The calculator already has a program named {name}; it was not changed"
        ));
    }
    Ok(())
}
fn read_program(data: &Path, name: &str) -> Result<hpprgm::Container, String> {
    read_program_path(&data.join(format!("C/DATA/{name}.hpprgm")))
}
fn read_program_path(path: &Path) -> Result<hpprgm::Container, String> {
    hpprgm::parse(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn invocation_script(keys: Vec<keys::Key>) -> ScriptDriver {
    // Home and ESC establish the editing context before consumption pacing.
    script(keys).with_consumption_pacing(2)
}
fn script(keys: Vec<keys::Key>) -> ScriptDriver {
    ScriptDriver::new(ScriptConfig {
        keys,
        settle_ms: 150,
        idle_observe_ms: 0,
        ..Default::default()
    })
}
fn execution_events(job: &Job, trace: &TraceHub) -> Vec<Event> {
    if !job.run || job.stored_name.is_none() {
        return vec![];
    }
    let Some(runtime) = trace.runtime.as_ref().map(|o| o.snapshot()) else {
        return vec![];
    };
    let Some(c) = runtime
        .invocations
        .last()
        .filter(|c| c.source_sha256 == crate::sha256::hex(job.source.as_bytes()))
    else {
        return vec![];
    };
    let status = c
        .status
        .or(runtime.terminal)
        .unwrap_or(ExecutionStatus::Indeterminate);
    let image = c
        .frame
        .as_ref()
        .map(|f| ppl_engine::session::ExecutionImage {
            width: f.width as usize,
            height: f.height as usize,
            pixels: f.pixels.clone(),
        });
    let observation = ppl_engine::session::ExecutionObservation {
        status: serde_json::to_value(status)
            .unwrap()
            .as_str()
            .unwrap()
            .into(),
        binding: job.binding.clone(),
        entry: c.entry.clone(),
        body_entered: c.body_entered,
        completed: c.ended_ms.is_some(),
        return_value: c
            .result
            .as_ref()
            .and_then(|v| v.value.as_ref())
            .filter(|v| !matches!(v, runtime_value::FirmwareValue::Error { .. }))
            .map(|v| serde_json::to_value(v).unwrap()),
        return_representation: serde_json::to_value(ReturnRepresentation::of(c.result.as_ref()))
            .unwrap(),
        output: c
            .output
            .iter()
            .map(|e| ppl_engine::OutputEvent {
                kind: e.kind.clone(),
                text: e.text.clone(),
                value: e.value.as_ref().map(|v| serde_json::to_value(v).unwrap()),
            })
            .collect(),
        diagnostics: c
            .diagnostic
            .as_ref()
            .map(|d| ppl_engine::firmware::diagnostic(&serde_json::to_value(d).unwrap()))
            .into_iter()
            .collect(),
        runtime: serde_json::to_value(&runtime).unwrap(),
        image,
    };
    vec![execution_wire_event(Event::Execution {
        request: job.request,
        program: job.stored_name.clone().unwrap_or_else(|| "ZZT_IDE".into()),
        source_sha256: c.source_sha256.clone(),
        observation: Box::new(observation),
    })]
}
/// Serialize only bounded wire evidence. The independent local report uses the
/// original trace, not this projection. Whole typed observations remain intact
/// unless their complete collection cannot fit; omissions are explicit gaps.
fn execution_wire_event(mut event: Event) -> Event {
    if packet_fits(&event) {
        return event;
    }
    let Event::Execution { request, .. } = &event else {
        return event;
    };
    let request = *request;
    if let Event::Execution { observation, .. } = &mut event {
        let omitted = strip_raw_value_evidence(&mut observation.runtime);
        if omitted != 0 {
            wire_gap(observation, request, "raw_value_representations", omitted);
        }
    }
    if packet_fits(&event) {
        return event;
    }
    if let Event::Execution { observation, .. } = &mut event {
        let invocations = observation.runtime["invocations"]
            .as_array()
            .map_or(0, Vec::len);
        // These are duplicated forensic call histories. Top-level return,
        // output, diagnostics and source identity remain authoritative.
        observation.runtime =
            serde_json::json!({"wire_runtime_summary":true,"invocations_omitted":invocations});
        wire_gap(observation, request, "runtime_call_history", invocations);
    }
    if packet_fits(&event) {
        return event;
    }
    if let Event::Execution { observation, .. } = &mut event {
        if observation.image.take().is_some() {
            wire_gap(observation, request, "image", 1);
        }
    }
    if packet_fits(&event) {
        return event;
    }
    if let Event::Execution { observation, .. } = &mut event {
        let outputs = observation.output.len();
        if outputs != 0 {
            observation.output.clear();
            wire_gap(observation, request, "complete_output_collection", outputs);
        }
    }
    if packet_fits(&event) {
        return event;
    }
    if let Event::Execution { observation, .. } = &mut event {
        let diagnostics = observation.diagnostics.len();
        observation.diagnostics.clear();
        wire_gap(
            observation,
            request,
            "complete_diagnostic_collection",
            diagnostics,
        );
    }
    if packet_fits(&event) {
        return event;
    }
    if let Event::Execution { observation, .. } = &mut event {
        // A codec-produced return is already <=1MiB. This is a final guard for
        // another future producer: never send a deceptively partial value.
        observation.return_value = None;
        observation.return_representation = serde_json::json!({"kind":"undecoded","reason":"Complete observed return omitted: session transport limit exceeded; inspect the local report if available"});
        wire_gap(observation, request, "complete_return_value", 1);
    }
    event
}
fn packet_fits(event: &Event) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("session packet budget exceeded"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Budget(ppl_engine::session::MAX_PACKET), event).is_ok()
}
fn strip_raw_value_evidence(value: &mut serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(object) => {
            let mut omitted = 0;
            if object.contains_key("address")
                && object.contains_key("unavailable")
                && object.contains_key("representation")
            {
                if let Some(graph) = object
                    .get_mut("representation")
                    .map(serde_json::Value::take)
                    .filter(|v| !v.is_null())
                {
                    object.insert("wire_representation_omitted".into(),serde_json::json!({"nodes":graph["nodes"].as_array().map_or(0,Vec::len),"captured_bytes":graph["captured_bytes"],"complete":graph["complete"],"coherent":graph["coherent"]}));
                    omitted += 1;
                }
                // Root address, tag/subtype, readable root prefix and original
                // unavailable diagnostic remain; duplicated bulk bytes do not.
                object.insert("probe".into(), serde_json::json!([]));
            }
            omitted
                + object
                    .values_mut()
                    .map(strip_raw_value_evidence)
                    .sum::<usize>()
        }
        serde_json::Value::Array(items) => items.iter_mut().map(strip_raw_value_evidence).sum(),
        _ => 0,
    }
}
fn wire_gap(
    observation: &mut ppl_engine::session::ExecutionObservation,
    request: u64,
    scope: &str,
    count: usize,
) {
    if !observation.runtime.is_object() {
        observation.runtime = serde_json::json!({});
    }
    let object = observation.runtime.as_object_mut().unwrap();
    let gaps = object
        .entry("wire_observation_gaps")
        .or_insert_with(|| serde_json::json!([]));
    if !gaps.is_array() {
        *gaps = serde_json::json!([]);
    }
    gaps.as_array_mut().unwrap().push(serde_json::json!({"scope":scope,"omitted":count,"reason":"session_packet_budget","local_report_candidate":format!("ide-reports/request-{request}.json")}));
    observation.diagnostics.push(ppl_engine::Diagnostic {source:"emulator-transport".into(),severity:"warning".into(),code:Some("firmware_observation_gap".into()),message:Some(format!("Execution evidence omitted from wire reply ({scope}, {count}) to fit the session packet limit. Inspect the local report if available; source identity and execution outcome are preserved.")),line:None,column:None,raw_position:None});
}
fn save_report(shared: &SharedState, trace: &TraceHub, job: &Job, success: bool, message: &str) {
    let report = serde_json::json!({"request":job.request,"source_sha256":crate::sha256::hex(job.source.as_bytes()),"success":success,"message":message,"check":job.check,"elapsed_ms":job.started.elapsed().as_secs_f64()*1000.0,"timeline":job.timeline,"compiler":trace.compiler.as_ref().map(|o|o.snapshot()),"runtime":trace.runtime.as_ref().map(|o|o.snapshot()),"debug":trace.diagnostic_bundle()});
    let dir = shared.data_dir.join("ide-reports");
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
            let _ = std::fs::write(dir.join(format!("request-{}.json", job.request)), bytes);
        }
    }
}
#[cfg(test)]
mod tests {
    fn oversized_unknown_event() -> ppl_engine::session::Event {
        use crate::{memory::GuestMemory, runtime_value};
        use ppl_engine::session::{Event, ExecutionImage, ExecutionObservation};
        let mut memory = GuestMemory::new();
        memory
            .map("synthetic objects", 0x10000, 4 * 1024 * 1024)
            .unwrap();
        memory.set_heap(0x10000, 4 * 1024 * 1024);
        let root = memory.heap_alloc(12).unwrap();
        let list = memory.heap_alloc(8 + 256 * 4).unwrap();
        memory.write_u32(root, 0x0016_0001).unwrap();
        memory.write_u32(root + 4, 1).unwrap();
        memory.write_u32(root + 8, list).unwrap();
        memory.write_u32(list, 0x0016_0001).unwrap();
        memory.write_u32(list + 4, 256).unwrap();
        for i in 0..256 {
            let address = memory.heap_alloc(8192).unwrap();
            memory.write_bytes(address, &[255; 8192]).unwrap();
            memory.write_u32(address, 0x0009_0001).unwrap();
            memory.write_u32(list + 8 + i * 4, address).unwrap();
        }
        let evidence = runtime_value::decode_return(&memory, root);
        assert!(evidence.value.is_none());
        assert!(evidence.representation.as_ref().unwrap().captured_bytes > 1024 * 1024 - 8192);
        Event::Execution {
            request: 42,
            program: "ZZT_WIRE".into(),
            source_sha256: "exact-source-hash".into(),
            observation: Box::new(ExecutionObservation {
                status: "completed".into(),
                binding: "stored".into(),
                entry: "WIRE".into(),
                body_entered: true,
                completed: true,
                return_value: None,
                return_representation: serde_json::to_value(
                    runtime_value::ReturnRepresentation::of(Some(&evidence)),
                )
                .unwrap(),
                output: vec![],
                diagnostics: vec![],
                runtime: serde_json::json!({"invocations":[{"result":evidence,"source_sha256":"exact-source-hash","entry":"WIRE","body_entered":true,"status":"completed"}]}),
                image: Some(ExecutionImage {
                    width: 320,
                    height: 240,
                    pixels: vec![u32::MAX; 320 * 240],
                }),
            }),
        }
    }
    #[test]
    fn nested_unknown_high_byte_evidence_fits_an_actual_execution_packet() {
        use ppl_engine::session::{self, Event};
        let original = oversized_unknown_event();
        let mut rejected = Vec::new();
        assert!(
            session::write_event(&mut rejected, &original).is_err(),
            "test did not reproduce oversized packet"
        );
        let event = super::execution_wire_event(original);
        let mut bytes = Vec::new();
        session::write_event(&mut bytes, &event)
            .expect("unsupported return must keep a correlated execution reply");
        assert!(bytes.len() <= session::MAX_PACKET + 5);
        let Event::Execution {
            request,
            source_sha256,
            observation,
            ..
        } = event
        else {
            panic!("lost execution identity");
        };
        assert_eq!(request, 42);
        assert_eq!(source_sha256, "exact-source-hash");
        assert!(observation.completed && observation.body_entered);
        assert!(observation.return_value.is_none());
        assert_eq!(observation.return_representation["kind"], "undecoded");
        assert!(
            observation.return_representation["reason"]
                .as_str()
                .unwrap()
                .contains("unverified")
        );
        assert!(
            observation.image.is_some(),
            "raw evidence must yield before actual pixels"
        );
        assert!(
            observation.runtime["wire_observation_gaps"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        );
        assert!(
            observation.runtime["invocations"][0]["result"]["bytes"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "root forensic prefix was lost"
        );
    }
    #[test]
    fn raw_wire_projection_preserves_whole_typed_return_output_and_error_diagnostics() {
        use ppl_engine::session::{self, Event};
        let mut event = oversized_unknown_event();
        let expected = serde_json::json!({"type":"matrix","rows":1,"columns":2,"vector":false,"value":[{"type":"complex","real":"2","imaginary":"3"},{"type":"real","value":"4"}]});
        if let Event::Execution { observation, .. } = &mut event {
            // Unknown value is an internal call argument, not the typed return.
            let capture = observation.runtime["invocations"][0]["result"].take();
            observation.runtime["invocations"][0]["calls"] =
                serde_json::json!([{"arguments":[capture]}]);
            observation.return_value = Some(expected.clone());
            observation.return_representation = serde_json::json!({"kind":"decoded"});
            observation.output.push(ppl_engine::OutputEvent {
                kind: "print".into(),
                text: Some("entire output".into()),
                value: None,
            });
            observation.diagnostics.push(ppl_engine::Diagnostic {
                source: "guest-firmware".into(),
                severity: "error".into(),
                code: Some("actual_guest_error".into()),
                message: Some("actual diagnostic".into()),
                line: Some(7),
                column: Some(3),
                raw_position: None,
            });
        }
        let event = super::execution_wire_event(event);
        session::write_event(&mut Vec::new(), &event).unwrap();
        let Event::Execution { observation, .. } = event else {
            panic!();
        };
        assert_eq!(observation.return_value, Some(expected));
        assert_eq!(observation.return_representation["kind"], "decoded");
        assert_eq!(observation.output.len(), 1);
        assert_eq!(observation.output[0].text.as_deref(), Some("entire output"));
        assert_eq!(
            observation.diagnostics[0].message.as_deref(),
            Some("actual diagnostic")
        );
        assert_eq!(observation.diagnostics[0].line, Some(7));
        assert!(observation.image.is_some());
    }
    #[test]
    fn oversized_output_collection_is_omitted_whole_with_an_explicit_gap() {
        use ppl_engine::session::{self, Event};
        let mut event = oversized_unknown_event();
        if let Event::Execution { observation, .. } = &mut event {
            observation.return_value = Some(serde_json::json!({"type":"real","value":"7"}));
            observation.return_representation = serde_json::json!({"kind":"decoded"});
            observation.output = (0..80)
                .map(|_| ppl_engine::OutputEvent {
                    kind: "print".into(),
                    text: Some("x".repeat(65536)),
                    value: None,
                })
                .collect();
        }
        let event = super::execution_wire_event(event);
        session::write_event(&mut Vec::new(), &event).unwrap();
        let Event::Execution { observation, .. } = event else {
            panic!();
        };
        assert_eq!(
            observation.return_value,
            Some(serde_json::json!({"type":"real","value":"7"}))
        );
        assert!(
            observation.output.is_empty(),
            "a partial print stream would pretend to be the full output"
        );
        assert!(
            observation.runtime["wire_observation_gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["scope"] == "complete_output_collection" && gap["omitted"] == 80)
        );
        assert!(
            observation
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("firmware_observation_gap"))
        );
    }
    #[test]
    fn ordinary_wire_events_remain_byte_for_byte_unchanged() {
        let mut event = oversized_unknown_event();
        if let ppl_engine::session::Event::Execution { observation, .. } = &mut event {
            observation.runtime = serde_json::json!({"kept":"unchanged"});
        }
        let before = serde_json::to_vec(&event).unwrap();
        assert_eq!(
            serde_json::to_vec(&super::execution_wire_event(event)).unwrap(),
            before
        );
    }
    #[test]
    fn diagnostic_context_requires_unique_complete_exact_source() {
        let source = "RETURN 1+;";
        let mut trace = crate::compiler::CompilerTrace::default();
        trace.checks.push(crate::compiler::CheckObservation {
            kind: Some("assign".into()),
            compiler_entered: true,
            returned_ms: Some(1.0),
            return_value: Some(0),
            program_va: Some(0x20001000),
            sources: vec![crate::compiler::ObservedSource {
                source_index: 0,
                address: 0x20002000,
                sha256: crate::sha256::hex(source.as_bytes()),
                utf16_units: source.encode_utf16().count(),
            }],
            ..Default::default()
        });
        assert_eq!(
            super::diagnostic_context(&trace, source).unwrap(),
            0x20001000
        );
        assert!(super::diagnostic_context(&trace, "edited").is_err());
        trace.dropped = 1;
        assert!(super::diagnostic_context(&trace, source).is_err());
        trace.dropped = 0;
        trace.checks.push(trace.checks[0].clone());
        assert!(super::diagnostic_context(&trace, source).is_err());
    }

    use super::{catalog_order, validate_import};
    #[test]
    fn source_attachment_matches_firmware_string_encoding() {
        // Firmware AFiles write/read/write capture; runtime string flags are
        // zero. UTF-16 counts surrogate pairs as two units.
        assert_eq!(
            super::source_attachment("").unwrap(),
            [0xff, 0xff, 0x12, 0, 0, 0, 0, 0, 0, 0]
        );
        let text = "Hello Ω😀\n\"quote\"\\slash";
        let bytes = super::source_attachment(text).unwrap();
        assert_eq!(&bytes[..8], &[0xff, 0xff, 0x12, 0, 23, 0, 0, 0]);
        assert_eq!(
            &bytes[8..],
            text.encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            &super::source_attachment("Hello Ω").unwrap()[8..],
            &[
                0x48, 0, 0x65, 0, 0x6c, 0, 0x6c, 0, 0x6f, 0, 0x20, 0, 0xa9, 3, 0, 0
            ]
        );
    }

    #[test]
    fn prepared_cache_requires_exact_storage_and_complete_mutation_evidence() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("C/DATA")).unwrap();
        let bytes = hpprgm::build_program("EXPORT P() BEGIN RETURN 1; END;")
            .unwrap()
            .to_bytes();
        std::fs::write(dir.path().join("C/DATA/ZZT_IDE.hpprgm"), &bytes).unwrap();
        let shared = SharedState::new(dir.path().into(), String::new());
        let trace = TraceHub::new(crate::trace::TraceConfig {
            fs_trace: true,
            compiler: true,
            verbosity: 0,
            ..Default::default()
        })
        .unwrap();
        let mut jobs = Jobs::new();
        jobs.prepared = Some(PreparedProgram {
            source: "source".into(),
            stored_sha: crate::sha256::hex(&bytes),
            check: Default::default(),
            fs_cursor: 0,
            compiler_cursor: 0,
        });
        assert!(jobs.prepared_matches("source", &shared, &trace));
        let no_evidence = TraceHub::new(crate::trace::TraceConfig::default()).unwrap();
        assert!(!jobs.prepared_matches("source", &shared, &no_evidence));
        let no_compiler = TraceHub::new(crate::trace::TraceConfig {
            fs_trace: true,
            ..Default::default()
        })
        .unwrap();
        assert!(!jobs.prepared_matches("source", &shared, &no_compiler));
        assert!(!jobs.prepared_matches("edited", &shared, &trace));
        trace.fs("wfopen", 0, 0, serde_json::json!({"mode":"rb"}));
        assert!(jobs.prepared_matches("source", &shared, &trace));
        trace.fs("wfopen", 0, 0, serde_json::json!({"mode":"wb"}));
        assert!(!jobs.prepared_matches("source", &shared, &trace));
        jobs.prepared.as_mut().unwrap().fs_cursor = 2;
        std::fs::write(dir.path().join("C/DATA/ZZT_IDE.hpprgm"), b"changed").unwrap();
        assert!(!jobs.prepared_matches("source", &shared, &trace));
        jobs.invalidate_prepared();
        assert!(!jobs.prepared_matches("source", &shared, &trace));
    }

    #[test]
    fn imports_never_reuse_existing_or_reserved_names() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("C/DATA");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("MYAPP.hpprgm"), b"x").unwrap();
        assert!(validate_import(root.path(), "M12HELLO").is_ok());
        for refused in ["MYAPP", "myapp", "ZZT_IDE", "zzt_x", "", "1ABC", "A B"] {
            assert!(validate_import(root.path(), refused).is_err(), "{refused}");
        }
    }
    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }
    #[test]
    fn a_full_read_is_the_catalog_order() {
        let all = v(&["ZZT_LOADER", "ZZT_IDE"]);
        assert_eq!(catalog_order(&[], all.clone(), &all), Ok(all));
    }
    #[test]
    fn re_read_programs_come_first_ahead_of_unchanged_ones() {
        let expected = v(&["ZZT_IDE", "ZZT_LOADER", "A"]);
        let previous = v(&["A", "ZZT_LOADER", "ZZT_IDE"]);
        assert_eq!(
            catalog_order(&previous, v(&["ZZT_IDE"]), &expected),
            Ok(v(&["ZZT_IDE", "A", "ZZT_LOADER"]))
        );
    }
    #[test]
    fn nothing_re_read_keeps_a_complete_earlier_order() {
        // The user already listed the catalog; nothing changed since.
        let expected = v(&["ZZT_IDE", "ZZT_LOADER"]);
        let previous = v(&["ZZT_LOADER", "ZZT_IDE"]);
        assert_eq!(catalog_order(&previous, vec![], &expected), Ok(previous));
    }
    #[test]
    fn an_unverifiable_order_is_refused() {
        let expected = v(&["ZZT_IDE", "ZZT_LOADER"]);
        // Nothing read without a complete earlier order, no complete earlier
        // order, or an unknown program.
        assert!(catalog_order(&[], vec![], &expected).is_err());
        assert!(catalog_order(&v(&["ZZT_IDE"]), vec![], &expected).is_err());
        assert!(catalog_order(&v(&["ZZT_IDE"]), v(&["ZZT_IDE"]), &expected).is_err());
        assert!(catalog_order(&expected, v(&["OTHER"]), &expected).is_err());
    }
}

#[cfg(test)]
mod idle_diagnostic_tests {
    use super::*;
    #[test]
    fn unequal_sampling_is_reported_without_replacing_strict_count_share() {
        let start = Instant::now();
        let meter = IdleMeter {
            samples: [
                (start, false),
                (start + Duration::from_millis(400), true),
                (start + Duration::from_millis(450), true),
                (start + Duration::from_millis(500), true),
            ]
            .into(),
        };
        let report = meter.diagnostic();
        assert_eq!(report["share"], 0.75);
        assert_eq!(report["required_share"], 0.8);
        assert_eq!(report["sample_interval_estimate"]["share"], 0.2);
        assert_eq!(report["sample_interval_estimate"]["maximum_gap_ms"], 400.0);
        assert_eq!(report["sample_interval_estimate"]["admission_input"], false);
    }
    #[test]
    fn missing_intervals_are_unknown_not_idle_evidence() {
        let meter = IdleMeter::default();
        assert!(meter.diagnostic()["sample_interval_estimate"]["share"].is_null());
        let meter = IdleMeter {
            samples: [(Instant::now(), true)].into(),
        };
        assert!(meter.diagnostic()["sample_interval_estimate"]["share"].is_null());
        assert_eq!(
            meter.diagnostic()["sample_interval_estimate"]["observed_ms"],
            0.0
        );
    }
}
