//! Source files, positions, and decoding of `.hpprgm` / text sources.

use std::path::Path;

/// A strictly decoded file. No PPL syntax or catalog-name validation is done.
/// `original_text` retains line endings; `text` is the compiler's LF view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedProgram {
    pub original_text: String,
    pub text: String,
    pub encoding: &'static str,
    pub bom: bool,
    pub container: bool,
    pub source_offset: Option<usize>,
    pub line_endings: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    Container(crate::hpprgm::Error),
    InvalidUtf8 { offset: usize },
    OddUtf16,
    InvalidUtf16,
    Nul,
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Container(e) => write!(f, "invalid container: {e}"),
            Self::InvalidUtf8 { offset } => write!(
                f,
                "invalid UTF-8 at byte {offset} (UTF-16 text needs a BOM)"
            ),
            Self::OddUtf16 => f.write_str("odd UTF-16 byte count"),
            Self::InvalidUtf16 => f.write_str("invalid UTF-16"),
            Self::Nul => f.write_str("NUL in source text"),
        }
    }
}

impl std::error::Error for FileError {}

/// Decode a recognized container strictly; never fall back to text on error.
/// Otherwise accept UTF-8 or BOM-marked UTF-16 text, whatever the filename.
pub fn decode_program(bytes: &[u8]) -> Result<DecodedProgram, FileError> {
    if bytes.starts_with(&crate::hpprgm::MAGIC.to_le_bytes()) {
        let c = crate::hpprgm::parse(bytes).map_err(FileError::Container)?;
        let source = c.source().map_err(FileError::Container)?;
        return Ok(decoded(
            source.source,
            "utf-16le-container",
            false,
            Some(source.offset),
        ));
    }
    decode_text(bytes)
}

/// Explicit text mode: rejects binary data rather than heuristically salvaging it.
pub fn decode_text(bytes: &[u8]) -> Result<DecodedProgram, FileError> {
    let (raw, encoding, bom) = if bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
        let be = bytes[0] == 254;
        let b = &bytes[2..];
        if b.len() % 2 != 0 {
            return Err(FileError::OddUtf16);
        }
        let units: Vec<_> = b
            .chunks_exact(2)
            .map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            })
            .collect();
        (
            String::from_utf16(&units).map_err(|_| FileError::InvalidUtf16)?,
            if be { "utf-16be" } else { "utf-16le" },
            true,
        )
    } else {
        let b = bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes);
        let text = std::str::from_utf8(b).map_err(|e| FileError::InvalidUtf8 {
            offset: bytes.len() - b.len() + e.valid_up_to(),
        })?;
        (text.to_owned(), "utf-8", b.len() != bytes.len())
    };
    if raw.contains('\0') {
        return Err(FileError::Nul);
    }
    Ok(decoded(raw, encoding, bom, None))
}

fn decoded(
    original_text: String,
    encoding: &'static str,
    bom: bool,
    source_offset: Option<usize>,
) -> DecodedProgram {
    let crlf = original_text.matches("\r\n").count();
    let cr = original_text.matches('\r').count() - crlf;
    let lf = original_text.matches('\n').count() - crlf;
    let line_endings = match (crlf > 0, cr > 0, lf > 0) {
        (false, false, _) => "lf",
        (true, false, false) => "crlf->lf",
        (false, true, false) => "cr->lf",
        _ => "mixed->lf",
    };
    DecodedProgram {
        text: normalise(&original_text),
        original_text,
        encoding,
        bom,
        container: source_offset.is_some(),
        source_offset,
        line_endings,
    }
}

/// Byte range within a [`SourceFile`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// One-based line and column (columns count characters, not bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile {
            name: name.into(),
            text,
            line_starts,
        }
    }

    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = (offset as usize).min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line];
        let col = self.text[start..offset].chars().count();
        LineCol {
            line: line as u32 + 1,
            col: col as u32 + 1,
        }
    }

    pub fn line_text(&self, line: u32) -> &str {
        let idx = line.saturating_sub(1) as usize;
        let Some(&start) = self.line_starts.get(idx) else {
            return "";
        };
        let end = self
            .line_starts
            .get(idx + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset of the first character on `line` (1-based).
    pub fn line_start(&self, line: u32) -> Option<usize> {
        self.line_starts
            .get(line.saturating_sub(1) as usize)
            .copied()
    }
}

/// How a source file's bytes were interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceEncoding {
    Utf8,
    Utf16Le,
    /// UTF-16LE text found inside a binary `.hpprgm` container.
    Utf16LeContainer {
        offset: usize,
    },
}

/// Legacy recovery decoder retained for editor/import compatibility.
///
/// May replace malformed UTF-8 or truncate UTF-16 at a control character,
/// malformed surrogate or heuristic container boundary. It cannot report
/// errors. New file consumers (`ppl check`, `ppl show`, firmware imports)
/// must use [`decode_program`] instead. This API is not format validation.
pub fn decode_source(bytes: &[u8]) -> (String, SourceEncoding) {
    if bytes.len() >= 4
        && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == crate::hpprgm::MAGIC
    {
        if let Ok(container) = crate::hpprgm::parse(bytes) {
            if let Ok(src) = container.source() {
                return (
                    normalise(&src.source),
                    SourceEncoding::Utf16LeContainer { offset: src.offset },
                );
            }
        }
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (
            normalise(&String::from_utf8_lossy(rest)),
            SourceEncoding::Utf8,
        );
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (
            normalise(&decode_utf16_until_junk(rest, false)),
            SourceEncoding::Utf16Le,
        );
    }
    let head = &bytes[..bytes.len().min(200)];
    let nuls = head.iter().filter(|&&b| b == 0).count();
    if nuls > 10 {
        let offset = find_utf16_marker(bytes).unwrap_or(0);
        let text = decode_utf16_until_junk(&bytes[offset..], true);
        let text = normalise(&text);
        return (text, SourceEncoding::Utf16LeContainer { offset });
    }
    (
        normalise(&String::from_utf8_lossy(bytes)),
        SourceEncoding::Utf8,
    )
}

/// Legacy adapter over [`decode_source`]. For validated loading use
/// [`decode_program`] or `ppl_engine::source::Source::file`.
pub fn read_source_file(path: &Path) -> std::io::Result<(SourceFile, SourceEncoding)> {
    let bytes = std::fs::read(path)?;
    let (text, enc) = decode_source(&bytes);
    Ok((SourceFile::new(path.display().to_string(), text), enc))
}

fn normalise(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

const MARKERS: [&str; 8] = [
    "#pragma",
    "EXPORT ",
    "export ",
    "LOCAL ",
    "PROCEDURE ",
    "// ",
    "#cas",
    "KEY ",
];

fn find_utf16_marker(bytes: &[u8]) -> Option<usize> {
    MARKERS
        .iter()
        .filter_map(|m| {
            let enc: Vec<u8> = m.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            bytes.windows(enc.len()).position(|w| w == enc.as_slice())
        })
        .min()
}

/// Decodes UTF-16LE up to the first control or non-character code unit.
/// `trim_end` drops trailing whitespace (container padding); a plain
/// UTF-16 file keeps its exact text, including the final newline.
fn decode_utf16_until_junk(bytes: &[u8], trim_end: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut out = String::new();
    for r in char::decode_utf16(units.iter().copied()) {
        let Ok(c) = r else { break };
        let junk = (c < ' ' && !matches!(c, '\t' | '\n' | '\r'))
            || ('\u{80}'..='\u{9F}').contains(&c)
            || matches!(c, '\u{FFFE}' | '\u{FFFF}');
        if junk {
            break;
        }
        out.push(c);
    }
    if trim_end {
        out.truncate(out.trim_end().len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_chars() {
        let f = SourceFile::new("t", "ab\nπx\n");
        assert_eq!(f.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(f.line_col(3), LineCol { line: 2, col: 1 });
        assert_eq!(f.line_col(5), LineCol { line: 2, col: 2 });
        assert_eq!(f.line_text(2), "πx");
    }

    /// Ported from tests/test_source_loader.py.
    #[test]
    fn utf16_container_with_junk() {
        let mut bytes = b"\x01\x02junk".to_vec();
        for u in "EXPORT T()\r\nBEGIN\r\nPRINT(\"OK\");\r\nEND;\r\n".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(b"\x80\x00\xff\xfftail");
        let (text, enc) = decode_source(&bytes);
        assert_eq!(text, "EXPORT T()\nBEGIN\nPRINT(\"OK\");\nEND;");
        assert!(matches!(enc, SourceEncoding::Utf16LeContainer { .. }));
    }

    #[test]
    fn utf16_with_bom_keeps_final_newline() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("A;\r\nB;\r\n".encode_utf16().flat_map(u16::to_le_bytes));
        let (text, enc) = decode_source(&bytes);
        assert_eq!(enc, SourceEncoding::Utf16Le);
        assert_eq!(text, "A;\nB;\n");
    }

    #[test]
    fn utf8_keeps_unicode_strings() {
        let (text, _) = decode_source("PRINT(\"π≠3\");\r\n".as_bytes());
        assert_eq!(text, "PRINT(\"π≠3\");\n");
    }
}
