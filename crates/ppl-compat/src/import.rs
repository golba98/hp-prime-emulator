//! Curated real-world program import (e.g. from https://www.hpcalc.org/).
//!
//! Archives are **untrusted input**. [`import_archive`] copies or extracts
//! them into a quarantined import directory with strict limits — no
//! absolute paths, no `..` traversal, no symlinks, bounded entry count and
//! size — and then classifies every file it finds:
//!
//! * PPL source (`.hpprgm`/`.ppl`/`.txt`): does it parse? does it compile?
//!   which built-ins does it need, and are they implemented or unsupported?
//!   which of those are side-effecting on hardware?
//! * MicroPython (`.py`): out of scope for the PPL runtime (reported, not
//!   executed).
//! * Anything else: recorded for provenance only.
//!
//! A provenance manifest (`manifest.json`) records the source URL, hashes
//! and the classification. Original bytes are never modified. Downloaded
//! third-party code stays in the (git-ignored) import directory and is
//! never committed.
//!
//! The caller is responsible for only downloading files it is permitted to
//! download (hpcalc.org asks for curated, non-bulk downloading).

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::sha256::sha256_hex;

/// Maximum entries in one archive.
pub const MAX_ENTRIES: usize = 4096;
/// Maximum total extracted bytes.
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum bytes of one extracted file.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Classification of one imported file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    /// Parses and compiles; only implemented built-ins.
    PplCompatible,
    /// Parses and compiles, but needs built-ins the emulator marks
    /// unsupported (listed).
    PplUnsupportedBuiltins { missing: Vec<String> },
    /// PPL-shaped but rejected by our parser (listed diagnostics).
    PplParseError { errors: Vec<String> },
    /// Parses but does not compile (semantic errors).
    PplCompileError { errors: Vec<String> },
    /// MicroPython source — outside the PPL runtime.
    MicroPython,
    /// Any other data (app data, images, docs…).
    Other,
}

impl FileClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileClass::PplCompatible => "ppl_compatible",
            FileClass::PplUnsupportedBuiltins { .. } => "ppl_unsupported_builtins",
            FileClass::PplParseError { .. } => "ppl_parse_error",
            FileClass::PplCompileError { .. } => "ppl_compile_error",
            FileClass::MicroPython => "micropython",
            FileClass::Other => "other",
        }
    }
}

/// One imported file and its analysis.
#[derive(Clone, Debug, Serialize)]
pub struct ImportedFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub class: FileClass,
    /// Exported entry points found (PPL only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
}

/// The full import report, also persisted as `manifest.json`.
#[derive(Clone, Debug, Serialize)]
pub struct ImportReport {
    /// Name of the imported archive/file.
    pub name: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub imported_at_utc: String,
    pub archive_sha256: String,
    pub import_dir: String,
    pub files: Vec<ImportedFile>,
    pub warnings: Vec<String>,
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "imported {} ({})", self.name, self.kind)?;
        if let Some(u) = &self.source_url {
            writeln!(f, "  source:    {u}")?;
        }
        if let Some(a) = &self.author {
            writeln!(f, "  author:    {a}")?;
        }
        writeln!(f, "  archive sha256: {}", self.archive_sha256)?;
        writeln!(f, "  files: {}", self.files.len())?;
        for w in &self.warnings {
            writeln!(f, "  warning: {w}")?;
        }
        for file in &self.files {
            writeln!(
                f,
                "  {:<40} {:>9} B  {}",
                file.path,
                file.bytes,
                file.class.as_str()
            )?;
            if let FileClass::PplUnsupportedBuiltins { missing } = &file.class {
                writeln!(f, "      unsupported built-ins: {}", missing.join(", "))?;
            }
            if let FileClass::PplParseError { errors } = &file.class {
                for e in errors.iter().take(3) {
                    writeln!(f, "      parse: {e}")?;
                }
            }
            if let FileClass::PplCompileError { errors } = &file.class {
                for e in errors.iter().take(3) {
                    writeln!(f, "      compile: {e}")?;
                }
            }
            if !file.exports.is_empty() {
                writeln!(f, "      exports: {}", file.exports.join(", "))?;
            }
        }
        writeln!(f, "  manifest: {}/manifest.json", self.import_dir)?;
        writeln!(
            f,
            "note: classification reflects the emulator today; it is not a hardware verdict"
        )
    }
}

/// Rejects path components that could escape the import directory.
fn safe_entry_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() {
        return Err("empty entry name".into());
    }
    if name.contains('\\') {
        return Err(format!("entry name `{name}` contains a backslash"));
    }
    let p = Path::new(name);
    if p.is_absolute() {
        return Err(format!("entry name `{name}` is absolute"));
    }
    for c in p.components() {
        match c {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.contains(':') {
                    return Err(format!("entry name `{name}` contains a drive/colon"));
                }
            }
            _ => return Err(format!("entry name `{name}` escapes the import dir")),
        }
    }
    Ok(p.to_path_buf())
}

fn is_ppl_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    // .hpappprgm is the PPL source inside an .hpappdir bundle
    l.ends_with(".hpprgm")
        || l.ends_with(".ppl")
        || l.ends_with(".txt")
        || l.ends_with(".hpappprgm")
}

fn is_python_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".py")
}

/// Extracts the PPL source from an `.hpappprgm` (app-embedded program)
/// container: a binary header followed by the UTF-16LE source. Returns
/// `None` when no plausible source start is found.
fn extract_hpappprgm_source(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let text = String::from_utf16_lossy(&units);
    for marker in ["EXPORT ", "VIEW \"", "IFERR"] {
        if let Some(at) = text.find(marker) {
            let candidate = &text[at..];
            if looks_like_source(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

/// Guards the parser against binary garbage: real PPL sources are almost
/// entirely printable ASCII. Pathological inputs (accidental binary
/// decoded as text) have crashed the parser before, so anything with more
/// than ~20% non-ASCII characters is not parsed.
fn looks_like_source(text: &str) -> bool {
    let sample: Vec<char> = text.chars().take(4096).collect();
    if sample.is_empty() {
        return false;
    }
    let suspicious = sample
        .iter()
        .filter(|c| {
            let v = **c;
            !(v == '\n' || v == '\r' || v == '\t' || (' '..='~').contains(&v))
        })
        .count();
    suspicious * 5 < sample.len()
}

/// Decodes source bytes through the loader (handles container headers and
/// UTF-16), then parses + compiles and returns the classification.
/// `.hpappprgm` containers get their embedded source extracted first.
fn classify_ppl(path: &str, bytes: &[u8]) -> (FileClass, Vec<String>) {
    if path.to_ascii_lowercase().ends_with(".hpappprgm") {
        return match extract_hpappprgm_source(bytes) {
            Some(source) => classify_text(path, &source),
            None => (FileClass::Other, Vec::new()),
        };
    }
    let (text, _) = ppl_syntax::decode_source(bytes);
    if !looks_like_source(&text) {
        return (FileClass::Other, Vec::new());
    }
    classify_text(path, &text)
}

/// Parses + compiles `text` and classifies the outcome.
fn classify_text(path: &str, text: &str) -> (FileClass, Vec<String>) {
    // 1. parse (syntax) stage
    let (_, parse_diags) = ppl_syntax::parse(text);
    if parse_diags.has_errors() {
        return (
            FileClass::PplParseError {
                errors: parse_diags.errors().map(|d| d.message.clone()).collect(),
            },
            Vec::new(),
        );
    }
    // 2. compile (semantic) stage
    let file = ppl_syntax::SourceFile::new(path.to_string(), text.to_string());
    let (prog, diags) =
        ppl_compiler::compile_source(&file, &ppl_compiler::CompileOptions::default());
    if prog.is_none() {
        return (
            FileClass::PplCompileError {
                errors: diags.errors().map(|d| d.message.clone()).collect(),
            },
            Vec::new(),
        );
    }
    let prog = prog.expect("checked above");
    let exports: Vec<String> = prog.exported().map(|(_, f)| f.name.clone()).collect();
    // 3. compiled: which unsupported built-ins does it call?
    let missing = unsupported_builtins(text);
    if missing.is_empty() {
        (FileClass::PplCompatible, exports)
    } else {
        (FileClass::PplUnsupportedBuiltins { missing }, exports)
    }
}

/// Names of unsupported built-ins referenced by word-boundary in `text`.
fn unsupported_builtins(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for spec in ppl_compiler::builtins::BUILTINS {
        if spec.status == ppl_compiler::builtins::Status::Unsupported
            && text_match_builtin(text, spec.name)
            && !out.iter().any(|n| n == spec.name)
        {
            out.push(spec.name.to_string());
        }
    }
    out
}

/// Whole-word, case-insensitive occurrence of a built-in name.
fn text_match_builtin(text: &str, name: &str) -> bool {
    let hay = text.to_ascii_uppercase();
    let needle = name.to_ascii_uppercase();
    let mut at = 0;
    while let Some(rel) = hay[at..].find(&needle) {
        let start = at + rel;
        let end = start + needle.len();
        let before_ok = start == 0
            || !hay.as_bytes()[start - 1].is_ascii_alphanumeric()
                && hay.as_bytes()[start - 1] != b'_';
        let after_ok = end >= hay.len()
            || !hay.as_bytes()[end].is_ascii_alphanumeric() && hay.as_bytes()[end] != b'_';
        if before_ok && after_ok {
            return true;
        }
        at = end;
    }
    false
}

fn read_limited<R: Read>(r: &mut R, cap: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut take = (&mut *r).take(cap + 1);
    take.read_to_end(&mut out)
        .map_err(|e| format!("read: {e}"))?;
    if out.len() as u64 > cap {
        return Err(format!("file larger than {cap} bytes"));
    }
    Ok(out)
}

/// Imports one archive or program file into `out_dir/<name>/`.
///
/// The import directory is a quarantine: nothing is executed, no third-party
/// byte is modified, and the caller keeps the originals.
pub fn import_archive(
    archive: &Path,
    url: Option<&str>,
    author: Option<&str>,
    out_root: &Path,
) -> Result<ImportReport, String> {
    let raw = std::fs::read(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let archive_sha = sha256_hex(&raw);
    let name = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("import")
        .to_string();
    let dir = out_root.join(&name);
    if dir.exists() {
        return Err(format!(
            "{} already exists; imports are never overwritten (delete it manually or choose another --out-dir)",
            dir.display()
        ));
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let mut report = ImportReport {
        name: name.clone(),
        kind: "file",
        source_url: url.map(String::from),
        author: author.map(String::from),
        imported_at_utc: crate::utc_now(),
        archive_sha256: archive_sha,
        import_dir: dir.display().to_string(),
        files: Vec::new(),
        warnings: Vec::new(),
    };

    let is_zip = raw.len() >= 4 && &raw[..4] == b"PK\x03\x04";
    let ext = archive
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if is_zip || ext == "zip" {
        report.kind = "zip";
        import_zip(&raw, &dir, &mut report)?;
    } else {
        // single program file: classify in place, copy into the quarantine
        let fname = archive
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("program")
            .to_string();
        let target = dir.join(safe_entry_path(&fname)?);
        std::fs::write(&target, &raw).map_err(|e| format!("{}: {e}", target.display()))?;
        let class = if is_ppl_name(&fname) {
            classify_ppl(&fname, &raw).0
        } else if is_python_name(&fname) {
            FileClass::MicroPython
        } else {
            FileClass::Other
        };
        let mut exports = Vec::new();
        if is_ppl_name(&fname) {
            exports = classify_ppl(&fname, &raw).1;
        }
        report.files.push(ImportedFile {
            path: fname,
            bytes: raw.len() as u64,
            sha256: sha256_hex(&raw),
            class,
            exports,
        });
    }
    if report
        .files
        .iter()
        .any(|f| f.class == FileClass::MicroPython)
    {
        report
            .warnings
            .push("contains MicroPython source, which is outside the PPL runtime".into());
    }
    let manifest = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), manifest).map_err(|e| format!("manifest: {e}"))?;
    Ok(report)
}

fn import_zip(raw: &[u8], dir: &Path, report: &mut ImportReport) -> Result<(), String> {
    let reader = std::io::Cursor::new(raw);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("bad zip: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!(
            "zip has {} entries (limit {MAX_ENTRIES})",
            zip.len()
        ));
    }
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let entry_name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        let rel = safe_entry_path(&entry_name)?;
        if entry_name.ends_with(".zip") {
            report.warnings.push(format!(
                "nested archive `{entry_name}` recorded but not extracted"
            ));
        }
        let bytes = read_limited(&mut entry, MAX_FILE_BYTES)?;
        total += bytes.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err("zip total size exceeds the import limit".into());
        }
        let target = dir.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if target.is_symlink() || target.exists() {
            return Err(format!("{} already exists", target.display()));
        }
        std::fs::write(&target, &bytes).map_err(|e| format!("{}: {e}", target.display()))?;
        let class = if is_ppl_name(&entry_name) {
            classify_ppl(&entry_name, &bytes).0
        } else if is_python_name(&entry_name) {
            FileClass::MicroPython
        } else {
            FileClass::Other
        };
        let mut exports = Vec::new();
        if is_ppl_name(&entry_name) {
            exports = classify_ppl(&entry_name, &bytes).1;
        }
        report.files.push(ImportedFile {
            path: entry_name,
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            class,
            exports,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zzt-import-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn safe_entry_path_rejects_traversal() {
        assert!(safe_entry_path("a/b.hpprgm").is_ok());
        assert!(safe_entry_path("/etc/passwd").is_err());
        assert!(safe_entry_path("../escape").is_err());
        assert!(safe_entry_path("a\\b").is_err());
        assert!(safe_entry_path("").is_err());
        assert!(safe_entry_path("C:/x").is_err());
    }

    #[test]
    fn classifies_ppl_program() {
        let src = b"EXPORT FOO()\nBEGIN\n  RETURN 1+2;\nEND;";
        let (class, exports) = classify_ppl("FOO.hpprgm", src);
        assert!(matches!(class, FileClass::PplCompatible));
        assert_eq!(exports, vec!["FOO"]);
    }

    #[test]
    fn classifies_parse_error_and_unsupported() {
        let (class, _) = classify_ppl("bad.ppl", b"EXPORT (\nBEGIN");
        assert!(matches!(class, FileClass::PplParseError { .. }));
        let (class, _) = classify_ppl("u.ppl", b"EXPORT MYU()\nBEGIN\n  STARTAPP(\"x\");\nEND;");
        match class {
            FileClass::PplUnsupportedBuiltins { missing } => assert!(!missing.is_empty()),
            other => panic!("expected unsupported built-ins, got {:?}", other),
        }
    }

    #[test]
    fn imports_a_zip_with_provenance() {
        let dir = tmp_dir("zip");
        let zip_path = dir.join("app.zip");
        write_zip(
            &zip_path,
            &[
                ("readme.txt", b"an app"),
                ("prog.hpprgm", b"EXPORT MYAPP()\nBEGIN\n  RETURN 42;\nEND;"),
                ("nested.zip", b"PK\x03\x04nothing"),
            ],
        );
        let out = dir.join("out");
        let report = import_archive(
            &zip_path,
            Some("https://www.hpcalc.org/prime/apps/x"),
            None,
            &out,
        )
        .unwrap();
        assert_eq!(report.kind, "zip");
        assert_eq!(report.files.len(), 3);
        assert_eq!(
            report
                .files
                .iter()
                .find(|f| f.path == "prog.hpprgm")
                .unwrap()
                .class,
            FileClass::PplCompatible
        );
        assert!(report.warnings.iter().any(|w| w.contains("nested")));
        // provenance manifest written
        let manifest =
            std::fs::read_to_string(format!("{}/manifest.json", report.import_dir)).unwrap();
        assert!(manifest.contains("hpcalc.org"));
        assert!(manifest.contains(&report.archive_sha256));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_never_overwrites() {
        let dir = tmp_dir("twice");
        let zip_path = dir.join("a.zip");
        write_zip(&zip_path, &[("x.hpprgm", b"EXPORT MYX() BEGIN END;")]);
        let out = dir.join("out");
        import_archive(&zip_path, None, None, &out).unwrap();
        assert!(import_archive(&zip_path, None, None, &out).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_hpprgm_import() {
        let dir = tmp_dir("single");
        let p = dir.join("SINGLE.hpprgm");
        std::fs::write(&p, b"EXPORT SINGLE()\nBEGIN\n  RETURN 7*6;\nEND;").unwrap();
        let out = dir.join("out");
        let report = import_archive(&p, None, Some("Some Author"), &out).unwrap();
        assert_eq!(report.kind, "file");
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].class, FileClass::PplCompatible);
        assert_eq!(report.files[0].exports, vec!["SINGLE"]);
        assert_eq!(report.author.as_deref(), Some("Some Author"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whole_word_builtin_match() {
        assert!(text_match_builtin("x:=STARTAPP(\"a\");", "STARTAPP"));
        assert!(!text_match_builtin("MYSTARTAPP()", "STARTAPP"));
        assert!(!text_match_builtin("STARTAPPS()", "STARTAPP"));
    }
}
