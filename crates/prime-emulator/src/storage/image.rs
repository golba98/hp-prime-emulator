//! Complete drive-letter storage image for coordinated live restoration.
//! Contains directories, unopened files, hard links, modified times and access
//! permissions. Host bookkeeping outside A–Z drives is deliberately excluded.
//! The caller pauses guest/host storage writers for the capture transaction.
use crate::sha256;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, File, FileTimes, Metadata},
    hash::{Hash, Hasher},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 8] = b"HPTREE01";
const VERSION: u32 = 1;
const MAX_ENTRIES: usize = 100_000;
const MAX_HEADER: usize = 16 * 1024 * 1024;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timestamp {
    before_epoch: bool,
    seconds: u64,
    nanos: u32,
}
impl Timestamp {
    fn capture(time: SystemTime) -> Self {
        let (before_epoch, duration) = match time.duration_since(UNIX_EPOCH) {
            Ok(duration) => (false, duration),
            Err(error) => (true, error.duration()),
        };
        Self {
            before_epoch,
            seconds: duration.as_secs(),
            nanos: duration.subsec_nanos(),
        }
    }
    fn time(&self) -> Result<SystemTime> {
        if self.nanos >= 1_000_000_000 || self.seconds > i64::MAX as u64 {
            return Err("Invalid storage timestamp".into());
        }
        let duration = Duration::new(self.seconds, self.nanos);
        if self.before_epoch {
            UNIX_EPOCH.checked_sub(duration)
        } else {
            UNIX_EPOCH.checked_add(duration)
        }
        .ok_or_else(|| "Storage timestamp is outside host range".into())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    pub modified: Timestamp,
    pub readonly: bool,
    /// Ordinary Unix access bits; host ownership and privilege bits are not
    /// guest filesystem attributes and are never reproduced by a saved state.
    pub unix_mode: Option<u32>,
}
impl Attributes {
    fn capture(metadata: &Metadata) -> Result<Self> {
        #[cfg(unix)]
        let unix_mode = {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode() & 0o777)
        };
        #[cfg(not(unix))]
        let unix_mode = None;
        Ok(Self {
            modified: Timestamp::capture(metadata.modified().map_err(|e| e.to_string())?),
            readonly: metadata.permissions().readonly(),
            unix_mode,
        })
    }
    fn validate(&self) -> Result<()> {
        self.modified.time()?;
        if self
            .unix_mode
            .is_some_and(|mode| mode > 0o777 || (mode & 0o222 == 0) != self.readonly)
        {
            return Err("Invalid storage permissions".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Kind {
    Directory,
    File { size: u64, sha256: String },
    HardLink { target: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub attributes: Attributes,
    pub object: Kind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub resource_identity: String,
    pub entries: Vec<Entry>,
    pub sha256: String,
}
pub struct StorageImage {
    manifest: Manifest,
    file: File,
    payload_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureProgress {
    Inventory,
    Hashing { completed: usize, total: usize },
    Writing { completed: u64, total: u64 },
    Publishing,
}

fn relative(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.len() > 4096 || path.contains(['\\', ':', '\0']) {
        return Err("Invalid storage image path".into());
    }
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() > MAX_DEPTH
        || parts[0].len() != 1
        || !parts[0].as_bytes()[0].is_ascii_uppercase()
        || parts.iter().any(|part| matches!(*part, "" | "." | ".."))
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Storage image path escapes a guest drive".into());
    }
    Ok(parts.iter().collect())
}
impl Manifest {
    fn digest(&self) -> Result<String> {
        serde_json::to_vec(&(self.version, &self.resource_identity, &self.entries))
            .map(|bytes| sha256::hex(&bytes))
            .map_err(|e| e.to_string())
    }
    fn validate(&self, identity: &str) -> Result<u64> {
        if self.version != VERSION
            || identity.is_empty()
            || identity.len() > 256
            || self.resource_identity != identity
            || self.sha256 != self.digest()?
            || self.entries.len() > MAX_ENTRIES
        {
            return Err("Storage image version/identity/checksum/bounds mismatch".into());
        }
        let mut entries: BTreeMap<&str, &Entry> = BTreeMap::new();
        let mut folded = std::collections::BTreeSet::new();
        let mut total = 0u64;
        for entry in &self.entries {
            relative(&entry.path)?;
            entry.attributes.validate()?;
            if entries
                .last_key_value()
                .is_some_and(|(previous, _)| *previous >= entry.path.as_str())
                || !folded.insert(entry.path.to_ascii_lowercase())
            {
                return Err("Duplicate, unsorted or case-colliding storage paths".into());
            }
            if let Some((parent, _)) = entry.path.rsplit_once('/') {
                if !entries
                    .get(parent)
                    .is_some_and(|e| e.object == Kind::Directory)
                {
                    return Err("Storage image has a missing/non-directory parent".into());
                }
            } else if entry.object != Kind::Directory {
                return Err("A guest drive must be a directory".into());
            }
            match &entry.object {
                Kind::File { size, sha256 } => {
                    total = total
                        .checked_add(*size)
                        .ok_or("Storage byte count overflow")?;
                    if total > MAX_BYTES
                        || sha256.len() != 64
                        || !sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(
                            "Storage image payload exceeds bounds or has an invalid digest".into(),
                        );
                    }
                }
                Kind::HardLink { target } => {
                    if !entries.get(target.as_str()).is_some_and(|file| {
                        matches!(file.object, Kind::File { .. })
                            && file.attributes == entry.attributes
                    }) {
                        return Err(
                            "Storage hard link does not refer to an earlier matching regular file"
                                .into(),
                        );
                    }
                }
                Kind::Directory => {}
            }
            entries.insert(&entry.path, entry);
        }
        Ok(total)
    }
}

struct Inventory<'a> {
    root: &'a Path,
    entries: Vec<Entry>,
    pending: BTreeSet<String>,
    // Retain only hashes and paths, not an open descriptor per stored file.
    // Hash collisions are resolved with actual file identity comparisons.
    identities: HashMap<u64, Vec<String>>,
    bytes: u64,
}
impl Inventory<'_> {
    fn visit(&mut self, path: &str) -> Result<()> {
        let relative_path = relative(path)?;
        if self.entries.len() == MAX_ENTRIES {
            return Err("Storage entry budget exceeded".into());
        }
        let host = self.root.join(relative_path);
        let metadata = fs::symlink_metadata(&host).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
            return Err("Storage images refuse symbolic links and special files".into());
        }
        let attributes = Attributes::capture(&metadata)?;
        let object = if metadata.is_dir() {
            Kind::Directory
        } else {
            let handle = same_file::Handle::from_path(&host).map_err(|e| e.to_string())?;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            handle.hash(&mut hasher);
            let hash = hasher.finish();
            drop(handle);
            let candidates = self.identities.entry(hash).or_default();
            let mut target = None;
            for candidate in candidates.iter() {
                if same_file::is_same_file(&host, self.root.join(candidate))
                    .map_err(|e| e.to_string())?
                {
                    target = Some(candidate.clone());
                    break;
                }
            }
            if let Some(target) = target {
                Kind::HardLink { target }
            } else {
                candidates.push(path.into());
                self.bytes = self
                    .bytes
                    .checked_add(metadata.len())
                    .ok_or("Storage byte count overflow")?;
                if self.bytes > MAX_BYTES {
                    return Err("Storage byte budget exceeded".into());
                }
                Kind::File {
                    size: metadata.len(),
                    sha256: String::new(),
                }
            }
        };
        self.entries.push(Entry {
            path: path.into(),
            attributes,
            object,
        });
        if metadata.is_dir() {
            let mut names = Vec::new();
            for entry in fs::read_dir(host).map_err(|e| e.to_string())? {
                if self.entries.len() + self.pending.len() + names.len() >= MAX_ENTRIES {
                    return Err("Storage entry budget exceeded".into());
                }
                names.push(
                    entry
                        .map_err(|e| e.to_string())?
                        .file_name()
                        .into_string()
                        .map_err(|_| "Storage filename is not Unicode")?,
                );
            }
            for name in names {
                self.pending.insert(format!("{path}/{name}"));
            }
        }
        Ok(())
    }
}
fn scan(root: &Path) -> Result<Vec<Entry>> {
    if fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Storage root is a symbolic link".into());
    }
    let mut inventory = Inventory {
        root,
        entries: vec![],
        pending: BTreeSet::new(),
        identities: HashMap::new(),
        bytes: 0,
    };
    for drive in b'A'..=b'Z' {
        let name = char::from(drive).to_string();
        match fs::symlink_metadata(root.join(&name)) {
            Ok(_) => {
                inventory.pending.insert(name);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    while let Some(path) = inventory.pending.pop_first() {
        inventory.visit(&path)?;
    }
    Ok(inventory.entries)
}
fn stream(file: &mut File, size: u64, mut output: Option<&mut File>) -> Result<String> {
    let mut remaining = size;
    let mut hash = sha256::Sha256::new();
    let mut buffer = [0; 65536];
    while remaining != 0 {
        let length = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..length])
            .map_err(|e| e.to_string())?;
        hash.update(&buffer[..length]);
        if let Some(output) = output.as_deref_mut() {
            output
                .write_all(&buffer[..length])
                .map_err(|e| e.to_string())?;
        }
        remaining -= length as u64;
    }
    Ok(hash.finalize_hex())
}
fn file_digest(path: &Path, size: u64) -> Result<String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file()
        || file.metadata().map_err(|e| e.to_string())?.len() != size
    {
        return Err("Storage file changed during capture".into());
    }
    stream(&mut file, size, None)
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(not(unix))]
    let _ = path; // File contents are flushed; directory durability is OS-managed.
    Ok(())
}
fn metadata_file(path: &Path) -> Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Directory handles require BACKUP_SEMANTICS; SetFileTime requires
        // FILE_WRITE_ATTRIBUTES (Microsoft fileapi/SetFileTime contract).
        File::options()
            .access_mode(0x100)
            .custom_flags(0x02000000)
            .open(path)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        File::open(path).map_err(|e| e.to_string())
    }
}
fn attributes(path: &Path, saved: &Attributes) -> Result<()> {
    let file = metadata_file(path)?;
    file.set_times(FileTimes::new().set_modified(saved.modified.time()?))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if let Some(mode) = saved.unix_mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    } else {
        let mut permissions = file.metadata().map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(saved.readonly);
        fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = file.metadata().map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(saved.readonly);
        fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
    }
    let actual = Attributes::capture(&fs::metadata(path).map_err(|e| e.to_string())?)?;
    if actual.modified != saved.modified || actual.readonly != saved.readonly {
        return Err("Restored storage filesystem cannot preserve saved attributes".into());
    }
    #[cfg(unix)]
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}
impl StorageImage {
    /// Publish an atomic archive after validation and a second inventory check.
    /// Writers must remain paused until this transaction returns.
    pub fn capture(source: &Path, destination: &Path, identity: &str) -> Result<()> {
        Self::capture_with_progress(source, destination, identity, |_| Ok(()))
    }
    /// Progress callbacks may return an error to cancel before publication.
    pub fn capture_with_progress(
        source: &Path,
        destination: &Path,
        identity: &str,
        mut progress: impl FnMut(CaptureProgress) -> Result<()>,
    ) -> Result<()> {
        let root = source.canonicalize().map_err(|e| e.to_string())?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize().map_err(|e| e.to_string())?;
        if let Ok(relative) = parent.strip_prefix(&root)
            && relative.components().next().is_some_and(|p| {
                let name = p.as_os_str().to_string_lossy();
                name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase()
            })
        {
            return Err("Storage image destination must be outside the guest drives".into());
        }
        progress(CaptureProgress::Inventory)?;
        let mut entries = scan(source)?;
        let total_files = entries
            .iter()
            .filter(|entry| matches!(entry.object, Kind::File { .. }))
            .count();
        let mut completed = 0;
        for entry in &mut entries {
            if let Kind::File { size, sha256 } = &mut entry.object {
                *sha256 = file_digest(&source.join(&entry.path), *size)?;
                completed += 1;
                progress(CaptureProgress::Hashing {
                    completed,
                    total: total_files,
                })?;
            }
        }
        let mut manifest = Manifest {
            version: VERSION,
            resource_identity: identity.into(),
            entries,
            sha256: String::new(),
        };
        manifest.sha256 = manifest.digest()?;
        let total_bytes = manifest.validate(identity)?;
        let header = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        if header.len() > MAX_HEADER {
            return Err("Storage image header exceeds bounds".into());
        }
        let mut output = tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
        output
            .write_all(MAGIC)
            .and_then(|_| output.write_all(&(header.len() as u32).to_le_bytes()))
            .and_then(|_| output.write_all(&header))
            .map_err(|e| e.to_string())?;
        let mut completed = 0;
        progress(CaptureProgress::Writing {
            completed,
            total: total_bytes,
        })?;
        for entry in &manifest.entries {
            if let Kind::File { size, sha256 } = &entry.object {
                let mut source_file =
                    File::open(source.join(&entry.path)).map_err(|e| e.to_string())?;
                if source_file.metadata().map_err(|e| e.to_string())?.len() != *size
                    || stream(&mut source_file, *size, Some(output.as_file_mut()))? != *sha256
                {
                    return Err("Storage file changed during capture".into());
                }
                completed += size;
                progress(CaptureProgress::Writing {
                    completed,
                    total: total_bytes,
                })?;
            }
        }
        let mut layout = manifest.entries;
        for entry in &mut layout {
            if let Kind::File { sha256, .. } = &mut entry.object {
                sha256.clear();
            }
        }
        if scan(source)? != layout {
            return Err("Storage topology or attributes changed during capture".into());
        }
        output.as_file().sync_all().map_err(|e| e.to_string())?;
        progress(CaptureProgress::Publishing)?;
        output.persist(destination).map_err(|e| e.to_string())?;
        sync_directory(&parent)
    }
    pub fn open(path: &Path, identity: &str) -> Result<Self> {
        if !fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err("Storage image must be a regular file".into());
        }
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if length > 12 + MAX_HEADER as u64 + MAX_BYTES {
            return Err("Storage image exceeds byte budget".into());
        }
        let mut magic = [0; 8];
        let mut header_size = [0; 4];
        file.read_exact(&mut magic)
            .and_then(|_| file.read_exact(&mut header_size))
            .map_err(|e| e.to_string())?;
        let header_size = u32::from_le_bytes(header_size) as usize;
        if &magic != MAGIC || header_size == 0 || header_size > MAX_HEADER {
            return Err("Invalid storage image header".into());
        }
        let mut header = vec![0; header_size];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let manifest: Manifest = serde_json::from_slice(&header).map_err(|e| e.to_string())?;
        let bytes = manifest.validate(identity)?;
        let payload_at = 12 + header_size as u64;
        if length != payload_at + bytes {
            return Err("Truncated or trailing storage image data".into());
        }
        for entry in &manifest.entries {
            if let Kind::File { size, sha256 } = &entry.object
                && stream(&mut file, *size, None)? != *sha256
            {
                return Err("Storage image payload checksum mismatch".into());
            }
        }
        Ok(Self {
            manifest,
            file,
            payload_at,
        })
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    /// Restore into an owned fresh directory. Failure never writes to an
    /// existing profile, and a corrupt payload cannot publish partial storage.
    pub fn restore(&mut self, parent: &Path) -> Result<tempfile::TempDir> {
        let bytes = self.manifest.validate(&self.manifest.resource_identity)?;
        if self.file.metadata().map_err(|e| e.to_string())?.len() != self.payload_at + bytes {
            return Err("Storage image changed length after validation".into());
        }
        self.file
            .seek(SeekFrom::Start(self.payload_at))
            .map_err(|e| e.to_string())?;
        let tree = tempfile::Builder::new()
            .prefix("prime-live-storage-")
            .tempdir_in(parent)
            .map_err(|e| e.to_string())?;
        for entry in &self.manifest.entries {
            let path = tree.path().join(relative(&entry.path)?);
            match &entry.object {
                Kind::Directory => fs::create_dir(&path).map_err(|e| e.to_string())?,
                Kind::File { size, sha256 } => {
                    let mut file = File::options()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(|e| e.to_string())?;
                    if stream(&mut self.file, *size, Some(&mut file))? != *sha256 {
                        return Err("Storage image changed before restoration completed".into());
                    }
                    file.sync_all().map_err(|e| e.to_string())?;
                }
                Kind::HardLink { target } => {
                    fs::hard_link(tree.path().join(relative(target)?), &path)
                        .map_err(|e| e.to_string())?
                }
            }
        }
        // Children must exist before directory times/access bits are restored.
        for entry in self.manifest.entries.iter().rev() {
            if matches!(entry.object, Kind::HardLink { .. }) {
                continue;
            }
            let path = tree.path().join(&entry.path);
            attributes(&path, &entry.attributes)?;
            if entry.object == Kind::Directory {
                sync_directory(&path)?;
            }
        }
        if self.file.metadata().map_err(|e| e.to_string())?.len() != self.payload_at + bytes {
            return Err("Storage image changed length during restoration".into());
        }
        sync_directory(tree.path())?;
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Manifest {
        let attributes = Attributes {
            modified: Timestamp {
                before_epoch: false,
                seconds: 1_600_000_000,
                nanos: 0,
            },
            readonly: false,
            unix_mode: Some(0o640),
        };
        let mut manifest = Manifest {
            version: VERSION,
            resource_identity: "test".into(),
            entries: vec![
                Entry {
                    path: "C".into(),
                    attributes: attributes.clone(),
                    object: Kind::Directory,
                },
                Entry {
                    path: "C/data".into(),
                    attributes,
                    object: Kind::File {
                        size: 1,
                        sha256: sha256::hex(b"x"),
                    },
                },
            ],
            sha256: String::new(),
        };
        manifest.sha256 = manifest.digest().unwrap();
        manifest
    }
    #[test]
    fn recomputed_checksums_cannot_admit_invalid_paths_links_permissions_or_timestamps() {
        let original = manifest();
        assert_eq!(original.validate("test").unwrap(), 1);
        for path in [
            "../escape",
            "/etc/passwd",
            "C/../escape",
            "C//data",
            "C\\data",
            "C:/data",
            "C/data/child",
            "c/data",
        ] {
            let mut invalid = original.clone();
            invalid.entries[1].path = path.into();
            invalid.sha256 = invalid.digest().unwrap();
            assert!(invalid.validate("test").is_err(), "{path}");
        }
        for target in ["C/data", "C/missing", "../outside", "C"] {
            let mut invalid = original.clone();
            invalid.entries[1].object = Kind::HardLink {
                target: target.into(),
            };
            invalid.sha256 = invalid.digest().unwrap();
            assert!(invalid.validate("test").is_err(), "{target}");
        }
        for mode in [0o1000, 0o4640, 0o400] {
            let mut invalid = original.clone();
            invalid.entries[1].attributes.unix_mode = Some(mode);
            invalid.sha256 = invalid.digest().unwrap();
            assert!(invalid.validate("test").is_err(), "{mode:o}");
        }
        let mut invalid = original.clone();
        invalid.entries[1].attributes.modified.nanos = 1_000_000_000;
        invalid.sha256 = invalid.digest().unwrap();
        assert!(invalid.validate("test").is_err());
        let mut invalid = original.clone();
        invalid.entries[0].path = "D".into();
        invalid.sha256 = invalid.digest().unwrap();
        assert!(invalid.validate("test").is_err());
        let mut invalid = original.clone();
        invalid.entries.swap(0, 1);
        invalid.sha256 = invalid.digest().unwrap();
        assert!(invalid.validate("test").is_err());
    }
    #[test]
    fn oversized_headers_are_refused_before_allocating_or_restoring_storage() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bad.bin");
        fs::write(&path, [MAGIC.as_slice(), &u32::MAX.to_le_bytes()].concat()).unwrap();
        assert!(StorageImage::open(&path, "test").is_err());
    }
}
