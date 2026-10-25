//! Strict physical file decoding. Unlike the legacy display helper this never
//! replaces invalid UTF-16, drops an odd byte, or normalizes source text.
use crate::{DeviceError, FileType, PrimeFile, Result, crc16};

impl crate::Prime {
    /// One read-only request. Silence is not absence, and the request is not retried.
    pub fn read_program_exact(&mut self, name: &str) -> Result<PrimeFile> {
        let n = name.encode_utf16().count();
        if n == 0 || n > 126 || name.contains('\0') {
            return Err(DeviceError::Refused(
                "invalid program name length or NUL".into(),
            ));
        }
        self.drain();
        self.send_message(&crate::files::encode_file_message(
            0xf8,
            FileType::Program,
            name,
            &[],
        ))?;
        let raw = self.recv_message(0xf7, "program source", self.read_timeout)?;
        let file = decode_record(&raw)?;
        if file.ty != FileType::Program || file.name != name {
            return Err(DeviceError::Protocol(
                "program reply name/type mismatch".into(),
            ));
        }
        Ok(file)
    }
}

pub fn decode_record(raw: &[u8]) -> Result<PrimeFile> {
    if raw.len() < 10 || raw[0] != 0xf7 || raw[1] != 1 {
        return Err(DeviceError::Protocol("invalid F7 record header".into()));
    }
    let declared = u32::from_be_bytes(raw[2..6].try_into().unwrap()) as usize;
    let name_bytes = raw[7] as usize;
    if declared.checked_add(6) != Some(raw.len())
        || name_bytes % 2 != 0
        || 10 + name_bytes > raw.len()
    {
        return Err(DeviceError::Protocol("invalid file/name length".into()));
    }
    let mut checked = raw.to_vec();
    let embedded = u16::from_le_bytes([checked[8], checked[9]]);
    checked[8..10].fill(0);
    if crc16(&checked[..declared]) != embedded {
        return Err(DeviceError::Protocol("file CRC mismatch".into()));
    }
    let name = utf16(&raw[10..10 + name_bytes])?;
    let name = name.strip_suffix('\0').unwrap_or(&name);
    if name.is_empty() || name.contains('\0') {
        return Err(DeviceError::Protocol("invalid object name".into()));
    }
    Ok(PrimeFile {
        ty: FileType::from_code(raw[6]),
        name: name.into(),
        data: raw[10 + name_bytes..].to_vec(),
        crc_ok: true,
    })
}

pub fn utf16(bytes: &[u8]) -> Result<String> {
    if bytes.len() % 2 != 0 {
        return Err(DeviceError::Protocol("odd UTF-16LE byte count".into()));
    }
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    String::from_utf16(&units).map_err(|_| DeviceError::Protocol("invalid UTF-16LE".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::encode_file_message;
    #[test]
    fn exact_source_including_crlf_bom_and_nuls_is_preserved() {
        let source = "\u{feff}EXPORT ZZT_A()\r\nBEGIN END;\0";
        let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let raw = encode_file_message(0xf7, FileType::Program, "ZZT_A", &bytes);
        let f = decode_record(&raw).unwrap();
        assert_eq!(f.data, bytes);
        assert_eq!(utf16(&f.data).unwrap(), source);
    }
    #[test]
    fn corrupt_or_ambiguous_records_are_rejected() {
        let mut raw = encode_file_message(0xf7, FileType::Program, "A", &[0, 0]);
        raw[2] = 1;
        assert!(decode_record(&raw).is_err());
        assert!(utf16(&[1]).is_err());
        assert!(utf16(&[0, 0xd8]).is_err());
        let mut raw = encode_file_message(0xf7, FileType::Program, "A", &[0; 20]);
        raw[12] ^= 1;
        assert!(decode_record(&raw).is_err());
    }
}
