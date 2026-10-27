//! Official F9 synchronization receiver. Identical broadcast/directed repeats
//! are retained as evidence; conflicting repeats prevent a complete snapshot.
use crate::{DeviceError, Prime, PrimeFile, Result, program::decode_record};
use std::time::{Duration, Instant};

pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL: usize = 64 * 1024 * 1024;
pub const MAX_RECORDS: usize = 4096;
pub const MAX_SECONDS: u64 = 120;

#[derive(Default)]
pub struct BackupDecoder {
    pub records: Vec<PrimeFile>,
    pub terminal_observed: bool,
    pub total_bytes: usize,
    pub received_records: usize,
    pub repetitions: Vec<Repetition>,
    message: Vec<u8>,
    reports: usize,
    decoded_bytes: usize,
}
impl BackupDecoder {
    pub fn push(&mut self, report: &[u8]) -> Result<()> {
        if self.terminal_observed {
            return Err(DeviceError::Protocol("data after backup terminal".into()));
        }
        if report.is_empty() || report.len() > 1024 {
            return Err(DeviceError::Protocol("invalid backup HID report".into()));
        }
        if report[0] == 0xff {
            return Ok(());
        }
        let sequence = ((self.reports + self.reports / 255) & 255) as u8;
        if report[0] != sequence {
            return Err(DeviceError::Protocol(format!(
                "backup report out of sequence: got {:02x}, expected {sequence:02x}",
                report[0]
            )));
        }
        self.reports += 1;
        self.total_bytes = self
            .total_bytes
            .checked_add(report.len())
            .filter(|n| *n <= MAX_TOTAL)
            .ok_or_else(|| DeviceError::Protocol("backup total limit exceeded".into()))?;
        self.message.extend_from_slice(&report[1..]);
        if self.message.len() > MAX_MESSAGE + 1024 {
            return Err(DeviceError::Protocol(
                "backup message limit exceeded".into(),
            ));
        }
        if self.message.len() < 6 {
            return Ok(());
        }
        if !matches!(self.message[0], 0xf7 | 0xf9) || !matches!(self.message[1], 1 | 3) {
            return Err(DeviceError::Protocol(
                "unrecognized backup record header".into(),
            ));
        }
        let body = u32::from_be_bytes(self.message[2..6].try_into().unwrap()) as usize;
        let length = body
            .checked_add(6)
            .filter(|n| *n <= MAX_MESSAGE)
            .ok_or_else(|| DeviceError::Protocol("invalid backup declared length".into()))?;
        if self.message.len() < length {
            return Ok(());
        }
        let raw = self.message[..length].to_vec();
        self.accept_message(&raw)?;
        // HID bytes beyond the logical length are unspecified padding.
        self.message.clear();
        self.reports = 0;
        Ok(())
    }
    fn accept_message(&mut self, raw: &[u8]) -> Result<()> {
        if self.terminal_observed
            || raw.len() < 6
            || raw.len() > MAX_MESSAGE
            || !matches!(raw[1], 1 | 3)
        {
            return Err(DeviceError::Protocol(
                "invalid inventory logical message".into(),
            ));
        }
        let declared = u32::from_be_bytes(raw[2..6].try_into().unwrap()) as usize;
        if declared.checked_add(6) != Some(raw.len()) {
            return Err(DeviceError::Protocol(
                "inventory declared length mismatch".into(),
            ));
        }
        if raw[0] == 0xf9 {
            if declared != 0 {
                return Err(DeviceError::Protocol(
                    "unsupported F9 terminal payload".into(),
                ));
            }
            self.terminal_observed = true;
            return Ok(());
        }
        if raw[0] != 0xf7 || raw.len() < 10 {
            return Err(DeviceError::Protocol(
                "invalid inventory object command".into(),
            ));
        }
        if self.received_records >= MAX_RECORDS {
            return Err(DeviceError::Protocol("backup record limit exceeded".into()));
        }
        let file = decode_record(raw)?;
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(file.data.len() + 10 + file.name.len() * 2)
            .filter(|n| *n <= MAX_TOTAL)
            .ok_or_else(|| {
                DeviceError::Protocol("decoded inventory total limit exceeded".into())
            })?;
        let record_index = self.received_records;
        self.received_records += 1;
        if let Some(first_index) = self
            .records
            .iter()
            .position(|f| f.ty == file.ty && f.name == file.name)
        {
            self.repetitions.push(Repetition {
                record_index,
                first_index,
                identical: self.records[first_index].data == file.data,
                record: file,
            });
        } else {
            self.records.push(file);
        }
        Ok(())
    }
}

/// An actual repeated F7 record, retained separately from the unique catalog.
/// Firmware emits broadcast and directed copies during F9 synchronization.
#[derive(Clone, Debug)]
pub struct Repetition {
    pub record_index: usize,
    pub first_index: usize,
    pub identical: bool,
    pub record: PrimeFile,
}

pub fn supported_profile(product_id: u16, info: &crate::CalcInfo) -> bool {
    product_id == 0x1541
        && info.build == Some(15157)
        && info.version.as_deref() == Some("V0.050.640")
}

pub struct Inventory {
    pub complete: bool,
    pub received_records: usize,
    pub repetitions: Vec<Repetition>,
    pub records: Vec<PrimeFile>,
    pub terminal_observed: bool,
    pub reason: String,
}
impl Prime {
    /// Read-only F9. Completeness requires the reference profile, valid terminal,
    /// consistent records and matching before/after device identity. Incomplete
    /// exchanges retire the connection. Mode 1 is recommended for large dumps.
    pub fn receive_inventory(&mut self) -> Result<Inventory> {
        let before = self.info()?;
        let profile = supported_profile(self.detected.product_id, &before);
        self.drain();
        self.send_message(&[0xf9])?;
        let deadline = Instant::now() + Duration::from_secs(MAX_SECONDS);
        let mut decoder = BackupDecoder::default();
        let mut report = vec![0; self.report_size];
        let mut last = Instant::now();
        let mut parse_error = None;
        let mut received = 0usize;
        let reason = loop {
            if self.mode == crate::TransportMode::Mode1 {
                if Instant::now() >= deadline {
                    break "inventory response deadline reached".into();
                }
                match self.recv_message(0xf7, "inventory object", Duration::from_secs(5)) {
                    Ok(raw) => {
                        received += raw.len();
                        if received > MAX_TOTAL {
                            break "inventory total limit exceeded".into();
                        }
                        if let Err(error) = decoder.accept_message(&raw) {
                            break error.to_string();
                        }
                        if decoder.terminal_observed {
                            break "F9 terminal observed".into();
                        }
                        continue;
                    }
                    Err(error) => break error.to_string(),
                }
            }
            if Instant::now() >= deadline || last.elapsed() >= Duration::from_secs(5) {
                break parse_error.unwrap_or_else(|| "backup response deadline reached".into());
            }
            match self.dev.read_timeout(&mut report, 100) {
                Ok(0) => continue,
                Ok(n) if n <= report.len() => {
                    self.log.push(crate::Exchange {
                        at: self.started.elapsed(),
                        outgoing: false,
                        drained: false,
                        bytes: report[..n].to_vec(),
                    });
                    last = Instant::now();
                    received += n;
                    if received > MAX_TOTAL {
                        break "backup total limit exceeded".into();
                    }
                    if parse_error.is_some() {
                        continue;
                    }
                    if let Err(e) = decoder.push(&report[..n]) {
                        // Consume the rest of this read-only response before
                        // releasing the handle. Never issue another command.
                        parse_error = Some(e.to_string());
                        continue;
                    }
                    if decoder.terminal_observed {
                        break "F9 terminal observed".into();
                    }
                }
                Ok(_) => break "oversized HID report".into(),
                Err(e) => break e.to_string(),
            }
        };
        let mut reason = reason;
        let conflicting = decoder.repetitions.iter().any(|r| !r.identical);
        let mut complete = decoder.terminal_observed && profile && !conflicting;
        if complete {
            match self.info() {
                Ok(after)
                    if after.serial.is_some()
                        && after.serial == before.serial
                        && after.build == before.build
                        && after.version == before.version => {}
                Ok(_) => {
                    complete = false;
                    reason = "device identity changed or unavailable after inventory".into();
                }
                Err(error) => {
                    complete = false;
                    reason = format!("post-inventory identity failed: {error}");
                }
            }
        } else if conflicting {
            reason = "conflicting repeated objects; inventory is ambiguous".into();
        } else if decoder.terminal_observed && !profile {
            reason = "F9 terminal observed on an unverified device profile".into();
        }
        if !complete {
            self.retired = Some(reason.clone());
        }
        Ok(Inventory {
            complete,
            received_records: decoder.received_records,
            repetitions: decoder.repetitions,
            records: decoder.records,
            terminal_observed: decoder.terminal_observed,
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileType, files::encode_file_message};
    fn feed(d: &mut BackupDecoder, message: &[u8]) -> Result<()> {
        for r in Prime::frame(message, 64) {
            d.push(&r[1..])?;
        }
        Ok(())
    }
    #[test]
    fn records_unknown_types_and_terminal_do_not_claim_completeness() {
        let mut d = BackupDecoder::default();
        feed(
            &mut d,
            &encode_file_message(0xf7, FileType::Program, "PERSONAL", &[0; 300]),
        )
        .unwrap();
        feed(
            &mut d,
            &encode_file_message(0xf7, FileType::Other(99), "opaque", &[1; 20]),
        )
        .unwrap();
        assert!(!d.terminal_observed);
        feed(&mut d, &[0xf9, 1, 0, 0, 0, 0]).unwrap();
        assert!(d.terminal_observed);
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.records[1].ty, FileType::Other(99));
        assert!(feed(&mut d, &[0xf9, 1, 0, 0, 0, 0]).is_err());
    }
    #[test]
    fn corruption_duplicates_and_lengths_are_rejected() {
        let raw = encode_file_message(0xf7, FileType::Program, "P", &[0; 100]);
        let mut d = BackupDecoder::default();
        feed(&mut d, &raw).unwrap();
        feed(&mut d, &raw).unwrap();
        assert_eq!(d.records.len(), 1);
        assert!(d.repetitions[0].identical);
        assert!(BackupDecoder::default().push(&[1, 0xf7]).is_err());
        assert!(
            BackupDecoder::default()
                .push(&[0, 0xf7, 1, 0xff, 0xff, 0xff, 0xff])
                .is_err()
        );
        let mut broken = raw;
        broken[12] ^= 1;
        assert!(feed(&mut BackupDecoder::default(), &broken).is_err());
    }

    #[test]
    fn padding_is_never_interpreted_as_an_extra_object() {
        let raw = encode_file_message(0xf7, FileType::Program, "A", &[0; 20]);
        let mut report = vec![0];
        report.extend(raw);
        report.resize(64, 0xa5);
        let mut decoder = BackupDecoder::default();
        decoder.push(&report).unwrap();
        assert_eq!(decoder.records.len(), 1);
        assert!(!decoder.terminal_observed);
        feed(&mut decoder, &[0xf9, 1, 0, 0, 0, 0]).unwrap();
        assert!(decoder.terminal_observed);
    }
    #[test]
    fn large_record_wrap_and_partial_record_never_lose_or_invent_objects() {
        let raw = encode_file_message(0xf7, FileType::Program, "LARGE", &[0; 40_000]);
        let mut decoder = BackupDecoder::default();
        feed(&mut decoder, &raw).unwrap();
        assert_eq!(decoder.records[0].data.len(), 40_000);
        let mut decoder = BackupDecoder::default();
        decoder.push(&[0, 0xf7, 1, 0, 0, 0, 20]).unwrap();
        assert!(decoder.records.is_empty());
        assert!(!decoder.terminal_observed);
    }
    // Synthetic transport; no physical calculator is opened.
    fn snapshot(messages: Vec<Vec<u8>>, build: u16) -> Prime {
        use std::collections::VecDeque;
        struct ReadOnly {
            sent: bool,
            stream: Vec<Vec<u8>>,
            replies: VecDeque<Vec<u8>>,
            build: u16,
        }
        impl crate::Transport for ReadOnly {
            fn write(&mut self, report: &[u8]) -> Result<usize> {
                let messages = match report {
                    [0, 0, 0xfa] => {
                        let mut raw = vec![0xfa, 1, 0, 0, 0, 108];
                        let mut body = vec![0; 108];
                        body[72..74].copy_from_slice(&self.build.to_le_bytes());
                        body[76..86].copy_from_slice(b"V0.050.640");
                        body[92..102].copy_from_slice(b"0123456789");
                        raw.extend(body);
                        vec![raw]
                    }
                    [0, 0, 0xf9] => {
                        assert!(!self.sent, "F9 never retried");
                        self.sent = true;
                        self.stream.clone()
                    }
                    _ => panic!("unexpected mutation or request {report:02x?}"),
                };
                for raw in messages {
                    self.replies
                        .extend(Prime::frame(&raw, 64).iter().map(|r| r[1..].to_vec()));
                }
                Ok(report.len())
            }
            fn read_timeout(&mut self, out: &mut [u8], _: i32) -> Result<usize> {
                if let Some(report) = self.replies.pop_front() {
                    out[..report.len()].copy_from_slice(&report);
                    Ok(report.len())
                } else {
                    Ok(0)
                }
            }
        }
        let detected = crate::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "mock",
            path: "offline".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        Prime::with_transport(
            detected,
            Box::new(ReadOnly {
                sent: false,
                stream: messages,
                replies: VecDeque::new(),
                build,
            }),
        )
    }
    #[test]
    fn complete_empty_inventory_requires_terminal_and_reference_identity() {
        let terminal = vec![0xf9, 1, 0, 0, 0, 0];
        let mut prime = snapshot(vec![terminal.clone()], 15157);
        let inventory = prime.receive_inventory().unwrap();
        assert!(inventory.complete && inventory.records.is_empty());
        let mut prime = snapshot(vec![terminal], 15158);
        assert!(!prime.receive_inventory().unwrap().complete);
        assert!(prime.check_ready().is_err(), "unverified session retired");
    }
    #[test]
    fn identical_repeats_are_evidence_but_conflicting_repeats_prevent_completeness() {
        let a = encode_file_message(0xf7, FileType::Program, "ALPHA", &[1, 2]);
        let b = encode_file_message(0xf7, FileType::Program, "BETA", &[3, 4]);
        let changed = encode_file_message(0xf7, FileType::Program, "ALPHA", &[5, 6]);
        let terminal = vec![0xf9, 1, 0, 0, 0, 0];
        let mut prime = snapshot(vec![a.clone(), a.clone(), b, terminal.clone()], 15157);
        let inventory = prime.receive_inventory().unwrap();
        assert!(inventory.complete);
        assert_eq!(inventory.records.len(), 2);
        assert_eq!(inventory.received_records, 3);
        assert_eq!(inventory.repetitions.len(), 1);
        assert!(inventory.repetitions[0].identical);
        let mut prime = snapshot(vec![a, changed, terminal], 15157);
        let inventory = prime.receive_inventory().unwrap();
        assert!(!inventory.complete && inventory.reason.contains("ambiguous"));
        assert!(!inventory.repetitions[0].identical);
        assert!(prime.check_ready().is_err());
    }
    #[test]
    fn missing_terminal_and_truncated_record_cannot_be_complete() {
        let mut d = BackupDecoder::default();
        feed(
            &mut d,
            &encode_file_message(0xf7, FileType::Program, "ALPHA", &[0; 100]),
        )
        .unwrap();
        assert!(!d.terminal_observed);
        let mut prime = snapshot(vec![vec![0xf7, 1, 0, 0, 0, 100]], 15157);
        let inventory = prime.receive_inventory().unwrap();
        assert!(!inventory.complete && inventory.records.is_empty());
    }
}
