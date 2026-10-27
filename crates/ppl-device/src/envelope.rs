//! Read-only version-3 F7 envelopes, from CK 59b930/59f8e0/5a2d20.
//! Outgoing requests remain version 1. No version-3 mutation is enabled.
use crate::{DeviceError, Result};
use std::borrow::Cow;

pub(crate) fn file(raw: &[u8]) -> Result<Cow<'_, [u8]>> {
    if raw.len() < 10 || raw[0] != 0xf7 || !matches!(raw[1], 1 | 3) {
        return Err(DeviceError::Protocol("unsupported F7 envelope".into()));
    }
    let size = u32::from_be_bytes(raw[2..6].try_into().unwrap()) as usize;
    if size.checked_add(6) != Some(raw.len()) || raw.len() > crate::inventory::MAX_MESSAGE {
        return Err(DeviceError::Protocol("F7 envelope length mismatch".into()));
    }
    if raw[1] != 3 || raw.get(10..12) != Some(&[0x78, 0x01]) {
        return Ok(Cow::Borrowed(raw));
    }
    let length = u32::from_le_bytes(raw[6..10].try_into().unwrap()) as usize;
    if !(4..=crate::inventory::MAX_MESSAGE - 6).contains(&length) {
        return Err(DeviceError::Protocol(
            "inflated F7 length exceeds bounds".into(),
        ));
    }
    let mut body = vec![0; length + 1];
    let mut decoder = flate2::Decompress::new(true);
    let status = decoder
        .decompress(&raw[10..], &mut body, flate2::FlushDecompress::Finish)
        .map_err(|e| DeviceError::Protocol(format!("invalid compressed F7: {e}")))?;
    if status != flate2::Status::StreamEnd
        || decoder.total_out() != length as u64
        || decoder.total_in() != (raw.len() - 10) as u64
    {
        return Err(DeviceError::Protocol(
            "truncated, trailing or oversized compressed F7".into(),
        ));
    }
    let mut decoded = Vec::with_capacity(length + 6);
    decoded.extend_from_slice(&[0xf7, 3]);
    decoded.extend_from_slice(&(length as u32).to_be_bytes());
    decoded.extend_from_slice(&body[..length]);
    Ok(Cow::Owned(decoded))
}

pub(crate) fn crc_length(raw: &[u8], declared: usize) -> usize {
    if raw[1] == 1 { declared } else { raw.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(compress: bool) -> Vec<u8> {
        // Synthetic content; envelope and CRC rules independently traced in CK.
        let mut raw =
            crate::files::encode_file_message(0xf7, crate::FileType::Program, "ZZT_A", &[7; 80]);
        raw[1] = 3;
        raw[8..10].fill(0);
        let crc = crate::crc16(&raw);
        raw[8..10].copy_from_slice(&crc.to_le_bytes());
        if !compress {
            return raw;
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw[6..]).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut wire = vec![0xf7, 3];
        wire.extend_from_slice(&((compressed.len() + 4) as u32).to_be_bytes());
        wire.extend_from_slice(&((raw.len() - 6) as u32).to_le_bytes());
        wire.extend_from_slice(&compressed);
        wire
    }

    #[test]
    fn compressed_and_plain_format_three_validate_full_crc_and_preserve_bytes() {
        for compressed in [false, true] {
            let raw = fixture(compressed);
            let file = crate::program::decode_record(&raw).unwrap();
            assert_eq!(file.data, [7; 80]);
            assert_eq!(file.name, "ZZT_A");
            assert!(crate::files::decode_file_message(&raw).unwrap().crc_ok);
        }
    }

    #[test]
    fn compressed_envelopes_reject_corruption_truncation_trailing_data_and_length_bombs() {
        let raw = fixture(true);
        for offset in [6, 12, raw.len() - 1] {
            let mut damaged = raw.clone();
            damaged[offset] ^= 1;
            assert!(crate::program::decode_record(&damaged).is_err());
        }
        let mut truncated = raw.clone();
        truncated.pop();
        assert!(file(&truncated).is_err());
        let mut trailing = raw.clone();
        trailing.push(0);
        let size = trailing.len() - 6;
        trailing[2..6].copy_from_slice(&(size as u32).to_be_bytes());
        assert!(file(&trailing).is_err());
        let mut bomb = raw.clone();
        bomb[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(file(&bomb).is_err());
        let mut crc = fixture(false);
        *crc.last_mut().unwrap() ^= 1;
        assert!(crate::program::decode_record(&crc).is_err());
    }
}
