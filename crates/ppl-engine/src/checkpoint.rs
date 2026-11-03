//! Portable receipts for firmware-persistence checkpoints.
//!
//! These checkpoints contain files saved by the calculator's own OFF path.
//! They are not suspended-machine images and never contain host pointers or
//! serialized Unicorn state.
use serde::{Deserialize, Serialize};

pub const RECEIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    FirmwarePersistence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSafePoint {
    FirmwareOff,
}

/// Identifies one atomically published, integrity-checked recovery generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointReceipt {
    pub version: u32,
    pub kind: CheckpointKind,
    pub safe_point: CheckpointSafePoint,
    pub generation: String,
    pub firmware: String,
    pub manifest_sha256: String,
    /// Guest file payload bytes; excludes the small manifest files.
    pub bytes: u64,
    pub files: u64,
    /// Explicitly false: this format never restores a running CPU.
    pub restores_cpu: bool,
    /// Explicitly false: firmware redraws after boot; a PNG is not machine state.
    pub restores_framebuffer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckpointResult {
    Saved {
        receipt: CheckpointReceipt,
    },
    /// No OFF sequence was sent. The caller may retry after resolving `reason`.
    Deferred {
        reason: String,
    },
    /// No success is claimed. Previously published generations are retained.
    Failed {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_identifies_partial_state_and_request_failure() {
        let receipt = CheckpointReceipt {
            version: RECEIPT_VERSION,
            kind: CheckpointKind::FirmwarePersistence,
            safe_point: CheckpointSafePoint::FirmwareOff,
            generation: "generation-0001".into(),
            firmware: "firmware-hash".into(),
            manifest_sha256: "manifest-hash".into(),
            bytes: 12,
            files: 1,
            restores_cpu: false,
            restores_framebuffer: false,
        };
        let result = CheckpointResult::Saved { receipt };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "saved");
        assert_eq!(json["receipt"]["kind"], "firmware_persistence");
        assert_eq!(json["receipt"]["safe_point"], "firmware_off");
        assert_eq!(json["receipt"]["restores_cpu"], false);
        assert_eq!(json["receipt"]["restores_framebuffer"], false);
        assert_eq!(
            serde_json::from_value::<CheckpointResult>(json).unwrap(),
            result
        );
        let deferred = CheckpointResult::Deferred {
            reason: "running".into(),
        };
        assert_eq!(
            serde_json::to_value(deferred).unwrap()["status"],
            "deferred"
        );
    }
}
