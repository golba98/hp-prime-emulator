//! Portable, non-resumable observations. No host pointers or Unicorn contexts.
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerPhase {
    Boundary,
    Executing,
    SvcEnter,
    SvcBlocked,
    SvcExit,
    Exited,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerObservation {
    pub thread_id: u32,
    pub phase: OwnerPhase,
    pub service_id: Option<u32>,
    pub service_name: Option<String>,
    pub caller_pc: Option<u32>,
    pub blocked_handle: Option<u32>,
    pub entered_ms: Option<f64>,
    pub completed_ms: Option<f64>,
    pub acknowledged_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotReadiness {
    pub memory_gate_acquired: bool,
    pub owners_paused: bool,
    pub active_engines: usize,
    pub parked_native_callbacks: usize,
    pub live_restore_ready: bool,
    pub blockers: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInspection {
    pub version: u32,
    pub resource_identity: Option<String>,
    pub lifecycle: crate::session::Lifecycle,
    pub readiness: SnapshotReadiness,
    pub owners: Vec<OwnerObservation>,
    pub resources: serde_json::Value,
    pub diagnostics: serde_json::Value,
}
