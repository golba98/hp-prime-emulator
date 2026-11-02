//! Strict, backend-neutral scenarios. This API does not perform USB operations.
//! Legacy Suite/EvidenceV2 remain separate and retain their recorded scope.
mod backend;
mod compare;
mod firmware;
mod model;
mod native;
mod replay;
pub use backend::{Backend, FirmwareHle, Native, Unavailable, execute};
pub use compare::{Comparison, ImageDifference, Mismatch, compare, compare_images};
pub use firmware::run_firmware;
pub use model::*;
pub use native::run_native;
pub use replay::{LegacyReplayReport, PhysicalReplay, load_physical_replay, replay_legacy_cases};
