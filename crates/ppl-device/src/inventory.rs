//! Documented hplp F9 backup receiver. End-marker observation is deliberately
//! separate from firmware-verified catalog completeness. No restore API exists.
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
    message: Vec<u8>,
    reports: usize,
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
        if !matches!(self.message[0], 0xf7 | 0xf9) || self.message[1] != 1 {
            return Err(DeviceError::Protocol(
                "unrecognized backup record header".into(),
            ));
        }
        let body = u32::from_be_bytes(self.message[2..6].try_into().unwrap()) as usize;
        let length = body
            .checked_add(6)
            .filter(|n| *n <= MAX_MESSAGE)
            .ok_or_else(|| DeviceError::Protocol("invalid backup declared length".into()))?;
        if self.message[0] == 0xf9 {
            // prime_data_size and calc_prime_r_recv_file in pinned hplp:
            // the empty version-1 F9 packet terminates the backup stream.
            if length != 6 {
                return Err(DeviceError::Protocol(
                    "unsupported F9 terminal payload".into(),
                ));
            }
            self.terminal_observed = true;
            return Ok(());
        }
        if length < 10 {
            return Err(DeviceError::Protocol("invalid F7 record length".into()));
        }
        if self.message.len() < length {
            return Ok(());
        }
        if self.records.len() >= MAX_RECORDS {
            return Err(DeviceError::Protocol("backup record limit exceeded".into()));
        }
        let file = decode_record(&self.message[..length])?;
        if self
            .records
            .iter()
            .any(|f| f.ty == file.ty && f.name == file.name)
        {
            return Err(DeviceError::Protocol("duplicate backup object".into()));
        }
        self.records.push(file);
        // Reports are fixed-size on the tested G1; bytes after the declared
        // packet length are unspecified HID padding, not a second object.
        // Never parse that padding (which may contain stale private bytes).
        self.message.clear();
        self.reports = 0;
        Ok(())
    }
}

pub struct Inventory {
    pub records: Vec<PrimeFile>,
    pub terminal_observed: bool,
    pub reason: String,
}
impl Prime {
    /// Read-only F9; no inferred completion, mutations, resends or unbounded logs.
    /// A caller must close the connection if no terminal was consumed.
    pub fn receive_inventory(&mut self) -> Result<Inventory> {
        self.drain();
        self.send_message(&[0xf9])?;
        let deadline = Instant::now() + Duration::from_secs(MAX_SECONDS);
        let mut decoder = BackupDecoder::default();
        let mut report = vec![0; self.report_size];
        let mut last = Instant::now();
        let mut parse_error = None;
        let mut received = 0usize;
        let reason = loop {
            if Instant::now() >= deadline || last.elapsed() >= Duration::from_secs(5) {
                break parse_error.unwrap_or_else(|| "backup response deadline reached".into());
            }
            match self.dev.read_timeout(&mut report, 100) {
                Ok(0) => continue,
                Ok(n) if n <= report.len() => {
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
                        break "F9 terminal observed; completeness on this firmware remains unverified".into();
                    }
                }
                Ok(_) => break "oversized HID report".into(),
                Err(e) => break e.to_string(),
            }
        };
        Ok(Inventory {
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
        assert!(feed(&mut d, &raw).is_err());
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
    #[test]
    fn backup_request_is_exactly_one_documented_byte_and_never_a_restore() {
        use std::collections::VecDeque;
        struct ReadOnly {
            sent: bool,
            replies: VecDeque<Vec<u8>>,
        }
        impl crate::Transport for ReadOnly {
            fn write(&mut self, report: &[u8]) -> Result<usize> {
                assert!(!self.sent, "read-only backup is never resent");
                assert_eq!(report, [0, 0, 0xf9]);
                self.sent = true;
                Ok(report.len())
            }
            fn read_timeout(&mut self, out: &mut [u8], _: i32) -> Result<usize> {
                if !self.sent {
                    return Ok(0);
                }
                if let Some(report) = self.replies.pop_front() {
                    out[..report.len()].copy_from_slice(&report);
                    Ok(report.len())
                } else {
                    Ok(0)
                }
            }
        }
        let mut replies = VecDeque::new();
        for raw in [
            encode_file_message(0xf7, FileType::Program, "EXISTING", &[0; 100]),
            vec![0xf9, 1, 0, 0, 0, 0],
        ] {
            for report in Prime::frame(&raw, 64) {
                replies.push_back(report[1..].to_vec());
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
        let mut prime = Prime::with_transport(
            detected,
            Box::new(ReadOnly {
                sent: false,
                replies,
            }),
        );
        let inventory = prime.receive_inventory().unwrap();
        assert!(inventory.terminal_observed);
        assert_eq!(inventory.records.len(), 1);
        assert!(inventory.reason.contains("unverified"));
    }
}
