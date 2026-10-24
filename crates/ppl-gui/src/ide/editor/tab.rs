//! One open program buffer: text, file identity, on-disk format and
//! editor bookkeeping.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ppl_syntax::{SourceEncoding, decode_source};

use super::document::DocumentCache;
use crate::ide::diagnostics::DiagnosticItem;

/// Stable identity of a tab for its whole lifetime. egui widget state
/// (cursor, undo history, scroll position) is keyed by it, so closing or
/// reordering tabs never hands one file's history to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// Compiler diagnostics for one revision of a tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabAnalysis {
    pub revision: u64,
    /// Sorted by line and column.
    pub items: Vec<DiagnosticItem>,
    /// The compiler failed internally (not a source error).
    pub failure: Option<String>,
}

impl TabId {
    fn next() -> TabId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        TabId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// How a file was stored, so saving writes it back the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileFormat {
    pub encoding: SourceEncoding,
    /// UTF-8 byte-order mark present.
    pub bom: bool,
    /// Lines ended with `\r\n` (the editor works on `\n`).
    pub crlf: bool,
}

impl FileFormat {
    /// Short status-bar label, e.g. `UTF-16LE (.hpprgm), CRLF`.
    pub fn label(&self) -> String {
        let enc = match self.encoding {
            SourceEncoding::Utf8 if self.bom => "UTF-8 BOM",
            SourceEncoding::Utf8 => "UTF-8",
            SourceEncoding::Utf16Le => "UTF-16LE",
            SourceEncoding::Utf16LeContainer { .. } => "UTF-16LE (.hpprgm)",
        };
        if self.crlf {
            format!("{enc}, CRLF")
        } else {
            enc.to_string()
        }
    }
}

impl Default for FileFormat {
    fn default() -> Self {
        FileFormat {
            encoding: SourceEncoding::Utf8,
            bom: false,
            crlf: false,
        }
    }
}

impl FileFormat {
    /// Decodes file bytes, remembering the format.
    pub fn decode(bytes: &[u8]) -> (String, FileFormat) {
        let (text, encoding) = decode_source(bytes);
        let crlf = match encoding {
            SourceEncoding::Utf8 => bytes.windows(2).any(|w| w == b"\r\n"),
            _ => bytes.windows(4).any(|w| w == [b'\r', 0, b'\n', 0]),
        };
        let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
        (
            text,
            FileFormat {
                encoding,
                bom,
                crlf,
            },
        )
    }

    /// Whether [`Self::encode`] can reproduce this format. A binary
    /// Connectivity Kit container cannot be rebuilt from the text alone.
    pub fn can_round_trip(&self) -> bool {
        !matches!(self.encoding, SourceEncoding::Utf16LeContainer { .. })
    }

    /// Encodes editor text in this format.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        let text = if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text.to_string()
        };
        match self.encoding {
            SourceEncoding::Utf16Le => {
                let mut out = vec![0xFF, 0xFE];
                out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
                out
            }
            _ => {
                let mut out = Vec::with_capacity(text.len() + 3);
                if self.bom {
                    out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                }
                out.extend_from_slice(text.as_bytes());
                out
            }
        }
    }
}

/// Why a save did not happen.
#[derive(Debug)]
pub enum SaveError {
    /// The tab has no path yet (needs Save As).
    NoPath,
    /// The file is a binary `.hpprgm` container; overwriting it with text
    /// would destroy the container, so Save As is required.
    Container,
    Io(std::io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NoPath => write!(f, "the program has not been saved yet"),
            SaveError::Container => write!(
                f,
                "this .hpprgm is a binary Connectivity Kit container; use Save As to write a text copy"
            ),
            SaveError::Io(e) => write!(f, "{e}"),
        }
    }
}

fn content_hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Writes `bytes` to `path` via a temporary file in the same directory and
/// a rename, so a failed write never leaves a truncated program behind.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".into());
    let tmp = match dir {
        Some(d) => d.join(format!(".{name}.saving-{}", std::process::id())),
        None => PathBuf::from(format!(".{name}.saving-{}", std::process::id())),
    };
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A single open file tab in the editor.
#[derive(Clone, Debug)]
pub struct EditorTab {
    pub id: TabId,
    pub path: Option<PathBuf>,
    pub title: String,
    pub content: String,
    /// Unsaved changes. Kept in sync with the content by
    /// [`EditorTab::sync`]: undoing back to the saved text clears it.
    pub modified: bool,
    /// The compiler's last result for this buffer, tagged with the revision
    /// it analysed. Only shown while that revision is current.
    pub analysis: Option<TabAnalysis>,
    pub breakpoints: BTreeSet<usize>,
    /// Pending cursor jump (char index) applied on the next frame.
    pub pending_jump: Option<usize>,
    /// Pending selection (char range) applied on the next frame.
    pub pending_select: Option<(usize, usize)>,
    /// Run configuration: exported function to call (None = auto-detect).
    pub run_entry: Option<String>,
    /// Run configuration: comma-separated PPL argument expressions.
    pub run_args: String,
    /// On-disk format to save back in.
    pub format: FileFormat,
    saved_hash: u64,
    pub(crate) doc: DocumentCache,
}

impl EditorTab {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        let content = content.into();
        Self {
            id: TabId::next(),
            path: None,
            title: title.into(),
            saved_hash: content_hash(&content),
            content,
            modified: false,
            analysis: None,
            breakpoints: BTreeSet::new(),
            pending_jump: None,
            pending_select: None,
            run_entry: None,
            run_args: String::new(),
            format: FileFormat::default(),
            doc: DocumentCache::default(),
        }
    }

    pub fn from_file(path: PathBuf, content: String) -> Self {
        Self::from_file_with_format(path, content, FileFormat::default())
    }

    pub fn from_file_with_format(path: PathBuf, content: String, format: FileFormat) -> Self {
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_string();
        let mut tab = Self::new(title, content);
        tab.path = Some(path);
        tab.format = format;
        tab
    }

    /// Loads a file from disk, remembering its encoding and line endings.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let bytes = std::fs::read(&path)?;
        let (text, format) = FileFormat::decode(&bytes);
        Ok(Self::from_file_with_format(path, text, format))
    }

    /// Refreshes derived data after the content may have changed. Returns
    /// true when it did change.
    pub fn sync(&mut self) -> bool {
        let changed = self.doc.sync(&self.content);
        if changed {
            self.modified = content_hash(&self.content) != self.saved_hash;
        }
        changed
    }

    /// The analysis of the current revision, if it has finished.
    pub fn current_analysis(&mut self) -> Option<&TabAnalysis> {
        let rev = self.revision();
        self.analysis.as_ref().filter(|a| a.revision == rev)
    }

    /// Diagnostics of the current revision; empty while it is unanalysed,
    /// so results computed for older text are never shown.
    pub fn current_diagnostics(&mut self) -> &[DiagnosticItem] {
        self.current_analysis().map_or(&[], |a| &a.items)
    }

    /// Content revision: advances on every real change (see [`Self::sync`]).
    pub fn revision(&mut self) -> u64 {
        self.sync();
        self.doc.revision()
    }

    pub fn document(&mut self) -> &DocumentCache {
        self.sync();
        &self.doc
    }

    /// The syntax-highlighted layout job of the current content at
    /// `font_size`. The language analysis behind it is cached per revision.
    pub fn layout(&mut self, font_size: f32) -> egui::text::LayoutJob {
        self.sync();
        let style = crate::ide::syntax::CodeStyle::new(font_size);
        (*self.doc.job(&self.content, style)).clone()
    }

    /// Records the current content as saved.
    pub fn mark_saved(&mut self) {
        self.saved_hash = content_hash(&self.content);
        self.modified = false;
    }

    /// Saves to `path` (or the tab's own path) in the tab's format.
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<(), SaveError> {
        let save_as = path.is_some();
        let target = path
            .or_else(|| self.path.clone())
            .ok_or(SaveError::NoPath)?;
        let mut format = self.format;
        if !format.can_round_trip() {
            if !save_as {
                return Err(SaveError::Container);
            }
            // A text copy of a container program: UTF-16LE with BOM is what
            // the Connectivity Kit reads back for `.hpprgm`; plain UTF-8
            // everywhere else.
            let is_hpprgm = target
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("hpprgm"));
            format = FileFormat {
                encoding: if is_hpprgm {
                    SourceEncoding::Utf16Le
                } else {
                    SourceEncoding::Utf8
                },
                bom: false,
                crlf: false,
            };
        }
        write_atomically(&target, &format.encode(&self.content)).map_err(SaveError::Io)?;
        if save_as || self.path.as_ref() != Some(&target) {
            self.title = target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Untitled")
                .to_string();
        }
        self.path = Some(target);
        self.format = format;
        self.mark_saved();
        Ok(())
    }

    /// Sets the cursor to a 1-based `line` and `col` on the next frame,
    /// clamped to the text.
    pub fn goto_line(&mut self, line: usize, col: usize) {
        let lines = self.document().lines().clone();
        self.pending_jump = Some(lines.char_of_line_col(line.max(1), col.max(1)));
    }

    /// Selects the char range on the next frame.
    pub fn select(&mut self, start: usize, end: usize) {
        self.pending_select = Some((start, end));
    }

    /// Replaces every occurrence of `find`; returns the count.
    pub fn replace_all(&mut self, find: &str, replace: &str) -> usize {
        if find.is_empty() {
            return 0;
        }
        let count = self.content.matches(find).count();
        if count > 0 {
            self.content = self.content.replace(find, replace);
            self.modified = true;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto_line_clamps_and_counts_chars() {
        let mut tab = EditorTab::new("t.ppl", "EXPORT F()\nBEGIN\n  PRINT(1);\nEND;");
        tab.goto_line(3, 8);
        assert_eq!(tab.pending_jump, Some(24), "line 3 col 8 is char 24");
        tab.goto_line(2, 1);
        assert_eq!(tab.pending_jump, Some(11));
        tab.goto_line(1, 1);
        assert_eq!(tab.pending_jump, Some(0));
        tab.goto_line(2, 999);
        assert_eq!(tab.pending_jump, Some(16));
        tab.goto_line(999, 999);
        assert_eq!(tab.pending_jump, Some(tab.content.chars().count()));
    }

    #[test]
    fn ids_are_unique_and_stable() {
        let a = EditorTab::new("a", "");
        let b = EditorTab::new("b", "");
        assert_ne!(a.id, b.id);
        let c = a.clone();
        assert_eq!(a.id, c.id);
    }

    #[test]
    fn modified_tracks_content_against_saved_state() {
        let mut tab = EditorTab::new("t.ppl", "A");
        assert!(!tab.sync() || !tab.modified);
        tab.content.push('B');
        assert!(tab.sync());
        assert!(tab.modified);
        // Undoing back to the saved text clears the flag.
        tab.content.pop();
        tab.sync();
        assert!(!tab.modified);
    }

    #[test]
    fn formats_round_trip() {
        let text = "EXPORT F()\nBEGIN\n  PRINT(\"π\");\nEND;\n";
        for format in [
            FileFormat::default(),
            FileFormat {
                bom: true,
                crlf: true,
                ..FileFormat::default()
            },
            FileFormat {
                encoding: SourceEncoding::Utf16Le,
                bom: false,
                crlf: true,
            },
        ] {
            let bytes = format.encode(text);
            let (back, detected) = FileFormat::decode(&bytes);
            assert_eq!(back, text);
            assert_eq!(detected, format);
        }
    }

    #[test]
    fn save_preserves_format_and_refuses_containers() {
        let dir = std::env::temp_dir().join(format!("ppl-gui-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // UTF-16LE + CRLF file: edited and saved back identically encoded.
        let path = dir.join("U16.hpprgm");
        let original = FileFormat {
            encoding: SourceEncoding::Utf16Le,
            bom: false,
            crlf: true,
        };
        std::fs::write(&path, original.encode("EXPORT A()\nBEGIN\nEND;\n")).unwrap();
        let mut tab = EditorTab::open(path.clone()).unwrap();
        assert_eq!(tab.format, original);
        tab.content = tab.content.replace("A()", "B()");
        tab.sync();
        assert!(tab.modified);
        tab.save(None).unwrap();
        assert!(!tab.modified);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, original.encode("EXPORT B()\nBEGIN\nEND;\n"));

        // A container cannot be overwritten in place.
        let mut tab = EditorTab::new("C.hpprgm", "EXPORT C() BEGIN END;");
        tab.path = Some(dir.join("C.hpprgm"));
        tab.format.encoding = SourceEncoding::Utf16LeContainer { offset: 16 };
        assert!(matches!(tab.save(None), Err(SaveError::Container)));
        assert!(!dir.join("C.hpprgm").exists());
        // Save As writes a text copy.
        tab.save(Some(dir.join("C.ppl"))).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("C.ppl")).unwrap(),
            "EXPORT C() BEGIN END;"
        );
        assert_eq!(tab.title, "C.ppl");

        // No temporary files are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".saving-"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
