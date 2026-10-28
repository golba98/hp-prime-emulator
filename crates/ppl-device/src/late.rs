//! Decode queued evidence message by message. Padding and sequence counters
//! are framing, never object data or operation identities.
use crate::{DeviceError, Exchange, PrimeFile, Result, program::decode_record};

pub(crate) fn files(reports: &[&Exchange], report_size: usize) -> Result<Vec<PrimeFile>> {
    let mut message = Vec::new();
    let mut sequence = 0u8;
    let mut found = Vec::new();
    for exchange in reports {
        let report = &exchange.bytes;
        if report.is_empty() || report.len() > report_size {
            return Err(DeviceError::Inconclusive(
                "invalid queued report length".into(),
            ));
        }
        if report[0] == 0xff {
            return Err(DeviceError::Inconclusive(
                "unmatched queued control report".into(),
            ));
        }
        if report[0] != sequence {
            return Err(DeviceError::Inconclusive(format!(
                "incomplete queued message: sequence {:02x}, expected {sequence:02x}",
                report[0]
            )));
        }
        sequence = if sequence == 0xfe { 0 } else { sequence + 1 };
        message.extend_from_slice(&report[1..]);
        if message.len() < 6 {
            continue;
        }
        if message[0] != 0xf7 || message[1] != 1 {
            return Err(DeviceError::Inconclusive(
                "unmatched queued response class".into(),
            ));
        }
        let length = u32::from_be_bytes(message[2..6].try_into().unwrap()) as usize + 6;
        if !(10..=16 * 1024 * 1024).contains(&length) {
            return Err(DeviceError::Inconclusive(
                "invalid queued message length".into(),
            ));
        }
        if message.len() >= length {
            found.push(decode_record(&message[..length])?);
            // The remaining bytes in the last report are unspecified padding.
            message.clear();
            sequence = 0;
        }
    }
    if !message.is_empty() {
        return Err(DeviceError::Inconclusive(
            "incomplete queued message".into(),
        ));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileType, Prime, files::encode_file_message};
    use std::time::Duration;
    #[test]
    fn padding_and_multiple_objects_do_not_change_identity() {
        let exchanges: Vec<_> = ["UNRELATED", "FIREWORKS"]
            .iter()
            .flat_map(|name| {
                Prime::frame(
                    &encode_file_message(0xf7, FileType::Program, name, &[0; 100]),
                    64,
                )
            })
            .map(|report| {
                let mut bytes = report[1..].to_vec();
                bytes.resize(64, 0xab);
                Exchange {
                    outgoing: false,
                    drained: true,
                    at: Duration::ZERO,
                    bytes,
                }
            })
            .collect();
        let files = files(&exchanges.iter().collect::<Vec<_>>(), 64).unwrap();
        assert_eq!(
            files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["UNRELATED", "FIREWORKS"]
        );
        let mut corrupt = exchanges.clone();
        corrupt[0].bytes[9] ^= 1;
        assert!(files_for(&corrupt).is_err());
        assert!(files_for(&exchanges[1..]).is_err());
    }
    fn files_for(e: &[Exchange]) -> Result<Vec<PrimeFile>> {
        files(&e.iter().collect::<Vec<_>>(), 64)
    }
}
