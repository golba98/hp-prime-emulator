//! Execution controller types and VM coordination.
//!
//! Defines commands sent from the IDE GUI to the background VM worker thread,
//! snapshots representing paused execution states, stack frames, and variable
//! inspection data, plus the thread-safe [`VmBridge`] the worker's debugger
//! uses to pause and resume.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use ppl_math::FormatOptions;
use ppl_syntax::{SourceFile, Span};
use ppl_vm::{DebugAction, Debugger, Machine, Value};

use super::worker::{WorkerConfig, WorkerHandle, WorkerOutcome};
use crate::host::GuiSharedState;

/// How long the debugger waits between interrupt re-checks while paused.
const PAUSE_SLICE: Duration = Duration::from_millis(50);

/// Command sent from IDE UI to the VM execution worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerCommand {
    /// Start or restart program execution.
    Run {
        source: String,
        entry: Option<String>,
        debug: bool,
    },
    /// Stop and interrupt program execution.
    Stop,
    /// Pause execution (for debugger).
    Pause,
    /// Continue execution from pause.
    Continue,
    /// Step into next statement or function call.
    StepInto,
    /// Step over current statement.
    StepOver,
    /// Step out of current function call.
    StepOut,
}

/// Command sent to a paused debugger via [`VmBridge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugCommand {
    Continue,
    StepInto,
    StepOver,
    StepOut,
    Stop,
}

/// Snapshot of the VM state when paused at a breakpoint or step.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugPauseSnapshot {
    pub line: usize,
    pub col: usize,
    pub function: String,
    pub depth: usize,
    pub frames: Vec<StackFrameSnapshot>,
    pub globals: Vec<VariableSnapshot>,
}

/// Snapshot of a single call stack frame.
#[derive(Clone, Debug, PartialEq)]
pub struct StackFrameSnapshot {
    pub function: String,
    pub line: usize,
    pub col: usize,
    pub locals: Vec<VariableSnapshot>,
}

/// Snapshot of a single variable (local or global).
#[derive(Clone, Debug, PartialEq)]
pub struct VariableSnapshot {
    pub name: String,
    pub type_name: &'static str,
    pub preview: String,
    pub details: VariableDetailSnapshot,
}

/// Detail snapshot for compound variable inspection.
#[derive(Clone, Debug, PartialEq)]
pub enum VariableDetailSnapshot {
    Scalar,
    List(Vec<String>),
    Matrix {
        rows: usize,
        cols: usize,
        elements: Vec<String>,
    },
    Complex {
        re: String,
        im: String,
    },
    StringValue(String),
}

impl VariableSnapshot {
    /// Builds a snapshot of a VM value, using the machine's display format.
    pub fn from_value(name: impl Into<String>, val: &Value, fmt: FormatOptions) -> Self {
        let name = name.into();
        let preview = val.display(fmt);
        let (type_name, details) = value_details(val, fmt);
        VariableSnapshot {
            name,
            type_name,
            preview,
            details,
        }
    }
}

fn value_details(val: &Value, fmt: FormatOptions) -> (&'static str, VariableDetailSnapshot) {
    match val {
        Value::Real(_) => ("Real", VariableDetailSnapshot::Scalar),
        Value::Int(_) => ("Int", VariableDetailSnapshot::Scalar),
        Value::Complex(c) => {
            let (re, im) = (c.re.format(fmt), c.im.format(fmt));
            ("Complex", VariableDetailSnapshot::Complex { re, im })
        }
        Value::Str(s) => ("String", VariableDetailSnapshot::StringValue(s.to_string())),
        Value::List(items) => (
            "List",
            VariableDetailSnapshot::List(items.iter().map(|v| v.display(fmt)).collect()),
        ),
        Value::Matrix(m) => (
            "Matrix",
            VariableDetailSnapshot::Matrix {
                rows: m.rows,
                cols: m.cols,
                elements: m.data.iter().map(|v| v.display(fmt)).collect(),
            },
        ),
        Value::Grob(_) => ("Grob", VariableDetailSnapshot::Scalar),
    }
}

/// Thread-safe bridge between the IDE UI thread and the worker's debugger.
///
/// The UI sets breakpoints and posts commands; the worker's [`WorkerDebugger`]
/// consults breakpoints, publishes pause snapshots, and blocks on commands.
#[derive(Default)]
pub struct VmBridge {
    /// Breakpoint line numbers (1-based) shared with the editor gutter.
    pub breakpoints: Mutex<BTreeSet<usize>>,
    commands: Mutex<VecDeque<DebugCommand>>,
    commands_wake: Condvar,
    /// Snapshot of the current pause, if paused.
    pub pause_snapshot: Mutex<Option<DebugPauseSnapshot>>,
    /// Bumped every time the debugger pauses. Lets the UI detect new pauses.
    pub pause_generation: AtomicU64,
    /// 1-based line the VM is currently paused on (0 when not paused).
    pub paused_line: AtomicU64,
    /// Whether the next run should stop at the first statement.
    pub stop_at_entry: AtomicBool,
    /// The source being debugged (set before the worker spawns).
    source: Mutex<Option<SourceFile>>,
    wake_ui: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl std::fmt::Debug for VmBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VmBridge").finish_non_exhaustive()
    }
}

impl VmBridge {
    pub fn set_wake_ui(&self, wake: impl Fn() + Send + 'static) {
        *lock(&self.wake_ui) = Some(Box::new(wake));
    }

    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_breakpoints(&self, lines: &BTreeSet<usize>) {
        *lock(&self.breakpoints) = lines.clone();
    }

    /// Stores the source being debugged (called before the worker spawns).
    pub fn set_source(&self, file: SourceFile) {
        *lock(&self.source) = Some(file);
    }

    fn take_source(&self) -> SourceFile {
        lock(&self.source)
            .clone()
            .unwrap_or_else(|| SourceFile::new("program.ppl", ""))
    }

    pub fn toggle_breakpoint(&self, line: usize) -> bool {
        let mut bps = lock(&self.breakpoints);
        if !bps.remove(&line) {
            bps.insert(line);
            true
        } else {
            false
        }
    }

    /// Posts a command to the worker (wakes it if paused).
    pub fn post_command(&self, cmd: DebugCommand) {
        lock(&self.commands).push_back(cmd);
        self.commands_wake.notify_all();
    }

    /// Clears the paused state on the UI side after consuming a snapshot.
    pub fn take_pause_snapshot(&self) -> Option<DebugPauseSnapshot> {
        lock(&self.pause_snapshot).clone()
    }

    pub fn paused_line(&self) -> usize {
        self.paused_line.load(Ordering::SeqCst) as usize
    }

    /// Waits for a command, checking the interrupt flag between slices.
    /// Returns `None` when interrupted while paused.
    fn wait_command(&self, interrupt: &AtomicBool) -> Option<DebugCommand> {
        loop {
            if interrupt.load(Ordering::SeqCst) {
                return None;
            }
            {
                let mut cmds = lock(&self.commands);
                if let Some(cmd) = cmds.pop_front() {
                    return Some(cmd);
                }
            }
            let (guard, _) = self
                .commands_wake
                .wait_timeout_while(lock(&self.commands), PAUSE_SLICE, |_| {
                    !interrupt.load(Ordering::SeqCst)
                })
                .unwrap_or_else(|e| e.into_inner());
            drop(guard);
        }
    }
}

enum StepMode {
    Step,
    Next(usize),
    Finish(usize),
    Continue,
}

/// Debugger installed on the worker thread's [`Machine`] when the IDE runs
/// with debugging enabled. Implements the same stop semantics as `ppl debug`.
pub struct WorkerDebugger {
    bridge: Arc<VmBridge>,
    source: SourceFile,
    mode: StepMode,
    prev_stmt: Option<(usize, usize)>,
    interrupt: Arc<AtomicBool>,
}

impl WorkerDebugger {
    pub fn new(bridge: Arc<VmBridge>, interrupt: Arc<AtomicBool>) -> Self {
        let mode = if bridge.stop_at_entry.load(Ordering::SeqCst) {
            StepMode::Step
        } else {
            StepMode::Continue
        };
        let source = bridge.take_source();
        Self {
            bridge,
            source,
            mode,
            prev_stmt: None,
            interrupt,
        }
    }

    fn should_stop(&self, depth: usize) -> bool {
        match self.mode {
            StepMode::Step => true,
            StepMode::Next(d) => depth <= d,
            StepMode::Finish(d) => depth < d,
            StepMode::Continue => false,
        }
    }

    fn build_snapshot(
        &self,
        machine: &Machine,
        function: &str,
        line: usize,
        col: usize,
    ) -> DebugPauseSnapshot {
        let fmt = machine.settings.format();
        let stack = machine.call_stack();
        let mut frames = Vec::with_capacity(stack.len());
        // Frame 0 is the current stop position.
        frames.push(StackFrameSnapshot {
            function: function.to_string(),
            line,
            col,
            locals: machine
                .locals(0)
                .into_iter()
                .map(|(n, v)| VariableSnapshot::from_value(n, &v, fmt))
                .collect(),
        });
        for (depth, (name, span)) in stack.iter().enumerate().skip(1) {
            let lc = self.source.line_col(span.start);
            frames.push(StackFrameSnapshot {
                function: name.clone(),
                line: lc.line as usize,
                col: lc.col as usize,
                locals: machine
                    .locals(depth)
                    .into_iter()
                    .map(|(n, v)| VariableSnapshot::from_value(n, &v, fmt))
                    .collect(),
            });
        }
        let globals = machine
            .globals()
            .into_iter()
            .map(|(n, v)| VariableSnapshot::from_value(n, &v, fmt))
            .collect();
        DebugPauseSnapshot {
            line,
            col,
            function: function.to_string(),
            depth: stack.len(),
            frames,
            globals,
        }
    }
}

impl Debugger for WorkerDebugger {
    fn on_statement(
        &mut self,
        machine: &Machine,
        function: &str,
        span: Span,
        depth: usize,
    ) -> DebugAction {
        if self.interrupt.load(Ordering::SeqCst) {
            return DebugAction::Abort;
        }
        let lc = self.source.line_col(span.start);
        let (line, col) = (lc.line as usize, lc.col as usize);
        let prev = self.prev_stmt.replace((line, depth));
        let breakpoints = lock(&self.bridge.breakpoints).clone();
        let at_breakpoint = breakpoints.contains(&line) && prev != Some((line, depth));

        if !at_breakpoint && !self.should_stop(depth) {
            return DebugAction::Continue;
        }

        // Publish the pause and wait for a UI command.
        let snapshot = self.build_snapshot(machine, function, line, col);
        *lock(&self.bridge.pause_snapshot) = Some(snapshot);
        self.bridge.pause_generation.fetch_add(1, Ordering::SeqCst);
        self.bridge.paused_line.store(line as u64, Ordering::SeqCst);
        if let Some(wake) = lock(&self.bridge.wake_ui).as_ref() {
            wake();
        }

        let cmd = match self.bridge.wait_command(&self.interrupt) {
            Some(cmd) => cmd,
            None => {
                // Interrupted while paused: clear state and abort.
                *lock(&self.bridge.pause_snapshot) = None;
                self.bridge.paused_line.store(0, Ordering::SeqCst);
                return DebugAction::Abort;
            }
        };

        *lock(&self.bridge.pause_snapshot) = None;
        self.bridge.paused_line.store(0, Ordering::SeqCst);

        match cmd {
            DebugCommand::Continue => self.mode = StepMode::Continue,
            DebugCommand::StepInto => self.mode = StepMode::Step,
            DebugCommand::StepOver => self.mode = StepMode::Next(depth),
            DebugCommand::StepOut => self.mode = StepMode::Finish(depth),
            DebugCommand::Stop => return DebugAction::Abort,
        }
        DebugAction::Continue
    }
}

/// Result of a run-start attempt.
#[derive(Debug)]
pub enum RunStart {
    Started,
    AlreadyRunning,
}

/// Execution controller coordinator between GUI and worker.
///
/// Owns the current worker handle and the shared VM state, and translates
/// [`ControllerCommand`]s into worker lifecycle operations.
pub struct ExecutionController {
    pub is_running: Arc<AtomicBool>,
    pub is_paused: Arc<AtomicBool>,
    pub is_interrupted: Arc<AtomicBool>,
    /// Shared VM host state (screen, keys, terminal, interrupt).
    pub shared: Arc<GuiSharedState>,
    /// Debug bridge shared with the worker's debugger.
    pub bridge: Arc<VmBridge>,
    worker: Option<WorkerHandle>,
    /// Number of terminal lines already drained into the console.
    terminal_seen: usize,
    /// Source file of the last run (for error line/col reporting).
    last_file: Option<SourceFile>,
}

impl Default for ExecutionController {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionController {
    pub fn new() -> Self {
        let shared = Arc::new(GuiSharedState::new());
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            is_paused: Arc::new(AtomicBool::new(false)),
            is_interrupted: Arc::new(AtomicBool::new(false)),
            bridge: VmBridge::new(),
            shared,
            worker: None,
            terminal_seen: 0,
            last_file: None,
        }
    }

    pub fn running(&self) -> bool {
        match &self.worker {
            Some(w) => !w.is_finished(),
            None => false,
        }
    }

    pub fn paused(&self) -> bool {
        self.bridge.paused_line() != 0
    }

    /// 1-based line the debugger is paused on (0 = not paused).
    pub fn paused_line(&self) -> usize {
        self.bridge.paused_line()
    }

    pub fn stop(&self) {
        self.is_interrupted.store(true, Ordering::SeqCst);
        self.is_running.store(false, Ordering::SeqCst);
        self.is_paused.store(false, Ordering::SeqCst);
        self.shared.interrupt();
        // Wake a paused debugger so it observes the interrupt and aborts.
        self.bridge.post_command(DebugCommand::Stop);
    }

    /// Close-time cleanup belongs to a reaper, never the UI event loop.
    pub fn reap_async(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.interrupt();
            std::thread::spawn(move || {
                let _ = worker.join_timeout(Duration::from_secs(2));
            });
        }
    }

    /// Interrupts and joins (bounded) the current worker, if any.
    pub fn join_current(&mut self) -> Option<WorkerOutcome> {
        if let Some(mut worker) = self.worker.take() {
            let outcome = worker.join_timeout(Duration::from_secs(2));
            if outcome.is_none() {
                eprintln!("warning: the program did not stop within 2 s; abandoning it");
            }
            return outcome;
        }
        None
    }

    /// Spawns a background VM worker for `prog`, which the caller compiled
    /// from `file` (the source the debugger shows). Taking the program
    /// rather than the source guarantees the bytecode that runs is exactly
    /// the one whose diagnostics the IDE displays.
    ///
    /// `debug` installs the [`WorkerDebugger`]; `stop_at_entry` makes it
    /// pause at the first statement. `args` is a comma-separated argument
    /// list (PPL expression per argument) evaluated on the worker thread.
    #[allow(clippy::too_many_arguments)]
    pub fn start_compiled(
        &mut self,
        prog: ppl_compiler::CompiledProgram,
        file: &SourceFile,
        entry: &str,
        args: Option<&str>,
        debug: bool,
        stop_at_entry: bool,
        max_steps: Option<u64>,
    ) -> RunStart {
        if self.running() {
            return RunStart::AlreadyRunning;
        }

        self.terminal_seen = 0;
        self.shared.reset_for_run();
        self.is_interrupted.store(false, Ordering::SeqCst);
        self.bridge
            .stop_at_entry
            .store(stop_at_entry, Ordering::SeqCst);
        self.bridge.paused_line.store(0, Ordering::SeqCst);
        *lock(&self.bridge.pause_snapshot) = None;
        self.bridge.set_source(file.clone());
        self.last_file = Some(file.clone());

        let config = WorkerConfig {
            prog,
            entry: Some(entry.to_string()),
            args: args.map(str::to_string),
            settings: ppl_vm::Settings::default(),
            limits: ppl_vm::Limits {
                max_time: None,
                max_steps,
                ..ppl_vm::Limits::default()
            },
            seed: 0x5EED,
            echo: false,
            debug: if debug {
                Some(self.bridge.clone())
            } else {
                None
            },
        };

        let worker = WorkerHandle::spawn(config, self.shared.clone());
        self.worker = Some(worker);
        self.is_running.store(true, Ordering::SeqCst);
        RunStart::Started
    }

    /// New terminal output lines since the last poll.
    pub fn drain_terminal(&mut self) -> Vec<String> {
        let lines = lock(&self.shared.terminal_lines).clone();
        if lines.len() < self.terminal_seen {
            self.terminal_seen = 0;
        }
        if lines.len() > self.terminal_seen {
            let new = lines[self.terminal_seen..].to_vec();
            self.terminal_seen = lines.len();
            new
        } else {
            Vec::new()
        }
    }

    /// The worker's outcome if it has finished.
    pub fn outcome(&mut self) -> Option<WorkerOutcome> {
        self.worker.as_ref()?.outcome()
    }

    /// True when the worker exists and is still executing.
    pub fn worker_alive(&self) -> bool {
        self.running()
    }

    /// Source file of the most recent run (for runtime error navigation).
    pub fn last_source_file(&self) -> Option<&SourceFile> {
        self.last_file.as_ref()
    }
}

/// Convenience constructor used by the worker: wraps [`VmBridge`] + the
/// interrupt flag into a [`WorkerDebugger`].
pub fn make_worker_debugger(bridge: Arc<VmBridge>, interrupt: Arc<AtomicBool>) -> WorkerDebugger {
    WorkerDebugger::new(bridge, interrupt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn bridge_breakpoint_toggle() {
        let bridge = VmBridge::new();
        assert!(bridge.toggle_breakpoint(3));
        assert!(bridge.breakpoints.lock().unwrap().contains(&3));
        assert!(!bridge.toggle_breakpoint(3));
        assert!(!bridge.breakpoints.lock().unwrap().contains(&3));
    }

    #[test]
    fn controller_stop_signals_interrupt() {
        let mut c = ExecutionController::new();
        assert!(!c.running());
        assert!(!c.paused());
        c.stop();
        assert!(c.is_interrupted.load(Ordering::SeqCst));
        assert!(c.shared.is_interrupted());
        assert!(c.join_current().is_none());
    }

    /// Compiles `file` and starts its first exported function.
    fn start(c: &mut ExecutionController, file: &SourceFile) -> RunStart {
        let (prog, _) =
            ppl_compiler::compile_source(file, &ppl_compiler::CompileOptions::default());
        let prog = prog.expect("test program compiles");
        let entry = prog.pick_entry(None).expect("entry");
        c.start_compiled(prog, file, &entry, None, false, false, None)
    }

    #[test]
    fn controller_start_and_finish_run() {
        let mut c = ExecutionController::new();
        let src = "EXPORT TEST() BEGIN PRINT(30); RETURN 1; END;";
        let file = SourceFile::new("test.ppl", src);
        match start(&mut c, &file) {
            RunStart::Started => {}
            other => panic!("expected start, got {other:?}"),
        }
        assert!(c.running());
        // Wait for completion (bounded).
        for _ in 0..200 {
            if c.outcome().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let Some(WorkerOutcome::Success { value_str, .. }) = c.outcome() else {
            panic!("expected success outcome");
        };
        assert_eq!(value_str, "1");
        let lines = c.drain_terminal();
        assert_eq!(lines, vec!["30".to_string()]);
    }

    #[test]
    fn debugger_pauses_at_breakpoint_and_resumes() {
        let bridge = VmBridge::new();
        bridge.set_breakpoints(&BTreeSet::from([3usize]));
        // NOTE: the function must not collide with a PPL built-in name.
        let src = "EXPORT TRYIT()\nBEGIN\nPRINT(1);\nRETURN 2;\nEND;";
        bridge.set_source(SourceFile::new("t.ppl", src));
        let interrupt = Arc::new(AtomicBool::new(false));
        let dbg = WorkerDebugger::new(bridge.clone(), interrupt);

        let file = SourceFile::new("t.ppl", src);
        let (prog, _) =
            ppl_compiler::compile_source(&file, &ppl_compiler::CompileOptions::default());
        let prog = Rc::new(prog.expect("must compile"));
        let host = ppl_vm::HeadlessHost::new();
        let mut machine =
            Machine::new(prog, Box::new(host), Default::default(), Default::default());
        machine.set_debugger(Some(Box::new(dbg)));

        // Spawn a driver thread that lets the first pause continue.
        let bridge2 = bridge.clone();
        let driver = std::thread::spawn(move || {
            for _ in 0..200 {
                if bridge2.paused_line() != 0 {
                    bridge2.post_command(DebugCommand::Continue);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let val = machine.call_function("TRYIT", Vec::new());
        driver.join().unwrap();
        assert!(val.is_ok(), "run must complete after continue: {val:?}");
    }

    #[test]
    fn test_controller_stop_and_subsequent_start() {
        let mut c = ExecutionController::new();
        let src = "EXPORT TEST() BEGIN RETURN 42; END;";
        let file = SourceFile::new("test.ppl", src);
        assert!(matches!(start(&mut c, &file), RunStart::Started));
        c.stop();
        let _ = c.join_current();
        // Subsequent run must succeed and not be immediately aborted
        assert!(matches!(start(&mut c, &file), RunStart::Started));
        for _ in 0..200 {
            if c.outcome().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(c.outcome(), Some(WorkerOutcome::Success { .. })));
    }
}
