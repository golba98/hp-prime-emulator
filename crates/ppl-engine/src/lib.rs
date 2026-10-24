//! Engine integration without GUI, USB, Unicorn, or global signal handlers.
pub mod arguments;
#[cfg(target_os = "linux")]
pub mod firmware;
#[cfg(not(target_os = "linux"))]
#[path = "firmware_unsupported.rs"]
pub mod firmware;
mod firmware_options;
pub mod native;
pub mod source;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Native,
    Firmware,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Check,
    Run,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Success,
    InvalidSource,
    InvalidRequest,
    RuntimeError,
    TimedOut,
    IoError,
    InvalidEntry,
    InvalidArguments,
    Unsupported,
    GuestFault,
    InfrastructureError,
    MissingFirmware,
    MissingResources,
    MissingRunner,
    Cancelled,
}
impl Status {
    pub fn firmware_exit(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::InvalidSource => 1,
            Self::InvalidRequest => 2,
            Self::RuntimeError => 3,
            Self::TimedOut => 4,
            Self::IoError => 5,
            Self::InvalidEntry => 6,
            Self::InvalidArguments => 7,
            Self::Unsupported => 8,
            Self::GuestFault => 9,
            Self::InfrastructureError => 10,
            Self::MissingFirmware => 11,
            Self::MissingResources => 12,
            Self::MissingRunner => 13,
            Self::Cancelled => 130,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct Cancellation(pub Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub source: String,
    pub severity: String,
    pub code: Option<String>,
    pub message: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub raw_position: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEvent {
    pub kind: String,
    pub text: Option<String>,
    pub value: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineError {
    pub kind: Status,
    pub phase: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub request_id: Option<String>,
    pub engine: Option<Engine>,
    pub requested_engine: Option<String>,
    pub operation: Operation,
    pub program: String,
    pub source: Option<source::SourceIdentity>,
    pub requested_entry: Option<String>,
    pub entry: Option<String>,
    pub arguments: Value,
    pub compilation_status: Option<String>,
    pub execution_status: Option<String>,
    pub status: Status,
    pub exit_code: u8,
    pub diagnostics: Vec<Diagnostic>,
    pub runtime_errors: Vec<Diagnostic>,
    pub output: Vec<OutputEvent>,
    pub return_value: Option<Value>,
    pub error: Option<EngineError>,
    pub timings: Value,
    pub artifacts: Vec<PathBuf>,
    pub work_retained: bool,
    pub backend: Value,
}
impl Report {
    pub fn new(engine: Engine, operation: Operation) -> Self {
        Self {
            schema_version: 1,
            request_id: None,
            engine: Some(engine),
            requested_engine: None,
            operation,
            program: String::new(),
            source: None,
            requested_entry: None,
            entry: None,
            arguments: Value::Null,
            compilation_status: None,
            execution_status: None,
            status: Status::InfrastructureError,
            exit_code: 10,
            diagnostics: vec![],
            runtime_errors: vec![],
            output: vec![],
            return_value: None,
            error: None,
            timings: serde_json::json!({}),
            artifacts: vec![],
            work_retained: false,
            backend: Value::Null,
        }
    }
    pub fn fail(&mut self, status: Status, phase: &str, message: impl Into<String>) {
        self.status = status;
        self.exit_code = if self.engine == Some(Engine::Firmware) {
            status.firmware_exit()
        } else {
            match status {
                Status::InvalidSource => 1,
                Status::InvalidEntry | Status::InvalidArguments | Status::InvalidRequest => 2,
                Status::RuntimeError | Status::Unsupported => 3,
                Status::TimedOut | Status::Cancelled => 4,
                _ => 5,
            }
        };
        self.error = Some(EngineError {
            kind: status,
            phase: phase.into(),
            message: message.into(),
        });
    }
}
