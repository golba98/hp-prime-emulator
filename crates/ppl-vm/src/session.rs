//! Calculator state shared by successive invocations, separate from VM stacks.
//! Portable values deliberately contain no `Rc`, so a GUI can transfer a
//! session safely between operation workers without unsafe Send impls.
mod memory;
use crate::{
    ErrorKind, Host, Limits, Machine, RtResult, RuntimeError, Settings, Value, host::DisplayMode,
    rng::Rng, value::Matrix,
};
pub use memory::{decode_value, encode_value};
use ppl_compiler::CompiledProgram;
use ppl_graphics::Grob;
use ppl_math::{Complex, HpInt, Real};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub enum PortableValue {
    Symbolic(ppl_cas::CasValue),
    Real(Real),
    Integer(HpInt),
    Complex(Complex),
    String(String),
    Utf16(Vec<u16>),
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
            Value::Symbolic(v) => Self::Symbolic((**v).clone()),
            Value::Real(v) => Self::Real(*v),
            Value::Int(v) => Self::Integer(*v),
            Value::Complex(v) => Self::Complex(*v),
            Value::Str(v) => Self::String(v.to_string()),
            Value::RawString(v) => Self::Utf16(v.to_vec()),
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
            Self::Symbolic(v) => crate::cas::from_cas(v.clone()),
            Self::Real(v) => Value::Real(*v),
            Self::Integer(v) => Value::Int(*v),
            Self::Complex(v) => Value::Complex(*v),
            Self::String(v) => Value::str(v.as_str()),
            Self::Utf16(v) => Value::from_utf16(v.clone()),
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    pub active_app: String,
    pub app_settings: BTreeMap<String, crate::AppSettings>,
    pub app_numeric_settings: BTreeMap<String, crate::AppNumericSettings>,
    pub app_plot_settings: BTreeMap<String, crate::AppPlotSettings>,
    pub geometry_plot_settings: Option<crate::GeometryPlotSettings>,
    pub statistics2_columns_revision: u64,
    pub app_files: BTreeMap<String, BTreeMap<String, PortableValue>>,
    pub cas: ppl_cas::CasSession,
    pub generation: u64,
    pub settings: Settings,
    pub globals: BTreeMap<String, PortableValue>,
    pub last_value: Option<PortableValue>,
    pub notes: BTreeMap<String, String>,
    pub note_bullets: BTreeMap<String, BTreeSet<usize>>,
    pub programs: BTreeMap<String, String>,
}
#[derive(Clone)]
pub(crate) struct ModuleState {
    pub(crate) revision: String,
    pub(crate) initialized: bool,
    pub(crate) values: BTreeMap<String, PortableValue>,
}

pub struct SessionContext {
    pub active_app: String,
    pub app_settings: BTreeMap<String, crate::AppSettings>,
    pub app_numeric_settings: BTreeMap<String, crate::AppNumericSettings>,
    pub app_plot_settings: BTreeMap<String, crate::AppPlotSettings>,
    pub geometry_plot_settings: Option<crate::GeometryPlotSettings>,
    statistics2_columns_revision: u64,
    pub app_files: BTreeMap<String, BTreeMap<String, PortableValue>>,
    pub cas: ppl_cas::CasSession,
    cas_home_values: BTreeMap<String, PortableValue>,
    pub settings: Settings,
    globals: BTreeMap<String, PortableValue>,
    modules: BTreeMap<String, ModuleState>,
    grobs: Vec<Option<Grob>>,
    notes: BTreeMap<String, String>,
    note_bullets: BTreeMap<String, BTreeSet<usize>>,
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
impl SessionSnapshot {
    pub fn geometry_plot_settings(&self) -> crate::GeometryPlotSettings {
        self.geometry_plot_settings.clone().unwrap_or_default()
    }
    /// Effective Plot Setup for an app with established variable contracts.
    /// These values are separate from actual Home globals and are never
    /// injected into snapshots merely for taking a read-only view.
    pub fn plot_settings_for(&self, app: &str) -> Option<crate::AppPlotSettings> {
        self.app_plot_settings
            .get(app)
            .cloned()
            .or_else(|| crate::AppPlotSettings::for_app(app))
    }
    /// An explicitly stored app field, or a legacy live value for the
    /// active app. Default fields do not become Home variables.
    pub fn plot_variable_for(&self, app: &str, name: &str) -> Option<PortableValue> {
        if app == "Geometry" {
            return self
                .geometry_plot_settings
                .as_ref()
                .and_then(|s| s.get(name).ok())
                .map(|v| PortableValue::from(&v));
        }
        self.app_plot_settings
            .get(app)
            .and_then(|settings| settings.get(name))
            .map(|value| PortableValue::from(&value))
            .or_else(|| {
                (app == self.active_app)
                    .then(|| self.globals.get(name).cloned())
                    .flatten()
            })
    }
    pub fn app_plot_settings(&self) -> RtResult<crate::AppPlotSettings> {
        self.app_plot_settings
            .get(&self.active_app)
            .cloned()
            .or_else(|| crate::AppPlotSettings::for_app(&self.active_app))
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!(
                        "Plot Setup defaults are not established for {}",
                        self.active_app
                    ),
                )
            })
    }
    pub fn app_numeric_settings(&self) -> crate::AppNumericSettings {
        self.app_numeric_settings
            .get(&self.active_app)
            .cloned()
            .unwrap_or_else(|| crate::AppNumericSettings::for_app(&self.active_app))
    }
    pub fn effective_app_settings(&self) -> Settings {
        self.app_settings
            .get(&self.active_app)
            .cloned()
            .unwrap_or_default()
            .effective(&self.settings)
    }
}
impl SessionContext {
    pub fn geometry_plot_settings(&self) -> crate::GeometryPlotSettings {
        self.geometry_plot_settings.clone().unwrap_or_default()
    }
    pub fn set_geometry_plot_settings(
        &mut self,
        settings: crate::GeometryPlotSettings,
    ) -> RtResult<()> {
        settings
            .validate()
            .map_err(|m| RuntimeError::new(ErrorKind::InvalidInput, m))?;
        if self.geometry_plot_settings.as_ref() != Some(&settings) {
            self.geometry_plot_settings = Some(settings);
            self.generation += 1;
        }
        Ok(())
    }
    /// Effective Plot Setup for an app with established variable contracts.
    /// These values are separate from actual Home globals and are never
    /// injected into snapshots merely for taking a read-only view.
    pub fn plot_settings_for(&self, app: &str) -> Option<crate::AppPlotSettings> {
        self.app_plot_settings
            .get(app)
            .cloned()
            .or_else(|| crate::AppPlotSettings::for_app(app))
    }
    /// Store one app's Plot Setup without changing the selected application.
    pub fn set_app_plot_settings(
        &mut self,
        app: &str,
        settings: crate::AppPlotSettings,
    ) -> RtResult<()> {
        if crate::AppPlotSettings::for_app(app).is_none() {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                format!("Plot Setup defaults are not established for {app}"),
            ));
        }
        settings
            .validate_for(app)
            .map_err(|message| RuntimeError::new(ErrorKind::BadArgValue, message))?;
        if self.app_plot_settings.get(app) != Some(&settings) {
            self.app_plot_settings.insert(app.into(), settings);
            self.generation += 1;
        }
        Ok(())
    }
    fn retain_linked_modules(
        &mut self,
        machine: &Machine,
        image: &crate::LinkedProgram,
        states: &[bool],
    ) {
        let mut values = vec![BTreeMap::new(); image.modules.len()];
        for (index, slot) in image.program.globals.iter().enumerate() {
            if !slot.name.starts_with('<')
                && let Some(owner) = image.global_owners[index]
            {
                values[owner].insert(
                    slot.name.clone(),
                    PortableValue::from(&machine.globals[index]),
                );
            }
        }
        for ((identity, initialized), values) in image.modules.iter().zip(states).zip(values) {
            self.modules.insert(
                identity.name.clone(),
                ModuleState {
                    revision: identity.revision.clone(),
                    initialized: *initialized,
                    values,
                },
            );
        }
    }
    pub fn app_settings(&self) -> crate::AppSettings {
        self.app_settings
            .get(&self.active_app)
            .cloned()
            .unwrap_or_default()
    }
    pub fn app_settings_mut(&mut self) -> &mut crate::AppSettings {
        self.app_settings
            .entry(self.active_app.clone())
            .or_default()
    }
    pub fn app_plot_settings(&self) -> RtResult<crate::AppPlotSettings> {
        self.app_plot_settings
            .get(&self.active_app)
            .cloned()
            .or_else(|| crate::AppPlotSettings::for_app(&self.active_app))
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!(
                        "Plot Setup defaults are not established for {}",
                        self.active_app
                    ),
                )
            })
    }
    pub fn app_numeric_settings(&self) -> crate::AppNumericSettings {
        self.app_numeric_settings
            .get(&self.active_app)
            .cloned()
            .unwrap_or_else(|| crate::AppNumericSettings::for_app(&self.active_app))
    }
    pub fn effective_app_settings(&self) -> Settings {
        self.app_settings().effective(&self.settings)
    }
    /// Mutations from calculator catalogs share exactly the memory used by PPL.
    pub fn set_graph3d_numeric_settings(
        &mut self,
        settings: crate::AppNumericSettings,
    ) -> RtResult<()> {
        if settings.graph3d.is_none() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                "Graph 3D requires separate X and Y Numeric settings",
            ));
        }
        settings
            .validate()
            .map_err(|message| RuntimeError::new(ErrorKind::InvalidInput, message))?;
        if self.app_numeric_settings.get("Graph 3D") != Some(&settings) {
            self.app_numeric_settings
                .insert("Graph 3D".into(), settings);
            self.generation += 1;
        }
        Ok(())
    }

    pub fn set_variable(&mut self, name: &str, value: PortableValue) -> RtResult<()> {
        if name.is_empty()
            || !name
                .chars()
                .enumerate()
                .all(|(i, c)| c == '_' || c.is_alphabetic() || (i > 0 && c.is_ascii_digit()))
        {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "Invalid calculator variable name",
            ));
        }
        let value = if crate::statistics_columns::is_column(name) {
            PortableValue::from(&crate::statistics_columns::coerce(name, value.to_value())?)
        } else {
            value
        };
        if self.active_app == "Geometry" && crate::GeometryPlotSettings::recognizes(name) {
            let mut next = self.geometry_plot_settings();
            next.set(name, &value.to_value())?;
            return self.set_geometry_plot_settings(next);
        }
        if crate::AppPlotSettings::for_app(&self.active_app).is_some()
            && crate::AppPlotSettings::canonical_name(name).is_some()
        {
            crate::AppPlotSettings::check_field(&self.active_app, name)?;
            let mut next = self.app_plot_settings()?;
            next.set_for(&self.active_app, name, &value.to_value())?;
            self.app_plot_settings.insert(self.active_app.clone(), next);
            self.generation += 1;
            return Ok(());
        }
        if let Some(setting) = crate::AppNumericSettings::setting(name) {
            let mut next = self.app_numeric_settings();
            next.set(setting, &value.to_value())?;
            self.app_numeric_settings
                .insert(self.active_app.clone(), next);
            self.generation += 1;
            return Ok(());
        }
        let setting = name.to_ascii_lowercase();
        if matches!(
            setting.as_str(),
            "aangle" | "aformat" | "adigits" | "acomplex"
        ) {
            let number = value.to_value().expect_i64("app setting")?;
            let mut app = self.app_settings();
            app.set_code(&setting, number)?;
            self.app_settings.insert(self.active_app.clone(), app);
            self.generation += 1;
            return Ok(());
        }
        if matches!(
            setting.as_str(),
            "hangle"
                | "hformat"
                | "hseparator"
                | "hdigits"
                | "hcomplex"
                | "base"
                | "bits"
                | "signed"
        ) {
            let number = value.to_value().expect_i64("setting")?;
            match setting.as_str() {
                "hangle" => {
                    self.settings.angle = crate::AngleMode::from_code(number).ok_or_else(|| {
                        RuntimeError::new(ErrorKind::BadArgValue, "HAngle must be 0, 1 or 2")
                    })?
                }
                "hformat" => self
                    .settings
                    .try_set_format(number, self.settings.hdigits)?,
                "hdigits" => self
                    .settings
                    .try_set_format(self.settings.hformat, number)?,
                "hcomplex" => self.settings.set_complex_code(number)?,
                "hseparator" => self.settings.set_separator(number)?,
                "base" => self.settings.set_integer_base(number)?,
                "bits" => self.settings.set_integer_bits(number)?,
                "signed" => self.settings.set_integer_signed(number != 0)?,
                _ => unreachable!(),
            }
        } else {
            if name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit() {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "Graphics references are read-only; use graphics commands",
                ));
            }
            if crate::statistics_columns::is_two_variable_column(name) {
                self.statistics2_columns_revision =
                    self.statistics2_columns_revision.wrapping_add(1);
            }
            self.globals.insert(name.into(), value);
        }
        self.generation += 1;
        Ok(())
    }
    pub fn delete_variable(&mut self, name: &str) -> bool {
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "hangle"
                | "hformat"
                | "hseparator"
                | "hdigits"
                | "hcomplex"
                | "aangle"
                | "aformat"
                | "adigits"
                | "acomplex"
                | "numstart"
                | "numstep"
                | "numzoom"
                | "numtype"
                | "numindep"
                | "base"
                | "bits"
                | "signed"
        ) {
            return false;
        }
        let removed = self.globals.remove(name).is_some();
        self.generation += u64::from(removed);
        removed
    }
    pub fn store_program(&mut self, name: &str, source: String) -> RtResult<()> {
        if name.trim().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "A program needs a name",
            ));
        }
        self.programs.insert(name.into(), source);
        self.invalidate_module(name);
        self.generation += 1;
        Ok(())
    }
    /// Rename the calculator object without changing its source or IDE files.
    pub fn rename_program(&mut self, old: &str, new: &str) -> RtResult<()> {
        if new.trim().is_empty() || self.programs.contains_key(new) {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "Choose a new, unused program name",
            ));
        }
        let source = self
            .programs
            .remove(old)
            .ok_or_else(|| RuntimeError::new(ErrorKind::Undefined, "Program does not exist"))?;
        self.programs.insert(new.into(), source);
        self.invalidate_module(old);
        self.invalidate_module(new);
        self.generation += 1;
        Ok(())
    }
    pub fn rename_note(&mut self, old: &str, new: &str) -> RtResult<()> {
        if new.trim().is_empty() || self.notes.contains_key(new) {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "Choose a new, unused note name",
            ));
        }
        let text = self
            .notes
            .remove(old)
            .ok_or_else(|| RuntimeError::new(ErrorKind::Undefined, "Note does not exist"))?;
        self.notes.insert(new.into(), text);
        if let Some(bullets) = self.note_bullets.remove(old) {
            self.note_bullets.insert(new.into(), bullets);
        }
        self.generation += 1;
        Ok(())
    }
    pub fn delete_program(&mut self, name: &str) -> bool {
        self.invalidate_module(name);
        let removed = self.programs.remove(name).is_some();
        self.generation += u64::from(removed);
        removed
    }
    pub fn store_note(&mut self, name: &str, text: String) -> RtResult<()> {
        self.store_bulleted_note(name, text, BTreeSet::new())
    }
    pub fn store_bulleted_note(
        &mut self,
        name: &str,
        text: String,
        mut bullets: BTreeSet<usize>,
    ) -> RtResult<()> {
        if name.trim().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "A note needs a name",
            ));
        }
        let paragraphs = text.split('\n').count();
        bullets.retain(|index| *index < paragraphs);
        if bullets.is_empty() {
            self.note_bullets.remove(name);
        } else {
            self.note_bullets.insert(name.into(), bullets);
        }
        self.notes.insert(name.into(), text);
        self.generation += 1;
        Ok(())
    }
    pub fn delete_note(&mut self, name: &str) -> bool {
        let removed = self.notes.remove(name).is_some();
        self.note_bullets.remove(name);
        self.generation += u64::from(removed);
        removed
    }
    pub fn new(settings: Settings, seed: u64) -> Self {
        let mut grobs = vec![None; 10];
        grobs[0] = Some(Grob::screen());
        Self {
            active_app: "Function".into(),
            app_settings: Default::default(),
            app_numeric_settings: Default::default(),
            app_plot_settings: Default::default(),
            geometry_plot_settings: None,
            statistics2_columns_revision: 0,
            app_files: Default::default(),
            cas: Default::default(),
            cas_home_values: Default::default(),
            settings,
            globals: BTreeMap::from([("Ans".into(), PortableValue::Real(Real::ZERO))]),
            modules: BTreeMap::new(),
            grobs,
            notes: BTreeMap::new(),
            note_bullets: BTreeMap::new(),
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
            active_app: self.active_app.clone(),
            app_settings: self.app_settings.clone(),
            app_numeric_settings: self.app_numeric_settings.clone(),
            app_plot_settings: self.app_plot_settings.clone(),
            geometry_plot_settings: self.geometry_plot_settings.clone(),
            statistics2_columns_revision: self.statistics2_columns_revision,
            app_files: self.app_files.clone(),
            cas: self.cas.clone(),
            generation: self.generation,
            settings: self.settings.clone(),
            globals: self.globals.clone(),
            last_value: self.last_value.clone(),
            notes: self.notes.clone(),
            note_bullets: self.note_bullets.clone(),
            programs: self.programs.clone(),
        }
    }
    /// An explicit source replacement invalidates initialization even if a
    /// later edit restores identical text. Ordinary unchanged Run does not.
    pub fn invalidate_module(&mut self, name: &str) {
        self.modules.remove(name);
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
        machine.root_identity = Some(identity.clone());
        machine.catalog_states = self.modules.clone();
        machine.active_app = self.active_app.clone();
        machine.app_settings = self.app_settings.clone();
        machine.app_numeric_settings = self.app_numeric_settings.clone();
        machine.app_plot_settings = self.app_plot_settings.clone();
        machine.statistics2_columns_revision = self.statistics2_columns_revision;
        machine.geometry_plot_settings = self.geometry_plot_settings.clone();
        machine.app_files = self.app_files.clone();
        machine.home_variables = self
            .globals
            .iter()
            .map(|(k, v)| (k.clone(), v.to_value()))
            .collect();
        machine.cas_context = self.cas.clone();
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
                self.globals
                    .get(&slot.name)
                    .or_else(|| self.cas_home_values.get(&slot.name))
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
            } else if let Some(previous) = self.cas_home_values.get(&slot.name) {
                if previous != &value {
                    machine.cas_context.bindings.insert(
                        slot.name.clone(),
                        crate::cas::to_cas(&value.to_value())
                            .map(|v| v.expression())
                            .unwrap_or_default(),
                    );
                }
            } else if !matches!(slot.init, ppl_compiler::GlobalInit::Setting(_))
                && machine.retain_home_slot(i)
            {
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
        if let Some((image, states)) = &machine.linked {
            self.retain_linked_modules(&machine, image, states);
        }
        self.active_app = machine.active_app;
        self.app_settings = machine.app_settings;
        self.app_numeric_settings = machine.app_numeric_settings;
        self.app_plot_settings = machine.app_plot_settings;
        self.statistics2_columns_revision = machine.statistics2_columns_revision;
        self.geometry_plot_settings = machine.geometry_plot_settings;
        self.app_files = machine.app_files;
        for (name, value) in machine.home_variables {
            self.globals.insert(name, PortableValue::from(&value));
        }
        self.cas_home_values.clear();
        self.cas = machine.cas_context.clone();
        self.settings = machine.settings.clone();
        self.grobs = machine.grobs;
        // A PPL text replacement discards styles attached to the previous text.
        self.note_bullets
            .retain(|name, _| self.notes.get(name) == machine.notes.get(name));
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

    /// Execute one independently linked catalog using the same VM stack and calculator state.
    /// Module initializer flags/private values are retained by exact source revision.
    pub fn run_linked(
        &mut self,
        image: crate::LinkedProgram,
        host: Box<dyn Host>,
        limits: Limits,
        promote_ans: bool,
        invoke: impl FnOnce(&mut Machine) -> RtResult<Value>,
    ) -> SessionRun {
        let mut machine = Machine::new(image.program.clone(), host, self.settings.clone(), limits);
        machine.catalog_states = self.modules.clone();
        machine.active_app = self.active_app.clone();
        machine.app_settings = self.app_settings.clone();
        machine.app_numeric_settings = self.app_numeric_settings.clone();
        machine.app_plot_settings = self.app_plot_settings.clone();
        machine.statistics2_columns_revision = self.statistics2_columns_revision;
        machine.geometry_plot_settings = self.geometry_plot_settings.clone();
        machine.app_files = self.app_files.clone();
        machine.home_variables = self
            .globals
            .iter()
            .map(|(k, v)| (k.clone(), v.to_value()))
            .collect();
        machine.cas_context = self.cas.clone();
        machine.grobs = self.grobs.clone();
        machine.notes = self.notes.clone();
        machine.programs = self.programs.clone();
        machine.terminal = self.terminal.clone();
        machine.display = self.display;
        machine.rng = self.rng.clone();
        let states: Vec<_> = image
            .modules
            .iter()
            .map(|id| {
                self.modules
                    .get(&id.name)
                    .filter(|m| m.revision == id.revision)
                    .is_some_and(|m| m.initialized)
            })
            .collect();
        for (i, slot) in image.program.globals.iter().enumerate() {
            let stored = match image.global_owners[i] {
                Some(module) => {
                    let id = &image.modules[module];
                    self.modules
                        .get(&id.name)
                        .filter(|m| m.revision == id.revision)
                        .and_then(|m| m.values.get(&slot.name))
                }
                None => self
                    .globals
                    .get(&slot.name)
                    .or_else(|| self.cas_home_values.get(&slot.name)),
            };
            if let Some(value) = stored {
                machine.globals[i] = value.to_value();
            }
        }
        machine.linked = Some((image.clone(), states));
        let start = Instant::now();
        let result = invoke(&mut machine);
        let elapsed = start.elapsed();
        let steps = machine.steps();
        // EXPR can have appended genuinely referenced catalog modules during
        // this invocation. Persist that actual image, including its private
        // variables and initializer flags, rather than the initial closure.
        let image = machine.linked.as_ref().unwrap().0.clone();
        let mut values = vec![BTreeMap::new(); image.modules.len()];
        for (i, slot) in image.program.globals.iter().enumerate() {
            if slot.name.starts_with('<') {
                continue;
            }
            let value = PortableValue::from(&machine.globals[i]);
            if let Some(module) = image.global_owners[i] {
                values[module].insert(slot.name.clone(), value);
            } else if let Some(previous) = self.cas_home_values.get(&slot.name) {
                if previous != &value {
                    machine.cas_context.bindings.insert(
                        slot.name.clone(),
                        crate::cas::to_cas(&value.to_value())
                            .map(|v| v.expression())
                            .unwrap_or_default(),
                    );
                }
            } else if !matches!(slot.init, ppl_compiler::GlobalInit::Setting(_))
                && machine.retain_home_slot(i)
            {
                self.globals.insert(slot.name.clone(), value);
            }
        }
        let (_, states) = machine.linked.take().unwrap();
        for ((id, initialized), values) in image.modules.into_iter().zip(states).zip(values) {
            self.modules.insert(
                id.name,
                ModuleState {
                    revision: id.revision,
                    initialized,
                    values,
                },
            );
        }
        self.active_app = machine.active_app;
        self.app_settings = machine.app_settings;
        self.app_numeric_settings = machine.app_numeric_settings;
        self.app_plot_settings = machine.app_plot_settings;
        self.statistics2_columns_revision = machine.statistics2_columns_revision;
        self.geometry_plot_settings = machine.geometry_plot_settings;
        self.app_files = machine.app_files;
        for (name, value) in machine.home_variables {
            self.globals.insert(name, PortableValue::from(&value));
        }
        self.cas_home_values.clear();
        self.cas = machine.cas_context.clone();
        self.settings = machine.settings.clone();
        self.grobs = machine.grobs;
        // A PPL text replacement discards styles attached to the previous text.
        self.note_bullets
            .retain(|name, _| self.notes.get(name) == machine.notes.get(name));
        self.notes = machine.notes;
        self.programs = machine.programs;
        self.terminal = machine.terminal;
        self.display = machine.display;
        self.rng = machine.rng;
        self.generation += 1;
        self.last_value = result.as_ref().ok().map(PortableValue::from);
        if promote_ans && let Some(value) = &self.last_value {
            self.globals.insert("Ans".into(), value.clone());
        }
        SessionRun {
            result,
            steps,
            elapsed,
        }
    }

    fn evaluate_measurement(
        &mut self,
        source: &str,
        host: Box<dyn Host>,
        limits: Limits,
        promote: bool,
    ) -> SessionRun {
        let start = Instant::now();
        let mut cas = self.cas.clone();
        cas.settings = crate::cas::settings(&self.settings);
        cas.last_value = self
            .globals
            .get("Ans")
            .and_then(|value| crate::cas::to_cas(&value.to_value()));
        let home = self
            .globals
            .iter()
            .filter_map(|(name, value)| {
                crate::cas::to_cas(&value.to_value()).map(|value| (name.clone(), value))
            })
            .collect();
        let mut options = ppl_cas::Options::default();
        if let Some(timeout) = limits.max_time {
            options.timeout = options.timeout.min(timeout);
        }
        let interrupt = host.interrupt_flag();
        let fallback = std::sync::atomic::AtomicBool::new(false);
        let result=cas.evaluate_with_home(source,&home,&options,interrupt.as_deref().unwrap_or(&fallback)).and_then(|evaluation|{
            if cas.bindings.keys().any(|name|!self.cas.bindings.contains_key(name)){return Err(ppl_cas::CasError::InvalidRequest("Create Home variables through the Variables catalog or HVars before assigning a measurement".into()));}
            for (name, value) in evaluation.home_updates {
    if crate::statistics_columns::is_two_variable_column(&name) {
        self.statistics2_columns_revision = self.statistics2_columns_revision.wrapping_add(1);
    }
    self.globals.insert(name, PortableValue::from(&crate::cas::from_cas_home_in_model(value, self.settings.numeric_version)));
}
            self.cas.bindings=cas.bindings;self.cas.helper_version=cas.helper_version;
            let value=crate::cas::from_cas_home_in_model(evaluation.value,self.settings.numeric_version);
            if promote{let portable=PortableValue::from(&value);self.globals.insert("Ans".into(),portable.clone());self.last_value=Some(portable);}
            Ok(value)
        }).map_err(|error|RuntimeError::new(match error{ppl_cas::CasError::Cancelled=>ErrorKind::Interrupted,ppl_cas::CasError::TimedOut=>ErrorKind::ResourceLimit(crate::ResourceKind::Time),_=>ErrorKind::InvalidInput},error.to_string()));
        self.generation += 1;
        SessionRun {
            result,
            steps: 0,
            elapsed: start.elapsed(),
        }
    }
    pub fn evaluate_home(&mut self, text: &str, host: Box<dyn Host>, limits: Limits) -> SessionRun {
        self.evaluate_home_with_sources(text, &BTreeMap::new(), host, limits)
    }
    /// Executes the whole Home expression in one VM. Open IDE sources are an
    /// immutable operation snapshot; calculator-installed programs retain their
    /// identity when an identical source is also open in an IDE tab.
    pub fn evaluate_home_with_sources(
        &mut self,
        text: &str,
        catalog: &BTreeMap<String, crate::ModuleSource>,
        host: Box<dyn Host>,
        limits: Limits,
    ) -> SessionRun {
        match ppl_cas::units::translate_home(text) {
            Ok(Some(source)) => return self.evaluate_measurement(&source, host, limits, true),
            Ok(None) => {}
            Err(error) => {
                self.record_failure();
                return SessionRun {
                    result: Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        error.to_string(),
                    )),
                    steps: 0,
                    elapsed: Duration::ZERO,
                };
            }
        }
        let compiled = self.compile_home(text);
        // An app-style name may also be an installed user export (for example
        // GEOBRIDGE). The initial isolated compile cannot see that catalog;
        // retain declared-function precedence by using the existing dependency
        // linker when such a name was provisionally emitted as app access.
        let app_export = compiled.as_ref().is_ok_and(|program| {
            let provisional = program
                .functions
                .iter()
                .flat_map(|f| &f.code)
                .filter_map(|op| {
                    if let ppl_compiler::Op::AppVariable { name, .. } = op {
                        let name = &program.names[*name as usize];
                        (!name.contains('.')).then_some(name.as_str())
                    } else {
                        None
                    }
                })
                .collect::<std::collections::BTreeSet<_>>();
            !provisional.is_empty()
                && self
                    .programs
                    .values()
                    .map(String::as_str)
                    .chain(catalog.values().map(|module| module.source.as_str()))
                    .any(|source| {
                        let (ast, diagnostics) = ppl_syntax::parse(source);
                        !diagnostics.has_errors()
                            && ast.items.iter().any(|item| match item {
                                ppl_syntax::ast::Item::Function(function) => {
                                    function.exported
                                        && provisional.contains(function.name.name.as_str())
                                }
                                ppl_syntax::ast::Item::Globals {
                                    exported: true,
                                    vars,
                                    ..
                                } => vars
                                    .iter()
                                    .any(|v| provisional.contains(v.name.name.as_str())),
                                _ => false,
                            })
                    })
        });
        if (compiled.is_err() || app_export) && (!self.programs.is_empty() || !catalog.is_empty()) {
            let root = "<native-home>";
            let mut sources = self
                .programs
                .iter()
                .map(|(name, source)| {
                    (
                        name.clone(),
                        crate::ModuleSource {
                            identity: SourceIdentity::new(name, source),
                            source: source.clone(),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();
            for (name, module) in catalog {
                if !sources
                    .values()
                    .any(|installed| installed.source == module.source)
                {
                    sources.insert(name.clone(), module.clone());
                }
            }
            let source = format!("EXPORT __HOME() BEGIN {text}; END;");
            sources.insert(
                root.into(),
                crate::ModuleSource {
                    identity: SourceIdentity::new(root, text),
                    source,
                },
            );
            let names = self
                .globals
                .keys()
                .chain(self.cas.bindings.keys())
                .filter(|name| !ppl_compiler::is_system_variable(name))
                .cloned()
                .collect();
            match crate::linker::link_home_sources_with_width(
                &sources,
                root,
                self.settings.int_base,
                self.settings.integer_width_code(),
                &names,
            ) {
                Ok(image) => {
                    let names = image
                        .program()
                        .globals
                        .iter()
                        .filter(|slot| !slot.program_var)
                        .map(|slot| slot.name.clone())
                        .collect::<Vec<_>>();
                    let prepared = Instant::now();
                    let remaining = match self.prepare_home_cas(&names, host.as_ref(), limits) {
                        Ok(remaining) => remaining,
                        Err(error) => {
                            return SessionRun {
                                result: Err(error),
                                steps: 0,
                                elapsed: prepared.elapsed(),
                            };
                        }
                    };
                    return self.run_linked(image, host, remaining, true, |machine| {
                        machine.call_function("__HOME", Vec::new())
                    });
                }
                Err(error) => {
                    self.record_failure();
                    return SessionRun {
                        result: Err(error),
                        steps: 0,
                        elapsed: Duration::ZERO,
                    };
                }
            }
        }
        match compiled {
            Ok(prog) => {
                let names = prog
                    .globals
                    .iter()
                    .filter(|slot| !slot.program_var)
                    .map(|slot| slot.name.clone())
                    .collect::<Vec<_>>();
                let prepared = Instant::now();
                let remaining = match self.prepare_home_cas(&names, host.as_ref(), limits) {
                    Ok(remaining) => remaining,
                    Err(error) => {
                        return SessionRun {
                            result: Err(error),
                            steps: 0,
                            elapsed: prepared.elapsed(),
                        };
                    }
                };
                self.run(
                    SourceIdentity::new("<home>", text),
                    Rc::new(prog),
                    host,
                    remaining,
                    true,
                    |machine| {
                        machine.seed_expression_catalog(catalog)?;
                        machine.call_function("__HOME", Vec::new())
                    },
                )
            }
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

    fn prepare_home_cas(
        &mut self,
        names: &[String],
        host: &dyn Host,
        mut limits: Limits,
    ) -> RtResult<Limits> {
        self.cas_home_values.clear();
        let started = Instant::now();
        let mut resolver = self.cas.clone();
        let cancel = host.interrupt_flag();
        let fallback = std::sync::atomic::AtomicBool::new(false);
        for name in names {
            if self.cas.bindings.contains_key(name) && !self.globals.contains_key(name) {
                let mut options = ppl_cas::Options::default();
                if let Some(limit) = limits.max_time {
                    options.timeout = options.timeout.min(limit.saturating_sub(started.elapsed()));
                }
                match resolver.evaluate(name, &options, cancel.as_deref().unwrap_or(&fallback)) {
                    Ok(value) => {
                        self.cas_home_values.insert(
                            name.clone(),
                            PortableValue::from(&crate::cas::from_cas_home_in_model(
                                value,
                                self.settings.numeric_version,
                            )),
                        );
                    }
                    Err(error) => {
                        self.cas_home_values.clear();
                        self.record_failure();
                        return Err(RuntimeError::new(
                            match error {
                                ppl_cas::CasError::Cancelled => ErrorKind::Interrupted,
                                ppl_cas::CasError::TimedOut => {
                                    ErrorKind::ResourceLimit(crate::ResourceKind::Time)
                                }
                                _ => ErrorKind::InvalidInput,
                            },
                            error.to_string(),
                        ));
                    }
                }
            }
        }
        let elapsed = started.elapsed();
        limits.max_time = limits.max_time.map(|limit| limit.saturating_sub(elapsed));
        Ok(limits)
    }

    /// Evaluate an argument in the calculator environment without publishing
    /// an extra operation or promoting its temporary result to Home Ans.
    pub fn evaluate_argument(
        &mut self,
        text: &str,
        host: Box<dyn Host>,
        limits: Limits,
    ) -> RtResult<Value> {
        if let Some(source) = ppl_cas::units::translate_home(text)
            .map_err(|e| RuntimeError::new(ErrorKind::InvalidInput, e.to_string()))?
        {
            return self
                .evaluate_measurement(&source, host, limits, false)
                .result;
        }
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
                default_int_width: self.settings.integer_width_code(),
                external_variables: self
                    .globals
                    .keys()
                    .chain(self.cas.bindings.keys())
                    .filter(|name| !ppl_compiler::is_system_variable(name))
                    .cloned()
                    .collect(),
                ..Default::default()
            },
        );
        program
            .map(|mut program| {
                // Home's external names are calculator memory, not linked module slots.
                for slot in &mut program.globals {
                    if slot.external {
                        slot.external = false;
                        slot.program_var = false;
                    }
                }
                program
            })
            .ok_or_else(|| {
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
