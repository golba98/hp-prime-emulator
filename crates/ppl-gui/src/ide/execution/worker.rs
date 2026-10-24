//! Background worker thread for PPL execution.
//!
//! Keeps the GUI event loop 100% responsive while long-running or interactive
//! PPL programs execute. Supports clean interruption and cancellation.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ppl_compiler::CompiledProgram;
use ppl_vm::error::{ErrorKind, RuntimeError};
use ppl_vm::{Limits, Machine, Settings, Value, eval_text_with};

use super::controller::VmBridge;
use crate::host::{GuiHost, GuiSharedState};

/// Outcome of a PPL program execution.
#[derive(Clone, Debug)]
pub enum WorkerOutcome {
    Success {
        value_str: String,
        steps: u64,
        elapsed: Duration,
    },
    Error(RuntimeError),
}

/// Configuration parameters for spawning the background worker.
#[derive(Clone)]
pub struct WorkerConfig {
    pub prog: CompiledProgram,
    pub entry: Option<String>,
    pub args: Option<String>,
    pub settings: Settings,
    pub limits: Limits,
    pub seed: u64,
    /// Also write `PRINT` output to stdout.
    pub echo: bool,
    /// Shared debug bridge; when present the worker installs a debugger
    /// that pauses on breakpoints and steps (IDE debugging).
    pub debug: Option<Arc<VmBridge>>,
}

impl WorkerConfig {
    pub fn new(prog: CompiledProgram) -> Self {
        Self {
            prog,
            entry: None,
            args: None,
            settings: Settings::default(),
            limits: Limits::default(),
            seed: 0x5EED,
            echo: false,
            debug: None,
        }
    }
}

/// Called once on the worker thread after the outcome is stored (success,
/// error or interrupt), e.g. to post `GuiCustomEvent::WorkerFinished`.
pub type FinishNotifier = Box<dyn FnOnce() + Send + 'static>;

/// Handle to a running background worker.
pub struct WorkerHandle {
    thread: Option<JoinHandle<()>>,
    state: Arc<GuiSharedState>,
    outcome: Arc<Mutex<Option<WorkerOutcome>>>,
    is_done: Arc<AtomicBool>,
}

impl WorkerHandle {
    /// Spawns a background thread to run the PPL program.
    pub fn spawn(config: WorkerConfig, state: Arc<GuiSharedState>) -> Self {
        Self::spawn_with_notify(config, state, None)
    }

    /// Like [`WorkerHandle::spawn`], calling `on_finish` when the program
    /// has finished for any reason.
    pub fn spawn_with_notify(
        config: WorkerConfig,
        state: Arc<GuiSharedState>,
        on_finish: Option<FinishNotifier>,
    ) -> Self {
        let outcome = Arc::new(Mutex::new(None));
        let is_done = Arc::new(AtomicBool::new(false));

        let state_worker = state.clone();
        let outcome_worker = outcome.clone();
        let is_done_worker = is_done.clone();

        let thread = std::thread::Builder::new()
            .name("ppl-worker".into())
            .spawn(move || {
                let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_program(config, state_worker.clone())
                })) {
                    Ok(outcome) => outcome,
                    Err(payload) => {
                        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "worker thread panicked".to_string()
                        };
                        WorkerOutcome::Error(RuntimeError::new(ErrorKind::Unsupported, msg))
                    }
                };
                *outcome_worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
                is_done_worker.store(true, Ordering::Release);
                state_worker.notify_redraw();
                if let Some(cb) = on_finish {
                    cb();
                }
            })
            .expect("failed to spawn worker thread");

        Self {
            thread: Some(thread),
            state,
            outcome,
            is_done,
        }
    }

    /// Signals the worker thread to interrupt execution immediately.
    pub fn interrupt(&self) {
        self.state.interrupt();
    }

    /// Checks if the worker thread has completed execution.
    pub fn is_finished(&self) -> bool {
        self.is_done.load(Ordering::Acquire)
            || self.thread.as_ref().is_some_and(|h| h.is_finished())
    }

    /// Returns a copy of the outcome if the worker has completed.
    pub fn outcome(&self) -> Option<WorkerOutcome> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Waits for the worker thread to exit and returns the outcome.
    pub fn join(&mut self) -> Option<WorkerOutcome> {
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        self.outcome()
    }

    /// Waits at most `timeout` for the worker thread to exit. On timeout the
    /// thread is detached (so dropping the handle cannot block either) and
    /// `None` is returned.
    pub fn join_timeout(&mut self, timeout: Duration) -> Option<WorkerOutcome> {
        let deadline = Instant::now() + timeout;
        if let Some(handle) = self.thread.take() {
            while !handle.is_finished() {
                if Instant::now() >= deadline {
                    drop(handle); // detach
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let _ = handle.join();
        }
        self.outcome()
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        if !self.is_finished() {
            self.interrupt();
        }
        if let Some(handle) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !handle.is_finished() {
                if Instant::now() >= deadline {
                    drop(handle); // detach
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let _ = handle.join();
        }
    }
}

/// Runs the configured program to completion on the current thread.
fn run_program(config: WorkerConfig, state: Arc<GuiSharedState>) -> WorkerOutcome {
    let prog_rc = Rc::new(config.prog);
    let entry_name = match prog_rc.pick_entry(config.entry.as_deref()) {
        Ok(e) => e,
        Err(msg) => return WorkerOutcome::Error(RuntimeError::new(ErrorKind::Undefined, msg)),
    };
    let args = match parse_args(config.args.as_deref(), &config.settings) {
        Ok(a) => a,
        Err(msg) => return WorkerOutcome::Error(RuntimeError::new(ErrorKind::BadArgValue, msg)),
    };
    let host = GuiHost::new(state.clone()).with_echo(config.echo);
    let mut machine = Machine::new(prog_rc, Box::new(host), config.settings, config.limits);
    machine.set_seed(config.seed);
    if let Some(bridge) = &config.debug {
        machine.set_debugger(Some(Box::new(super::controller::make_worker_debugger(
            bridge.clone(),
            state.interrupt.clone(),
        ))));
    }

    let t0 = Instant::now();
    let res = machine.call_function(&entry_name, args);
    let elapsed = t0.elapsed();
    match res {
        Ok(v) => WorkerOutcome::Success {
            value_str: v.display(machine.settings.format()),
            steps: machine.steps(),
            elapsed,
        },
        Err(e) => WorkerOutcome::Error(e),
    }
}

fn parse_args(text: Option<&str>, settings: &Settings) -> Result<Vec<Value>, String> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '"' => in_str = !in_str,
            '{' | '[' | '(' if !in_str => depth += 1,
            '}' | ']' | ')' if !in_str => depth -= 1,
            ',' if depth == 0 && !in_str => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    parts.push(cur);
    parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| eval_text_with(p, settings).map_err(|e| format!("bad argument `{p}`: {e}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_compiler::{CompileOptions, compile_source};
    use ppl_syntax::SourceFile;

    #[test]
    fn test_worker_execution_success() {
        let code = "EXPORT TEST() BEGIN RETURN 42; END;";
        let file = SourceFile::new("test.ppl", code);
        let opts = CompileOptions::default();
        let (prog, _) = compile_source(&file, &opts);
        let prog = prog.expect("compiles");

        let state = Arc::new(GuiSharedState::new());
        let config = WorkerConfig::new(prog);
        let mut worker = WorkerHandle::spawn(config, state);

        let outcome = worker.join().expect("must produce outcome");
        match outcome {
            WorkerOutcome::Success { value_str, .. } => {
                assert_eq!(value_str, "42");
            }
            WorkerOutcome::Error(e) => panic!("unexpected error: {:?}", e),
        }
    }

    fn compile(code: &str) -> CompiledProgram {
        let file = SourceFile::new("t.ppl", code);
        let (prog, diags) = compile_source(&file, &CompileOptions::default());
        prog.unwrap_or_else(|| panic!("compile failed: {:?}", diags.items))
    }

    fn spawn_notified(code: &str) -> (WorkerHandle, std::sync::mpsc::Receiver<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let w = WorkerHandle::spawn_with_notify(
            WorkerConfig::new(compile(code)),
            Arc::new(GuiSharedState::new()),
            Some(Box::new(move || {
                let _ = tx.send(());
            })),
        );
        (w, rx)
    }

    #[test]
    fn test_worker_notifies_finish_on_success_error_and_interrupt() {
        let (mut ok, rx) = spawn_notified("EXPORT TST() BEGIN RETURN 1; END;");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("finish notification on success");
        assert!(ok.is_finished());
        assert!(matches!(ok.join(), Some(WorkerOutcome::Success { .. })));

        let (mut err, rx) = spawn_notified("EXPORT TST() BEGIN RETURN 1/0; END;");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("finish notification on error");
        assert!(matches!(err.join(), Some(WorkerOutcome::Error(_))));

        let (mut int, rx) = spawn_notified("EXPORT TST() BEGIN WAIT(0); END;");
        std::thread::sleep(Duration::from_millis(20));
        assert!(rx.try_recv().is_err(), "still waiting for a key");
        int.interrupt();
        rx.recv_timeout(Duration::from_secs(2))
            .expect("finish notification on interrupt");
        match int.join() {
            Some(WorkerOutcome::Error(e)) => assert_eq!(e.kind, ErrorKind::Interrupted),
            other => panic!("expected interrupt, got {other:?}"),
        }
    }

    #[test]
    fn test_join_timeout_returns_promptly_after_interrupt() {
        let (mut w, _rx) = spawn_notified("EXPORT TST() BEGIN WAIT(0); END;");
        std::thread::sleep(Duration::from_millis(10));
        w.interrupt();
        let t0 = Instant::now();
        let out = w.join_timeout(Duration::from_secs(2));
        assert!(t0.elapsed() < Duration::from_millis(500));
        assert!(matches!(out, Some(WorkerOutcome::Error(_))));
    }

    #[test]
    fn test_worker_interruption() {
        // Infinite loop
        let code = "EXPORT LOOP_TEST() BEGIN WHILE 1 DO END; END;";
        let file = SourceFile::new("loop.ppl", code);
        let opts = CompileOptions::default();
        let (prog, _) = compile_source(&file, &opts);
        let prog = prog.expect("compiles");

        let state = Arc::new(GuiSharedState::new());
        let config = WorkerConfig::new(prog);
        let mut worker = WorkerHandle::spawn(config, state);

        // Allow worker to start
        std::thread::sleep(Duration::from_millis(20));
        assert!(!worker.is_finished());

        // Interrupt it
        worker.interrupt();

        let outcome = worker.join().expect("must finish after interrupt");
        match outcome {
            WorkerOutcome::Error(e) => {
                assert_eq!(e.kind, ErrorKind::Interrupted);
            }
            WorkerOutcome::Success { .. } => panic!("expected interrupted error"),
        }
    }

    #[test]
    fn test_worker_pick_entry_single_unexported_function() {
        let code = "FOO() BEGIN RETURN 99; END;";
        let file = SourceFile::new("single.ppl", code);
        let opts = CompileOptions::default();
        let (prog, _) = compile_source(&file, &opts);
        let prog = prog.expect("compiles");
        let entry = prog
            .pick_entry(None)
            .expect("single function picked as entry");
        assert_eq!(entry, "FOO");

        let state = Arc::new(GuiSharedState::new());
        let config = WorkerConfig::new(prog);
        let mut worker = WorkerHandle::spawn(config, state);
        let out = worker.join().expect("finished");
        match out {
            WorkerOutcome::Success { value_str, .. } => assert_eq!(value_str, "99"),
            other => panic!("expected success, got {other:?}"),
        }
    }
}
