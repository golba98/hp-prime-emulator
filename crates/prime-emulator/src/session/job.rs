//! Same-session source replacement using documented Programs and AFilesB APIs.
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
/// Reserved source loader. It reads the attachment in chunks: one byte list
/// of a whole larger program (FIREWORKS_G1, 10 KB: about 21 000 bytes of
/// UTF-16) fails in the firmware with "Error: Insufficient memory". Each
/// chunk becomes text with one `CHAR(list)`: appending character by
/// character took over 20 s for that program.
pub const LOADER: &str = "EXPORT ZZTIDELOAD()\nBEGIN\n LOCAL n:=Function.AFilesB(\"ZZT_IDE_SOURCE\");\n LOCAL s:=\"\";\n LOCAL o,b,c,i;\n FOR o FROM 0 TO n-1 STEP 4096 DO\n  b:=Function.AFilesB(\"ZZT_IDE_SOURCE\",o,MIN(4096,n-o));\n  c:={};\n  FOR i FROM 1 TO SIZE(b)/2 DO\n   c(i):=b(2*i-1)+256*b(2*i);\n  END;\n  s:=s+CHAR(c);\n END;\n Programs(\"ZZT_IDE\"):=s;\n RETURN SIZE(s);\nEND;\n";
/// The first loader (M10), which IDE profiles may still hold; the session
/// replaces exactly this text with [`LOADER`] before boot.
pub const LOADER_V1: &str = "EXPORT ZZTIDELOAD()\nBEGIN\n LOCAL b:=Function.AFilesB(\"ZZT_IDE_SOURCE\",0,Function.AFilesB(\"ZZT_IDE_SOURCE\"));\n LOCAL s:=\"\";\n LOCAL i;\n FOR i FROM 1 TO SIZE(b) STEP 2 DO\n  s:=s+CHAR(b(i)+256*b(i+1));\n END;\n Programs(\"ZZT_IDE\"):=s;\n RETURN SIZE(s);\nEND;\n";
/// Reserved importer: stores ZZT_IDE's source as a new program whose name
/// the host passes in an app attachment (`Programs(name):=`).
pub const IMPORTER: &str = "EXPORT ZZTIDEIMPORT()\nBEGIN\n LOCAL b:=Function.AFilesB(\"ZZT_IDE_NAME\",0,Function.AFilesB(\"ZZT_IDE_NAME\"));\n LOCAL n:=\"\";\n LOCAL i;\n FOR i FROM 1 TO SIZE(b) STEP 2 DO\n  n:=n+CHAR(b(i)+256*b(i+1));\n END;\n Programs(n):=Programs(\"ZZT_IDE\");\n RETURN SIZE(n);\nEND;\n";
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
}
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    CheckLoader,
    Load,
    /// Check the reserved importer once, to learn its program context.
    CheckImporter,
    /// The importer stores the loaded source under the requested name.
    Import,
    CheckTarget,
    Invoke,
    Execute,
    /// A direct compiler call plan is running on the UI thread.
    Direct,
}
pub struct Job {
    pub request: u64,
    source: String,
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
pub struct Jobs {
    pub active: Option<Job>,
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
            interrupted: None,
            idle: IdleMeter::default(),
            break_presses: (0, None),
            on_held: None,
            deferred: None,
            loader_context: None,
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
    /// Learns the catalog order from catalog listings the user opened while
    /// no job ran: the firmware reads every program the first time it shows
    /// its catalog and later only changed ones, so a job opening a catalog
    /// the user already saw reads nothing new.
    fn observe_user_catalog(&mut self, shared: &SharedState, trace: &TraceHub) {
        let (events, _) = trace.fs_log();
        if self.active.is_some() {
            self.user_seen = events.len();
            self.user_burst.clear();
            return;
        }
        for event in events.iter().skip(self.user_seen) {
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
        self.user_seen = events.len();
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
        write_attachment(&shared.data_dir, "ZZT_IDE_SOURCE", &source)?;
        self.refresh_catalog(shared, trace);
        let stage = if self.loader_context.is_some() {
            Stage::Load
        } else {
            Stage::CheckLoader
        };
        let mut job = Job {
            request,
            source,
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
            check: None,
            timeline: Vec::new(),
            direct: false,
            direct_out: None,
            direct_text: 0,
            direct_started: None,
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
                    check: job.check,
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
                        check: job.check,
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
                // A program that keeps drawing, or a helper that runs long
                // (the loader with a large source), never lets the
                // invocation's ENTER settle; once the guest entered the
                // function, its outcome comes from the runtime observer, not
                // the script.
                Tick::Failed(e)
                    if matches!(
                        job.stage,
                        Stage::Invoke | Stage::Execute | Stage::Load | Stage::Import
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
                    let sha = crate::sha256::hex(job.source.as_bytes());
                    let snapshot = trace.compiler.as_ref().unwrap().snapshot();
                    let ctx = snapshot
                        .checks
                        .iter()
                        .filter(|c| {
                            c.kind.as_deref() == Some("assign")
                                && c.return_value.is_some()
                                && c.sources.len() == 1
                                && c.sources[0].sha256 == sha
                        })
                        .map(|c| c.program_va)
                        .collect::<Vec<_>>();
                    let [Some(ctx)] = ctx.as_slice() else {
                        return Err("The loader's program assignment was not observed".into());
                    };
                    self.direct_ctx = Some(*ctx);
                    return begin_direct(job, *ctx, mem, shared, trace).map(|()| None);
                }
                if job.import.is_some() {
                    self.import_script(job, trace)?;
                    return Ok(None);
                }
                job.stage = Stage::CheckTarget;
                self.check_script(job, "ZZT_IDE", trace)?;
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
                let message = "G1 firmware compiled the exact current source (direct)".to_string();
                save_report(shared, trace, job, true, &message);
                return Ok(Some(message));
            }
            Stage::CheckTarget => {
                let observations = trace.compiler.as_ref().unwrap().snapshot();
                let c = checkprog::validate(&observations, &job.source, job.check_step)?;
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
fn stage_name(stage: Stage, catalog: bool) -> &'static str {
    match (stage, catalog) {
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
    job.script = Some(script(runtime_value::invocation_keys(entry, &[])?));
    Ok(())
}
/// Installs `text` (UTF-16LE) as a `Function` app attachment for the guest.
fn write_attachment(data: &Path, name: &str, text: &str) -> Result<(), String> {
    let attachment = data.join("C/DATA/&Function.hpappdir").join(name);
    std::fs::create_dir_all(attachment.parent().unwrap()).map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
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
fn script(keys: Vec<keys::Key>) -> ScriptDriver {
    ScriptDriver::new(ScriptConfig {
        keys,
        settle_ms: 150,
        idle_observe_ms: 0,
        ..Default::default()
    })
}
fn save_report(shared: &SharedState, trace: &TraceHub, job: &Job, success: bool, message: &str) {
    let report = serde_json::json!({"request":job.request,"source_sha256":crate::sha256::hex(job.source.as_bytes()),"success":success,"message":message,"check":job.check,"elapsed_ms":job.started.elapsed().as_secs_f64()*1000.0,"timeline":job.timeline,"compiler":trace.compiler.as_ref().map(|o|o.snapshot()),"runtime":trace.runtime.as_ref().map(|o|o.snapshot())});
    let dir = shared.data_dir.join("ide-reports");
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
            let _ = std::fs::write(dir.join(format!("request-{}.json", job.request)), bytes);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{catalog_order, validate_import};
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
