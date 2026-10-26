//! Strict representation detection. This module never parses PPL syntax.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub path: Option<PathBuf>,
    pub display_name: String,
    pub input_sha256: String,
    pub source_sha256: String,
    pub engine_source_sha256: String,
    pub encoding: String,
    pub representation: String,
    pub transformations: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct Source {
    pub identity: SourceIdentity,
    pub text: String,
    /// Strictly decoded source before compiler newline normalization.
    pub original_text: String,
    pub firmware_text: String,
    pub bytes: Vec<u8>,
    pub container: bool,
    pub stem: String,
}
impl Source {
    pub fn file(path: &Path) -> Result<Self, String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        Self::decode(bytes, Some(absolute), path.display().to_string())
    }
    pub fn text(text: &str, name: &str) -> Result<Self, String> {
        Self::decode(text.as_bytes().to_vec(), None, name.into())
    }
    pub fn decode(bytes: Vec<u8>, path: Option<PathBuf>, name: String) -> Result<Self, String> {
        let container = bytes.starts_with(&ppl_syntax::hpprgm::MAGIC.to_le_bytes());
        let (raw, encoding) = if container {
            let c =
                ppl_syntax::hpprgm::parse(&bytes).map_err(|e| format!("invalid container: {e}"))?;
            (
                c.source().map_err(|e| e.to_string())?.source,
                "utf-16le-container",
            )
        } else if bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
            let be = bytes[0] == 254;
            let b = &bytes[2..];
            if b.len() % 2 != 0 {
                return Err("odd UTF-16 byte count".into());
            }
            let units: Vec<_> = b
                .chunks_exact(2)
                .map(|x| {
                    if be {
                        u16::from_be_bytes([x[0], x[1]])
                    } else {
                        u16::from_le_bytes([x[0], x[1]])
                    }
                })
                .collect();
            (
                String::from_utf16(&units).map_err(|e| e.to_string())?,
                if be { "utf-16be" } else { "utf-16le" },
            )
        } else {
            (
                String::from_utf8(
                    bytes
                        .strip_prefix(&[239, 187, 191])
                        .unwrap_or(&bytes)
                        .to_vec(),
                )
                .map_err(|e| format!("invalid UTF-8: {e}"))?,
                "utf-8",
            )
        };
        if raw.contains('\0') {
            return Err("NUL in source text".into());
        }
        let text = raw.replace("\r\n", "\n").replace('\r', "\n");
        let transformations = if raw != text {
            vec!["line endings normalized to LF".into()]
        } else {
            vec![]
        };
        let stem = path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        let identity = SourceIdentity {
            path,
            display_name: name,
            input_sha256: hash(&bytes),
            source_sha256: hash(text.as_bytes()),
            engine_source_sha256: hash(text.as_bytes()),
            encoding: encoding.into(),
            representation: if container {
                "hpprgm-container"
            } else {
                "text-source"
            }
            .into(),
            transformations,
        };
        let original_text = raw.clone();
        let firmware_text = if container { raw } else { text.clone() };
        Ok(Self {
            identity,
            text,
            firmware_text,
            original_text,
            bytes,
            container,
            stem,
        })
    }
    pub fn guest_name(&self, explicit: Option<&str>) -> Result<String, String> {
        let name = explicit.map(str::to_owned).unwrap_or_else(|| {
            if valid_name(&self.stem) {
                self.stem.clone()
            } else {
                format!("PPL_{}", &self.identity.source_sha256[..16])
            }
        });
        if !valid_name(&name) {
            return Err("program name must start with an ASCII letter and contain at most 31 letters, digits or underscores".into());
        }
        Ok(name)
    }
}
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 31
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
