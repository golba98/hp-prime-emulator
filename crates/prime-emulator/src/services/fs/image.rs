//! Portable open-file and directory-search state, including writable handles.
//! Capture requires coordinated paused owners and a private storage tree that
//! no external process mutates. Restore checks a corresponding storage copy;
//! this resource image does not itself contain unopened files or guest RAM.
use super::{
    FIRST_HANDLE, FindCtx, VFile, Vfs,
    component::{digest_file, jailed, relative},
};
use crate::{memory::GuestMemory, sha256};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

const VERSION: u32 = 1;
const MAX_HANDLES: usize = 512;
const MAX_NAMES: usize = 32768;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST: usize = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenFile {
    pub handle: u32,
    pub path: String,
    pub readable: bool,
    pub writable: bool,
    pub append: bool,
    pub position: u64,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub handle: u32,
    pub directory: String,
    pub remaining: Vec<String>,
    pub pattern: String,
    pub attributes: u32,
    pub short_name: u32,
    pub long_name: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfsImage {
    pub version: u32,
    pub resource_identity: String,
    pub files: Vec<OpenFile>,
    pub searches: Vec<Search>,
    pub next_handle: u64,
    pub next_find: u64,
    pub cwd: Vec<String>,
    pub drive: u8,
    pub sha256: String,
}
fn path_under(root: &Path, host: &Path) -> Result<String> {
    let path = host
        .strip_prefix(root)
        .map_err(|_| "VFS resource is outside storage root")?
        .to_str()
        .ok_or("VFS resource path is not Unicode")?
        .replace(std::path::MAIN_SEPARATOR, "/");
    relative(&path)?;
    Ok(path)
}
fn directory(root: &Path, path: &str) -> Result<PathBuf> {
    let path = relative(path)?;
    let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut host = canonical_root.clone();
    for part in path.components() {
        host.push(part);
        let metadata = std::fs::symlink_metadata(&host).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("VFS search path is not a jailed directory".into());
        }
    }
    if !host.starts_with(canonical_root) {
        return Err("VFS search path escapes storage root".into());
    }
    // Keep the admitted root spelling for snapshot capture and relocation.
    // File descriptors use the same representation after canonical validation.
    Ok(root.join(path))
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\', '\0', ':'])
}
impl VfsImage {
    fn digest(&self) -> Result<String> {
        serde_json::to_vec(&(
            self.version,
            &self.resource_identity,
            &self.files,
            &self.searches,
            self.next_handle,
            self.next_find,
            &self.cwd,
            self.drive,
        ))
        .map(|bytes| sha256::hex(&bytes))
        .map_err(|e| e.to_string())
    }
    fn validate(&self, identity: &str, memory: &GuestMemory) -> Result<()> {
        if self.version != VERSION
            || identity.is_empty()
            || identity.len() > 256
            || self.resource_identity != identity
            || self.sha256 != self.digest()?
            || self.files.len() + self.searches.len() > MAX_HANDLES
            || !self.drive.is_ascii_uppercase()
            || self.cwd.len() != 26
            || self.next_handle < u64::from(FIRST_HANDLE)
            || self.next_handle > u64::from(u32::MAX) + 1
            || self.next_find == 0
            || self.next_find > u64::from(u32::MAX) + 1
        {
            return Err("Invalid VFS image version, identity, checksum or bounds".into());
        }
        let mut handles = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            relative(&file.path)?;
            total = total
                .checked_add(file.size)
                .ok_or("VFS byte budget overflow")?;
            if file.handle < FIRST_HANDLE
                || u64::from(file.handle) >= self.next_handle
                || !handles.insert(file.handle)
                || (!file.readable && !file.writable)
                || (file.append && !file.writable)
                || file.position > i64::MAX as u64
                || total > MAX_BYTES
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid VFS image file handle/access/payload bounds".into());
            }
        }
        handles.clear();
        let mut allocations = BTreeSet::new();
        let mut names = 0usize;
        for search in &self.searches {
            relative(&search.directory)?;
            names = names
                .checked_add(search.remaining.len())
                .ok_or("VFS search budget overflow")?;
            if search.handle == 0
                || u64::from(search.handle) >= self.next_find
                || !handles.insert(search.handle)
                || names > MAX_NAMES
                || search.remaining.iter().any(|name| !valid_name(name))
                || search.pattern.len() > 1024
                || search.pattern.contains(['/', '\\', '\0', ':'])
            {
                return Err("Invalid VFS image directory search".into());
            }
            for address in [search.short_name, search.long_name] {
                if address != 0
                    && (!allocations.insert(address) || memory.heap_size_of(address).is_none())
                {
                    return Err(
                        "VFS search strings do not own distinct restored allocations".into(),
                    );
                }
            }
        }
        for (index, cwd) in self.cwd.iter().enumerate() {
            let prefix = format!("{}:", char::from(b'A' + index as u8));
            if !cwd.starts_with(&prefix)
                || cwd.len() > 4096
                || (cwd.len() > 2 && !cwd[2..].starts_with('\\'))
                || cwd[2..]
                    .split('\\')
                    .any(|part| matches!(part, "." | "..") || part.contains(['/', ':', '\0']))
            {
                return Err("Invalid VFS image current directory".into());
            }
        }
        Ok(())
    }
    pub fn capture(vfs: &Vfs, memory: &GuestMemory, identity: &str) -> Result<Self> {
        if vfs
            .durability_error
            .lock()
            .map_err(|_| "VFS durability lock poisoned")?
            .is_some()
        {
            return Err("VFS image refuses unresolved storage write errors".into());
        }
        let mut files = vfs.files.lock().map_err(|_| "VFS file lock poisoned")?;
        let finds = vfs.finds.lock().map_err(|_| "VFS search lock poisoned")?;
        if files.len() + finds.len() > MAX_HANDLES {
            return Err("VFS image handle budget exceeded".into());
        }
        if finds
            .values()
            .try_fold(0usize, |count, find| {
                count.checked_add(find.remaining.len())
            })
            .is_none_or(|count| count > MAX_NAMES)
        {
            return Err("VFS image search-name budget exceeded".into());
        }
        let mut saved_files = Vec::new();
        let mut total = 0u64;
        for (&handle, file) in files.iter_mut() {
            if file.writable {
                file.file
                    .flush()
                    .and_then(|_| file.file.sync_all())
                    .map_err(|e| e.to_string())?;
            }
            let path = path_under(&vfs.root, &file.host_path)?;
            let host = jailed(&vfs.root, &path)?;
            // A path may have been replaced after fopen. Rebinding a descriptor
            // to that new object would silently lose the original open file.
            let descriptor =
                same_file::Handle::from_file(file.file.try_clone().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let resource = same_file::Handle::from_path(&host).map_err(|e| e.to_string())?;
            if descriptor != resource {
                return Err("VFS open descriptor differs from its storage path".into());
            }
            let mut reader = std::fs::File::open(host).map_err(|e| e.to_string())?;
            let size = reader.metadata().map_err(|e| e.to_string())?.len();
            total = total.checked_add(size).ok_or("VFS byte budget overflow")?;
            if total > MAX_BYTES {
                return Err("VFS image file byte budget exceeded".into());
            }
            saved_files.push(OpenFile {
                handle,
                path,
                readable: file.readable,
                writable: file.writable,
                append: file.append,
                position: file.file.stream_position().map_err(|e| e.to_string())?,
                size,
                sha256: digest_file(&mut reader, size)?,
            });
        }
        saved_files.sort_by_key(|file| file.handle);
        let mut searches = finds
            .iter()
            .map(|(&handle, search)| {
                Ok(Search {
                    handle,
                    directory: path_under(&vfs.root, &search.host_dir)?,
                    remaining: search.remaining.iter().cloned().collect(),
                    pattern: search.pattern.clone(),
                    attributes: search.attrib_mask,
                    short_name: search.sfn_va,
                    long_name: search.lfn_va,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        searches.sort_by_key(|search| search.handle);
        let mut image = Self {
            version: VERSION,
            resource_identity: identity.into(),
            files: saved_files,
            searches,
            next_handle: vfs.next_handle.load(Ordering::Relaxed),
            next_find: vfs.next_find.load(Ordering::Relaxed),
            cwd: vfs
                .cwd
                .lock()
                .map_err(|_| "VFS cwd lock poisoned")?
                .to_vec(),
            drive: *vfs
                .cur_drive
                .lock()
                .map_err(|_| "VFS drive lock poisoned")?,
            sha256: String::new(),
        };
        image.sha256 = image.digest()?;
        image.validate(identity, memory)?;
        Ok(image)
    }
    /// Reconstruct over a fresh VFS and validated private storage copy. No
    /// create/truncate operations occur, and no target state changes on failure.
    pub fn restore(&self, target: &mut Vfs, memory: &GuestMemory, identity: &str) -> Result<()> {
        self.validate(identity, memory)?;
        if !target
            .files
            .get_mut()
            .map_err(|_| "VFS file lock poisoned")?
            .is_empty()
            || !target
                .finds
                .get_mut()
                .map_err(|_| "VFS search lock poisoned")?
                .is_empty()
            || target
                .durability_error
                .get_mut()
                .map_err(|_| "VFS durability lock poisoned")?
                .is_some()
            || target.next_handle.load(Ordering::Relaxed) != u64::from(FIRST_HANDLE)
            || target.next_find.load(Ordering::Relaxed) != 1
        {
            return Err("VFS restore requires an unused resource owner".into());
        }
        // Validate all targets before constructing or installing writable handles.
        let mut paths = HashMap::new();
        for saved in &self.files {
            let path = jailed(&target.root, &saved.path)?;
            let mut reader = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            if reader.metadata().map_err(|e| e.to_string())?.len() != saved.size
                || digest_file(&mut reader, saved.size)? != saved.sha256
            {
                return Err(format!("VFS restored storage mismatch: {}", saved.path));
            }
            paths.insert(saved.handle, path);
        }
        let mut searches = HashMap::new();
        for saved in &self.searches {
            searches.insert(
                saved.handle,
                FindCtx {
                    host_dir: directory(&target.root, &saved.directory)?,
                    remaining: saved.remaining.iter().cloned().collect(),
                    pattern: saved.pattern.clone(),
                    attrib_mask: saved.attributes,
                    sfn_va: saved.short_name,
                    lfn_va: saved.long_name,
                },
            );
        }
        let cwd: [String; 26] = self.cwd.clone().try_into().map_err(|_| "Invalid VFS cwd")?;
        target.cwd.get_mut().map_err(|_| "VFS cwd lock poisoned")?;
        target
            .cur_drive
            .get_mut()
            .map_err(|_| "VFS drive lock poisoned")?;
        let mut files = HashMap::new();
        for saved in &self.files {
            let path = &paths[&saved.handle];
            let mut file = std::fs::OpenOptions::new()
                .read(saved.readable)
                .write(saved.writable)
                .append(saved.append)
                .open(path)
                .map_err(|e| e.to_string())?;
            file.seek(SeekFrom::Start(saved.position))
                .map_err(|e| e.to_string())?;
            files.insert(
                saved.handle,
                VFile {
                    file,
                    host_path: target.root.join(relative(&saved.path)?),
                    readable: saved.readable,
                    writable: saved.writable,
                    append: saved.append,
                },
            );
        }
        *target.files.get_mut().unwrap() = files;
        *target.finds.get_mut().unwrap() = searches;
        *target.cwd.get_mut().unwrap() = cwd;
        *target.cur_drive.get_mut().unwrap() = self.drive;
        target
            .next_handle
            .store(self.next_handle, Ordering::Relaxed);
        target.next_find.store(self.next_find, Ordering::Relaxed);
        Ok(())
    }
    pub fn save(&self, path: &Path, memory: &GuestMemory) -> Result<()> {
        self.validate(&self.resource_identity, memory)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("VFS manifest exceeds bounds".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn load(path: &Path, memory: &GuestMemory, identity: &str) -> Result<Self> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("VFS manifest exceeds bounds".into());
        }
        let image: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        image.validate(identity, memory)?;
        Ok(image)
    }
}
