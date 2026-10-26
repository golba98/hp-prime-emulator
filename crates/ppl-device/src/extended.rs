//! Offline negotiated-mode framing, independently compared with official x86.
//!
//! This codec deliberately has no Transport access or negotiation entry point.
//! Framing evidence alone cannot enable physical uploads: negotiation, receive
//! flow control and device acceptance still need independent verification.

use crate::{DeviceError, Result};

impl crate::Prime {
    /// Bounded official transport negotiation probe. No application command or
    /// file mutation is sent. This handle is retired afterwards because legacy
    /// framing cannot safely resume on a peer that may have switched modes.
    pub fn probe_extended_transport(&mut self) -> Result<Vec<u8>> {
        self.require_healthy()?;
        let info = self.info()?;
        if self.detected.product_id != 0x1541 || info.build != Some(15157) {
            return Err(DeviceError::Refused(
                "extended probe requires reference G1 build 15157".into(),
            ));
        }
        self.drain();
        self.retired = Some("official negotiation probe changed or may have changed transport mode; close this handle".into());
        // Official PE32 4c9dd9..4c9dec: 8 transport bytes; hidapi additionally
        // requires report ID zero. This EC is not a framed application key.
        self.write_report(&[0, 0xff, 0xec, 0, 0, 0, 0, 0, 0])?;
        if let Some(report) = self.read_report(std::time::Duration::from_secs(3))? {
            if report.first() == Some(&0xff) && report.len() >= 8 {
                return Ok(report);
            }
            return Err(DeviceError::Inconclusive(
                "unrelated report during extended negotiation; no application operation sent"
                    .into(),
            ));
        }
        Err(DeviceError::Timeout("official extended negotiation"))
    }
}

/// Significant bytes only. A physical HID writer would need a separately
/// validated padding policy and its platform's report-ID prefix.
pub fn frames(message_id: u32, message: &[u8]) -> Result<Vec<Vec<u8>>> {
    let length = u32::try_from(message.len())
        .map_err(|_| DeviceError::Protocol("extended message exceeds u32 length".into()))?;
    if length == 0 {
        return Err(DeviceError::Protocol("empty extended message".into()));
    }
    let first_len = message.len().min(55);
    let mut first = vec![1];
    first.extend_from_slice(&message_id.to_le_bytes());
    first.extend_from_slice(&length.to_le_bytes());
    first.extend_from_slice(&message[..first_len]);
    let mut reports = vec![first];
    let mut sequence = 2;
    for chunk in message[first_len..].chunks(63) {
        let mut report = vec![sequence];
        report.extend_from_slice(chunk);
        reports.push(report);
        sequence = if sequence == 0xfd { 2 } else { sequence + 1 };
    }
    Ok(reports)
}

/// Mode-1 transport acknowledgement. Its ID identifies a framed message, not
/// an application operation, object, successful upload, or deletion.
#[derive(Debug, PartialEq, Eq)]
pub struct Acknowledgement {
    pub message_id: u32,
    pub position: u32,
    pub restart_sequence: Option<u8>,
}

impl Acknowledgement {
    /// Decode offsets confirmed at official PE32 4c9f9e..4c9fd9. Reject unknown
    /// control classes instead of treating them as progress.
    pub fn parse(report: &[u8]) -> Result<Self> {
        if report.len() < 12 || report.len() > 64 || report[0] != 0xfe {
            return Err(DeviceError::Protocol(
                "invalid mode-1 acknowledgement".into(),
            ));
        }
        let restart_sequence = match report[1] {
            0 if (2..=0xfd).contains(&report[2]) => Some(report[2]),
            0 if report[2] == 0 => None,
            1 => None,
            _ => {
                return Err(DeviceError::Protocol(
                    "unsupported mode-1 ACK control".into(),
                ));
            }
        };
        Ok(Self {
            position: u32::from_le_bytes(report[4..8].try_into().unwrap()),
            message_id: u32::from_le_bytes(report[8..12].try_into().unwrap()),
            restart_sequence,
        })
    }

    /// Conservative correlation for research callers. A stale/future ID or
    /// unsent offset must never be accepted as completion of this message.
    pub fn validates_progress(&self, message_id: u32, sent: u32) -> bool {
        self.message_id == message_id && self.position <= sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn matches_actual_official_instructions_across_wrap_and_large_objects() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/ck-mode1-oracle.json")).unwrap();
        assert_eq!(fixture["hardware_access"], false);
        for case in fixture["cases"].as_array().unwrap() {
            let size = case["size"].as_u64().unwrap() as usize;
            let payload: Vec<_> = (0..size).map(|i| (i * 37 + 11) as u8).collect();
            let reports = frames(case["message_id"].as_u64().unwrap() as u32, &payload).unwrap();
            assert_eq!(reports.len(), case["reports"].as_u64().unwrap() as usize);
            let mut hash = Sha256::new();
            for report in &reports {
                hash.update((report.len() as u32).to_le_bytes());
                hash.update(report);
            }
            assert_eq!(
                format!("{:x}", hash.finalize()),
                case["canonical_sha256"].as_str().unwrap()
            );
            assert!(reports.iter().skip(1).all(|r| (2..=0xfd).contains(&r[0])));
        }
    }

    #[test]
    fn ack_identity_and_position_cannot_complete_another_message() {
        let ack = Acknowledgement::parse(&[0xfe, 1, 0, 0, 100, 0, 0, 0, 7, 0, 0, 0]).unwrap();
        assert!(ack.validates_progress(7, 100));
        assert!(!ack.validates_progress(6, 100));
        assert!(!ack.validates_progress(8, 100));
        assert!(!ack.validates_progress(7, 99));
        assert_eq!(ack.restart_sequence, None);
        assert!(Acknowledgement::parse(&[0xfe; 11]).is_err());
        assert!(Acknowledgement::parse(&[0xfe, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(frames(1, &[]).is_err());
    }
}
