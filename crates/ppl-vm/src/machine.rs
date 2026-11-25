//! The stack machine that executes compiled PPL.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ppl_compiler::{CompiledProgram, Const, GlobalInit, Op, Setting};
use ppl_graphics::Grob;
use ppl_math::{Complex, Real};
use ppl_syntax::{Span, ast::BinOp};

use crate::builtins;
use crate::collections::{append_list, index_value, store_index};
use crate::error::{ErrorKind, ResourceKind, RtResult, RuntimeError, TraceFrame};
use crate::host::{DisplayMode, Host};
use crate::ops;
use crate::rng::Rng;
use crate::settings::{AngleMode, Settings};
use crate::value::{Matrix, Value};

mod app_variables;
mod expression;

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
    kind: TryKind,
}

#[derive(Clone, Copy, Debug)]
enum TryKind {
    IfErr,
    StddevCell,
    AppendCallback { collection_base: bool },
}

#[derive(Clone, Copy, Debug)]
enum AppendCallbackFault {
    DivisionByZero = 1,
    BadArgType = 2,
}

impl AppendCallbackFault {
    fn from_error(error: &ErrorKind) -> Option<Self> {
        match error {
            ErrorKind::DivisionByZero => Some(Self::DivisionByZero),
            ErrorKind::BadArgType => Some(Self::BadArgType),
            _ => None,
        }
    }

    fn from_status(status: i64) -> Option<Self> {
        match status {
            0 => None,
            1 => Some(Self::DivisionByZero),
            2 => Some(Self::BadArgType),
            _ => unreachable!("invalid APPEND callback status"),
        }
    }
}

impl TryKind {
    fn catches(self, error: &ErrorKind) -> bool {
        match self {
            Self::IfErr => true,
            Self::StddevCell => matches!(
                error,
                ErrorKind::DivisionByZero | ErrorKind::InvalidDimension | ErrorKind::BadArgType
            ),
            Self::AppendCallback { collection_base } => {
                collection_base && AppendCallbackFault::from_error(error).is_some()
            }
        }
    }
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
    pub cas_context: ppl_cas::CasSession,
    pub active_app: String,
    pub app_settings: std::collections::BTreeMap<String, crate::AppSettings>,
    pub app_numeric_settings: std::collections::BTreeMap<String, crate::AppNumericSettings>,
    pub app_plot_settings: std::collections::BTreeMap<String, crate::AppPlotSettings>,
    pub geometry_plot_settings: Option<crate::GeometryPlotSettings>,
    pub(crate) statistics2_columns_revision: u64,
    pub app_files: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, crate::PortableValue>,
    >,
    pub home_variables: std::collections::BTreeMap<String, Value>,
    pub(crate) legacy_type_cache: std::collections::BTreeMap<String, i64>,
    prog: Rc<CompiledProgram>,
    pub(crate) linked: Option<(crate::LinkedProgram, Vec<bool>)>,
    pub(crate) root_identity: Option<crate::SourceIdentity>,
    pub(crate) catalog_states: std::collections::BTreeMap<String, crate::session::ModuleState>,
    dynamic_dependencies: std::collections::BTreeMap<usize, Vec<usize>>,
    initializing_modules: std::collections::BTreeSet<usize>,
    consts: Vec<Value>,
    pub(crate) globals: Vec<Value>,
    setting_slots: Vec<Option<Setting>>,
    /// Temporary expression slots which refer to a suspended caller's locals.
    captured_globals: std::collections::BTreeMap<usize, usize>,
    scoped_plot_slots: std::collections::BTreeSet<usize>,
    legacy_plot_writes: std::collections::BTreeSet<usize>,
    expression_scopes: std::collections::BTreeMap<usize, expression::Scope>,
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
    /// Initialize graph/inspection workers from an immutable calculator snapshot
    /// before executing any code. Never replace a live module from inside a call.
    pub fn seed_object_stores(
        &mut self,
        notes: &std::collections::BTreeMap<String, String>,
        programs: &std::collections::BTreeMap<String, String>,
    ) -> RtResult<()> {
        if !self.frames.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                "Object stores can only be seeded before execution",
            ));
        }
        self.notes = notes.clone();
        self.programs = programs.clone();
        Ok(())
    }
    pub fn remaining_time(&self) -> Option<Duration> {
        self.limits
            .max_time
            .map(|limit| limit.saturating_sub(self.started.elapsed()))
    }

    pub fn cas_home_values(&self) -> std::collections::BTreeMap<String, ppl_cas::CasValue> {
        let mut home = self
            .home_variables
            .iter()
            .filter_map(|(name, value)| {
                crate::cas::to_cas(value).map(|value| (name.clone(), value))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        for (index, (slot, value)) in self.prog.globals.iter().zip(&self.globals).enumerate() {
            let owned = self
                .linked
                .as_ref()
                .map_or(!slot.program_var, |(image, _)| {
                    image.global_owners[index].is_none()
                });
            if owned
                && !matches!(slot.init, ppl_compiler::GlobalInit::Setting(_))
                && let Some(value) = crate::cas::to_cas(value)
            {
                home.insert(slot.name.clone(), value);
            }
        }
        home
    }
    pub fn store_home_value(&mut self, name: &str, value: Value) -> RtResult<()> {
        if self.active_app == "Geometry" && crate::GeometryPlotSettings::recognizes(name) {
            let mut next = self.geometry_plot_settings();
            next.set(name, &value)?;
            self.geometry_plot_settings = Some(next);
            return Ok(());
        }
        if crate::AppPlotSettings::recognizes(&self.active_app, name)
            && crate::AppPlotSettings::for_app(&self.active_app).is_some()
        {
            crate::AppPlotSettings::check_field(&self.active_app, name)?;
            let mut next = self.plot_settings(&self.active_app)?;
            next.set_for(&self.active_app, name, &value)?;
            self.app_plot_settings.insert(self.active_app.clone(), next);
            return Ok(());
        }
        if let Some(setting) = crate::AppNumericSettings::setting(name) {
            let mut next = self.numeric_settings();
            next.set(setting, &value)?;
            self.app_numeric_settings
                .insert(self.active_app.clone(), next);
            return Ok(());
        }
        let value = crate::statistics_columns::coerce(name, value)?;
        if let Some(slot) = self
            .prog
            .globals
            .iter()
            .enumerate()
            .find_map(|(index, slot)| {
                let owned = self
                    .linked
                    .as_ref()
                    .map_or(!slot.program_var, |(image, _)| {
                        image.global_owners[index].is_none()
                    });
                (slot.name == name && owned).then_some(index)
            })
        {
            self.store_global(slot, value.clone())?;
        } else if crate::statistics_columns::is_two_variable_column(name) {
            self.statistics2_columns_revision = self.statistics2_columns_revision.wrapping_add(1);
        }
        self.home_variables.insert(name.into(), value);
        Ok(())
    }
    /// Immutable calculator-owned values for a presentation requested during a run.
    pub fn presentation_snapshot(&self) -> crate::SessionSnapshot {
        let mut globals = self
            .home_variables
            .iter()
            .map(|(name, value)| (name.clone(), crate::PortableValue::from(value)))
            .collect::<std::collections::BTreeMap<_, _>>();
        for (i, slot) in self.prog.globals.iter().enumerate() {
            let shared = self
                .linked
                .as_ref()
                .map_or(!slot.program_var, |(image, _)| {
                    image.global_owners[i].is_none()
                });
            if shared && !self.is_plot_slot(i) {
                globals.insert(
                    slot.name.clone(),
                    crate::PortableValue::from(&self.load_global(i)),
                );
            }
        }
        crate::SessionSnapshot {
            active_app: self.active_app.clone(),
            app_settings: self.app_settings.clone(),
            app_numeric_settings: self.app_numeric_settings.clone(),
            app_plot_settings: self.app_plot_settings.clone(),
            geometry_plot_settings: self.geometry_plot_settings.clone(),
            statistics2_columns_revision: self.statistics2_columns_revision,
            app_files: self.app_files.clone(),
            cas: self.cas_context.clone(),
            settings: self.settings.clone(),
            globals,
            generation: 0,
            last_value: None,
            notes: self.notes.clone(),
            note_bullets: Default::default(),
            programs: self.programs.clone(),
        }
    }
    pub fn calculator_global(&self, name: &str) -> Option<&Value> {
        self.prog
            .globals
            .iter()
            .enumerate()
            .find_map(|(index, slot)| {
                let home = self
                    .linked
                    .as_ref()
                    .map_or(!slot.program_var, |(image, _)| {
                        image.global_owners[index].is_none()
                    });
                (home && slot.name == name && !self.captured_globals.contains_key(&index))
                    .then_some(&self.globals[index])
            })
            .or_else(|| self.home_variables.get(name))
    }
    pub fn new(
        prog: Rc<CompiledProgram>,
        host: Box<dyn Host>,
        settings: Settings,
        limits: Limits,
    ) -> Machine {
        let consts = prog
            .consts
            .iter()
            .map(|value| expression::constant_value(value, settings.numeric_version))
            .collect();
        let mut grobs = vec![None; 10];
        grobs[0] = Some(Grob::screen());
        let globals = prog
            .globals
            .iter()
            .map(|g| expression::initial_value(&g.init))
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
            cas_context: Default::default(),
            active_app: "Function".into(),
            app_settings: Default::default(),
            app_numeric_settings: Default::default(),
            app_plot_settings: Default::default(),
            geometry_plot_settings: None,
            statistics2_columns_revision: 0,
            app_files: Default::default(),
            home_variables: Default::default(),
            legacy_type_cache: Default::default(),
            prog,
            linked: None,
            initializing_modules: Default::default(),
            consts,
            globals,
            setting_slots,
            captured_globals: Default::default(),
            scoped_plot_slots: Default::default(),
            legacy_plot_writes: Default::default(),
            expression_scopes: Default::default(),
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
            root_identity: None,
            catalog_states: Default::default(),
            dynamic_dependencies: Default::default(),
        }
    }

    pub fn new_linked(
        image: crate::LinkedProgram,
        host: Box<dyn Host>,
        settings: Settings,
        limits: Limits,
    ) -> Self {
        let states = vec![false; image.modules.len()];
        let mut machine = Self::new(image.program.clone(), host, settings, limits);
        machine.linked = Some((image, states));
        machine
    }
    /// Install reachable dependencies in deterministic catalog order before
    /// the root's file initializers. Already initialized units are not repeated.
    pub fn initialize_dependencies(&mut self) -> RtResult<()> {
        let Some((image, _)) = &self.linked else {
            return Ok(());
        };
        let root = image.root;
        let count = image.modules.len();
        for module in 0..count {
            if module != root {
                self.initialize_module(module)?;
            }
        }
        Ok(())
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
        self.call_stack_frames()
            .into_iter()
            .map(|frame| (frame.function, frame.span))
            .collect()
    }

    /// Original source/revision and span for every frame, innermost first.
    /// Single-source machines have no linked module identity.
    pub fn call_stack_frames(&self) -> Vec<TraceFrame> {
        self.frames
            .iter()
            .rev()
            .map(|frame| TraceFrame {
                module: self.frame_source(frame).cloned(),
                function: self.fn_name(frame.func).to_owned(),
                span: self.frame_span(frame),
            })
            .collect()
    }

    fn frame_source(&self, frame: &Frame) -> Option<&crate::SourceIdentity> {
        if let Some(scope) = self.expression_scopes.get(&frame.func) {
            return Some(&scope.source);
        }
        let (image, _) = self.linked.as_ref()?;
        image
            .function_owners
            .get(frame.func)
            .map(|&m| &image.modules[m])
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
        self.globals_for_frame(0)
    }

    /// Shared calculator slots and the selected frame's own file variables.
    pub fn globals_for_frame(&self, depth: usize) -> Vec<(String, Value)> {
        if depth > 0 && depth >= self.frames.len() {
            return Vec::new();
        }
        if let Some((image, _)) = &self.linked {
            let owner = self
                .frames
                .iter()
                .rev()
                .nth(depth)
                .and_then(|f| image.function_owners.get(f.func))
                .copied()
                .unwrap_or(image.root);
            let mut visible = std::collections::BTreeMap::new();
            for private in [false, true] {
                for (i, slot) in self.prog.globals.iter().enumerate() {
                    if if private {
                        image.global_owners[i] == Some(owner)
                    } else {
                        image.global_owners[i].is_none() || slot.exported
                    } {
                        visible.insert(slot.name.clone(), self.load_global(i));
                    }
                }
            }
            return visible.into_iter().collect();
        }
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
        if let Some((image, _)) = &self.linked {
            return self.initialize_module(image.root);
        }
        if self.init_done {
            return Ok(());
        }
        self.init_done = true;
        let result = self.execute(INIT_FN, Vec::new()).map(|_| ());
        if result.is_err() {
            self.init_done = false;
        }
        // A file initializer can itself use EXPR and upgrade this formerly
        // unlinked machine. Its real success/failure still owns the root flag.
        if let Some((image, states)) = &mut self.linked {
            states[image.root] = self.init_done;
        }
        result
    }

    fn initialize_module(&mut self, module: usize) -> RtResult<()> {
        let (func, initialized) = {
            let (image, states) = self.linked.as_ref().expect("linked call");
            (image.initializers[module], states[module])
        };
        if initialized {
            return Ok(());
        }
        self.linked.as_mut().unwrap().1[module] = true;
        self.initializing_modules.insert(module);
        let result = (|| {
            if let Some(dependencies) = self.dynamic_dependencies.get(&module).cloned() {
                for dependency in dependencies {
                    if dependency != module {
                        self.initialize_module(dependency)?;
                    }
                }
            }
            self.execute(func, vec![]).map(|_| ())
        })();
        self.initializing_modules.remove(&module);
        if result.is_err() {
            self.linked.as_mut().unwrap().1[module] = false;
        }
        result
    }
    pub fn current_source(&self) -> Option<&crate::SourceIdentity> {
        self.frame_source(self.frames.last()?)
    }
    pub fn arithmetic_settings(&self) -> Settings {
        self.app_settings
            .get(&self.active_app)
            .cloned()
            .unwrap_or_default()
            .arithmetic(&self.settings)
    }
    fn binary_value(&mut self, op: BinOp, left: &Value, right: &Value) -> RtResult<Value> {
        if matches!(left, Value::Symbolic(_)) || matches!(right, Value::Symbolic(_)) {
            crate::cas::binary(self, op, left, right)
        } else {
            ops::binary(op, left, right, &self.arithmetic_settings())
        }
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
        // Both original G1 builds saturate runtime real arithmetic. Parsing
        // and standalone math validation remain strict outside this scope.
        ppl_math::with_overflow_policy(ppl_math::OverflowPolicy::Saturate, || {
            self.execute_in_numeric_context(func, args)
        })
    }
    fn execute_in_numeric_context(&mut self, func: usize, args: Vec<Value>) -> RtResult<Value> {
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
        let argc = args.len();
        self.stack.extend(args);
        self.push_frame(func, argc)?;
        self.run_loop(stop_depth)
    }

    /// Evaluated arguments already occupy the first slots of the new frame.
    fn push_frame(&mut self, func: usize, argc: usize) -> RtResult<()> {
        let base = self.stack.len() - argc;
        if self.frames.len() >= self.limits.max_depth {
            self.stack.truncate(base);
            return Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Depth),
                format!("call depth exceeded {}", self.limits.max_depth),
            ));
        }
        let locals = self.chunk(func).locals as usize;
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
                    if e.kind.catchable()
                        && let Some(index) = self.tries.iter().rposition(|handler| {
                            handler.frame_depth > stop_depth && handler.kind.catches(&e.kind)
                        })
                    {
                        let handler = self.tries[index];
                        self.tries.truncate(index);
                        self.frames.truncate(handler.frame_depth);
                        self.stack.truncate(handler.stack_len);
                        if let TryKind::AppendCallback { .. } = handler.kind {
                            let fault = AppendCallbackFault::from_error(&e.kind)
                                .expect("APPEND handler catches supported callback errors");
                            self.stack.push(Value::zero());
                            self.stack.push(Value::from_i64(fault as i64));
                        }
                        self.frames.last_mut().unwrap().ip = handler.handler;
                        continue;
                    }
                    let mut e = e;
                    if e.trace.is_empty() {
                        e.trace = self.frames[stop_depth..]
                            .iter()
                            .rev()
                            .map(|f| TraceFrame {
                                function: self.fn_name(f.func).to_string(),
                                span: self.frame_span(f),
                                module: self.frame_source(f).cloned(),
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
        if self.screen_dirty {
            self.host.output_requested();
        }
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

    fn prepare_global(&mut self, slot: usize) -> RtResult<()> {
        if self.is_plot_slot(slot) {
            self.scoped_plot_slots.insert(slot);
        }
        if self.is_geometry_plot_slot(slot) {
            self.geometry_plot_settings()
                .get(&self.prog.globals[slot].name)?;
        } else if self.is_plot_slot(slot) {
            crate::AppPlotSettings::check_field(&self.active_app, &self.prog.globals[slot].name)?;
            self.plot_settings(&self.active_app)?;
        }
        if self.prog.globals[slot].external {
            return Err(RuntimeError::new(
                ErrorKind::Undefined,
                format!(
                    "external variable {} requires a linked module catalog",
                    self.prog.globals[slot].name
                ),
            ));
        }
        if self.prog.globals[slot].exported
            && let Some((image, _)) = &self.linked
        {
            let owner = image.global_owners[slot].expect("exported file variable owner");
            let current = self
                .frames
                .last()
                .and_then(|frame| image.function_owners.get(frame.func))
                .copied();
            if self.initializing_modules.contains(&owner) && current != Some(owner) {
                return Err(RuntimeError::new(
                    ErrorKind::Unsupported,
                    "cross-module variable initializer cycle has unverified firmware semantics; execute this catalog in firmware mode",
                ));
            }
            self.initialize_module(owner)?;
        }
        Ok(())
    }

    pub fn geometry_plot_settings(&self) -> crate::GeometryPlotSettings {
        self.geometry_plot_settings.clone().unwrap_or_default()
    }
    fn is_geometry_plot_slot(&self, slot: usize) -> bool {
        self.active_app == "Geometry"
            && self.is_home_slot(slot)
            && crate::GeometryPlotSettings::recognizes(&self.prog.globals[slot].name)
    }
    pub(crate) fn retain_home_slot(&self, slot: usize) -> bool {
        self.legacy_plot_writes.contains(&slot)
            || (!self.scoped_plot_slots.contains(&slot) && !self.is_plot_slot(slot))
    }
    fn is_home_slot(&self, slot: usize) -> bool {
        !self.captured_globals.contains_key(&slot)
            && self
                .linked
                .as_ref()
                .map_or(!self.prog.globals[slot].program_var, |(image, _)| {
                    image.global_owners[slot].is_none()
                })
    }
    pub fn plot_settings(&self, app: &str) -> RtResult<crate::AppPlotSettings> {
        self.app_plot_settings
            .get(app)
            .cloned()
            .or_else(|| crate::AppPlotSettings::for_app(app))
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!("Plot Setup defaults are not established for {app}"),
                )
            })
    }
    fn is_plot_slot(&self, slot: usize) -> bool {
        if self.captured_globals.contains_key(&slot) {
            return false;
        }
        let global = &self.prog.globals[slot];
        let home = self
            .linked
            .as_ref()
            .map_or(!global.program_var, |(image, _)| {
                image.global_owners[slot].is_none()
            });
        self.is_geometry_plot_slot(slot)
            || home
                && crate::AppPlotSettings::recognizes(&self.active_app, &global.name)
                && crate::AppPlotSettings::for_app(&self.active_app).is_some()
    }
    fn numeric_settings(&self) -> crate::AppNumericSettings {
        self.app_numeric_settings
            .get(&self.active_app)
            .cloned()
            .unwrap_or_else(|| crate::AppNumericSettings::for_app(&self.active_app))
    }
    fn load_global(&self, slot: usize) -> Value {
        if let Some(&local) = self.captured_globals.get(&slot) {
            return self.stack[local].clone();
        }
        if self.is_geometry_plot_slot(slot) {
            return self
                .geometry_plot_settings()
                .get(&self.prog.globals[slot].name)
                .unwrap_or_else(|_| Value::zero());
        }
        if self.is_plot_slot(slot)
            && let Ok(settings) = self.plot_settings(&self.active_app)
        {
            return settings.get(&self.prog.globals[slot].name).unwrap();
        }
        match self.setting_slots.get(slot).copied().flatten() {
            Some(
                s @ (Setting::NumStart
                | Setting::NumStep
                | Setting::NumZoom
                | Setting::NumType
                | Setting::NumIndep),
            ) => self.numeric_settings().get(s),
            Some(s) => {
                let st = &self.settings;
                let app = self
                    .app_settings
                    .get(&self.active_app)
                    .cloned()
                    .unwrap_or_default();
                Value::from_i64(match s {
                    Setting::NumStart
                    | Setting::NumStep
                    | Setting::NumZoom
                    | Setting::NumType
                    | Setting::NumIndep => {
                        unreachable!()
                    }
                    Setting::AAngle => app.angle,
                    Setting::AFormat => app.format,
                    Setting::ADigits => app.digits,
                    Setting::AComplex => app.complex,
                    Setting::HAngle => st.angle.code(),
                    Setting::HFormat => st.hformat,
                    Setting::HSeparator => st.digit_grouping.code(),
                    Setting::HDigits => st.hdigits,
                    Setting::HComplex => st.complex_code(),
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
        if let Some(&local) = self.captured_globals.get(&slot) {
            self.stack[local] = v;
            return Ok(());
        }
        if let Some(
            s @ (Setting::NumStart
            | Setting::NumStep
            | Setting::NumZoom
            | Setting::NumType
            | Setting::NumIndep),
        ) = self.setting_slots.get(slot).copied().flatten()
        {
            let mut next = self.numeric_settings();
            next.set(s, &v)?;
            self.app_numeric_settings
                .insert(self.active_app.clone(), next);
            return Ok(());
        }
        if self.is_geometry_plot_slot(slot) {
            let mut next = self.geometry_plot_settings();
            next.set(&self.prog.globals[slot].name, &v)?;
            self.geometry_plot_settings = Some(next);
            return Ok(());
        }
        if self.is_plot_slot(slot) {
            crate::AppPlotSettings::check_field(&self.active_app, &self.prog.globals[slot].name)?;
            let mut next = self.plot_settings(&self.active_app)?;
            next.set_for(&self.active_app, &self.prog.globals[slot].name, &v)?;
            self.app_plot_settings.insert(self.active_app.clone(), next);
            return Ok(());
        }
        let name = &self.prog.globals[slot].name;
        let home_owned = self
            .linked
            .as_ref()
            .map_or(!self.prog.globals[slot].program_var, |(image, _)| {
                image.global_owners[slot].is_none()
            });
        let v = if home_owned {
            crate::statistics_columns::coerce(name, v)?
        } else {
            v
        };
        if home_owned
            && (crate::AppPlotSettings::canonical_name(name).is_some()
                || crate::GeometryPlotSettings::recognizes(name))
        {
            self.legacy_plot_writes.insert(slot);
        }
        if home_owned && crate::statistics_columns::is_two_variable_column(name) {
            self.statistics2_columns_revision = self.statistics2_columns_revision.wrapping_add(1);
        }
        if home_owned && self.home_variables.contains_key(name) {
            self.home_variables.insert(name.clone(), v.clone());
        }
        match self.setting_slots.get(slot).copied().flatten() {
            Some(s) => {
                let n = v.expect_i64("setting")?;
                let st = &mut self.settings;
                match s {
                    Setting::NumStart
                    | Setting::NumStep
                    | Setting::NumZoom
                    | Setting::NumType
                    | Setting::NumIndep => {
                        unreachable!()
                    }
                    Setting::AAngle | Setting::AFormat | Setting::ADigits | Setting::AComplex => {
                        let name = match s {
                            Setting::AAngle => "aangle",
                            Setting::AFormat => "aformat",
                            Setting::ADigits => "adigits",
                            Setting::AComplex => "acomplex",
                            _ => unreachable!(),
                        };
                        let mut app = self
                            .app_settings
                            .get(&self.active_app)
                            .cloned()
                            .unwrap_or_default();
                        app.set_code(name, n)?;
                        self.app_settings.insert(self.active_app.clone(), app);
                    }
                    Setting::HAngle => {
                        st.angle = AngleMode::from_code(n).ok_or_else(|| {
                            RuntimeError::new(ErrorKind::BadArgValue, "HAngle must be 0, 1 or 2")
                        })?
                    }
                    Setting::HFormat => {
                        let d = st.hdigits;
                        st.try_set_format(n, d)?
                    }
                    Setting::HDigits => {
                        let f = st.hformat;
                        st.try_set_format(f, n)?
                    }
                    Setting::HComplex => st.set_complex_code(n)?,
                    Setting::HSeparator => st.set_separator(n)?,
                    Setting::Base => st.set_integer_base(n)?,
                    Setting::Bits => st.set_integer_bits(n)?,
                    Setting::Signed => st.set_integer_signed(n != 0)?,
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
                self.prepare_global(g as usize)?;
                let v = self.load_global(g as usize);
                self.stack.push(v);
            }
            Op::StoreGlobal(g) => {
                self.prepare_global(g as usize)?;
                let v = self.pop();
                self.store_global(g as usize, v)?;
            }
            Op::GeometryAssignmentResult(slot) => {
                if self.is_geometry_plot_slot(slot as usize) {
                    self.pop();
                    let value = self.load_global(slot as usize);
                    self.stack.push(value);
                }
            }
            Op::Pop => {
                self.pop();
            }
            Op::AppVariable { name, argc, store } => {
                let qualified = &prog.names[name as usize];
                // An unqualified app name follows the live caller app. A
                // qualified owner changes the addressed model, not the caller's
                // active app/settings carried by the presentation snapshot.
                let (app, name) = qualified
                    .split_once('.')
                    .unwrap_or((&self.active_app, qualified));
                let (app, name) = (app.to_owned(), name.to_owned());
                let value = store.then(|| crate::PortableValue::from(&self.pop()));
                let indices: Vec<_> = self
                    .pop_n(argc as usize)
                    .iter()
                    .map(crate::PortableValue::from)
                    .collect();
                if app == "Geometry" && crate::GeometryPlotSettings::recognizes(&name) {
                    if !indices.is_empty() {
                        return Err(RuntimeError::new(
                            ErrorKind::InvalidInput,
                            "Geometry Plot Setup values are scalar",
                        ));
                    }
                    let mut settings = self.geometry_plot_settings();
                    if let Some(value) = value {
                        settings.set(&name, &value.to_value())?;
                        self.geometry_plot_settings = Some(settings.clone());
                    }
                    self.stack.push(settings.get(&name)?);
                    return Ok(Flow::Next);
                }
                if crate::AppPlotSettings::recognizes(&app, &name)
                    && crate::AppPlotSettings::for_app(&app).is_some()
                {
                    crate::AppPlotSettings::check_field(&app, &name)?;
                    if !indices.is_empty() {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgType,
                            "Plot Setup variables are scalar",
                        ));
                    }
                    let mut settings = self.plot_settings(&app)?;
                    if let Some(value) = value {
                        settings.set_for(&app, &name, &value.to_value())?;
                        self.app_plot_settings.insert(app, settings.clone());
                    }
                    self.stack.push(settings.get(&name).unwrap());
                    return Ok(Flow::Next);
                }
                if app == "Graph 3D" && crate::AppNumericSettings::graph3d_field(&name) {
                    if !indices.is_empty() {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgType,
                            "Graph 3D Numeric Setup requires an unindexed field",
                        ));
                    }
                    let mut settings = self
                        .app_numeric_settings
                        .get(&app)
                        .cloned()
                        .unwrap_or_else(|| crate::AppNumericSettings::for_app(&app));
                    if let Some(value) = value {
                        settings.set_graph3d(&name, &value.to_value())?;
                        self.app_numeric_settings.insert(app, settings.clone());
                    }
                    self.stack.push(settings.get_graph3d(&name)?);
                    return Ok(Flow::Next);
                }
                if app == "Statistics 1Var"
                    && name.starts_with('D')
                    && crate::statistics_columns::is_column(&name)
                {
                    let result = self.statistics_column(&name, &indices, value)?;
                    self.stack.push(result);
                    return Ok(Flow::Next);
                }
                let request = crate::host::AppVariableRequest {
                    app,
                    name,
                    indices,
                    value,
                };
                let context = if request.app == "Geometry" && request.name == "__Expression" {
                    self.app_expression_snapshot()
                } else {
                    self.presentation_snapshot()
                };
                let result = self.host.app_variable_with_context(request, context)?;
                self.stack.push(result.to_value());
            }
            Op::CasVariable { name, store } => {
                let name = &prog.names[name as usize];
                let source = if store {
                    format!("{name}:={}", crate::cas::expression(&self.pop())?)
                } else {
                    name.clone()
                };
                let answer = self.cas_context.last_value.clone();
                let result = crate::cas::evaluate_namespace(self, &source);
                self.cas_context.last_value = answer;
                let value = result?;
                if !store {
                    self.stack.push(value);
                }
            }
            Op::Dup => {
                let v = self.stack.last().expect("dup").clone();
                self.stack.push(v);
            }
            Op::DupFrom(depth) => {
                let index = self.stack.len() - 1 - depth as usize;
                self.stack.push(self.stack[index].clone());
            }
            Op::Bin(b) | Op::StddevCellBin { op: b, .. } => {
                let r = self.pop();
                let l = self.pop();
                let encoded = match op {
                    Op::StddevCellBin {
                        literal_zero_quotient,
                        ..
                    } => {
                        builtins::stddev_exact::inline_arithmetic(b, &l, &r, literal_zero_quotient)
                    }
                    _ => None,
                };
                let v = if let Some(result) = encoded {
                    result?
                } else {
                    self.binary_value(b, &l, &r)?
                };
                self.check_value(&v)?;
                self.stack.push(v);
            }
            Op::Neg | Op::StddevCellNeg => {
                let v = self.pop();
                let value = if matches!(op, Op::StddevCellNeg)
                    && builtins::stddev_exact::is_inline_undefined(&v)
                {
                    v
                } else if matches!(v, Value::Symbolic(_)) {
                    match ops::infinity::negate(&v) {
                        Some(value) => value,
                        None => crate::cas::evaluate(
                            self,
                            &format!("-({})", crate::cas::expression(&v)?),
                        )?,
                    }
                } else {
                    ops::negate(&v)?
                };
                self.stack.push(value);
            }
            Op::Not => {
                let v = self.pop();
                self.stack.push(match v {
                    Value::Int(word) => Value::Int(word.not()),
                    other => Value::from_bool(!other.truthy()?),
                });
            }
            Op::Sqrt => {
                let v = self.pop();
                let r = if matches!(v, Value::Symbolic(_)) {
                    match ops::infinity::sqrt(&v) {
                        Some(result) => result?,
                        None => crate::cas::evaluate(
                            self,
                            &format!("sqrt({})", crate::cas::expression(&v)?),
                        )?,
                    }
                } else {
                    builtins::math::sqrt(self, &v)?
                };
                self.stack.push(r);
            }
            Op::Sexagesimal => {
                let value = self.pop().expect_real("hexagesimal literal")?;
                self.stack.push(Value::Real(
                    value.with_presentation(ppl_math::RealPresentation::Sexagesimal),
                ));
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
                let operation = ppl_syntax::ast::BinOp::Pow;
                let r = match ops::infinity::binary(operation, &v, &e) {
                    Some(result) => result?,
                    None => ops::binary(operation, &v, &e, &self.arithmetic_settings())?,
                };
                self.stack.push(r);
            }
            Op::Jump(t) => self.frames.last_mut().unwrap().ip = t as usize,
            Op::JumpIfFalse(t) => {
                if !self.pop().truthy()? {
                    self.frames.last_mut().unwrap().ip = t as usize;
                }
            }
            Op::LogicJump { or, target } => {
                let left = self.stack.last().expect("compiled logic operand");
                let decided = match left {
                    Value::Int(word) => {
                        if or {
                            word.not().numeric_value() == 0
                        } else {
                            word.numeric_value() == 0
                        }
                    }
                    other => other.truthy()? == or,
                };
                if decided {
                    if !matches!(left, Value::Int(_)) {
                        *self.stack.last_mut().unwrap() = Value::from_bool(or);
                    }
                    self.frames.last_mut().unwrap().ip = target as usize;
                }
            }
            Op::Call { func, argc } => {
                self.push_frame(func as usize, argc as usize)?;
            }
            Op::CallExternal { name, .. } => {
                return Err(RuntimeError::new(
                    ErrorKind::Undefined,
                    format!(
                        "external {} requires a linked module catalog",
                        prog.names[name as usize]
                    ),
                ));
            }
            Op::CallLinked { func, argc } => {
                let args = self.pop_n(argc as usize);
                if self.chunk(func as usize).params as usize != args.len() {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        "external argument count changed",
                    ));
                }
                let module = self
                    .linked
                    .as_ref()
                    .expect("linked image")
                    .0
                    .function_owners[func as usize];
                self.initialize_module(module)?;
                self.stack.extend(args);
                self.push_frame(func as usize, argc as usize)?;
            }
            Op::CasCompose { argc } => {
                let marker = self.pop().expect_str("CAS template marker")?;
                let source = self.pop().expect_str("CAS template")?;
                let values = self.pop_n(argc as usize);
                let expression = crate::cas::compose(&source, &marker, &values)?;
                self.stack
                    .push(Value::Symbolic(Rc::new(ppl_cas::CasValue::Symbolic(
                        expression,
                    ))));
            }
            Op::CasCall { assignment, source } => {
                let argument = self.pop();
                let value = if assignment {
                    // G1 exposes this as an ordinary string and leaves both
                    // environments unchanged; IFERR must not intercept it.
                    Value::str(RuntimeError::kind(ErrorKind::BadArgType).to_string())
                } else if source {
                    let source = argument.expect_str("CAS source")?;
                    crate::cas::evaluate_ppl_cas(self, &source)?
                } else {
                    builtins::call(
                        self,
                        ppl_compiler::builtins::Builtin::CasCall,
                        vec![argument],
                    )?
                };
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::Builtin { id, argc } => {
                let args = self.pop_n(argc as usize);
                let v = builtins::call(self, id, args)?;
                self.check_value(&v)?;
                self.stack.push(v);
            }
            Op::AppendCollection {
                literal_payload,
                callback_payload,
                encoded_base,
            } => {
                let (item, fault) = self.pop_append_operand(callback_payload)?;
                let base = self.pop();
                let value = if let Some(fault) = fault {
                    self.append_callback_fault(base, encoded_base, fault)?
                } else {
                    builtins::lists::append_collection(
                        self,
                        &[base, item],
                        literal_payload,
                        encoded_base,
                    )?
                };
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::AppendQuotient {
                literal_operands,
                callback_operands: [numerator_call, denominator_call],
                encoded_base,
            } => {
                let (denominator, denominator_fault) = self.pop_append_operand(denominator_call)?;
                let (numerator, numerator_fault) = self.pop_append_operand(numerator_call)?;
                let base = self.pop();
                let result = if let Some(fault) = numerator_fault.or(denominator_fault) {
                    self.append_callback_fault(base, encoded_base, fault)?
                } else if matches!(base, Value::List(_) | Value::Matrix(_))
                    && let (Value::Real(numerator), Value::Real(denominator)) =
                        (&numerator, &denominator)
                    && numerator.is_integer()
                    && denominator.is_zero()
                {
                    builtins::lists::append_nonfinite_quotient(
                        self,
                        base,
                        literal_operands || numerator.is_zero(),
                    )?
                } else {
                    let item = self.append_quotient_value(
                        matches!(base, Value::List(_) | Value::Matrix(_)),
                        &numerator,
                        &denominator,
                    )?;
                    self.check_value(&item)?;
                    builtins::lists::append_collection(self, &[base, item], false, encoded_base)?
                };
                self.check_value(&result)?;
                self.stack.push(result);
            }
            Op::AppendCallbackArgumentQuotient { preceding } => {
                let denominator = self.pop();
                let numerator = self.pop();
                let base = &self.stack[self.stack.len() - 1 - usize::from(preceding)];
                let value = self.append_quotient_value(
                    matches!(base, Value::List(_) | Value::Matrix(_)),
                    &numerator,
                    &denominator,
                )?;
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::StddevExact {
                argc,
                encoded_vector,
                evaluated_matrices,
                row_mismatch_diagnostic,
                list_sources,
            } => {
                let diagnostic = row_mismatch_diagnostic.then(|| self.pop());
                let args = self.pop_n(argc as usize);
                let value = if let Some(diagnostic) = diagnostic
                    && matches!(&args[..], [Value::Matrix(data), Value::Matrix(weights)]
                        if !data.vector && !weights.vector && data.rows != weights.rows
                            && data.rows != 0 && weights.rows != 0
                            && data.cols != 0 && weights.cols != 0)
                {
                    diagnostic
                } else if let Some(value) = list_sources.and_then(|index| {
                    let ppl_compiler::bytecode::StddevMetadata::ListSources(sources) =
                        self.chunk(frame.func).stddev_metadata(index)
                    else {
                        unreachable!("expected STDDEV list sources");
                    };
                    stddev_list_length_diagnostic(&args, sources.each_ref().map(Option::as_deref))
                }) {
                    value
                } else if let Some(value) =
                    builtins::stddev_exact::ordinary_complex_deviation(self, &args)?
                {
                    value
                } else {
                    match builtins::stddev_exact::deviation(
                        self,
                        &args,
                        encoded_vector,
                        evaluated_matrices,
                    )? {
                        Some(value) => value,
                        None => {
                            builtins::call(self, ppl_compiler::builtins::Builtin::Stddev, args)?
                        }
                    }
                };
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::StddevEncodedComplexMatrix => {
                let args = self.pop_n(2);
                let value = builtins::stddev_exact::encoded_complex_matrix(self, args)?;
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::StddevInlineCollection {
                shapes,
                encoded_weights,
            } => {
                let &ppl_compiler::bytecode::StddevMetadata::InlineShapes(shapes) =
                    self.chunk(frame.func).stddev_metadata(shapes)
                else {
                    unreachable!("expected STDDEV collection shapes");
                };
                let diagnostic = self.pop();
                let failed = self.pop().truthy()?;
                let args = self.pop_n(2);
                let value = if failed || !encoded_weights {
                    diagnostic
                } else {
                    let [Value::List(data_cells), Value::List(weight_cells)] = args.as_slice()
                    else {
                        unreachable!("encoded stddev operands are cell lists");
                    };
                    builtins::stddev_exact::inline_collection(
                        self,
                        [data_cells, weight_cells],
                        shapes,
                        diagnostic,
                    )?
                };
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::StddevMatrixDiagnostic => {
                let diagnostic = self.pop();
                let args = self.pop_n(2);
                let value = if matches!(&args[..], [Value::Matrix(data), Value::Matrix(weights)]
                    if !data.vector && !weights.vector)
                {
                    diagnostic
                } else {
                    builtins::call(self, ppl_compiler::builtins::Builtin::Stddev, args)?
                };
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::StddevMixedVectorDiagnostic {
                encoded_first,
                evaluated_list,
            } => {
                let ordinary_source = self.pop().expect_str("stddev operand source")?;
                let encoded_source = self.pop().expect_str("stddev encoded vector source")?;
                let ordinary = self.pop();
                let materialized = evaluated_list.then(|| ordinary.display(Default::default()));
                let ordinary_source = materialized.as_deref().unwrap_or(&ordinary_source);
                let (left, right) = if encoded_first {
                    (encoded_source.as_ref(), ordinary_source)
                } else {
                    (ordinary_source, encoded_source.as_ref())
                };
                let error = if encoded_first && ordinary.as_real().is_some() {
                    "Invalid dimension"
                } else {
                    "Bad Argument Value"
                };
                let value = Value::str(format!("stddev({left},{right}) \n Error: {error}"));
                self.check_value(&value)?;
                self.stack.push(value);
            }
            Op::BuiltinRef { id, argc, refs } => {
                let args = self.pop_n(argc as usize);
                let (ret, updated) = builtins::call_ref(self, id, args)?;
                debug_assert_eq!(updated.len(), refs as usize);
                self.check_value(&ret)?;
                for value in &updated {
                    self.check_value(value)?;
                }
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
                let items = self
                    .pop_n(n as usize)
                    .into_iter()
                    .map(numeric_entry)
                    .collect::<RtResult<Vec<_>>>()?;
                self.stack.push(Value::Matrix(Rc::new(Matrix {
                    rows: 1,
                    cols: n as usize,
                    data: items,
                    vector: true,
                })));
            }
            Op::MakeMatrix { rows, cols } => {
                let items = self
                    .pop_n((rows * cols) as usize)
                    .into_iter()
                    .map(numeric_entry)
                    .collect::<RtResult<Vec<_>>>()?;
                self.stack.push(Value::Matrix(Rc::new(Matrix {
                    rows: rows as usize,
                    cols: cols as usize,
                    data: items,
                    vector: false,
                })));
            }
            Op::MakeComplex | Op::StddevCellPair => {
                let im = self.pop();
                let re = self.pop();
                if matches!(op, Op::StddevCellPair)
                    && let Some(value) = builtins::stddev_exact::inline_pair(&re, &im)
                {
                    self.stack.push(value);
                    return Ok(Flow::Next);
                }
                let im = im.expect_real("complex")?;
                let re = re.expect_real("complex")?;
                // Both original G1 builds retain the explicit complex type,
                // including a zero imaginary component. Matrices normalize
                // their cells separately through numeric_entry.
                self.stack
                    .push(Value::Complex(Complex::new(re.decimal(), im.decimal())));
            }
            Op::Index(n) => {
                let idx = self.pop_n(n as usize);
                let base = self.pop();
                let v = if matches!(base, Value::Symbolic(_)) {
                    crate::cas::evaluate_math(
                        self,
                        &format!(
                            "({})({})",
                            crate::cas::expression(&base)?,
                            idx.iter()
                                .map(crate::cas::expression)
                                .collect::<RtResult<Vec<_>>>()?
                                .join(",")
                        ),
                    )?
                } else {
                    index_value(&base, &idx, self)?
                };
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
            Op::StoreIndexZeroLocal(slot) => {
                let value = self.pop();
                let max = self.limits.max_collection_len;
                let target = &mut self.stack[frame.base + slot as usize];
                if let Value::List(items) = target {
                    append_list(items, value.clone(), max)?;
                } else {
                    store_index(target, &[Value::zero()], value.clone(), max)?;
                }
                self.stack.push(value);
            }
            Op::StoreIndexGlobal { slot, n } => {
                self.prepare_global(slot as usize)?;
                let v = self.pop();
                let idx = self.pop_n(n as usize);
                let max = self.limits.max_collection_len;
                let mut target = self.load_global(slot as usize);
                store_index(&mut target, &idx, v.clone(), max)?;
                self.store_global(slot as usize, target)?;
                self.stack.push(v);
            }
            Op::StoreHomeParameters => {
                let parameters = self.pop();
                let selector = self.pop();
                let index = self.pop();
                if selector.expect_i64("HVars")? != 2 {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "HVars metadata selector must be 2",
                    ));
                }
                let name = match index {
                    Value::Str(name) => name.to_string(),
                    other => {
                        let n = other.expect_i64("HVars")?;
                        if n < 1 {
                            return Err(RuntimeError::kind(ErrorKind::BadArgValue));
                        }
                        self.home_variables
                            .keys()
                            .filter(|name| !ppl_compiler::is_system_variable(name))
                            .nth((n - 1) as usize)
                            .cloned()
                            .ok_or_else(|| RuntimeError::kind(ErrorKind::Undefined))?
                    }
                };
                let value = self
                    .home_variables
                    .get(&name)
                    .ok_or_else(|| RuntimeError::kind(ErrorKind::Undefined))?
                    .clone();
                let function =
                    crate::cas::with_parameters(&value, &parameters.expect_list("HVars")?)?;
                self.store_home_value(&name, function)?;
                self.stack.push(parameters);
            }
            Op::StoreHomeVariable | Op::StoreAppFile => {
                let value = self.pop();
                let index = self.pop();
                if op == Op::StoreAppFile {
                    let Value::Str(name) = index else {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgType,
                            "AFiles needs a file name",
                        ));
                    };
                    self.app_files
                        .entry(self.active_app.clone())
                        .or_default()
                        .insert(name.to_string(), crate::PortableValue::from(&value));
                } else {
                    let name = match index {
                        Value::Str(name) => name.to_string(),
                        other => {
                            let n = other.expect_i64("HVars")?;
                            if n < 1 {
                                return Err(RuntimeError::kind(ErrorKind::BadArgValue));
                            }
                            self.home_variables
                                .keys()
                                .filter(|n| !ppl_compiler::is_system_variable(n))
                                .nth((n - 1) as usize)
                                .cloned()
                                .ok_or_else(|| RuntimeError::kind(ErrorKind::BadArgValue))?
                        }
                    };
                    self.home_variables.insert(name.clone(), value.clone());
                    if let Some(slot) = self
                        .prog
                        .globals
                        .iter()
                        .position(|s| s.name == name && !s.program_var)
                    {
                        self.store_global(slot, value.clone())?;
                    }
                }
                self.stack.push(value);
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
                if op == Op::StoreProgram
                    && self.linked.as_ref().is_some_and(|(image, _)| {
                        image
                            .modules
                            .iter()
                            .any(|m| m.name.eq_ignore_ascii_case(&name))
                    })
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Unsupported,
                        "replacing an executable catalog module during a call is unsupported; edit between invocations",
                    ));
                }
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
            Op::TryBegin(h) | Op::TryStddevCell(h) => self.tries.push(TryRec {
                frame_depth: self.frames.len(),
                stack_len: self.stack.len(),
                handler: h as usize,
                kind: if matches!(op, Op::TryStddevCell(_)) {
                    TryKind::StddevCell
                } else {
                    TryKind::IfErr
                },
            }),
            Op::TryAppendCallback {
                handler,
                argc,
                preceding,
            } => {
                let stack_len = self.stack.len() - usize::from(argc);
                let base = &self.stack[stack_len - 1 - usize::from(preceding)];
                let collection_base = matches!(base, Value::List(_) | Value::Matrix(_));
                self.tries.push(TryRec {
                    frame_depth: self.frames.len(),
                    stack_len,
                    handler: handler as usize,
                    kind: TryKind::AppendCallback { collection_base },
                });
            }
            Op::TryEnd => {
                self.tries.pop();
            }
            Op::ForExit { down, target } => {
                let limit = self.pop();
                let counter = self.pop();
                let past_limit = if down {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
                if ops::compare(&counter, &limit)? == past_limit {
                    self.frames.last_mut().unwrap().ip = target as usize;
                }
            }
            Op::AppendLocal(slot) => {
                let v = self.pop();
                let max = self.limits.max_collection_len;
                let target = &mut self.stack[frame.base + slot as usize];
                if let Value::List(items) = target {
                    append_list(items, v, max)?;
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

    fn pop_append_operand(
        &mut self,
        callback: bool,
    ) -> RtResult<(Value, Option<AppendCallbackFault>)> {
        let fault = if callback {
            AppendCallbackFault::from_status(self.pop().expect_i64("APPEND callback status")?)
        } else {
            None
        };
        Ok((self.pop(), fault))
    }

    fn append_quotient_value(
        &mut self,
        collection_base: bool,
        numerator: &Value,
        denominator: &Value,
    ) -> RtResult<Value> {
        if collection_base
            && matches!(
                (numerator, denominator),
                (Value::Real(_), Value::Str(_)) | (Value::Str(_), Value::Real(_))
            )
        {
            Ok(Value::str("string / Error: Bad Argument Value"))
        } else {
            self.binary_value(BinOp::Div, numerator, denominator)
        }
    }

    fn append_callback_fault(
        &mut self,
        base: Value,
        encoded_base: bool,
        fault: AppendCallbackFault,
    ) -> RtResult<Value> {
        match fault {
            AppendCallbackFault::DivisionByZero => {
                builtins::lists::append_callback_division_error(self, base, encoded_base)
            }
            AppendCallbackFault::BadArgType => {
                let message = RuntimeError::kind(ErrorKind::BadArgType).to_string();
                builtins::lists::append_collection(
                    self,
                    &[base, Value::str(message)],
                    false,
                    encoded_base,
                )
            }
        }
    }

    fn check_value(&self, v: &Value) -> RtResult<()> {
        match v {
            Value::Str(s) if s.len() > self.limits.max_string_bytes => Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Memory),
                format!("string of {} bytes", s.len()),
            )),
            Value::RawString(units) if units.len() > self.limits.max_string_bytes / 2 => {
                Err(RuntimeError::new(
                    ErrorKind::ResourceLimit(ResourceKind::Memory),
                    format!("string of {} UTF-16 units", units.len()),
                ))
            }
            Value::List(l) => self.check_len(l.len()),
            _ => Ok(()),
        }
    }
}

fn stddev_list_length_diagnostic(args: &[Value], sources: [Option<&str>; 2]) -> Option<Value> {
    let [Value::List(data), Value::List(weights)] = args else {
        return None;
    };
    if data.len() == weights.len()
        || !data
            .iter()
            .chain(weights.iter())
            .all(|entry| matches!(entry, Value::Real(_) | Value::Complex(_)))
    {
        return None;
    }
    let mut diagnostic = String::from("stddev(");
    for (index, (entries, source)) in [data, weights].into_iter().zip(sources).enumerate() {
        if index != 0 {
            diagnostic.push(',');
        }
        if let Some(source) = source {
            diagnostic.push_str(source);
            continue;
        }
        diagnostic.push('{');
        for (index, entry) in entries.iter().enumerate() {
            if index != 0 {
                diagnostic.push(',');
            }
            let text = entry.display(Default::default());
            if matches!(entry, Value::Complex(_)) {
                diagnostic.push_str(&text.replace('i', "\u{e003}"));
            } else {
                diagnostic.push_str(&text);
            }
        }
        diagnostic.push('}');
    }
    diagnostic.push_str(") \n Error: Bad Argument Value");
    Some(Value::str(diagnostic))
}

pub(crate) fn numeric_entry(v: Value) -> RtResult<Value> {
    match v {
        Value::Int(number) => Ok(Value::Real(number.to_real())),
        Value::Real(value) if value.origin() != ppl_math::RealOrigin::LegacyBcd => {
            Ok(Value::Real(value.decimal()))
        }
        Value::Complex(value) if value.im.is_zero() => Ok(Value::Real(value.re.decimal())),
        Value::Complex(value) => Ok(Value::Complex(
            Complex::new(value.re.decimal(), value.im.decimal())
                .with_presentation(value.presentation()),
        )),
        Value::Real(_) => Ok(v),
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
    let (prog, diags) = ppl_compiler::compile_source(
        &file,
        &ppl_compiler::CompileOptions {
            default_int_base: settings.int_base,
            default_int_width: settings.integer_width_code(),
            ..Default::default()
        },
    );
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
