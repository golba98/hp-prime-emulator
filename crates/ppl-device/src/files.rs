//! `F7`/`F8` file messages.
//!
//! Layout (hplp `calc_prime_s_send_file` / `calc_prime_r_recv_file`):
//! `[cmd][0x01][size u32 BE][type][namelen][crc lo][crc hi][name UTF-16LE][data]`
//! where `size` counts everything after the first 6 bytes and the CRC
//! (CRC-16/XMODEM, little-endian) covers the first `size` bytes of the
//! message with the CRC field zeroed.

use crate::{DeviceError, Result, crc16};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Settings,
    App,
    List,
    Matrix,
    Note,
    Program,
    AppNote,
    AppProgram,
    Complex,
    Real,
    Other(u8),
}

impl FileType {
    pub fn code(self) -> u8 {
        match self {
            FileType::Settings => 0x00,
            FileType::App => 0x02,
            FileType::List => 0x03,
            FileType::Matrix => 0x04,
            FileType::Note => 0x05,
            FileType::Program => 0x06,
            FileType::AppNote => 0x07,
            FileType::AppProgram => 0x08,
            FileType::Complex => 0x09,
            FileType::Real => 0x0A,
            FileType::Other(c) => c,
        }
    }

    pub fn from_code(c: u8) -> FileType {
        match c {
            0x00 => FileType::Settings,
            0x02 => FileType::App,
            0x03 => FileType::List,
            0x04 => FileType::Matrix,
            0x05 => FileType::Note,
            0x06 => FileType::Program,
            0x07 => FileType::AppNote,
            0x08 => FileType::AppProgram,
            0x09 => FileType::Complex,
            0x0A => FileType::Real,
            other => FileType::Other(other),
        }
    }

    /// Lower-case display name (`other` for unknown codes).
    pub fn label(self) -> &'static str {
        match self {
            FileType::Settings => "settings",
            FileType::App => "app",
            FileType::List => "list",
            FileType::Matrix => "matrix",
            FileType::Note => "note",
            FileType::Program => "program",
            FileType::AppNote => "appnote",
            FileType::AppProgram => "appprogram",
            FileType::Complex => "complex",
            FileType::Real => "real",
            FileType::Other(_) => "other",
        }
    }

    pub fn parse(s: &str) -> Option<FileType> {
        Some(match s.to_ascii_lowercase().as_str() {
            "settings" => FileType::Settings,
            "app" => FileType::App,
            "list" => FileType::List,
            "matrix" => FileType::Matrix,
            "note" => FileType::Note,
            "program" | "prgm" => FileType::Program,
            "complex" => FileType::Complex,
            "real" => FileType::Real,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PrimeFile {
    pub ty: FileType,
    pub name: String,
    pub data: Vec<u8>,
    pub crc_ok: bool,
}

impl PrimeFile {
    /// Lossy legacy display helper; management must use `program::utf16` and
    /// retain the raw data, including terminators and original whitespace.
    pub fn text(&self) -> String {
        let d = self.data.strip_prefix(&[0xFF, 0xFE]).unwrap_or(&self.data);
        let units: Vec<u16> = d
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .to_string()
    }
}

pub fn encode_file_message(cmd: u8, ty: FileType, name: &str, data: &[u8]) -> Vec<u8> {
    let mut name16: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    if cmd == crate::CMD_REQ_FILE && !name16.ends_with(&[0, 0]) {
        name16.extend_from_slice(&[0, 0]);
    }
    let size = (4 + name16.len() + data.len()) as u32;
    let mut m = vec![cmd, 0x01];
    m.extend_from_slice(&size.to_be_bytes());
    m.push(ty.code());
    m.push(name16.len() as u8);
    m.extend_from_slice(&[0, 0]);
    m.extend_from_slice(&name16);
    m.extend_from_slice(data);
    let crc = crc16(&m[..size as usize]);
    m[8] = crc as u8;
    m[9] = (crc >> 8) as u8;
    m
}

pub fn decode_file_message(raw: &[u8]) -> Result<PrimeFile> {
    if raw.len() < 10 {
        return Err(DeviceError::Protocol(format!(
            "file message too short ({} bytes)",
            raw.len()
        )));
    }
    let declared = u32::from_be_bytes(raw[2..6].try_into().unwrap()) as usize;
    if raw[1] != 1 || declared.checked_add(6) != Some(raw.len()) {
        return Err(DeviceError::Protocol(
            "file header/declared length mismatch".into(),
        ));
    }
    let ty = FileType::from_code(raw[6]);
    let namelen = raw[7] as usize;
    if namelen % 2 != 0 || raw.len() < 10 + namelen {
        return Err(DeviceError::Protocol(
            "file name runs past the message".into(),
        ));
    }
    let embedded = u16::from_le_bytes([raw[8], raw[9]]);
    let mut copy = raw.to_vec();
    copy[8] = 0;
    copy[9] = 0;
    let computed = crc16(&copy[..raw.len().saturating_sub(6)]);
    let units: Vec<u16> = raw[10..10 + namelen]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let decoded = String::from_utf16(&units)
        .map_err(|_| DeviceError::Protocol("invalid UTF-16 object name".into()))?;
    let name = decoded.strip_suffix('\0').unwrap_or(&decoded).to_string();
    if name.is_empty() || name.contains('\0') {
        return Err(DeviceError::Protocol("ambiguous object name".into()));
    }
    Ok(PrimeFile {
        ty,
        name,
        data: raw[10 + namelen..].to_vec(),
        crc_ok: computed == embedded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let data: Vec<u8> = "EXPORT T() BEGIN END;"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let m = encode_file_message(0xF7, FileType::Program, "ZZT_A", &data);
        assert_eq!(m[0], 0xF7);
        assert_eq!(
            u32::from_be_bytes([m[2], m[3], m[4], m[5]]) as usize,
            m.len() - 6
        );
        let f = decode_file_message(&m).unwrap();
        assert_eq!(f.name, "ZZT_A");
        assert_eq!(f.ty, FileType::Program);
        assert!(f.crc_ok);
        assert_eq!(f.text(), "EXPORT T() BEGIN END;");
    }
}

#[cfg(test)]
mod deletion_fixture_tests {
    #[test]
    fn connectivity_kit_v1_eb_fixture_has_exact_name_type_size_and_crc() {
        // Independently calculated with Python binascii.crc_hqx over the v1
        // declared length, after selecting EB, matching the CK CRC routine.
        let fixture = [
            0xeb, 0x01, 0, 0, 0, 0x14, 0x06, 0x10, 0x66, 0xc9, 0x5a, 0, 0x5a, 0, 0x54, 0, 0x5f, 0,
            0x54, 0, 0x45, 0, 0x53, 0, 0x54, 0,
        ];
        assert_eq!(
            super::encode_file_message(
                crate::CMD_DELETE_OBJECT,
                super::FileType::Program,
                "ZZT_TEST",
                &[]
            ),
            fixture
        );
        assert_eq!(
            crate::Prime::frame(&fixture, 64)[0],
            [&[0, 0][..], &fixture].concat()
        );
    }
}
