//! Passive runtime lifecycle observations for the M6-pinned G1 image.
//! No guest functions, registers, values or framebuffer pixels are modified.
use crate::runtime_value::{self, FirmwareValue, NumericArgument, ValueEvidence};
use crate::{
    compiler,
    display::{Frame, LcdView},
    memory::GuestMemory,
    services::SharedState,
    trace::TraceHub,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use unicorn_engine::{RegisterARM, Unicorn, uc_error};

pub const FUNCTION_ENTRY: u32 = 0x306d2b48;
pub const BODY_ENTRY: u32 = 0x306d2b90;
pub const EVALUATOR_RETURN: u32 = 0x306d171c;
pub const PRINT_TEXT: u32 = 0x30978c08;
pub const PRINT_VALUE: u32 = 0x30978be0;
pub const PRINT_FORMATTED: u32 = 0x30978c40;
pub const PRINT_CLEAR: u32 = 0x30978b84;
pub const ERROR_TEXT: u32 = 0x306dddc4;
pub const ERROR_CONTEXT: u32 = 0x306d0b6c;
pub const MAX_INVOCATIONS: usize = 32;
pub const MAX_EVENTS: usize = 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    RuntimeError,
    CompileError,
    InvalidEntry,
    InvalidArguments,
    TimedOut,
    GuestExecutionFault,
    InfrastructureError,
    Unsupported,
    Cancelled,
    /// The function body has not returned yet (live sessions only).
    Running,
    /// No terminal firmware evidence was observed; never a success claim.
    Indeterminate,
}
impl ExecutionStatus {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Completed => 0,
            Self::RuntimeError => 11,
            Self::CompileError => 7,
            Self::InvalidEntry => 9,
            Self::InvalidArguments => 10,
            Self::TimedOut => 3,
            Self::GuestExecutionFault => 1,
            Self::InfrastructureError => 4,
            Self::Unsupported => 8,
            Self::Cancelled => 130,
            Self::Running => 13,
            Self::Indeterminate => 12,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub entry: String,
    pub arguments: Vec<NumericArgument>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub definition: u32,
    pub compiled_node: u32,
    pub program_context: u32,
    pub evaluator_context: u32,
    pub thread: u32,
    pub argument_count: u32,
    pub declared_argument_count: u8,
    pub arguments: Vec<ValueEvidence>,
    pub time_ms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEvent {
    pub invocation: usize,
    pub kind: String,
    pub text: Option<String>,
    pub value: Option<ValueEvidence>,
    pub capture_ms: Option<f64>,
    pub pc: u32,
    pub time_ms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDiagnostic {
    pub source: String,
    pub raw_code: Option<u8>,
    pub message: Option<String>,
    pub raw_context: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEvent {
    pub address: u32,
    pub raw_code: u8,
    pub message: String,
    pub time_ms: f64,
}
/// A framebuffer capture taken at a scripted checkpoint while the program
/// ran (`SNAP:label` / `DELAY:ms`), as the guest drew it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub label: String,
    /// SHA-256 of the raw XRGB pixels (encoder independent).
    pub pixels_sha256: String,
    pub png: Option<String>,
    pub elapsed_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestFault {
    pub kind: String,
    pub pc: u32,
    pub thread: u32,
    pub registers: [u32; 16],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invocation {
    pub id: usize,
    pub entry: String,
    pub requested_arguments: Vec<NumericArgument>,
    pub export_cache_flags: Option<u32>,
    pub source_sha256: String,
    pub issued_ms: f64,
    pub started_ms: Option<f64>,
    pub ended_ms: Option<f64>,
    pub thread: Option<u32>,
    pub root_evaluator: Option<u32>,
    pub body_entered: bool,
    pub calls: Vec<FunctionCall>,
    pub result: Option<ValueEvidence>,
    pub output: Vec<OutputEvent>,
    pub errors: Vec<ErrorEvent>,
    pub raw_error_context: Option<String>,
    pub diagnostic: Option<RuntimeDiagnostic>,
    pub status: Option<ExecutionStatus>,
    pub unavailable: Vec<String>,
    /// Observation gaps that do not change the firmware-evidenced outcome,
    /// e.g. a returned representation the host cannot decode yet.
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    pub truncated: bool,
    pub heap_before: Option<(usize, usize, usize)>,
    pub heap_after: Option<(usize, usize, usize)>,
    pub framebuffer_writes: Option<u64>,
    pub capture_ms: Option<f64>,
    pub capture_error: Option<String>,
    pub screenshot: Option<String>,
    pub screenshot_sha256: Option<String>,
    #[serde(skip)]
    pub frame: Option<Frame>,
    #[serde(skip)]
    pub writes_before: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeTrace {
    pub boot_settled_ms: Option<u64>,
    pub compile_verified: bool,
    pub terminal: Option<ExecutionStatus>,
    pub reason: Option<String>,
    pub invocations: Vec<Invocation>,
    pub fault: Option<GuestFault>,
    pub dropped: usize,
}
struct Inner {
    trace: RuntimeTrace,
    program_context: Option<u32>,
}
pub struct Observer(Mutex<Inner>);
impl Default for Observer {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            trace: RuntimeTrace::default(),
            program_context: None,
        }))
    }
}
impl Observer {
    pub fn reset(&self) {
        *self.0.lock().unwrap() = Inner {
            trace: RuntimeTrace::default(),
            program_context: None,
        };
    }
    pub fn snapshot(&self) -> RuntimeTrace {
        self.0.lock().unwrap().trace.clone()
    }
    /// The latest invocation entered its function and has neither returned
    /// nor been given a final status (e.g. an observed interruption).
    pub fn unfinished(&self) -> bool {
        let g = self.0.lock().unwrap();
        g.trace
            .invocations
            .last()
            .is_some_and(|c| c.started_ms.is_some() && c.ended_ms.is_none() && c.status.is_none())
    }
    pub fn progress(&self) -> (bool, bool) {
        let g = self.0.lock().unwrap();
        g.trace.invocations.last().map_or((false, false), |c| {
            (c.started_ms.is_some(), c.ended_ms.is_some())
        })
    }
    /// Bounded passive evidence for supervising an active program externally.
    pub fn body_entry_evidence(&self) -> Option<serde_json::Value> {
        let g = self.0.lock().unwrap();
        let c = g.trace.invocations.last()?;
        c.body_entered.then(||serde_json::json!({"source":"guest-firmware","invocation":c.id,"entry":c.entry,"source_sha256":c.source_sha256,"body_entered":true,"thread":c.thread,"started_ms":c.started_ms}))
    }
    pub fn terminal_status(&self) -> Option<ExecutionStatus> {
        self.0.lock().unwrap().trace.terminal
    }
    pub fn capture_error_presentation(&self, mem: &GuestMemory, shared: &SharedState) {
        let mut g = self.0.lock().unwrap();
        if let Some(c) = g.trace.invocations.last_mut()
            && matches!(
                c.status,
                Some(ExecutionStatus::RuntimeError | ExecutionStatus::InvalidArguments)
            )
        {
            if let Ok(frame) = LcdView::locate(mem, *shared.lcd_object.lock().unwrap())
                .and_then(|v| v.capture(mem))
            {
                c.frame = Some(frame);
            }
        }
    }
    /// Home can reject argument count before a user-function frame exists.
    /// Preserve its actual error object/text rather than inventing a runtime return.
    pub fn invocation_rejected(
        &self,
        mem: &GuestMemory,
        shared: &SharedState,
        arity_mismatch: bool,
    ) -> ExecutionStatus {
        let mut g = self.0.lock().unwrap();
        let Some(c) = g.trace.invocations.last_mut() else {
            return ExecutionStatus::Indeterminate;
        };
        let status = if let Some(e) = c.errors.last() {
            c.diagnostic = Some(RuntimeDiagnostic {
                source: "guest-firmware".into(),
                raw_code: Some(e.raw_code),
                message: Some(e.message.clone()),
                raw_context: None,
                line: None,
                column: None,
            });
            if arity_mismatch {
                ExecutionStatus::InvalidArguments
            } else {
                ExecutionStatus::Indeterminate
            }
        } else {
            ExecutionStatus::Indeterminate
        };
        c.status = Some(status);
        if let Ok(frame) =
            LcdView::locate(mem, *shared.lcd_object.lock().unwrap()).and_then(|v| v.capture(mem))
        {
            c.frame = Some(frame);
        }
        status
    }
    pub fn boot_ms(&self, ms: Option<u64>) {
        self.0.lock().unwrap().trace.boot_settled_ms = ms;
    }
    pub fn compiled(&self) {
        self.0.lock().unwrap().trace.compile_verified = true;
    }
    pub fn arm(
        &self,
        context: u32,
        source_hash: &str,
        request: &ExecutionRequest,
        time_ms: f64,
    ) -> Result<(), String> {
        let mut g = self.0.lock().unwrap();
        if g.trace.invocations.len() >= MAX_INVOCATIONS {
            return Err("invocation observation limit reached".into());
        }
        g.program_context = Some(context);
        let id = g.trace.invocations.len();
        g.trace.invocations.push(Invocation {
            id,
            entry: request.entry.clone(),
            requested_arguments: request.arguments.clone(),
            export_cache_flags: None,
            source_sha256: source_hash.into(),
            issued_ms: time_ms,
            started_ms: None,
            ended_ms: None,
            thread: None,
            root_evaluator: None,
            body_entered: false,
            calls: Vec::new(),
            result: None,
            output: Vec::new(),
            errors: Vec::new(),
            raw_error_context: None,
            diagnostic: None,
            status: None,
            unavailable: Vec::new(),
            caveats: Vec::new(),
            checkpoints: Vec::new(),
            truncated: false,
            heap_before: None,
            heap_after: None,
            framebuffer_writes: None,
            capture_ms: None,
            capture_error: None,
            screenshot: None,
            screenshot_sha256: None,
            frame: None,
            writes_before: 0,
        });
        Ok(())
    }
    pub fn checkpoint(&self, checkpoint: Checkpoint) {
        if let Some(c) = self.0.lock().unwrap().trace.invocations.last_mut()
            && c.checkpoints.len() < MAX_EVENTS
        {
            c.checkpoints.push(checkpoint);
        }
    }
    pub fn cache_flags(&self, flags: u32) {
        if let Some(c) = self.0.lock().unwrap().trace.invocations.last_mut() {
            c.export_cache_flags = Some(flags);
        }
    }
    pub fn terminal(&self, status: ExecutionStatus, reason: impl Into<String>) {
        let mut g = self.0.lock().unwrap();
        if g.trace.terminal.is_none() {
            g.trace.terminal = Some(status);
            g.trace.reason = Some(reason.into());
        }
    }
    /// Classify the latest invocation from positive firmware evidence, shared
    /// by batch and live sessions. Completion requires the root evaluator's
    /// teardown with a non-error result object; host-side decoding gaps are
    /// recorded as caveats and never turn a completed run into a failure.
    pub fn finish_observed(&self) -> ExecutionStatus {
        let snapshot = self.snapshot();
        let Some(c) = snapshot.invocations.last() else {
            return ExecutionStatus::Indeterminate;
        };
        if c.ended_ms.is_none() {
            return ExecutionStatus::Running;
        }
        let (status, caveats) = classify(c);
        if let Some(c) = self.0.lock().unwrap().trace.invocations.last_mut() {
            c.caveats = caveats;
        }
        self.finish_invocation(status);
        status
    }
    pub fn finish_invocation(&self, status: ExecutionStatus) {
        if let Some(c) = self.0.lock().unwrap().trace.invocations.last_mut() {
            c.status = Some(status);
        }
    }
    pub fn fault(&self, f: &crate::engine::FaultReport) {
        let mut g = self.0.lock().unwrap();
        g.trace.fault = Some(GuestFault {
            kind: f.kind.clone(),
            pc: f.pc,
            thread: f.thread_id,
            registers: f.regs,
        });
        g.trace.terminal = Some(ExecutionStatus::GuestExecutionFault);
        g.trace.reason = Some(f.kind.clone());
    }
    fn error(&self, e: String) {
        if let Some(c) = self.0.lock().unwrap().trace.invocations.last_mut()
            && c.unavailable.len() < 16
        {
            c.unavailable.push(e);
        }
    }
    fn observe(
        &self,
        uc: &Unicorn<'_, ()>,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
        tid: u32,
        pc: u32,
    ) -> Result<(), String> {
        let reg = |r| uc.reg_read(r).map(|x| x as u32).map_err(|e| e.to_string());
        let offset = |p: u32, n: u32| {
            p.checked_add(n)
                .ok_or_else(|| "guest pointer overflow".to_owned())
        };
        let mut g = self.0.lock().unwrap();
        let expected = g.program_context;
        let Some(c) = g.trace.invocations.last_mut() else {
            return Ok(());
        };
        if c.status.is_some() {
            return Ok(());
        }
        let now = trace.elapsed_secs() * 1000.0;
        match pc {
            FUNCTION_ENTRY => {
                let def = reg(RegisterARM::R0)?;
                let program = mem
                    .read_u32(offset(def, 0x10)?)
                    .map_err(|e| e.to_string())?;
                if c.started_ms.is_none() && Some(program) != expected {
                    return Ok(());
                }
                let node = mem
                    .read_u32(offset(def, 0x20)?)
                    .map_err(|e| e.to_string())?;
                let name = compiler::read_utf16(mem, offset(node, 8)?, 128)?;
                if c.started_ms.is_none() && name != c.entry {
                    return Err(format!("guest selected {name}, expected {}", c.entry));
                }
                let argc = reg(RegisterARM::R2)?;
                if argc > 31 {
                    return Err("unverified guest argument-vector length".into());
                }
                let argv = reg(RegisterARM::R1)?;
                let evaluator = mem
                    .read_u32(offset(argv, argc * 4)?)
                    .map_err(|e| e.to_string())?;
                if c.started_ms.is_none() {
                    c.started_ms = Some(now);
                    c.thread = Some(tid);
                    c.root_evaluator = Some(evaluator);
                    c.heap_before = Some(mem.heap_stats());
                    c.writes_before = shared.lcd_watch.writes();
                }
                if c.thread != Some(tid) || c.ended_ms.is_some() {
                    return Ok(());
                }
                if c.calls.len() >= MAX_EVENTS {
                    c.truncated = true;
                    return Ok(());
                }
                let mut arguments = Vec::new();
                for i in 0..argc {
                    arguments.push(runtime_value::decode(
                        mem,
                        mem.read_u32(offset(argv, i * 4)?)
                            .map_err(|e| e.to_string())?,
                    ));
                }
                c.calls.push(FunctionCall {
                    name,
                    definition: def,
                    compiled_node: node,
                    program_context: program,
                    evaluator_context: evaluator,
                    thread: tid,
                    argument_count: argc,
                    declared_argument_count: ((mem.read_u32(node).map_err(|e| e.to_string())?
                        >> 24)
                        & 31) as u8,
                    arguments,
                    time_ms: now,
                });
            }
            BODY_ENTRY if c.thread == Some(tid) && c.ended_ms.is_none() => {
                if c.calls
                    .first()
                    .is_some_and(|f| f.definition == reg(RegisterARM::R6).unwrap_or(0))
                {
                    c.body_entered = true;
                }
            }
            EVALUATOR_RETURN
                if c.thread == Some(tid)
                    && c.ended_ms.is_none()
                    && c.root_evaluator == Some(reg(RegisterARM::R5)?) =>
            {
                let evaluator = reg(RegisterARM::R5)?;
                if mem
                    .read_u32(offset(evaluator, 8)?)
                    .map_err(|e| e.to_string())?
                    != 0
                {
                    return Ok(());
                }
                c.result = Some(runtime_value::decode(mem, reg(RegisterARM::R4)?));
                c.ended_ms = Some(now);
                c.heap_after = Some(mem.heap_stats());
                if shared
                    .lcd_watch
                    .enabled
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    c.framebuffer_writes = Some(shared.lcd_watch.writes() - c.writes_before);
                }
                let started = Instant::now();
                match LcdView::locate(mem, *shared.lcd_object.lock().unwrap())
                    .and_then(|v| v.capture_from_hook(mem))
                {
                    Ok(frame) => c.frame = Some(frame),
                    Err(e) => c.capture_error = Some(e.to_string()),
                }
                c.capture_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                if let Some(FirmwareValue::Error { raw_code }) =
                    c.result.as_ref().and_then(|v| v.value.as_ref())
                {
                    let message = c
                        .errors
                        .iter()
                        .rev()
                        .find(|e| e.raw_code == *raw_code)
                        .map(|e| e.message.clone());
                    c.diagnostic = Some(RuntimeDiagnostic {
                        source: "guest-firmware".into(),
                        raw_code: Some(*raw_code),
                        message,
                        raw_context: c.raw_error_context.clone(),
                        line: None,
                        column: None,
                    });
                }
            }
            PRINT_TEXT | PRINT_FORMATTED if c.thread == Some(tid) && c.ended_ms.is_none() => {
                let capture_started = Instant::now();
                let text = compiler::read_utf16(mem, reg(RegisterARM::R1)?, 65537)?;
                if c.output.len() >= MAX_EVENTS
                    || c.output
                        .iter()
                        .filter_map(|e| e.text.as_ref())
                        .map(String::len)
                        .sum::<usize>()
                        + text.len()
                        > 1_048_576
                {
                    c.truncated = true;
                    return Ok(());
                }
                c.output.push(OutputEvent {
                    invocation: c.id,
                    kind: "print".into(),
                    text: Some(text),
                    value: None,
                    capture_ms: Some(capture_started.elapsed().as_secs_f64() * 1000.0),
                    pc,
                    time_ms: now,
                });
            }
            PRINT_VALUE if c.thread == Some(tid) && c.ended_ms.is_none() => {
                if c.output.len() >= MAX_EVENTS {
                    c.truncated = true;
                    return Ok(());
                }
                let capture_started = Instant::now();
                let value = Some(runtime_value::decode(mem, reg(RegisterARM::R1)?));
                let capture_ms = Some(capture_started.elapsed().as_secs_f64() * 1000.0);
                c.output.push(OutputEvent {
                    invocation: c.id,
                    kind: "print_value".into(),
                    text: None,
                    value,
                    capture_ms,
                    pc,
                    time_ms: now,
                });
            }
            PRINT_CLEAR if c.thread == Some(tid) && c.ended_ms.is_none() => {
                if c.output.len() < MAX_EVENTS {
                    c.output.push(OutputEvent {
                        invocation: c.id,
                        kind: "clear".into(),
                        text: None,
                        value: None,
                        capture_ms: None,
                        pc,
                        time_ms: now,
                    });
                } else {
                    c.truncated = true;
                }
            }
            ERROR_TEXT
                if (c.thread == Some(tid) || (c.thread.is_none() && tid == 0))
                    && c.ended_ms.is_none() =>
            {
                let object = reg(RegisterARM::R4)?;
                let h = mem.read_u32(object).map_err(|e| e.to_string())?;
                if (h >> 16) & 15 != 5 {
                    return Ok(());
                }
                let message = compiler::read_utf16(mem, reg(RegisterARM::R0)?, 4096)?;
                if c.errors.len() < MAX_EVENTS {
                    c.errors.push(ErrorEvent {
                        address: object,
                        raw_code: (h >> 24) as u8,
                        message,
                        time_ms: now,
                    });
                } else {
                    c.truncated = true;
                }
            }
            ERROR_CONTEXT
                if c.thread == Some(tid)
                    && c.ended_ms.is_none()
                    && c.root_evaluator == Some(reg(RegisterARM::R5)?) =>
            {
                c.raw_error_context = Some(compiler::read_utf16(mem, reg(RegisterARM::R1)?, 8192)?);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Status and caveats of an invocation whose root evaluator has returned.
pub fn classify(c: &Invocation) -> (ExecutionStatus, Vec<String>) {
    let mut caveats = Vec::new();
    if !c.unavailable.is_empty() {
        caveats.push(format!("observation errors: {}", c.unavailable.join("; ")));
    }
    if c.truncated {
        caveats.push("event capture truncated at its bound".into());
    }
    if let Some(r) = c.result.as_ref().filter(|r| r.value.is_none()) {
        caveats.push(format!(
            "return value not decoded: {}",
            r.unavailable.as_deref().unwrap_or("unknown representation")
        ));
    }
    if c.calls
        .first()
        .is_some_and(|f| f.arguments.iter().any(|a| a.value.is_none()))
    {
        caveats.push("an argument value was not decoded".into());
    }
    if c.output
        .iter()
        .any(|o| o.value.as_ref().is_some_and(|v| v.value.is_none()))
    {
        caveats.push("a printed value was not decoded".into());
    }
    let status = match c.result.as_ref() {
        _ if !c.body_entered => ExecutionStatus::InvalidArguments,
        Some(r) if matches!(r.value, Some(FirmwareValue::Error { .. })) => {
            ExecutionStatus::RuntimeError
        }
        // An unreadable header could still be an error object.
        Some(r) if r.value.is_some() || r.tag.is_some_and(|t| t & 15 != 5) => {
            ExecutionStatus::Completed
        }
        _ => ExecutionStatus::Indeterminate,
    };
    (status, caveats)
}

pub fn install(
    engine: &mut Unicorn<'static, ()>,
    mem: &Arc<GuestMemory>,
    shared: &Arc<SharedState>,
    trace: &Arc<TraceHub>,
    tid: u32,
) -> Result<(), uc_error> {
    if trace.runtime.is_none() {
        return Ok(());
    }
    for pc in [
        FUNCTION_ENTRY,
        BODY_ENTRY,
        EVALUATOR_RETURN,
        PRINT_TEXT,
        PRINT_FORMATTED,
        PRINT_VALUE,
        PRINT_CLEAR,
        ERROR_TEXT,
        ERROR_CONTEXT,
    ] {
        let mem = mem.clone();
        let shared = shared.clone();
        let trace = trace.clone();
        engine.add_code_hook(
            pc as u64,
            pc as u64,
            Box::new(move |uc: &mut Unicorn<'_, ()>, _, _| {
                if let Err(e) = trace
                    .runtime
                    .as_ref()
                    .unwrap()
                    .observe(uc, &mem, &shared, &trace, tid, pc)
                {
                    trace.runtime.as_ref().unwrap().error(e);
                }
            }),
        )?;
    }
    Ok(())
}
