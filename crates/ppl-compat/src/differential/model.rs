use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: u32 = 2;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendId {
    Native,
    FirmwareHle,
    PhysicalCapture,
    FullSystem,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub source: Option<String>,
    pub file: Option<PathBuf>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InitialState {
    /// Supported settings: HAngle, HFormat, HDigits. Others are rejected.
    pub settings: BTreeMap<String, i64>,
    /// Authored PPL setup routine. No host memory/state injection.
    pub setup_entry: Option<String>,
    pub seed: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Constraints {
    pub firmware_build: Option<u32>,
    pub firmware_sha256: Option<String>,
    pub optional: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Checkpoint {
        id: String,
        wake: bool,
    },
    Start {
        id: String,
        program: String,
        entry: String,
        #[serde(default)]
        args: Vec<Value>,
    },
    Await {
        id: String,
        run: String,
        timeout_ms: u64,
    },
    Import {
        id: String,
        program: String,
    },
    RunProgram {
        id: String,
        program: String,
        entry: String,
        #[serde(default)]
        args: Vec<Value>,
    },
    Probe {
        id: String,
        program: String,
        entry: String,
        names: Vec<String>,
    },
    Edit {
        id: String,
        program: String,
        source: String,
    },
    Check {
        id: String,
    },
    Run {
        id: String,
        entry: String,
        #[serde(default)]
        args: Vec<Value>,
    },
    /// Variable observations after a run, including program-local globals.
    Observe {
        id: String,
        names: Vec<String>,
    },
    Capture {
        id: String,
    },
    Key {
        id: String,
        code: i64,
        down: bool,
    },
    Touch {
        id: String,
        status: u32,
        x: u16,
        y: u16,
    },
    PowerOff {
        id: String,
    },
    Restart {
        id: String,
    },
}
impl Step {
    pub fn id(&self) -> &str {
        match self {
            Self::Checkpoint { id, .. }
            | Self::Start { id, .. }
            | Self::Await { id, .. }
            | Self::Import { id, .. }
            | Self::RunProgram { id, .. }
            | Self::Probe { id, .. }
            | Self::Edit { id, .. }
            | Self::Check { id }
            | Self::Run { id, .. }
            | Self::Observe { id, .. }
            | Self::Capture { id }
            | Self::Key { id, .. }
            | Self::Touch { id, .. }
            | Self::PowerOff { id }
            | Self::Restart { id } => id,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub step: String,
    pub path: String,
    pub value: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub program: Program,
    #[serde(default)]
    pub programs: BTreeMap<String, Program>,
    #[serde(default)]
    pub initial: InitialState,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub expect: Vec<Expectation>,
    #[serde(default)]
    pub constraints: BTreeMap<BackendId, Constraints>,
}
#[derive(Debug, Clone)]
pub struct LoadedScenario {
    pub scenario: Scenario,
    pub source: String,
    pub sources: BTreeMap<String, String>,
    pub fixture_sha256: String,
    pub source_sha256: String,
    pub root: PathBuf,
    content_identity: String,
}
impl LoadedScenario {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let scenario: Scenario =
            toml::from_str(text).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_spec(
            scenario,
            path.parent().unwrap_or(Path::new(".")),
            crate::sha256_hex(&bytes),
        )
    }
    pub fn from_spec(
        mut scenario: Scenario,
        root: &Path,
        fixture_sha256: String,
    ) -> Result<Self, String> {
        if ![1, SCHEMA_VERSION].contains(&scenario.schema_version) {
            return Err(format!(
                "unsupported differential schema {}",
                scenario.schema_version
            ));
        }
        if scenario.id.is_empty() || scenario.steps.is_empty() {
            return Err("scenario id and steps must be nonempty".into());
        }
        if scenario.schema_version == 1
            && scenario
                .steps
                .iter()
                .any(|s| matches!(s, Step::Checkpoint { .. }))
        {
            return Err("checkpoint requires scenario schema 2".into());
        }
        let mut ids = BTreeSet::new();
        for step in &scenario.steps {
            if step.id().is_empty() || !ids.insert(step.id()) {
                return Err("step ids must be nonempty and unique".into());
            }
            match step {
                Step::Touch { status, x, y, .. }
                    if ![1, 2, 8].contains(status) || *x >= 320 || *y >= 240 =>
                {
                    return Err("touch sample outside G1 screen/status range".into());
                }
                Step::Key { code, .. } if !(0..=50).contains(code) => {
                    return Err("keys use HP physical codes 0..50".into());
                }
                Step::Run { entry, .. } if entry.is_empty() => {
                    return Err("run entry must be nonempty".into());
                }
                Step::Run { args, .. } if args.iter().any(contains_float) => {
                    return Err("floating-point fixture arguments lose decimal precision; use {type=\"real\",value=\"1.25\"}".into());
                }
                Step::Observe { names, .. } if names.is_empty() => {
                    return Err("observe names must be nonempty".into());
                }
                _ => {}
            }
        }
        for e in &scenario.expect {
            if !ids.contains(e.step.as_str()) || (!e.path.is_empty() && !e.path.starts_with('/')) {
                return Err("expectation requires an existing step and JSON pointer path".into());
            }
        }
        for (name, v) in &scenario.initial.settings {
            let valid = match name.as_str() {
                "HAngle" => (0..=2).contains(v),
                "HFormat" => (0..=3).contains(v),
                "HDigits" => (0..=11).contains(v),
                _ => false,
            };
            if !valid {
                return Err(format!("unsupported initial setting {name}={v}"));
            }
        }
        if scenario.schema_version == 1 && !scenario.programs.is_empty() {
            return Err("schema 1 supports one program".into());
        }
        let mut sources = BTreeMap::new();
        let mut folded_names = BTreeSet::new();
        for (name, program) in &scenario.programs {
            if !folded_names.insert(name.to_ascii_uppercase())
                || [
                    "ZZT_IDE",
                    "ZZT_IMPORT",
                    "ZZT_LOADER",
                    "ZZT_NAMED",
                    "ZZT_DFSETUP",
                    "ZZT_DFDEFAULT",
                ]
                .contains(&name.to_ascii_uppercase().as_str())
            {
                return Err(format!(
                    "Reserved or case-ambiguous scenario program {name}"
                ));
            }
            if !name.starts_with("ZZT_")
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || name.len() > 31
            {
                return Err(format!(
                    "scenario program requires a disposable ZZT_ identifier: {name}"
                ));
            }
            let text = match (&program.source, &program.file) {
                (Some(s), None) => s.clone(),
                (None, Some(p)) => ppl_engine::source::Source::file(&root.join(p))?.text,
                _ => {
                    return Err(format!(
                        "program {name} requires exactly one source or file"
                    ));
                }
            };
            sources.insert(
                name.clone(),
                ppl_engine::source::Source::text(&text, name)?.text,
            );
        }
        for step in &mut scenario.steps {
            if let Step::Edit {
                source, program, ..
            } = step
            {
                *source = ppl_engine::source::Source::text(source, program)?.text;
            }
        }
        for step in &scenario.steps {
            let name = match step {
                Step::Start { program, .. }
                | Step::Import { program, .. }
                | Step::RunProgram { program, .. }
                | Step::Probe { program, .. }
                | Step::Edit { program, .. } => Some(program),
                _ => None,
            };
            if name.is_some_and(|n| !sources.contains_key(n)) {
                return Err("step references an undeclared program".into());
            }
            if scenario.schema_version == 1 && name.is_some() {
                return Err("named steps require schema 2".into());
            }
            match step {
                Step::Start { entry, args, .. } | Step::RunProgram { entry, args, .. }
                    if entry.is_empty() || args.iter().any(contains_float) =>
                {
                    return Err("run_program needs an entry and exact typed arguments".into());
                }
                Step::Probe { entry, names, .. }
                    if entry.is_empty()
                        || names.is_empty()
                        || names.iter().collect::<BTreeSet<_>>().len() != names.len() =>
                {
                    return Err("probe needs an entry and unique nonempty names".into());
                }
                Step::Await {
                    run, timeout_ms, ..
                } if !scenario
                    .steps
                    .iter()
                    .any(|s| matches!(s,Step::Start{id,..} if id==run))
                    || *timeout_ms == 0
                    || *timeout_ms > 600_000 =>
                {
                    return Err("await requires a start id and bounded timeout 1..600000ms".into());
                }
                _ => {}
            }
        }
        let source = match (&scenario.program.source, &scenario.program.file) {
            (Some(s), None) => s.clone(),
            (None, Some(p)) => ppl_engine::source::Source::file(&root.join(p))?.text,
            (None, None) if scenario.schema_version == 2 && !sources.is_empty() => String::new(),
            _ => return Err("program requires exactly one of source or file".into()),
        };
        let source = ppl_engine::source::Source::text(&source, &scenario.id)?.text;
        let source_sha256 = if scenario.schema_version == 1 {
            crate::sha256_hex(source.as_bytes())
        } else {
            crate::sha256_hex(&serde_json::to_vec(&manifest(&scenario, &source, &sources)).unwrap())
        };
        let content_identity = identity_hash(
            &scenario,
            &source,
            &sources,
            &fixture_sha256,
            &source_sha256,
        );
        Ok(Self {
            scenario,
            source,
            sources,
            fixture_sha256,
            source_sha256,
            root: root.to_owned(),
            content_identity,
        })
    }
    /// Source, setup, expectations and constraints remain bound after loading.
    pub fn verify_identity(&self) -> Result<(), String> {
        if self.content_identity
            != identity_hash(
                &self.scenario,
                &self.source,
                &self.sources,
                &self.fixture_sha256,
                &self.source_sha256,
            )
        {
            return Err(
                "loaded scenario changed after identity validation; reload it before execution"
                    .into(),
            );
        }
        Ok(())
    }
    /// Exact initial and edited source identities; v2's top-level digest covers this manifest.
    pub fn source_manifest(&self) -> BTreeMap<String, String> {
        manifest(&self.scenario, &self.source, &self.sources)
    }
    pub fn optional(&self, backend: BackendId) -> bool {
        self.scenario
            .constraints
            .get(&backend)
            .is_some_and(|c| c.optional)
    }
}
fn manifest(
    spec: &Scenario,
    default: &str,
    sources: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut hashes: BTreeMap<_, _> = sources
        .iter()
        .map(|(n, s)| (n.clone(), crate::sha256_hex(s.as_bytes())))
        .collect();
    if !default.is_empty() {
        hashes.insert("default".into(), crate::sha256_hex(default.as_bytes()));
    }
    for step in &spec.steps {
        if let Step::Edit {
            id,
            program,
            source,
        } = step
        {
            hashes.insert(
                format!("{program}@{id}"),
                crate::sha256_hex(source.as_bytes()),
            );
        }
    }
    hashes
}
fn contains_float(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_f64(),
        Value::Array(a) => a.iter().any(contains_float),
        Value::Object(o) => o.values().any(contains_float),
        _ => false,
    }
}
fn identity_hash(
    spec: &Scenario,
    source: &str,
    sources: &BTreeMap<String, String>,
    fixture: &str,
    source_hash: &str,
) -> String {
    crate::sha256_hex(
        &serde_json::to_vec(&(spec, source, sources, fixture, source_hash))
            .expect("scenario JSON serialization"),
    )
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Complete,
    Unsupported,
    Unavailable,
    InfrastructureError,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub origin: String,
    pub source_sha256: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub program_sources: BTreeMap<String, String>,
    pub fixture_sha256: String,
    pub firmware_build: Option<u32>,
    pub firmware_sha256: Option<String>,
    pub capture_sha256: Option<String>,
    pub evidence_scope: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub step: String,
    pub data: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendRun {
    pub backend: BackendId,
    pub status: RunStatus,
    pub reason: Option<String>,
    pub provenance: Provenance,
    pub observations: Vec<Observation>,
    /// Unmodified structured engine reports, not just a success message.
    pub raw_reports: Vec<Value>,
}
impl BackendRun {
    pub fn new(backend: BackendId, s: &LoadedScenario) -> Self {
        Self {
            backend,
            status: RunStatus::Complete,
            reason: None,
            provenance: Provenance {
                origin: match backend {
                    BackendId::Native => "native",
                    BackendId::FirmwareHle => "local-firmware",
                    BackendId::PhysicalCapture => "physical-capture",
                    BackendId::FullSystem => "unavailable",
                }
                .into(),
                source_sha256: s.source_sha256.clone(),
                program_sources: if s.scenario.schema_version == 2 {
                    s.source_manifest()
                } else {
                    BTreeMap::new()
                },
                fixture_sha256: s.fixture_sha256.clone(),
                firmware_build: None,
                firmware_sha256: None,
                capture_sha256: None,
                evidence_scope: "declared-scenario-observations".into(),
            },
            observations: vec![],
            raw_reports: vec![],
        }
    }
    pub fn unavailable(backend: BackendId, s: &LoadedScenario, reason: impl Into<String>) -> Self {
        let mut r = Self::new(backend, s);
        r.status = RunStatus::Unavailable;
        r.reason = Some(reason.into());
        r
    }
    pub fn unsupported(backend: BackendId, s: &LoadedScenario, reason: impl Into<String>) -> Self {
        let mut r = Self::new(backend, s);
        r.status = RunStatus::Unsupported;
        r.reason = Some(reason.into());
        r
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}
impl From<&ppl_graphics::Grob> for Image {
    fn from(g: &ppl_graphics::Grob) -> Self {
        Self {
            width: g.width(),
            height: g.height(),
            pixels: g.pixels().to_vec(),
        }
    }
}
