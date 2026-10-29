//! The stack machine that executes compiled PPL.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ppl_compiler::{CompiledProgram, Const, GlobalInit, Op, Setting};
use ppl_graphics::Grob;
use ppl_math::{Complex, Real};
use ppl_syntax::Span;

use crate::builtins;
use crate::collections::{index_value, store_index};
use crate::error::{ErrorKind, ResourceKind, RtResult, RuntimeError, TraceFrame};
use crate::host::{DisplayMode, Host};
use crate::ops;
use crate::rng::Rng;
use crate::settings::{AngleMode, Settings};
use crate::value::{Matrix, Value};

/// Host-side protections. These are **not** calculator limits; they stop
/// runaway programs on the development machine.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_steps: Option<u64>,
    pub max_time: Option<Duration>,
    pub max_depth: usize,
    pub max_collection_len: usize,
    pub max_string_bytes: usize,
    pub max_output_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_steps: None,
            max_time: Some(Duration::from_secs(30)),
            max_depth: 1000,
            max_collection_len: 1_000_000,
            max_string_bytes: 16 * 1024 * 1024,
            max_output_chars: 4 * 1024 * 1024,
        }
    }
}

const INIT_FN: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
struct Frame {
    func: usize,
    ip: usize,
    base: usize,
}

#[derive(Clone, Copy, Debug)]
struct TryRec {
    frame_depth: usize,
    stack_len: usize,
    handler: usize,
}

/// What a debugger wants after a statement stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugAction {
    Continue,
    Abort,
}

/// Called before every statement when attached.
pub trait Debugger {
    fn on_statement(
        &mut self,
        machine: &Machine,
        function: &str,
        span: Span,
        depth: usize,
    ) -> DebugAction;
}

pub struct Machine {
    prog: Rc<CompiledProgram>,
    consts: Vec<Value>,
    pub(crate) globals: Vec<Value>,
    setting_slots: Vec<Option<Setting>>,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    tries: Vec<TryRec>,
    pub settings: Settings,
    pub(crate) grobs: Vec<Option<Grob>>,
    /// Calculator notes, as exposed by the `Notes` system variable
    /// (`Notes("X")` reads, `Notes("X"):=…` writes, empty value deletes).
    /// In-memory only: the store lives as long as the machine.
    pub notes: std::collections::BTreeMap<String, String>,
    /// Calculator programs, as exposed by the `Programs` system variable.
    pub programs: std::collections::BTreeMap<String, String>,
    pub terminal: Vec<String>,
    pub(crate) display: DisplayMode,
    pub(crate) host: Box<dyn Host>,
    pub limits: Limits,
    steps: u64,
    started: Instant,
    pub(crate) rng: Rng,
    debugger: Option<Box<dyn Debugger>>,
    pub(crate) screen_dirty: bool,
    last_present: Instant,
    output_chars: usize,
    interrupt: Option<Arc<AtomicBool>>,
    pub(crate) init_done: bool,
}

enum Flow {
    Next,
    Done(Value),
}

impl Machine {
    pub fn new(
        prog: Rc<CompiledProgram>,
        host: Box<dyn Host>,
        settings: Settings,
        limits: Limits,
    ) -> Machine {
        let consts = prog
            .consts
            .iter()
            .map(|c| match c {
                Const::Real(r) => Value::Real(*r),
                Const::Int(i) => Value::Int(*i),
                Const::Complex(c) => Value::Complex(*c),
                Const::Str(s) => Value::str(s.as_str()),
            })
            .collect();
        let mut grobs = vec![None; 10];
        grobs[0] = Some(Grob::screen());
        let globals = prog
            .globals
            .iter()
            .map(|g| match &g.init {
                GlobalInit::Zero | GlobalInit::Setting(_) => Value::zero(),
                GlobalInit::EmptyList => Value::list(Vec::new()),
                GlobalInit::ZeroMatrix => Value::Matrix(Rc::new(Matrix::zeros(1, 1))),
                GlobalInit::ComplexZero => Value::Complex(Complex::new(Real::ZERO, Real::ZERO)),
                GlobalInit::Grob(n) => Value::Grob(*n),
            })
            .collect();
        let setting_slots = prog
            .globals
            .iter()
            .map(|g| match g.init {
                GlobalInit::Setting(s) => Some(s),
                _ => None,
            })
            .collect();
        let interrupt = host.interrupt_flag();
        Machine {
            prog,
            consts,
            globals,
            setting_slots,
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            tries: Vec::new(),
            settings,
            grobs,
            notes: std::collections::BTreeMap::new(),
            programs: std::collections::BTreeMap::new(),
            terminal: Vec::new(),
            display: DisplayMode::Graphics,
            host,
            limits,
            steps: 0,
            started: Instant::now(),
            rng: Rng::new(0x5EED),
            debugger: None,
            screen_dirty: false,
            last_present: Instant::now(),
            output_chars: 0,
            interrupt,
            init_done: false,
        }
    }

    pub fn program(&self) -> &CompiledProgram {
        &self.prog
    }

    pub fn set_debugger(&mut self, d: Option<Box<dyn Debugger>>) {
        self.debugger = d;
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn host(&mut self) -> &mut dyn Host {
        self.host.as_mut()
    }

    pub fn into_host(self) -> Box<dyn Host> {
        self.host
    }

    /// The current contents of G0.
    pub fn screen(&self) -> &Grob {
        self.grobs[0].as_ref().expect("G0 always exists")
    }

    pub fn grob(&self, n: usize) -> Option<&Grob> {
        self.grobs.get(n).and_then(|g| g.as_ref())
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.display
    }

    // ---- inspection (debugger) ---------------------------------------------

    /// `(function, span)` for each active frame, innermost first.
    pub fn call_stack(&self) -> Vec<(String, Span)> {
        self.frames
            .iter()
            .rev()
            .map(|f| (self.fn_name(f.func).to_string(), self.frame_span(f)))
            .collect()
    }

    /// Local variable names and values of the frame `depth` levels up.
    pub fn locals(&self, depth: usize) -> Vec<(String, Value)> {
        let Some(f) = self.frames.iter().rev().nth(depth) else {
            return Vec::new();
        };
        let chunk = self.chunk(f.func);
        chunk
            .local_names
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.starts_with('<'))
            .filter_map(|(i, n)| self.stack.get(f.base + i).map(|v| (n.clone(), v.clone())))
            .collect()
    }

    /// Program-level variables and calculator variables used by the program.
    pub fn globals(&self) -> Vec<(String, Value)> {
        self.prog
            .globals
            .iter()
            .enumerate()
            .map(|(i, g)| (g.name.clone(), self.load_global(i)))
            .collect()
    }

    fn fn_name(&self, func: usize) -> &str {
        if func == INIT_FN {
            "<file variables>"
        } else {
            &self.prog.functions[func].name
        }
    }

    fn chunk(&self, func: usize) -> &ppl_compiler::Chunk {
        if func == INIT_FN {
            &self.prog.init
        } else {
            &self.prog.functions[func]
        }
    }

    fn frame_span(&self, f: &Frame) -> Span {
        let c = self.chunk(f.func);
        c.spans
            .get(f.ip.saturating_sub(1))
            .copied()
            .unwrap_or(c.span)
    }

    // ---- entry points ------------------------------------------------------

    /// Runs file-level variable initialisers once.
    pub fn run_init(&mut self) -> RtResult<()> {
        if self.init_done {
            return Ok(());
        }
        self.init_done = true;
        let result = self.execute(INIT_FN, Vec::new()).map(|_| ());
        if result.is_err() {
            self.init_done = false;
        }
        result
    }

    pub fn call_function(&mut self, name: &str, args: Vec<Value>) -> RtResult<Value> {
        let idx = self.prog.function_index(name).ok_or_else(|| {
            RuntimeError::new(ErrorKind::Undefined, format!("no function named {name}"))
        })?;
        self.run_init()?;
        let result = self.execute(idx, args);
        self.present_now();
        result
    }

    fn execute(&mut self, func: usize, args: Vec<Value>) -> RtResult<Value> {
        let chunk = self.chunk(func);
        if func != INIT_FN && args.len() != chunk.params as usize {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                format!(
                    "{} expects {} argument(s), got {}",
                    chunk.name,
                    chunk.params,
                    args.len()
                ),
            ));
        }
        let stop_depth = self.frames.len();
        self.push_frame(func, args)?;
        self.run_loop(stop_depth)
    }

    fn push_frame(&mut self, func: usize, args: Vec<Value>) -> RtResult<()> {
        if self.frames.len() >= self.limits.max_depth {
            return Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Depth),
                format!("call depth exceeded {}", self.limits.max_depth),
            ));
        }
        let base = self.stack.len();
        let locals = self.chunk(func).locals as usize;
        let argc = args.len();
        self.stack.extend(args);
        for _ in argc..locals {
            self.stack.push(Value::zero());
        }
        self.frames.push(Frame { func, ip: 0, base });
        Ok(())
    }

    fn run_loop(&mut self, stop_depth: usize) -> RtResult<Value> {
        let prog = self.prog.clone();
        loop {
            let frame = *self.frames.last().expect("active frame");
            let chunk = if frame.func == INIT_FN {
                &prog.init
            } else {
                &prog.functions[frame.func]
            };
            let op = chunk.code[frame.ip];
            self.frames.last_mut().unwrap().ip += 1;
            self.steps += 1;
            let result = if self.steps & 0x3FF == 0 {
                self.check_limits()
                    .and_then(|_| self.exec(op, &prog, frame, stop_depth))
            } else {
                self.exec(op, &prog, frame, stop_depth)
            };
            match result {
                Ok(Flow::Next) => {}
                Ok(Flow::Done(v)) => return Ok(v),
                Err(e) => {
                    if e.kind.catchable() {
                        if let Some(t) = self.tries.last().copied() {
                            if t.frame_depth > stop_depth {
                                self.tries.pop();
                                self.frames.truncate(t.frame_depth);
                                self.stack.truncate(t.stack_len);
                                self.frames.last_mut().unwrap().ip = t.handler;
                                continue;
                            }
                        }
                    }
                    let mut e = e;
                    if e.trace.is_empty() {
                        e.trace = self.frames[stop_depth..]
                            .iter()
                            .rev()
                            .map(|f| TraceFrame {
                                function: self.fn_name(f.func).to_string(),
                                span: self.frame_span(f),
                            })
                            .collect();
                    }
                    // Leave the machine reusable.
                    if let Some(f) = self.frames.get(stop_depth) {
                        self.stack.truncate(f.base);
                    }
                    self.frames.truncate(stop_depth);
                    self.tries.retain(|t| t.frame_depth <= stop_depth);
                    return Err(e);
                }
            }
        }
    }

    pub(crate) fn check_limits(&mut self) -> RtResult<()> {
        if let Some(max) = self.limits.max_steps {
            if self.steps > max {
                return Err(RuntimeError::new(
                    ErrorKind::ResourceLimit(ResourceKind::Steps),
                    format!("more than {max} instructions"),
                ));
            }
        }
        if let Some(max) = self.limits.max_time {
            if self.started.elapsed() > max {
                return Err(RuntimeError::new(
                    ErrorKind::ResourceLimit(ResourceKind::Time),
                    format!(
                        "ran longer than {:.2}s (use --no-time-limit)",
                        max.as_secs_f64()
                    ),
                ));
            }
        }
        if self
            .interrupt
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
        {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        if self.screen_dirty && self.last_present.elapsed() > Duration::from_millis(16) {
            self.present_now();
        }
        Ok(())
    }

    /// Pauses counting wall time during blocking host calls.
    pub(crate) fn blocking<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Host) -> RtResult<T>,
    ) -> RtResult<T> {
        self.present_now();
        let t0 = Instant::now();
        let r = f(self.host.as_mut());
        self.started += t0.elapsed();
        r
    }

    pub(crate) fn present_now(&mut self) {
        let terminal = if self.display == DisplayMode::Terminal {
            Some(self.terminal.as_slice())
        } else {
            None
        };
        let screen = self.grobs[0].as_ref().expect("G0");
        self.host.present(screen, terminal);
        self.screen_dirty = false;
        self.last_present = Instant::now();
    }

    pub(crate) fn print_line(&mut self, line: String) -> RtResult<()> {
        self.output_chars += line.len() + 1;
        if self.output_chars > self.limits.max_output_chars {
            return Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Output),
                "PRINT output too large",
            ));
        }
        self.display = DisplayMode::Terminal;
        self.host.print(&line);
        self.terminal.push(line);
        self.screen_dirty = true;
        Ok(())
    }

    pub(crate) fn check_len(&self, n: usize) -> RtResult<()> {
        if n > self.limits.max_collection_len {
            return Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Memory),
                format!("collection of {n} elements"),
            ));
        }
        Ok(())
    }

    fn load_global(&self, slot: usize) -> Value {
        match self.setting_slots.get(slot).copied().flatten() {
            Some(s) => {
                let st = &self.settings;
                Value::from_i64(match s {
                    Setting::HAngle => st.angle.code(),
                    Setting::HFormat => st.hformat,
                    Setting::HDigits => st.hdigits,
                    Setting::HComplex => st.complex_results as i64,
                    Setting::Base => match st.int_base {
                        ppl_math::IntBase::Bin => 0,
                        ppl_math::IntBase::Oct => 1,
                        ppl_math::IntBase::Dec => 2,
                        ppl_math::IntBase::Hex => 3,
                    },
                    Setting::Bits => st.int_bits as i64,
                    Setting::Signed => st.int_signed as i64,
                })
            }
            None => self.globals[slot].clone(),
        }
    }

    fn store_global(&mut self, slot: usize, v: Value) -> RtResult<()> {
        match self.setting_slots.get(slot).copied().flatten() {
            Some(s) => {
                let n = v.expect_i64("setting")?;
                let st = &mut self.settings;
                match s {
                    Setting::HAngle => {
                        st.angle = AngleMode::from_code(n).ok_or_else(|| {
                            RuntimeError::new(ErrorKind::BadArgValue, "HAngle must be 0, 1 or 2")
                        })?
                    }
                    Setting::HFormat => {
                        let d = st.hdigits;
                        st.set_format(n, d)
                    }
                    Setting::HDigits => {
                        let f = st.hformat;
                        st.set_format(f, n)
                    }
                    Setting::HComplex => st.complex_results = n != 0,
                    Setting::Base => {
                        st.int_base = match n {
                            0 => ppl_math::IntBase::Bin,
                            1 => ppl_math::IntBase::Oct,
                            2 => ppl_math::IntBase::Dec,
                            _ => ppl_math::IntBase::Hex,
                        }
                    }
                    Setting::Bits => st.int_bits = n.clamp(1, 64) as u8,
                    Setting::Signed => st.int_signed = n != 0,
                }
                Ok(())
            }
            None => {
                self.globals[slot] = v;
                Ok(())
            }
        }
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().expect("stack underflow")
    }

    fn pop_n(&mut self, n: usize) -> Vec<Value> {
        let at = self.stack.len() - n;
        self.stack.split_off(at)
    }

    fn exec(
        &mut self,
        op: Op,
        prog: &CompiledProgram,
        frame: Frame,
        stop_depth: usize,
    ) -> RtResult<Flow> {
        match op {
            Op::Const(k) => self.stack.push(self.consts[k as usize].clone()),
            Op::Zero => self.stack.push(Value::zero()),
            Op::LoadLocal(s) => {
                let v = self.stack[frame.base + s as usize].clone();
                self.stack.push(v);
            }
            Op::StoreLocal(s) => {
                let v = self.pop();
                self.stack[frame.base + s as usize] = v;
            }
            Op::LoadGlobal(g) => {
                let v = self.load_global(g as usize);
                self.stack.push(v);
            }
            Op::StoreGlobal(g) => {
                let v = self.pop();
                self.store_global(g as usize, v)?;
            }
            Op::Pop => {
                self.pop();
            }
            Op::Dup => {
                let v = self.stack.last().expect("dup").clone();
                self.stack.push(v);
            }
            Op::Bin(b) => {
                let r = self.pop();
                let l = self.pop();
                let v = ops::binary(b, &l, &r, &self.settings)?;
                self.check_value(&v)?;
                self.stack.push(v);
            }
            Op::Neg => {
                let v = self.pop();
                self.stack.push(ops::negate(&v)?);
            }
            Op::Not => {
                let v = self.pop();
                self.stack.push(Value::from_bool(!v.truthy()?));
            }
            Op::Sqrt => {
                let v = self.pop();
                let r = builtins::math::sqrt(self, &v)?;
                self.stack.push(r);
            }
            Op::Factorial => {
                let v = self.pop();
                let r = ops::map_numeric(&v, &mut |x| {
                    Ok(Value::Real(ops::factorial(x.expect_real("!")?)?))
                })?;
                self.stack.push(r);
            }
            Op::Square | Op::Cube => {
                let v = self.pop();
                let e = Value::from_i64(if op == Op::Square { 2 } else { 3 });
                let r = ops::binary(ppl_syntax::ast::BinOp::Pow, &v, &e, &self.settings)?;
                self.stack.push(r);
            }
            Op::Jump(t) => self.frames.last_mut().unwrap().ip = t as usize,
            Op::JumpIfFalse(t) => {
                if !self.pop().truthy()? {
                    self.frames.last_mut().unwrap().ip = t as usize;
                }
            }
            Op::Call { func, argc } => {
                let args = self.pop_n(argc as usize);
                self.push_frame(func as usize, args)?;
            }
            Op::Builtin { id, argc } => {
                let args = self.pop_n(argc as usize);
                let v = builtins::call(self, id, args)?;
                self.check_value(&v)?;
                self.stack.push(v);
            }
            Op::BuiltinRef { id, argc, refs } => {
                let args = self.pop_n(argc as usize);
                let (ret, updated) = builtins::call_ref(self, id, args)?;
                debug_assert_eq!(updated.len(), refs as usize);
                self.stack.push(ret);
                self.stack.extend(updated);
            }
            Op::Unsupported { name, argc } => {
                self.pop_n(argc as usize);
                let n = &prog.names[name as usize];
                return Err(RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!("{n} is not implemented by this emulator"),
                ));
            }
            Op::Return => {
                let v = self.pop();
                let f = self.frames.pop().expect("frame");
                self.stack.truncate(f.base);
                let depth = self.frames.len();
                self.tries.retain(|t| t.frame_depth <= depth);
                if depth == stop_depth {
                    return Ok(Flow::Done(v));
                }
                self.stack.push(v);
            }
            Op::MakeList(n) => {
                self.check_len(n as usize)?;
                let items = self.pop_n(n as usize);
                self.stack.push(Value::list(items));
            }
            Op::MakeVector(n) => {
                let items = self.pop_n(n as usize);
                for v in &items {
                    numeric_entry(v)?;
                }
                self.stack.push(Value::Matrix(Rc::new(Matrix {
                    rows: 1,
                    cols: n as usize,
                    data: items,
                    vector: true,
                })));
            }
            Op::MakeMatrix { rows, cols } => {
                let items = self.pop_n((rows * cols) as usize);
                for v in &items {
                    numeric_entry(v)?;
                }
                self.stack.push(Value::Matrix(Rc::new(Matrix {
                    rows: rows as usize,
                    cols: cols as usize,
                    data: items,
                    vector: false,
                })));
            }
            Op::MakeComplex => {
                let im = self.pop().expect_real("complex")?;
                let re = self.pop().expect_real("complex")?;
                self.stack.push(ops::norm_complex(Complex::new(re, im)));
            }
            Op::Index(n) => {
                let idx = self.pop_n(n as usize);
                let base = self.pop();
                let v = index_value(&base, &idx, self)?;
                self.stack.push(v);
            }
            Op::StoreIndexLocal { slot, n } => {
                let v = self.pop();
                let idx = self.pop_n(n as usize);
                let max = self.limits.max_collection_len;
                let target = &mut self.stack[frame.base + slot as usize];
                store_index(target, &idx, v.clone(), max)?;
                self.stack.push(v);
            }
            Op::StoreIndexGlobal { slot, n } => {
                let v = self.pop();
                let idx = self.pop_n(n as usize);
                let max = self.limits.max_collection_len;
                let target = &mut self.globals[slot as usize];
                store_index(target, &idx, v.clone(), max)?;
                self.stack.push(v);
            }
            Op::StoreNote | Op::StoreProgram => {
                let v = self.pop();
                let name_v = self.pop();
                let name = match &name_v {
                    Value::Str(s) => s.to_string(),
                    other => other.display(self.settings.format()),
                };
                let content = match &v {
                    Value::Str(s) => s.to_string(),
                    other => other.display(self.settings.format()),
                };
                let store = if op == Op::StoreNote {
                    &mut self.notes
                } else {
                    &mut self.programs
                };
                if content.is_empty() {
                    // Documented Prime behaviour: assigning the empty
                    // string deletes the object.
                    store.remove(&name);
                } else {
                    store.insert(name, content);
                }
                self.stack.push(v);
            }
            Op::TryBegin(h) => self.tries.push(TryRec {
                frame_depth: self.frames.len(),
                stack_len: self.stack.len(),
                handler: h as usize,
            }),
            Op::TryEnd => {
                self.tries.pop();
            }
            Op::ForTest { down } => {
                let limit = self.pop();
                let var = self.pop();
                let ord = ops::compare(&var, &limit)?;
                let go = if down {
                    ord != std::cmp::Ordering::Less
                } else {
                    ord != std::cmp::Ordering::Greater
                };
                self.stack.push(Value::from_bool(go));
            }
            Op::AppendLocal(slot) => {
                let v = self.pop();
                let max = self.limits.max_collection_len;
                let target = &mut self.stack[frame.base + slot as usize];
                if let Value::List(l) = target {
                    if l.len() >= max {
                        return Err(RuntimeError::new(
                            ErrorKind::ResourceLimit(ResourceKind::Memory),
                            "list too long",
                        ));
                    }
                    Rc::make_mut(l).push(v);
                }
            }
            Op::Kill => return Err(RuntimeError::kind(ErrorKind::Killed)),
            Op::Stmt => {
                if self.debugger.is_some() {
                    let mut d = self.debugger.take().unwrap();
                    let chunk = self.chunk(frame.func);
                    let span = chunk.spans[frame.ip];
                    let name = chunk.name.clone();
                    let depth = self.frames.len();
                    let action = d.on_statement(self, &name, span, depth);
                    self.debugger = Some(d);
                    if action == DebugAction::Abort {
                        return Err(RuntimeError::new(
                            ErrorKind::Interrupted,
                            "stopped by debugger",
                        ));
                    }
                }
            }
        }
        Ok(Flow::Next)
    }

    fn check_value(&self, v: &Value) -> RtResult<()> {
        match v {
            Value::Str(s) if s.len() > self.limits.max_string_bytes => Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Memory),
                format!("string of {} bytes", s.len()),
            )),
            Value::List(l) => self.check_len(l.len()),
            _ => Ok(()),
        }
    }

    /// Evaluates an expression typed as text (INPUT fields, EXPR, CLI
    /// arguments) in a fresh machine sharing this machine's settings.
    pub fn eval_text(&self, text: &str) -> RtResult<Value> {
        eval_text_with(text, &self.settings)
    }
}

fn numeric_entry(v: &Value) -> RtResult<()> {
    match v {
        Value::Real(_) | Value::Int(_) | Value::Complex(_) => Ok(()),
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("matrix elements must be numbers, got {}", other.type_name()),
        )),
    }
}

/// Compiles and evaluates a standalone expression.
pub fn eval_text_with(text: &str, settings: &Settings) -> RtResult<Value> {
    let src = format!("EXPORT __EVAL()\nBEGIN\nRETURN {text};\nEND;");
    let file = ppl_syntax::SourceFile::new("<expr>", src);
    let (prog, diags) =
        ppl_compiler::compile_source(&file, &ppl_compiler::CompileOptions::default());
    let Some(prog) = prog else {
        let msg = diags
            .errors()
            .next()
            .map(|d| d.message.clone())
            .unwrap_or_default();
        return Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            format!("cannot evaluate `{text}`: {msg}"),
        ));
    };
    let mut m = Machine::new(
        Rc::new(prog),
        Box::new(crate::host::HeadlessHost::new()),
        settings.clone(),
        Limits {
            max_steps: Some(1_000_000),
            ..Limits::default()
        },
    );
    m.call_function("__EVAL", Vec::new())
}
