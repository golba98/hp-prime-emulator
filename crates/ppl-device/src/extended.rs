//! Verified official negotiated Mode-1 transport framing and flow control.
//!
//! Reconstructed from official HP Connectivity Kit x86 disassembly and confirmed
//! on physical HP Prime G1 firmware build 15157.

use std::time::{Duration, Instant};

use crate::{DeviceError, Result, TransportMode};

impl crate::Prime {
    /// Returns true if the connected calculator firmware is build 10517 or later on G1,
    /// where official extended transport (Mode 1) is implemented.
    pub fn supports_extended_transport(&mut self) -> Result<bool> {
        let info = self.info()?;
        Ok(self.detected.product_id == 0x1541 && info.build.is_some_and(|b| b >= 10517))
    }

    /// Negotiates Mode 1 on the connected calculator.
    ///
    /// In accordance with official PE32 instructions (4c9dd9..4c9dec) and the calculator
    /// firmware (0x30c5fb24..0x30c5fba4), this transmits an 8-byte control report
    /// `[FF, EC, 0, 0, 0, 0, 0, 0]` padded to full report capacity (65 bytes with report ID 0).
    /// The firmware updates its internal state to Mode 1 without sending a reply report.
    pub fn switch_to_mode1(&mut self) -> Result<()> {
        if self.mode == TransportMode::Mode1 {
            return Ok(());
        }
        self.require_healthy()?;
        self.drain();
        let mut rep = vec![0u8; self.report_size + 1];
        rep[1] = 0xFF;
        rep[2] = 0xEC;
        // rep[8] = 0 selects Mode 1 (1 selects Mode 2)
        self.write_report(&rep)?;
        std::thread::sleep(Duration::from_millis(50));
        self.mode = TransportMode::Mode1;
        self.drain();
        Ok(())
    }

    /// Ensures that the connection is running in Mode 1 if supported, returning an error
    /// if the target hardware or firmware build does not support extended transport.
    pub fn ensure_mode1(&mut self) -> Result<()> {
        if self.mode == TransportMode::Mode1 {
            return Ok(());
        }
        if !self.supports_extended_transport()? {
            return Err(DeviceError::Refused(
                "extended transfer requires HP Prime G1 firmware build 10517 or later".into(),
            ));
        }
        self.switch_to_mode1()
    }

    /// Bounded official transport negotiation probe. Performs Mode 1 switch and
    /// requests `calc.settings` header in Mode 1 to verify bidirectional communication.
    pub fn probe_extended_transport(&mut self) -> Result<Vec<u8>> {
        self.ensure_mode1()?;
        let msg = crate::files::encode_file_message(
            crate::CMD_REQ_FILE,
            crate::FileType::Settings,
            "calc.settings",
            &[],
        );
        self.send_message_mode1(&msg)?;
        let raw =
            self.recv_message_mode1("extended probe calc.settings", Duration::from_secs(3))?;
        Ok(raw)
    }

    /// Sends a complete logical message using official Mode-1 framing and flow control.
    pub(crate) fn send_message_mode1(&mut self, message: &[u8]) -> Result<()> {
        let msg_id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1).max(1);

        let reports = frames(msg_id, message)?;
        let chunk_size = self.report_size - 1;
        let mut window_limit = 240 * chunk_size;
        let mut sent_bytes = 0usize;

        for (i, rep) in reports.iter().enumerate() {
            let chunk_len = if i == 0 {
                message.len().min(55)
            } else {
                (message.len() - sent_bytes).min(chunk_size)
            };

            // Window flow control: if sent_bytes >= window_limit, wait for 0xFE ACK
            while sent_bytes >= window_limit && sent_bytes < message.len() {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut acked = false;
                while Instant::now() < deadline {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if let Some(in_rep) = self.read_report(remaining)? {
                        if in_rep.first() == Some(&0xFE) {
                            if let Ok(ack) = Acknowledgement::parse(&in_rep) {
                                if ack.validates_progress(msg_id, sent_bytes as u32) {
                                    window_limit = (ack.position as usize) + 240 * chunk_size;
                                    acked = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if !acked {
                    return Err(DeviceError::Timeout("mode-1 window acknowledgement"));
                }
            }

            // Write report padded to self.report_size + 1 (report ID 0)
            let mut buf = vec![0u8; self.report_size + 1];
            buf[1..1 + rep.len()].copy_from_slice(rep);
            self.write_report(&buf)?;
            sent_bytes += chunk_len;
        }

        // Wait for final ACK from calculator
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(in_rep) = self.read_report(remaining)? {
                if in_rep.first() == Some(&0xFE) {
                    if let Ok(ack) = Acknowledgement::parse(&in_rep) {
                        if ack.message_id == msg_id && ack.position as usize >= message.len() {
                            return Ok(());
                        }
                    }
                }
            }
        }
        Err(DeviceError::Timeout("mode-1 completion acknowledgement"))
    }

    /// Receives a complete logical message using official Mode-1 framing and flow control.
    pub(crate) fn recv_message_mode1(
        &mut self,
        what: &'static str,
        first: Duration,
    ) -> Result<Vec<u8>> {
        let mut assembled = Vec::new();
        let mut expected_len: Option<usize> = None;
        let mut in_msg_id: Option<u32> = None;
        let chunk_size = self.report_size - 1;
        let mut ack_threshold = 76 * chunk_size;

        let deadline = Instant::now() + first + Duration::from_secs(30);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DeviceError::Timeout(what));
            }
            let timeout = if assembled.is_empty() {
                first
            } else {
                Duration::from_secs(2)
            };

            let rep = match self.read_report(timeout.min(remaining))? {
                Some(r) if !r.is_empty() => r,
                _ if assembled.is_empty() => return Err(DeviceError::Timeout(what)),
                _ => return Err(DeviceError::Timeout("mode-1 stream continuation")),
            };

            let seq = rep[0];
            if seq == 0xFE || seq == 0xFF {
                // Outbound or delayed ACK, or out-of-band report, ignore
                continue;
            }
            if assembled.is_empty() && seq == 0x00 {
                // Calculator answered with standard report framing (seq 0x00)
                return self.recv_message_legacy_from(None, what, first, Some(rep));
            }
            if seq == 0x01 {
                if rep.len() < 9 {
                    return Err(DeviceError::Protocol("short mode-1 header report".into()));
                }
                let mid = u32::from_le_bytes(rep[1..5].try_into().unwrap());
                let total = u32::from_le_bytes(rep[5..9].try_into().unwrap()) as usize;
                in_msg_id = Some(mid);
                expected_len = Some(total);
                let chunk_len = (rep.len() - 9).min(total);
                assembled.extend_from_slice(&rep[9..9 + chunk_len]);
            } else if (2..=0xFD).contains(&seq) {
                let Some(total) = expected_len else {
                    return Err(DeviceError::Protocol(format!(
                        "mode-1 continuation report (seq 0x{seq:02x}) before header"
                    )));
                };
                let needed = total.saturating_sub(assembled.len());
                let chunk_len = (rep.len() - 1).min(needed);
                assembled.extend_from_slice(&rep[1..1 + chunk_len]);

                // Periodic flow-control ACK
                if assembled.len() >= ack_threshold && assembled.len() < total {
                    let mut ack = vec![0u8; self.report_size + 1];
                    ack[1] = 0xFE;
                    ack[2] = 0x01;
                    ack[5..9].copy_from_slice(&(assembled.len() as u32).to_le_bytes());
                    ack[9..13].copy_from_slice(&in_msg_id.unwrap().to_le_bytes());
                    self.write_report(&ack)?;
                    ack_threshold += 76 * chunk_size;
                }
            } else {
                return Err(DeviceError::Protocol(format!(
                    "unexpected report in mode-1 stream: 0x{seq:02x}"
                )));
            }

            if let Some(total) = expected_len {
                if assembled.len() >= total {
                    // Send final ACK to calculator
                    let mut ack = vec![0u8; self.report_size + 1];
                    ack[1] = 0xFE;
                    ack[2] = 0x01;
                    ack[5..9].copy_from_slice(&(total as u32).to_le_bytes());
                    ack[9..13].copy_from_slice(&in_msg_id.unwrap().to_le_bytes());
                    self.write_report(&ack)?;
                    return Ok(assembled);
                }
            }
        }
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
