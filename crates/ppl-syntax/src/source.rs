//! Source files, positions, and decoding of `.hpprgm` / text sources.

use std::path::Path;

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

/// Decodes PPL source bytes.
///
/// * A UTF-8 file (optionally with BOM) is used as is.
/// * A UTF-16LE file with BOM is decoded.
/// * Anything else with many NUL bytes is treated as a Connectivity Kit
///   `.hpprgm` container: the UTF-16LE program text is located by the first
///   PPL marker and read until the first control or non-character code unit.
///
/// Line breaks are normalised to `\n`. Unlike the Python loader nothing
/// inside the text is removed or rewritten, so string contents survive and
/// line numbers stay exact.
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
