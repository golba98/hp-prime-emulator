//! Calculator state shared by successive invocations, separate from VM stacks.
//! Portable values deliberately contain no `Rc`, so a GUI can transfer a
//! session safely between operation workers without unsafe Send impls.
use crate::{
    ErrorKind, Host, Limits, Machine, RtResult, RuntimeError, Settings, Value, host::DisplayMode,
    rng::Rng, value::Matrix,
};
use ppl_compiler::CompiledProgram;
use ppl_graphics::Grob;
use ppl_math::{Complex, HpInt, Real};
use std::{
    collections::BTreeMap,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub enum PortableValue {
    Real(Real),
    Integer(HpInt),
    Complex(Complex),
    String(String),
    List(Vec<PortableValue>),
    Matrix {
        rows: usize,
        cols: usize,
        vector: bool,
        data: Vec<PortableValue>,
    },
    Grob(u8),
}
impl From<&Value> for PortableValue {
    fn from(value: &Value) -> Self {
        match value {
            Value::Real(v) => Self::Real(*v),
            Value::Int(v) => Self::Integer(*v),
            Value::Complex(v) => Self::Complex(*v),
            Value::Str(v) => Self::String(v.to_string()),
            Value::List(v) => Self::List(v.iter().map(Self::from).collect()),
            Value::Matrix(v) => Self::Matrix {
                rows: v.rows,
                cols: v.cols,
                vector: v.vector,
                data: v.data.iter().map(Self::from).collect(),
            },
            Value::Grob(v) => Self::Grob(*v),
        }
    }
}
impl PortableValue {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Real(v) => Value::Real(*v),
            Self::Integer(v) => Value::Int(*v),
            Self::Complex(v) => Value::Complex(*v),
            Self::String(v) => Value::str(v.as_str()),
            Self::List(v) => Value::list(v.iter().map(Self::to_value).collect()),
            Self::Matrix {
                rows,
                cols,
                vector,
                data,
            } => Value::Matrix(Rc::new(Matrix {
                rows: *rows,
                cols: *cols,
                vector: *vector,
                data: data.iter().map(Self::to_value).collect(),
            })),
            Self::Grob(v) => Value::Grob(*v),
        }
    }
    pub fn display(&self, settings: &Settings) -> String {
        self.to_value().display(settings.format())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub name: String,
    pub revision: String,
}
impl SourceIdentity {
    pub fn new(name: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            revision: revision.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionSnapshot {
    pub generation: u64,
    pub settings: Settings,
    pub globals: BTreeMap<String, PortableValue>,
    pub last_value: Option<PortableValue>,
    pub notes: BTreeMap<String, String>,
    pub programs: BTreeMap<String, String>,
}
struct ModuleState {
    revision: String,
    initialized: bool,
    values: BTreeMap<String, PortableValue>,
}

pub struct SessionContext {
    pub settings: Settings,
    globals: BTreeMap<String, PortableValue>,
    modules: BTreeMap<String, ModuleState>,
    grobs: Vec<Option<Grob>>,
    notes: BTreeMap<String, String>,
    programs: BTreeMap<String, String>,
    terminal: Vec<String>,
    display: DisplayMode,
    rng: Rng,
    generation: u64,
    last_value: Option<PortableValue>,
}
pub struct SessionRun {
    pub result: RtResult<Value>,
    pub steps: u64,
    pub elapsed: Duration,
}
impl Default for SessionContext {
    fn default() -> Self {
        Self::new(Settings::default(), 0x5EED)
    }
}
impl SessionContext {
    pub fn new(settings: Settings, seed: u64) -> Self {
        let mut grobs = vec![None; 10];
        grobs[0] = Some(Grob::screen());
        Self {
            settings,
            globals: BTreeMap::from([("Ans".into(), PortableValue::Real(Real::ZERO))]),
            modules: BTreeMap::new(),
            grobs,
            notes: BTreeMap::new(),
            programs: BTreeMap::new(),
            terminal: Vec::new(),
            display: DisplayMode::Graphics,
            rng: Rng::new(seed),
            generation: 0,
            last_value: None,
        }
    }
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            generation: self.generation,
            settings: self.settings.clone(),
            globals: self.globals.clone(),
            last_value: self.last_value.clone(),
            notes: self.notes.clone(),
            programs: self.programs.clone(),
        }
    }
    pub fn record_failure(&mut self) {
        self.generation += 1;
        self.last_value = None;
    }

    /// Creates fresh execution state, retaining calculator state even on an
    /// ordinary runtime error or interruption. Program globals are isolated by
    /// source name and reset when that source's exact revision changes.
    pub fn run(
        &mut self,
        identity: SourceIdentity,
        prog: Rc<CompiledProgram>,
        host: Box<dyn Host>,
        limits: Limits,
        promote_ans: bool,
        invoke: impl FnOnce(&mut Machine) -> RtResult<Value>,
    ) -> SessionRun {
        let mut machine = Machine::new(prog.clone(), host, self.settings.clone(), limits);
        machine.grobs = self.grobs.clone();
        machine.notes = self.notes.clone();
        machine.programs = self.programs.clone();
        machine.terminal = self.terminal.clone();
        machine.display = self.display;
        machine.rng = self.rng.clone();
        let module = self
            .modules
            .get(&identity.name)
            .filter(|m| m.revision == identity.revision);
        machine.init_done = module.is_some_and(|m| m.initialized);
        for (i, slot) in prog.globals.iter().enumerate() {
            let stored = if slot.program_var {
                module.and_then(|m| m.values.get(&slot.name))
            } else {
                self.globals.get(&slot.name)
            };
            if let Some(value) = stored {
                machine.globals[i] = value.to_value();
            }
        }
        let start = Instant::now();
        let result = invoke(&mut machine);
        let elapsed = start.elapsed();
        let steps = machine.steps();
        let mut module_values = BTreeMap::new();
        for (i, slot) in prog.globals.iter().enumerate() {
            let value = PortableValue::from(&machine.globals[i]);
            if slot.program_var {
                module_values.insert(slot.name.clone(), value);
            } else if !matches!(slot.init, ppl_compiler::GlobalInit::Setting(_)) {
                self.globals.insert(slot.name.clone(), value);
            }
        }
        self.modules.insert(
            identity.name,
            ModuleState {
                revision: identity.revision,
                initialized: machine.init_done,
                values: module_values,
            },
        );
        self.settings = machine.settings.clone();
        self.grobs = machine.grobs;
        self.notes = machine.notes;
        self.programs = machine.programs;
        self.terminal = machine.terminal;
        self.display = machine.display;
        self.rng = machine.rng;
        self.generation += 1;
        self.last_value = result.as_ref().ok().map(PortableValue::from);
        if promote_ans {
            if let Some(value) = &self.last_value {
                self.globals.insert("Ans".into(), value.clone());
            }
        }
        SessionRun {
            result,
            steps,
            elapsed,
        }
    }

    pub fn evaluate_home(&mut self, text: &str, host: Box<dyn Host>, limits: Limits) -> SessionRun {
        let compiled = self.compile_home(text);
        match compiled {
            Ok(prog) => self.run(
                SourceIdentity::new("<home>", text),
                Rc::new(prog),
                host,
                limits,
                true,
                |machine| machine.call_function("__HOME", Vec::new()),
            ),
            Err(error) => {
                self.record_failure();
                SessionRun {
                    result: Err(error),
                    steps: 0,
                    elapsed: Duration::ZERO,
                }
            }
        }
    }

    /// Evaluate an argument in the calculator environment without publishing
    /// an extra operation or promoting its temporary result to Home Ans.
    pub fn evaluate_argument(
        &mut self,
        text: &str,
        host: Box<dyn Host>,
        limits: Limits,
    ) -> RtResult<Value> {
        let program = self.compile_home(text)?;
        let generation = self.generation;
        let previous_value = self.last_value.clone();
        let run = self.run(
            SourceIdentity::new("<argument>", text),
            Rc::new(program),
            host,
            limits,
            false,
            |machine| machine.call_function("__HOME", Vec::new()),
        );
        self.generation = generation;
        self.last_value = previous_value;
        run.result
    }
    fn compile_home(&self, text: &str) -> RtResult<CompiledProgram> {
        let source = format!("EXPORT __HOME()\nBEGIN\n{text};\nEND;");
        let (mut ast, parse_diags) = ppl_syntax::parse(&source);
        if parse_diags.has_errors() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                parse_diags.errors().next().unwrap().message.clone(),
            ));
        }
        let [ppl_syntax::ast::Item::Function(function)] = ast.items.as_mut_slice() else {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                "Home expects an expression or statement",
            ));
        };
        if let Some(last) = function.body.last_mut() {
            if let ppl_syntax::ast::StmtKind::Expr(expr) = &last.kind {
                last.kind = ppl_syntax::ast::StmtKind::Return(Some(expr.clone()));
            }
        }
        let (program, diags) = ppl_compiler::compile(
            &ast,
            &ppl_compiler::CompileOptions {
                default_int_base: self.settings.int_base,
            },
        );
        program.ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::InvalidInput,
                diags
                    .errors()
                    .next()
                    .map(|d| d.message.clone())
                    .unwrap_or_else(|| "Home compilation failed".into()),
            )
        })
    }
}
