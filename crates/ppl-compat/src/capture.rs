//! Offline, immutable scenario captures. This module never opens a device.
//!
//! A physical binding may implement [`CaptureReader`] using read-only commands.
//! Program transfer and key delivery remain in the existing guarded/manual flow.
//! Checksums detect damaged evidence; they do not authenticate its author.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::Instant,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    DeviceRecord,
    differential::{
        self, BackendId, BackendRun, Comparison, LoadedScenario, Observation, Provenance, RunStatus,
    },
    sha256_hex,
};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ARTIFACTS: usize = 256;
pub const MAX_STEPS: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedStep {
    pub id: String,
    /// The exact shared-scenario action, including key/touch parameters.
    pub action: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePlan {
    pub schema_version: u32,
    pub capture_id: String,
    pub scenario_id: String,
    pub fixture_sha256: String,
    pub source_sha256: String,
    /// Resolved source; file-based scenarios need no later source-file lookup.
    pub source: String,
    /// Full shared specification, including settings, expectations and constraints.
    pub scenario: Value,
    pub steps: Vec<PlannedStep>,
}

impl CapturePlan {
    pub fn prepare(s: &LoadedScenario, capture_id: &str) -> Result<Self, String> {
        s.verify_identity()?;
        let scenario = serde_json::to_value(&s.scenario).map_err(|e| e.to_string())?;
        let steps = scenario["steps"]
            .as_array()
            .ok_or("scenario has no steps")?
            .iter()
            .map(|action| {
                Ok(PlannedStep {
                    id: action["id"]
                        .as_str()
                        .ok_or("scenario step has no id")?
                        .into(),
                    action: action.clone(),
                })
            })
            .collect::<Result<_, String>>()?;
        let plan = Self {
            schema_version: SCHEMA_VERSION,
            capture_id: capture_id.into(),
            scenario_id: s.scenario.id.clone(),
            fixture_sha256: s.fixture_sha256.clone(),
            source_sha256: s.source_sha256.clone(),
            source: s.source.clone(),
            scenario,
            steps,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported capture plan version".into());
        }
        identifier(&self.capture_id)?;
        if self.scenario_id.is_empty() || self.scenario["id"] != self.scenario_id {
            return Err("capture scenario identity differs".into());
        }
        let _: differential::Scenario = serde_json::from_value(self.scenario.clone())
            .map_err(|e| format!("invalid shared scenario specification: {e}"))?;
        if self.source.len() > MAX_DOCUMENT_BYTES {
            return Err("capture source exceeds 16 MiB".into());
        }
        if let Some(source) = self.scenario["program"]["source"].as_str()
            && source != self.source
        {
            return Err("capture source differs from embedded scenario source".into());
        }
        digest(&self.fixture_sha256)?;
        digest(&self.source_sha256)?;
        if sha256_hex(self.source.as_bytes()) != self.source_sha256 {
            return Err("capture source hash differs".into());
        }
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return Err("capture requires 1..=1024 steps".into());
        }
        let actions = self.scenario["steps"]
            .as_array()
            .ok_or("capture specification has no steps")?;
        if actions.len() != self.steps.len() {
            return Err("capture steps differ from shared scenario".into());
        }
        let mut ids = BTreeSet::new();
        for (step, action) in self.steps.iter().zip(actions) {
            identifier(&step.id)?;
            if !ids.insert(&step.id) || action != &step.action || action["id"] != step.id {
                return Err("duplicate or modified capture step".into());
            }
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, String> {
        self.validate()?;
        json_hash(self)
    }

    pub fn verify_scenario(&self, s: &LoadedScenario) -> Result<(), String> {
        s.verify_identity()?;
        let expected = Self::prepare(s, &self.capture_id)?;
        if self.sha256()? != expected.sha256()? {
            return Err("capture plan differs from the supplied scenario/source".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOrigin {
    /// Explicit human attestation, supported by retained raw artifacts.
    ManualPhysical,
    /// Infrastructure tests; never eligible for a physical verification label.
    ControlledMock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Delivery {
    ManualConfirmed {
        statement: String,
    },
    /// Permitted only for a capture action, which requests no guest input.
    ReadOnlyObservation,
    NotDelivered {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Initialization {
    pub source_sha256: String,
    /// Confirms source import, settings and authored setup in the supplied plan.
    pub statement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Original reassembled FA01 reply (contains the device serial; private).
    RawDeviceInfo,
    /// A JSON [`ObservationTranscript`] fetched verbatim from a result note.
    RawNote,
    /// Conventional RGB/RGBA/grayscale PNG, not HP's packed RGB555 format.
    ScreenshotPng,
    /// The original FC08 packed RGB555 PNG, decoded with the HP-specific decoder.
    PrimeScreenshotPng,
    UserConfirmedTranscript,
    TransportLog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub name: String,
    pub kind: ArtifactKind,
    pub size: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedStep {
    pub step: String,
    pub action_sha256: String,
    pub delivery: Delivery,
    pub elapsed_ms: u64,
    /// Exact shared differential observation, with no display normalization.
    pub data: Value,
    pub artifacts: Vec<String>,
}

/// A source/step-correlated result transcript. For screenshots transcribed by
/// a human, retain the screenshot as an additional artifact and use
/// `UserConfirmedTranscript`; this is an attestation, not automatic OCR.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationTranscript {
    pub schema_version: u32,
    pub capture_id: String,
    pub source_sha256: String,
    pub step: String,
    pub action_sha256: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub schema_version: u32,
    pub capture_id: String,
    pub plan_sha256: String,
    pub origin: CaptureOrigin,
    pub device: DeviceRecord,
    /// UTC RFC3339, e.g. 2026-11-02T12:00:00Z. No local timezone ambiguity.
    pub captured_at_utc: String,
    pub initialization: Initialization,
    /// Distinct FA replies retained before/after a manually confirmed capture.
    #[serde(default)]
    pub identity_artifacts: Vec<String>,
    pub steps: Vec<CapturedStep>,
    pub artifacts: Vec<Artifact>,
}

impl Recording {
    pub fn validate(&self, plan: &CapturePlan) -> Result<(), String> {
        plan.validate()?;
        if self.schema_version != SCHEMA_VERSION
            || self.capture_id != plan.capture_id
            || self.plan_sha256 != plan.sha256()?
        {
            return Err("recording version/capture/plan identity differs".into());
        }
        validate_timestamp(&self.captured_at_utc)?;
        if self.device.model.trim().is_empty() || self.device.firmware_build.is_none() {
            return Err("capture requires model and firmware build".into());
        }
        if self.origin == CaptureOrigin::ManualPhysical {
            if !matches!(self.device.usb_pid.as_str(), "03f0:1541" | "03f0:0441") {
                return Err("physical G1 capture requires a known G1 USB identity".into());
            }
            digest(
                self.device
                    .serial_id
                    .as_deref()
                    .ok_or("physical capture requires a serial fingerprint")?,
            )?;
        }
        if let Some(build) =
            plan.scenario["constraints"]["physical_capture"]["firmware_build"].as_u64()
            && self.device.firmware_build.map(u64::from) != Some(build)
        {
            return Err("physical firmware build violates scenario constraint".into());
        }
        if !plan.scenario["constraints"]["physical_capture"]["firmware_sha256"].is_null() {
            return Err("physical firmware SHA-256 cannot be established by this capture interface; use an observable firmware build constraint".into());
        }
        if self.initialization.source_sha256 != plan.source_sha256
            || self.initialization.statement.trim().is_empty()
        {
            return Err("source import/settings/setup have not been explicitly confirmed".into());
        }
        if self.artifacts.is_empty() || self.artifacts.len() > MAX_ARTIFACTS {
            return Err("capture requires 1..=256 raw artifacts".into());
        }
        let mut names = BTreeSet::new();
        let mut total = 0usize;
        for artifact in &self.artifacts {
            artifact_name(&artifact.name)?;
            digest(&artifact.sha256)?;
            if artifact.size == 0 || artifact.size > MAX_ARTIFACT_BYTES {
                return Err("artifact is empty or exceeds 8 MiB".into());
            }
            if !names.insert(&artifact.name) {
                return Err("duplicate artifact name".into());
            }
            total = total
                .checked_add(artifact.size)
                .ok_or("artifact size overflow")?;
        }
        if total > MAX_TOTAL_ARTIFACT_BYTES {
            return Err("capture artifacts exceed 32 MiB".into());
        }
        if self.steps.len() != plan.steps.len() {
            return Err(
                "incomplete capture: every planned step requires one ordered observation".into(),
            );
        }
        let mut referenced = BTreeSet::new();
        if self.origin == CaptureOrigin::ManualPhysical && self.identity_artifacts.len() < 2 {
            return Err("physical capture requires retained FA identity replies before and after the sequence".into());
        }
        for name in &self.identity_artifacts {
            if !names.contains(name) || !referenced.insert(name) {
                return Err("missing or duplicate raw identity artifact".into());
            }
            if self
                .artifacts
                .iter()
                .find(|a| &a.name == name)
                .map(|a| a.kind)
                != Some(ArtifactKind::RawDeviceInfo)
            {
                return Err("identity artifact must contain a raw FA reply".into());
            }
        }
        for (record, step) in self.steps.iter().zip(&plan.steps) {
            if record.step != step.id || record.action_sha256 != json_hash(&step.action)? {
                return Err(format!("step {}: action/order identity differs", step.id));
            }
            match &record.delivery {
                Delivery::ManualConfirmed { statement } if !statement.trim().is_empty() => {}
                Delivery::ReadOnlyObservation if step.action["action"] == "capture" => {}
                _ => {
                    return Err(format!(
                        "step {}: input was not confirmed delivered",
                        step.id
                    ));
                }
            }
            if !record.data.is_object() || !record.data["status"].is_string() {
                return Err(format!(
                    "step {}: observation requires an explicit status",
                    step.id
                ));
            }
            if record.artifacts.is_empty() {
                return Err(format!("step {}: no retained raw evidence", step.id));
            }
            let mut step_names = BTreeSet::new();
            for name in &record.artifacts {
                if !names.contains(name) || !step_names.insert(name) {
                    return Err(format!(
                        "step {}: missing or duplicate artifact {name}",
                        step.id
                    ));
                }
                referenced.insert(name);
            }
        }
        if referenced.len() != names.len() {
            return Err("capture contains unreferenced artifacts".into());
        }
        Ok(())
    }
}

/// Bytes returned by a read-only collector. They are retained without rewriting.
pub struct ArtifactBytes {
    pub name: String,
    pub kind: ArtifactKind,
    pub bytes: Vec<u8>,
}

pub struct ReadObservation {
    pub data: Value,
    pub artifacts: Vec<ArtifactBytes>,
}

/// No upload, delete, keyboard, memory-write or clock-setting capability.
/// A concrete implementation must retain exclusive ownership of its transport.
pub trait CaptureReader {
    fn origin(&self) -> CaptureOrigin;
    fn identity(&mut self) -> Result<DeviceRecord, String>;
    fn observe(&mut self, step: &PlannedStep) -> Result<ReadObservation, String>;
    /// Retained FA replies from the checked identity reads. A physical binding
    /// must provide at least the first and last; controlled mocks may omit them.
    fn identity_evidence(&mut self) -> Result<Vec<ArtifactBytes>, String> {
        Ok(vec![])
    }
}

/// Human confirmations are sequenced before each observation. Implementations
/// must never turn these callbacks into unapproved physical-device writes.
pub trait ManualOperator {
    fn initialize(&mut self, plan: &CapturePlan) -> Result<Initialization, String>;
    fn confirm(&mut self, step: &PlannedStep) -> Result<Delivery, String>;
}

pub struct PendingCapture {
    pub recording: Recording,
    pub artifacts: BTreeMap<String, Vec<u8>>,
}

/// Run a script with explicit manual delivery and read-only collection. Device
/// identity is checked before initialization and after every observation.
pub fn collect(
    plan: &CapturePlan,
    captured_at_utc: &str,
    operator: &mut dyn ManualOperator,
    reader: &mut dyn CaptureReader,
) -> Result<PendingCapture, String> {
    plan.validate()?;
    validate_timestamp(captured_at_utc)?;
    let device = reader.identity()?;
    let mut recording = Recording {
        schema_version: SCHEMA_VERSION,
        capture_id: plan.capture_id.clone(),
        plan_sha256: plan.sha256()?,
        origin: reader.origin(),
        device: device.clone(),
        captured_at_utc: captured_at_utc.into(),
        initialization: operator.initialize(plan)?,
        identity_artifacts: vec![],
        steps: vec![],
        artifacts: vec![],
    };
    let mut artifacts = BTreeMap::new();
    let mut total = 0usize;
    if recording.initialization.source_sha256 != plan.source_sha256
        || recording.initialization.statement.trim().is_empty()
    {
        return Err("initialization was not confirmed for this source".into());
    }
    for step in &plan.steps {
        let started = Instant::now();
        let delivery = operator.confirm(step)?;
        match &delivery {
            Delivery::ManualConfirmed { statement } if !statement.trim().is_empty() => {}
            Delivery::ReadOnlyObservation if step.action["action"] == "capture" => {}
            _ => {
                return Err(format!(
                    "step {}: input was not confirmed delivered",
                    step.id
                ));
            }
        }
        let observed = reader.observe(step)?;
        if reader.identity()? != device {
            return Err(format!(
                "step {}: device identity changed during capture",
                step.id
            ));
        }
        let mut step_artifacts = vec![];
        for artifact in observed.artifacts {
            artifact_name(&artifact.name)?;
            total = total
                .checked_add(artifact.bytes.len())
                .ok_or("artifact size overflow")?;
            if artifact.bytes.is_empty()
                || artifact.bytes.len() > MAX_ARTIFACT_BYTES
                || total > MAX_TOTAL_ARTIFACT_BYTES
                || artifacts.len() >= MAX_ARTIFACTS
            {
                return Err("capture artifact bounds exceeded".into());
            }
            let meta = Artifact {
                name: artifact.name.clone(),
                kind: artifact.kind,
                size: artifact.bytes.len(),
                sha256: sha256_hex(&artifact.bytes),
            };
            if artifacts
                .insert(artifact.name.clone(), artifact.bytes)
                .is_some()
            {
                return Err("collector returned duplicate artifact name".into());
            }
            recording.artifacts.push(meta);
            step_artifacts.push(artifact.name);
        }
        recording.steps.push(CapturedStep {
            step: step.id.clone(),
            action_sha256: json_hash(&step.action)?,
            delivery,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            data: observed.data,
            artifacts: step_artifacts,
        });
    }
    for artifact in reader.identity_evidence()? {
        artifact_name(&artifact.name)?;
        total = total
            .checked_add(artifact.bytes.len())
            .ok_or("artifact size overflow")?;
        if artifact.kind != ArtifactKind::RawDeviceInfo
            || artifact.bytes.is_empty()
            || artifact.bytes.len() > MAX_ARTIFACT_BYTES
            || total > MAX_TOTAL_ARTIFACT_BYTES
            || artifacts.len() >= MAX_ARTIFACTS
        {
            return Err("invalid or oversized raw identity evidence".into());
        }
        recording.artifacts.push(Artifact {
            name: artifact.name.clone(),
            kind: artifact.kind,
            size: artifact.bytes.len(),
            sha256: sha256_hex(&artifact.bytes),
        });
        recording.identity_artifacts.push(artifact.name.clone());
        if artifacts.insert(artifact.name, artifact.bytes).is_some() {
            return Err("duplicate identity artifact".into());
        }
    }
    recording.validate(plan)?;
    verify_observations(plan, &recording, &artifacts)?;
    Ok(PendingCapture {
        recording,
        artifacts,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    plan: CapturePlan,
    recording: Recording,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema_version: u32,
    payload_sha256: String,
    payload: Payload,
}

/// Validated complete capture. Construction always verifies every retained byte.
pub struct SealedCapture {
    payload: Payload,
    sha256: String,
}

impl SealedCapture {
    pub fn plan(&self) -> &CapturePlan {
        &self.payload.plan
    }
    pub fn recording(&self) -> &Recording {
        &self.payload.recording
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let bytes = read_bounded(&dir.join("capture.json"), MAX_DOCUMENT_BYTES)?;
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if envelope.schema_version != SCHEMA_VERSION
            || json_hash(&envelope.payload)? != envelope.payload_sha256
        {
            return Err("capture envelope version/checksum differs".into());
        }
        envelope
            .payload
            .recording
            .validate(&envelope.payload.plan)?;
        let artifacts = dir.join("artifacts");
        let meta = fs::symlink_metadata(&artifacts).map_err(|e| e.to_string())?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("artifact directory must be a real directory".into());
        }
        let mut retained = BTreeMap::new();
        for artifact in &envelope.payload.recording.artifacts {
            let bytes = read_bounded(&artifacts.join(&artifact.name), MAX_ARTIFACT_BYTES)?;
            verify_artifact(artifact, &bytes)?;
            retained.insert(artifact.name.clone(), bytes);
        }
        verify_observations(
            &envelope.payload.plan,
            &envelope.payload.recording,
            &retained,
        )?;
        Ok(Self {
            payload: envelope.payload,
            sha256: envelope.payload_sha256,
        })
    }

    fn backend_run(&self, s: &LoadedScenario) -> Result<BackendRun, String> {
        self.plan().verify_scenario(s)?;
        let r = self.recording();
        Ok(BackendRun {
            backend: BackendId::PhysicalCapture,
            status: RunStatus::Complete,
            reason: None,
            provenance: Provenance {
                origin: match r.origin {
                    CaptureOrigin::ManualPhysical => "manual-physical-scenario",
                    CaptureOrigin::ControlledMock => "controlled-mock-capture",
                }
                .into(),
                source_sha256: self.plan().source_sha256.clone(),
                fixture_sha256: self.plan().fixture_sha256.clone(),
                firmware_build: r.device.firmware_build,
                firmware_sha256: None,
                capture_sha256: Some(self.sha256.clone()),
                evidence_scope: "exact-declared-scenario-observations".into(),
            },
            observations: r
                .steps
                .iter()
                .map(|step| Observation {
                    step: step.step.clone(),
                    data: step.data.clone(),
                })
                .collect(),
            raw_reports: vec![serde_json::to_value(r).map_err(|e| e.to_string())?],
        })
    }
}

/// Publish into a new directory only. Artifact files are flushed before the
/// final manifest is atomically published. An interrupted directory is invalid
/// and is never reused or silently overwritten.
pub fn seal(
    plan: &CapturePlan,
    recording: &Recording,
    artifact_dir: &Path,
    output: &Path,
) -> Result<SealedCapture, String> {
    recording.validate(plan)?;
    let mut bytes = BTreeMap::new();
    for artifact in &recording.artifacts {
        let data = read_bounded(&artifact_dir.join(&artifact.name), MAX_ARTIFACT_BYTES)?;
        verify_artifact(artifact, &data)?;
        bytes.insert(artifact.name.clone(), data);
    }
    seal_bytes(plan, recording, &bytes, output)
}

pub fn seal_bytes(
    plan: &CapturePlan,
    recording: &Recording,
    artifacts: &BTreeMap<String, Vec<u8>>,
    output: &Path,
) -> Result<SealedCapture, String> {
    recording.validate(plan)?;
    if artifacts.len() != recording.artifacts.len() {
        return Err("supplied artifact set differs from recording".into());
    }
    for artifact in &recording.artifacts {
        verify_artifact(
            artifact,
            artifacts
                .get(&artifact.name)
                .ok_or("missing artifact bytes")?,
        )?;
    }
    verify_observations(plan, recording, artifacts)?;
    let payload = Payload {
        plan: plan.clone(),
        recording: recording.clone(),
    };
    let envelope = Envelope {
        schema_version: SCHEMA_VERSION,
        payload_sha256: json_hash(&payload)?,
        payload,
    };
    let manifest = serde_json::to_vec_pretty(&envelope).map_err(|e| e.to_string())?;
    if manifest.len() > MAX_DOCUMENT_BYTES {
        return Err("capture manifest exceeds 16 MiB".into());
    }
    fs::create_dir(output)
        .map_err(|e| format!("new capture directory {}: {e}", output.display()))?;
    let target = output.join("artifacts");
    fs::create_dir(&target).map_err(|e| e.to_string())?;
    for (name, bytes) in artifacts {
        write_new(&target.join(name), bytes)?;
    }
    sync_dir(&target)?;
    atomic_new(&output.join("capture.json"), &manifest)?;
    sync_dir(output)?;
    if let Some(parent) = output.parent() {
        sync_dir(parent)?;
    }
    SealedCapture::load(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCategory {
    NativeOnly,
    HleVerified,
    FirmwareVersusNative,
    HistoricalPhysicalReplay,
    NewlyPhysicalVerified,
    Untested,
}

#[derive(Debug, Serialize)]
pub struct CaptureComparison {
    pub schema_version: u32,
    pub category: VerificationCategory,
    pub physical_attestation: bool,
    pub comparison: Comparison,
}

/// Mocks are compared to exercise the pipeline but can never qualify physical
/// evidence. Every returned mismatch retains backend, step and JSON pointer.
pub fn compare_capture(
    s: &LoadedScenario,
    capture: &SealedCapture,
    actual: BackendRun,
) -> Result<CaptureComparison, String> {
    if !matches!(
        actual.backend,
        BackendId::Native | BackendId::FirmwareHle | BackendId::FullSystem
    ) {
        return Err("capture comparison requires an executing backend result".into());
    }
    let physical_attestation = capture.recording().origin == CaptureOrigin::ManualPhysical;
    let comparison = differential::compare(s, vec![capture.backend_run(s)?, actual]);
    let category = if comparison.passed && physical_attestation {
        VerificationCategory::NewlyPhysicalVerified
    } else {
        VerificationCategory::Untested
    };
    Ok(CaptureComparison {
        schema_version: SCHEMA_VERSION,
        category,
        physical_attestation,
        comparison,
    })
}

/// Derived labels for ordinary executing-backend comparisons. Physical scope
/// requires [`compare_capture`] or the separately qualified historical replayer.
pub fn classify_execution(comparison: &Comparison) -> VerificationCategory {
    if !comparison.passed {
        return VerificationCategory::Untested;
    }
    let complete = |backend| {
        comparison
            .runs
            .iter()
            .any(|r| r.backend == backend && r.status == RunStatus::Complete)
    };
    match (
        complete(BackendId::Native),
        complete(BackendId::FirmwareHle),
    ) {
        (true, true) => VerificationCategory::FirmwareVersusNative,
        (false, true) => VerificationCategory::HleVerified,
        (true, false) => VerificationCategory::NativeOnly,
        _ => VerificationCategory::Untested,
    }
}

pub fn classify_historical(report: &differential::LegacyReplayReport) -> VerificationCategory {
    if report.passed {
        VerificationCategory::HistoricalPhysicalReplay
    } else {
        VerificationCategory::Untested
    }
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&read_bounded(path, MAX_DOCUMENT_BYTES)?).map_err(|e| e.to_string())
}

pub fn save_plan(plan: &CapturePlan, path: &Path) -> Result<(), String> {
    plan.validate()?;
    let bytes = serde_json::to_vec_pretty(plan).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err("capture plan exceeds 16 MiB".into());
    }
    atomic_new(path, &bytes)
}

fn verify_artifact(artifact: &Artifact, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != artifact.size || sha256_hex(bytes) != artifact.sha256 {
        return Err(format!("artifact {}: size/checksum differs", artifact.name));
    }
    if matches!(
        artifact.kind,
        ArtifactKind::ScreenshotPng | ArtifactKind::PrimeScreenshotPng
    ) {
        // Check the IHDR dimensions before invoking the decoder: a tiny encoded
        // PNG must not request unbounded allocation through its dimensions.
        if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
            return Err(format!("artifact {}: invalid PNG header", artifact.name));
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        if width != 320 || height != 240 {
            return Err(format!(
                "artifact {}: G1 screenshot must be 320x240",
                artifact.name
            ));
        }
        decode_image(artifact.kind, bytes)
            .map_err(|e| format!("artifact {}: {e}", artifact.name))?;
    }
    Ok(())
}

fn decode_image(kind: ArtifactKind, bytes: &[u8]) -> Result<ppl_graphics::Grob, String> {
    if kind == ArtifactKind::PrimeScreenshotPng {
        ppl_graphics::Grob::from_prime_screenshot(bytes)
    } else {
        ppl_graphics::Grob::read_png(bytes)
    }
}

fn verify_observations(
    plan: &CapturePlan,
    recording: &Recording,
    artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    for name in &recording.identity_artifacts {
        verify_raw_identity(
            artifacts.get(name).ok_or("missing raw identity bytes")?,
            &recording.device,
        )?;
    }
    for step in &recording.steps {
        let mut transcript_matches = false;
        let mut image_matches = false;
        for name in &step.artifacts {
            let meta = recording
                .artifacts
                .iter()
                .find(|a| &a.name == name)
                .ok_or("missing artifact metadata")?;
            let bytes = artifacts.get(name).ok_or("missing artifact bytes")?;
            match meta.kind {
                ArtifactKind::RawNote | ArtifactKind::UserConfirmedTranscript => {
                    let transcript: ObservationTranscript = serde_json::from_slice(bytes)
                        .map_err(|e| format!("step {} transcript {name}: {e}", step.step))?;
                    if transcript.schema_version != SCHEMA_VERSION
                        || transcript.capture_id != plan.capture_id
                        || transcript.source_sha256 != plan.source_sha256
                        || transcript.step != step.step
                        || transcript.action_sha256 != step.action_sha256
                        || transcript.data != step.data
                    {
                        return Err(format!(
                            "step {}: stale or mismatched transcript {name}",
                            step.step
                        ));
                    }
                    transcript_matches = true;
                }
                ArtifactKind::ScreenshotPng | ArtifactKind::PrimeScreenshotPng => {
                    if let Some(image) = step.data.get("image") {
                        let decoded = decode_image(meta.kind, bytes)?;
                        let decoded = serde_json::to_value(differential::Image::from(&decoded))
                            .map_err(|e| e.to_string())?;
                        if &decoded != image {
                            return Err(format!(
                                "step {}: observed pixels differ from retained screenshot {name}",
                                step.step
                            ));
                        }
                        image_matches = true;
                    }
                }
                ArtifactKind::TransportLog | ArtifactKind::RawDeviceInfo => {}
            }
        }
        if step.data.get("image").is_some() && !image_matches {
            return Err(format!(
                "step {}: image observation requires matching screenshot pixels",
                step.step
            ));
        }
        // A screenshot can establish only status and pixels. Other values,
        // errors and state require a correlated raw/manual transcript.
        let only_screen = step
            .data
            .as_object()
            .is_some_and(|data| data.keys().all(|k| k == "status" || k == "image"));
        if !(transcript_matches || only_screen && image_matches) {
            return Err(format!(
                "step {}: values/state require a correlated transcript",
                step.step
            ));
        }
    }
    Ok(())
}

fn verify_raw_identity(raw: &[u8], device: &DeviceRecord) -> Result<(), String> {
    // Same confirmed FA layout as ppl-device::CalcInfo::parse. No HID backend
    // dependency is needed for offline CI. Never put the serial in an error.
    if raw.len() < 42 || raw.len() > 4096 || raw[..2] != [0xfa, 0x01] {
        return Err("invalid raw FA identity reply".into());
    }
    let body = &raw[6..];
    let n = body.len();
    let ascii = |start, len| -> Result<String, String> {
        let bytes: Vec<_> = body[start..start + len]
            .iter()
            .copied()
            .take_while(|b| *b != 0)
            .collect();
        if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_graphic) {
            return Err("invalid FA identity fields".into());
        }
        String::from_utf8(bytes).map_err(|_| "invalid FA identity fields".into())
    };
    let build = u32::from(u16::from_le_bytes([body[n - 36], body[n - 35]]));
    let version = ascii(n - 32, 10)?;
    let serial_hash = crate::serial_fingerprint(&ascii(n - 16, 10)?);
    if device.firmware_build != Some(build)
        || device.boot_version.as_deref() != Some(&version)
        || device.serial_id.as_deref() != Some(&serial_hash)
    {
        return Err("raw FA identity differs from capture identity".into());
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err(format!(
            "{}: requires a bounded regular file",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("file grew beyond size limit".into());
    }
    Ok(bytes)
}

fn atomic_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(path)
        .map_err(|e| format!("new file {}: {e}", path.display()))?;
    sync_dir(parent)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn sync_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn identifier(s: &str) -> Result<(), String> {
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(
            "capture/step identifier must contain 1..=128 ASCII letters, digits, _ or -".into(),
        );
    }
    Ok(())
}

fn artifact_name(s: &str) -> Result<(), String> {
    if s.is_empty()
        || s.len() > 128
        || s.starts_with('.')
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err("artifact name must be a safe flat filename".into());
    }
    Ok(())
}

fn digest(s: &str) -> Result<(), String> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("expected lowercase SHA-256 digest".into());
    }
    Ok(())
}

fn validate_timestamp(s: &str) -> Result<(), String> {
    // Capture times are metadata, never compared as calculator semantics.
    let bytes = s.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19) || b.is_ascii_digit())
    {
        return Err("capture time must be YYYY-MM-DDTHH:MM:SSZ".into());
    }
    let number = |start, end| s[start..end].parse::<u32>().unwrap();
    let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        1..=12 => 31,
        _ => 0,
    };
    if year == 0
        || day == 0
        || day > max_day
        || number(11, 13) > 23
        || number(14, 16) > 59
        || number(17, 19) > 59
    {
        return Err("invalid UTC capture date/time".into());
    }
    Ok(())
}

fn json_hash<T: Serialize>(value: &T) -> Result<String, String> {
    Ok(sha256_hex(
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
    ))
}

#[cfg(test)]
mod tests;
