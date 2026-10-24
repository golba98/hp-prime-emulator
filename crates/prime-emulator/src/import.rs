//! M5-B: import a host PPL program into a guest data directory as the
//! firmware stores it: `C:\DATA\<NAME>.hpprgm` (host `<data>/C/DATA/`),
//! the location the firmware enumerates at boot (`_wfindfirst`
//! `C:\DATA\*.hpprgm`) and writes when its Program Catalog creates a
//! program.
//!
//! Inputs: PPL source text (UTF-8, or UTF-16 with a BOM), or a genuine
//! container (validated by [`crate::hpprgm::parse`] and copied unchanged).
//! A `.hpprgm` without the container magic is plain text under the wrong
//! extension and is refused unless the caller passes it as source.
//!
//! The write is transactional: everything is validated first, the bytes
//! go to a temporary file in the target directory (fsync), which is then
//! renamed over the final name (directory fsync). Nothing else in the
//! data directory is touched, and the report proves it with before/after
//! hashes of the `C:` tree.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::hpprgm;

/// Prefix of the importer's temporary files (never `*.hpprgm`, so the
/// firmware's enumeration cannot see a half-written import).
pub const TEMP_PREFIX: &str = ".prime-import-";
pub const EXTENSION: &str = "hpprgm";
/// Longest name the firmware's New Program dialog accepts (31 accepted,
/// 32 rejected with "Invalid input"; a `name64` field holds 31 UTF-16
/// units plus the NUL).
pub const MAX_NAME_CHARS: usize = 31;

#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub program: PathBuf,
    pub data_dir: PathBuf,
    /// Program name; default: the input file's stem.
    pub name: Option<String>,
    pub replace: bool,
    /// Treat the input as source text whatever its extension.
    pub as_source: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputInfo {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// `ppl-source` or `hpprgm-container`.
    pub kind: &'static str,
    /// `utf-8`, `utf-8-bom`, `utf-16le-bom`, `utf-16be-bom` (source only).
    pub encoding: Option<&'static str>,
    /// `lf`, `crlf->lf`, `cr->lf`, `mixed->lf` (source only).
    pub line_endings: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub imported: bool,
    pub name: String,
    pub guest_path: String,
    pub host_path: String,
    pub input: InputInfo,
    pub size: u64,
    pub sha256: String,
    /// Source text as stored (LF line endings).
    pub source_chars: usize,
    pub source_lines: usize,
    pub source_sha256: String,
    /// Export cache carried by the container (empty for source imports:
    /// the firmware derives it when it checks the program).
    pub container_exports: Vec<(String, u32)>,
    /// Path replaced by `--replace`, with its previous SHA-256.
    pub replaced: Option<(String, String)>,
    /// Stale importer temp files found and removed.
    pub removed_stale_temps: Vec<String>,
    /// Every other file under `C/` hashes the same after the import.
    pub other_files_unchanged: bool,
}

#[derive(Debug)]
pub enum ImportError {
    Io(String),
    InvalidName(String),
    NotInitialized(PathBuf),
    PlainTextContainer(PathBuf),
    Container(hpprgm::Error),
    Source(String),
    Exists { existing: String },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Io(e) => write!(f, "I/O: {e}"),
            ImportError::InvalidName(e) => write!(f, "invalid program name: {e}"),
            ImportError::NotInitialized(p) => write!(
                f,
                "{} has no firmware-initialised C:\\DATA (boot the firmware on this data \
                 directory once first)",
                p.display()
            ),
            ImportError::PlainTextContainer(p) => write!(
                f,
                "{} has no HP Prime container magic: it is plain text under a .hpprgm name; \
                 import it as source with --as-source",
                p.display()
            ),
            ImportError::Container(e) => write!(f, "invalid container: {e}"),
            ImportError::Source(e) => write!(f, "invalid source: {e}"),
            ImportError::Exists { existing } => write!(
                f,
                "a program with this name already exists ({existing}); pass --replace to replace it"
            ),
        }
    }
}

impl std::error::Error for ImportError {}

fn io(e: impl std::fmt::Display) -> ImportError {
    ImportError::Io(e.to_string())
}

/// Program name rules: an ASCII letter, then ASCII letters, digits or `_`,
/// at most [`MAX_NAME_CHARS`] characters. Firmware evidence (its New
/// Program dialog, M5): a leading digit (`1A`) and 32 characters are
/// rejected with "Invalid input", 31 letters are accepted, and an existing
/// name is refused ("A program with same name already exists"). `_` and
/// other characters are not probed: the rule stays a PPL identifier.
pub fn validate_name(name: &str) -> Result<(), ImportError> {
    let bad = |why: &str| Err(ImportError::InvalidName(format!("`{name}`: {why}")));
    let mut chars = name.chars();
    match chars.next() {
        None => return bad("empty"),
        Some(c) if !c.is_ascii_alphabetic() => return bad("must start with a letter A-Z/a-z"),
        _ => {}
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return bad("only letters, digits and _ are allowed");
    }
    if name.len() > MAX_NAME_CHARS {
        return bad(&format!("longer than {MAX_NAME_CHARS} characters"));
    }
    Ok(())
}

/// Decoded source text, LF line endings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSource {
    pub text: String,
    pub encoding: &'static str,
    pub line_endings: &'static str,
}

pub fn decode_source(bytes: &[u8]) -> Result<DecodedSource, ImportError> {
    let utf16 = |b: &[u8], be: bool| -> Result<String, ImportError> {
        if b.len() % 2 != 0 {
            return Err(ImportError::Source("odd UTF-16 byte count".into()));
        }
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| ImportError::Source("invalid UTF-16".into()))
    };
    let (raw, encoding) = if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (utf16(b, false)?, "utf-16le-bom")
    } else if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (utf16(b, true)?, "utf-16be-bom")
    } else if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (
            String::from_utf8(b.to_vec())
                .map_err(|_| ImportError::Source("invalid UTF-8".into()))?,
            "utf-8-bom",
        )
    } else {
        (
            String::from_utf8(bytes.to_vec()).map_err(|_| {
                ImportError::Source("invalid UTF-8 (UTF-16 input needs a BOM)".into())
            })?,
            "utf-8",
        )
    };
    if raw.contains('\0') {
        return Err(ImportError::Source("contains a NUL character".into()));
    }
    let crlf = raw.matches("\r\n").count();
    let cr = raw.matches('\r').count() - crlf;
    let lf = raw.matches('\n').count() - crlf;
    let line_endings = match (crlf > 0, cr > 0, lf > 0) {
        (false, false, _) => "lf",
        (true, false, false) => "crlf->lf",
        (false, true, false) => "cr->lf",
        _ => "mixed->lf",
    };
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    if text.trim().is_empty() {
        return Err(ImportError::Source("empty program".into()));
    }
    Ok(DecodedSource {
        text,
        encoding,
        line_endings,
    })
}

/// Host directory of `C:\DATA`.
pub fn data_dir_of(data_dir: &Path) -> PathBuf {
    data_dir.join("C").join("DATA")
}

/// Existing `*.hpprgm` whose name equals `name` ignoring ASCII case (the
/// guest file system is case-insensitive).
pub fn find_program(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        let stem = p.file_stem()?.to_str()?;
        let ext = p.extension()?.to_str()?;
        (ext.eq_ignore_ascii_case(EXTENSION) && stem.eq_ignore_ascii_case(name) && p.is_file())
            .then_some(p)
    })
}

/// All programs in `C:\DATA` (names without extension, sorted).
pub fn list_programs(data_dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(data_dir_of(data_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?;
            (ext.eq_ignore_ascii_case(EXTENSION) && p.is_file())
                .then(|| p.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect();
    v.sort();
    v
}

fn c_tree_snapshot(data_dir: &Path) -> crate::storage::Snapshot {
    crate::storage::snapshot(&data_dir.join("C"))
}

fn fsync_dir(dir: &Path) -> Result<(), ImportError> {
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(io)
}

/// Removes leftovers of interrupted imports (only the importer's own
/// temp-file prefix).
pub fn remove_stale_temps(dir: &Path) -> Vec<String> {
    let mut removed = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(TEMP_PREFIX)
            && e.path().is_file()
            && std::fs::remove_file(e.path()).is_ok()
        {
            removed.push(name);
        }
    }
    removed.sort();
    removed
}

/// Prepared bytes for one program (validated, not yet written).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub name: String,
    pub bytes: Vec<u8>,
    pub source: String,
    pub input: InputInfo,
    pub exports: Vec<(String, u32)>,
}

/// Validates the input and builds the container bytes.
pub fn prepare(req: &ImportRequest) -> Result<Prepared, ImportError> {
    let input_bytes = std::fs::read(&req.program)
        .map_err(|e| ImportError::Io(format!("{}: {e}", req.program.display())))?;
    let stem = req
        .program
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_owned();
    let name = req.name.clone().unwrap_or(stem);
    validate_name(&name)?;
    let is_container_ext = req
        .program
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION));
    let has_magic = input_bytes.get(..4) == Some(&hpprgm::MAGIC.to_le_bytes()[..]);
    let mut input = InputInfo {
        path: req.program.display().to_string(),
        size: input_bytes.len() as u64,
        sha256: crate::sha256::hex(&input_bytes),
        kind: "ppl-source",
        encoding: None,
        line_endings: None,
    };
    if has_magic && !req.as_source {
        let c = hpprgm::parse(&input_bytes).map_err(ImportError::Container)?;
        let src = c.source().map_err(ImportError::Container)?;
        input.kind = "hpprgm-container";
        return Ok(Prepared {
            name,
            exports: c.exports(),
            source: src.source,
            bytes: input_bytes,
            input,
        });
    }
    if is_container_ext && !req.as_source {
        return Err(ImportError::PlainTextContainer(req.program.clone()));
    }
    let decoded = decode_source(&input_bytes)?;
    input.encoding = Some(decoded.encoding);
    input.line_endings = Some(decoded.line_endings);
    let container =
        hpprgm::build_program(&decoded.text).map_err(|e| ImportError::Source(e.to_string()))?;
    let bytes = container.to_bytes();
    // Self-check: the bytes parse back to the same source.
    let back = hpprgm::parse(&bytes)
        .and_then(|c| c.source())
        .map_err(ImportError::Container)?;
    assert_eq!(back.source, decoded.text, "serializer round trip");
    Ok(Prepared {
        name,
        bytes,
        source: decoded.text,
        input,
        exports: Vec::new(),
    })
}

/// Imports one program. On error nothing in the data directory changes
/// (except the removal of stale importer temp files).
pub fn import(req: &ImportRequest) -> Result<ImportReport, ImportError> {
    let prepared = prepare(req)?;
    let dir = data_dir_of(&req.data_dir);
    if !dir.is_dir() {
        return Err(ImportError::NotInitialized(req.data_dir.clone()));
    }
    let removed_stale_temps = remove_stale_temps(&dir);
    let final_path = dir.join(format!("{}.{EXTENSION}", prepared.name));
    let existing = find_program(&dir, &prepared.name);
    let mut replaced = None;
    if let Some(ex) = &existing {
        if !req.replace {
            return Err(ImportError::Exists {
                existing: ex.display().to_string(),
            });
        }
        let old = std::fs::read(ex).map_err(io)?;
        replaced = Some((ex.display().to_string(), crate::sha256::hex(&old)));
    }
    let before = c_tree_snapshot(&req.data_dir);

    let tmp = dir.join(format!(
        "{TEMP_PREFIX}{}-{}.tmp",
        prepared.name,
        std::process::id()
    ));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&prepared.bytes)?;
        f.sync_all()
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    if let Err(e) = std::fs::rename(&tmp, &final_path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    // A differently-cased old file is a second name on a case-sensitive
    // host: remove it once the new file is in place.
    if let Some(ex) = &existing
        && ex != &final_path
    {
        std::fs::remove_file(ex).map_err(io)?;
    }
    fsync_dir(&dir)?;

    let after = c_tree_snapshot(&req.data_dir);
    let rel = |p: &Path| {
        p.strip_prefix(req.data_dir.join("C"))
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    };
    let touched = [Some(rel(&final_path)), existing.as_deref().map(rel)];
    let other_files_unchanged = before
        .iter()
        .filter(|(k, _)| !touched.iter().flatten().any(|t| t == *k))
        .all(|(k, v)| after.get(k) == Some(v))
        && after
            .keys()
            .all(|k| before.contains_key(k) || touched.iter().flatten().any(|t| t == k));

    let written = std::fs::read(&final_path).map_err(io)?;
    Ok(ImportReport {
        imported: true,
        guest_path: format!("C:\\DATA\\{}.{EXTENSION}", prepared.name),
        host_path: final_path.display().to_string(),
        size: written.len() as u64,
        sha256: crate::sha256::hex(&written),
        source_chars: prepared.source.chars().count(),
        source_lines: prepared.source.lines().count(),
        source_sha256: crate::sha256::hex(prepared.source.as_bytes()),
        container_exports: prepared.exports,
        name: prepared.name,
        input: prepared.input,
        replaced,
        removed_stale_temps,
        other_files_unchanged,
    })
}
