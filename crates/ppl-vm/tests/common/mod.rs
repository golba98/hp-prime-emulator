//! Shared helpers for tests that script INPUT / MSGBOX dialogs: a
//! [`HeadlessHost`] the test can still inspect after the machine ran.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use ppl_compiler::{CompileOptions, compile_source};
use ppl_graphics::Grob;
use ppl_syntax::SourceFile;
use ppl_vm::host::{Dialog, DialogResult, Touch};
use ppl_vm::{HeadlessHost, Host, Limits, Machine, RuntimeError, ScriptedInput, Settings, Value};

/// Delegates to a shared [`HeadlessHost`].
pub struct SharedHost(pub Rc<RefCell<HeadlessHost>>);

impl Host for SharedHost {
    fn print(&mut self, text: &str) {
        self.0.borrow_mut().print(text)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.0.borrow_mut().poll_key()
    }
    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.0.borrow_mut().is_key_down(key)
    }
    fn wait_key(&mut self, timeout_s: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.0.borrow_mut().wait_key(timeout_s)
    }
    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.0.borrow_mut().sleep(seconds)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.0.borrow_mut().touch()
    }
    fn dialog(&mut self, dialog: &Dialog, screen: &Grob) -> Result<DialogResult, RuntimeError> {
        self.0.borrow_mut().dialog(dialog, screen)
    }
    fn now_ms(&mut self) -> f64 {
        self.0.borrow_mut().now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.0.borrow_mut().local_time()
    }
}

/// Scripted dialog answers: `"<ESC>"` cancels, anything else is typed.
pub fn script(inputs: &[&str]) -> Vec<ScriptedInput> {
    inputs.iter().map(|t| ScriptedInput::parse(t)).collect()
}

/// A compiled program on a machine whose host the test keeps a handle to.
pub struct Run {
    pub machine: Machine,
    pub host: Rc<RefCell<HeadlessHost>>,
}

impl Run {
    pub fn new(name: &str, src: &str) -> Run {
        let file = SourceFile::new(name, src.to_string());
        let (prog, diags) = compile_source(&file, &CompileOptions::default());
        let errors: Vec<String> = diags.errors().map(|e| e.message.clone()).collect();
        assert!(errors.is_empty(), "compile errors in {name}: {errors:?}");
        let host = Rc::new(RefCell::new(HeadlessHost::new()));
        let machine = Machine::new(
            Rc::new(prog.expect("compiled program")),
            Box::new(SharedHost(host.clone())),
            Settings::default(),
            Limits {
                max_steps: Some(1_000_000),
                ..Limits::default()
            },
        );
        Run { machine, host }
    }

    /// Queues `inputs` and calls `func`.
    pub fn call(&mut self, func: &str, inputs: &[&str]) -> Result<Value, RuntimeError> {
        {
            let mut h = self.host.borrow_mut();
            h.inputs = script(inputs).into();
            h.dialogs.clear();
            h.output.clear();
        }
        self.machine.call_function(func, Vec::new())
    }

    /// Scripted answers the program did not consume.
    pub fn unused_inputs(&self) -> usize {
        self.host.borrow().inputs.len()
    }

    pub fn dialogs(&self) -> Vec<String> {
        self.host.borrow().dialogs.clone()
    }

    pub fn output(&self) -> Vec<String> {
        self.host.borrow().output.clone()
    }
}

/// The text a returned string holds (or the display of any other value).
pub fn text(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        other => other.display(Settings::default().format()),
    }
}
