//! Read-only VFS resource reconstruction, not a running-machine snapshot.
//!
//! Exclusive `&mut Vfs` is required for capture/restore. Open writers, searches
//! and remembered durability errors are refused. Files must belong to a private,
//! nonmutating storage tree; reopened contents are checked against their digest.
//! No host descriptor, pointer, mutex or native callback is serialized.
use super::{FIRST_HANDLE, VFile, Vfs};
use crate::sha256::{self, Sha256};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::Ordering,
};

const VERSION: u32 = 1;
const KIND: &str = "prime.readonly-vfs-component";
const MAX_HANDLES: usize = 512;
const MAX_MANIFEST: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH: usize = 4096;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadOnlyFile {
    pub guest_handle: u32,
    /// Platform-independent path relative to the storage root, with `/` separators.
    pub path: String,
    pub position: u64,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadOnlyVfsComponent {
    pub kind: String,
    pub version: u32,
    pub live_resume: bool,
    pub resource_identity: String,
    pub files: Vec<ReadOnlyFile>,
    pub next_handle: u64,
    pub next_find: u64,
    pub cwd: Vec<String>,
    pub current_drive: u8,
    pub manifest_sha256: String,
}
fn relative(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.len() > MAX_PATH || path.contains(['\\', '\0', ':']) {
        return Err("invalid VFS component relative path".into());
    }
    let mut out = PathBuf::new();
    for p in Path::new(path).components() {
        match p {
            Component::Normal(s) => out.push(s),
            _ => return Err("VFS component path escapes its storage root".into()),
        }
    }
    // Normalization must not silently change any persisted path.
    if out
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
        != path
    {
        return Err("VFS component path is not canonical".into());
    }
    Ok(out)
}
fn jailed(root: &Path, path: &str) -> Result<PathBuf> {
    let relative = relative(path)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut host = root.clone();
    for part in relative.components() {
        host.push(part);
        let metadata = std::fs::symlink_metadata(&host).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("VFS component rejects symlink resources".into());
        }
    }
    // A FIFO open can wait indefinitely even in an otherwise stable private
    // tree. Reject nonregular final resources before opening a descriptor.
    if !std::fs::symlink_metadata(&host)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("VFS component resource is not a regular file".into());
    }
    let canonical = host.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("VFS component resource escapes storage root".into());
    }
    Ok(canonical)
}
fn digest_file(file: &mut std::fs::File, expected_size: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or("VFS component file length overflow")?;
        if total > expected_size {
            return Err("VFS component file changed while reading".into());
        }
        digest.update(&buffer[..n]);
    }
    if total != expected_size {
        return Err("VFS component file length changed".into());
    }
    Ok(digest.finalize_hex())
}
impl ReadOnlyVfsComponent {
    fn digest(&self) -> Result<String> {
        serde_json::to_vec(&(
            &self.kind,
            self.version,
            self.live_resume,
            &self.resource_identity,
            &self.files,
            self.next_handle,
            self.next_find,
            &self.cwd,
            self.current_drive,
        ))
        .map(|b| sha256::hex(&b))
        .map_err(|e| e.to_string())
    }
    fn validate(&self, identity: &str) -> Result<()> {
        if self.kind != KIND || self.version != VERSION || self.live_resume {
            return Err("unsupported VFS component kind/version or live-resume claim".into());
        }
        if identity.is_empty() || identity.len() > 256 || self.resource_identity != identity {
            return Err("VFS component resource identity mismatch".into());
        }
        if self.files.len() > MAX_HANDLES
            || self.cwd.len() != 26
            || !self.current_drive.is_ascii_uppercase()
            || self.next_handle < u64::from(FIRST_HANDLE)
            || self.next_handle > u64::from(u32::MAX)
            || self.next_find == 0
            || self.next_find > u64::from(u32::MAX)
        {
            return Err("VFS component handle, drive or path bounds invalid".into());
        }
        if self.manifest_sha256 != self.digest()? {
            return Err("VFS component manifest checksum mismatch".into());
        }
        let mut previous = FIRST_HANDLE - 1;
        let mut total = 0u64;
        for file in &self.files {
            relative(&file.path)?;
            if file.guest_handle <= previous
                || u64::from(file.guest_handle) >= self.next_handle
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err("VFS component handles/digests are invalid or duplicated".into());
            }
            previous = file.guest_handle;
            total = total
                .checked_add(file.size)
                .ok_or("VFS component byte budget overflow")?;
            if total > MAX_TOTAL_BYTES {
                return Err("VFS component file byte budget exceeded".into());
            }
        }
        for (drive, cwd) in self.cwd.iter().enumerate() {
            let prefix = format!("{}:", char::from(b'A' + drive as u8));
            if cwd.len() > MAX_PATH || !cwd.starts_with(&prefix) {
                return Err("VFS component cwd drive invalid".into());
            }
            if cwd.len() > 2 {
                let tail = cwd.get(3..).ok_or("VFS component cwd invalid")?;
                if cwd.as_bytes()[2] != b'\\' {
                    return Err("VFS component cwd invalid".into());
                }
                relative(&tail.replace('\\', "/"))?;
            }
        }
        Ok(())
    }
    /// Capture only with exclusive ownership, while external storage is stable.
    pub fn capture(vfs: &mut Vfs, identity: &str) -> Result<Self> {
        if !vfs
            .finds
            .get_mut()
            .map_err(|_| "VFS search lock poisoned")?
            .is_empty()
        {
            return Err("VFS component cannot reconstruct active directory searches".into());
        }
        if vfs
            .durability_error
            .get_mut()
            .map_err(|_| "VFS durability lock poisoned")?
            .is_some()
        {
            return Err("VFS component refuses previous storage durability errors".into());
        }
        let root = vfs.root.canonicalize().map_err(|e| e.to_string())?;
        let open = vfs.files.get_mut().map_err(|_| "VFS files lock poisoned")?;
        if open.len() > MAX_HANDLES || open.values().any(|f| f.writable || !f.readable) {
            return Err("VFS component supports bounded read-only file handles only".into());
        }
        let mut files = Vec::new();
        let mut total = 0u64;
        for (&handle, file) in open.iter_mut() {
            let path = file
                .host_path
                .strip_prefix(&vfs.root)
                .map_err(|_| "VFS component source path outside root")?
                .to_str()
                .ok_or("VFS component source path is not UTF-8")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            let host = jailed(&root, &path)?;
            // A fresh descriptor avoids try_clone's shared stream-position behavior.
            let mut reader = std::fs::File::open(&host).map_err(|e| e.to_string())?;
            let metadata = reader.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err("VFS component resource is not a regular file".into());
            }
            let size = metadata.len();
            total = total
                .checked_add(size)
                .ok_or("VFS component byte budget overflow")?;
            if total > MAX_TOTAL_BYTES {
                return Err("VFS component file byte budget exceeded".into());
            }
            let sha256 = digest_file(&mut reader, size)?;
            let position = file.file.stream_position().map_err(|e| e.to_string())?;
            // The open descriptor may predate a path replacement. Check its
            // actual bytes too, restoring its cursor even when hashing fails.
            let original = digest_file(&mut file.file, size);
            file.file
                .seek(SeekFrom::Start(position))
                .map_err(|e| e.to_string())?;
            if original? != sha256 {
                return Err("VFS component open descriptor differs from resource path".into());
            }
            files.push(ReadOnlyFile {
                guest_handle: handle,
                path,
                position,
                size,
                sha256,
            });
        }
        files.sort_unstable_by_key(|f| f.guest_handle);
        let mut component = Self {
            kind: KIND.into(),
            version: VERSION,
            live_resume: false,
            resource_identity: identity.into(),
            files,
            next_handle: vfs.next_handle.load(Ordering::Relaxed),
            next_find: vfs.next_find.load(Ordering::Relaxed),
            cwd: vfs
                .cwd
                .get_mut()
                .map_err(|_| "VFS cwd lock poisoned")?
                .to_vec(),
            current_drive: *vfs
                .cur_drive
                .get_mut()
                .map_err(|_| "VFS drive lock poisoned")?,
            manifest_sha256: String::new(),
        };
        component.manifest_sha256 = component.digest()?;
        component.validate(identity)?;
        Ok(component)
    }
    /// Reopen files and seek them only after validating a fresh target and manifest.
    /// Failure drops all provisional descriptors without changing the target.
    pub fn restore(&self, target: &mut Vfs, identity: &str) -> Result<()> {
        self.validate(identity)?;
        if !target
            .files
            .get_mut()
            .map_err(|_| "VFS files lock poisoned")?
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
            return Err("VFS component restore requires a fresh resource owner".into());
        }
        let cwd: [String; 26] = self
            .cwd
            .clone()
            .try_into()
            .map_err(|_| "invalid cwd array")?;
        // Detect every poisoned target field before installing any resource.
        target.cwd.get_mut().map_err(|_| "VFS cwd lock poisoned")?;
        target
            .cur_drive
            .get_mut()
            .map_err(|_| "VFS drive lock poisoned")?;
        let mut files = HashMap::new();
        for saved in &self.files {
            let host = jailed(&target.root, &saved.path)?;
            let mut file = std::fs::File::open(&host).map_err(|e| e.to_string())?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file()
                || metadata.len() != saved.size
                || digest_file(&mut file, saved.size)? != saved.sha256
            {
                return Err(format!(
                    "VFS component content mismatch for handle {} ({})",
                    saved.guest_handle, saved.path
                ));
            }
            file.seek(SeekFrom::Start(saved.position))
                .map_err(|e| e.to_string())?;
            files.insert(
                saved.guest_handle,
                VFile {
                    file,
                    host_path: target.root.join(relative(&saved.path)?),
                    readable: true,
                    writable: false,
                },
            );
        }
        *target
            .files
            .get_mut()
            .map_err(|_| "VFS files lock poisoned")? = files;
        *target.cwd.get_mut().map_err(|_| "VFS cwd lock poisoned")? = cwd;
        *target
            .cur_drive
            .get_mut()
            .map_err(|_| "VFS drive lock poisoned")? = self.current_drive;
        target
            .next_handle
            .store(self.next_handle, Ordering::Relaxed);
        target.next_find.store(self.next_find, Ordering::Relaxed);
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate(&self.resource_identity)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("VFS component manifest exceeds size bound".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        temp.persist(path).map_err(|e| e.to_string())?;
        std::fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
    pub fn load(path: &Path, identity: &str) -> Result<Self> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_MANIFEST as u64 {
            return Err("VFS component file exceeds size bound".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("VFS component file exceeds size bound".into());
        }
        let result: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        result.validate(identity)?;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "synthetic-vfs-resource-v1";
    fn populate(root: &Path) {
        std::fs::create_dir_all(root.join("A")).unwrap();
        std::fs::write(root.join("A/alpha.bin"), b"abcdefgh").unwrap();
    }
    #[test]
    fn readonly_vfs_restores_stream_identity_and_position_in_a_fresh_process() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let mut vfs = Vfs::new(dir.path().to_owned());
        let handle = vfs.open("A:\\alpha.bin", "rb").unwrap();
        vfs.seek_set(handle, 2).unwrap();
        let component = ReadOnlyVfsComponent::capture(&mut vfs, ID).unwrap();
        assert_eq!(vfs.tell(handle), Some(2));
        let path = dir.path().join("vfs.json");
        component.save(&path).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::fs::component::tests::readonly_vfs_child",
                "--nocapture",
            ])
            .env("PRIME_VFS_COMPONENT_CHILD", dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("vfs-child-restored"));
    }
    #[test]
    fn readonly_vfs_child() {
        let Some(root) = std::env::var_os("PRIME_VFS_COMPONENT_CHILD") else {
            return;
        };
        let root = PathBuf::from(root);
        let component = ReadOnlyVfsComponent::load(&root.join("vfs.json"), ID).unwrap();
        #[cfg(target_os = "linux")]
        let descriptors = std::fs::read_dir("/proc/self/fd").unwrap().count();
        for _ in 0..32 {
            let mut vfs = Vfs::new(root.clone());
            component.restore(&mut vfs, ID).unwrap();
            let mut out = [0; 3];
            assert_eq!(vfs.read(FIRST_HANDLE, &mut out), Some(3));
            assert_eq!(&out, b"cde");
            assert_eq!(vfs.open("A:\\alpha.bin", "rb"), Some(FIRST_HANDLE + 1));
            assert_eq!(vfs.current_path(), "A:");
        }
        #[cfg(target_os = "linux")]
        assert_eq!(
            std::fs::read_dir("/proc/self/fd").unwrap().count(),
            descriptors,
            "component restore leaked file descriptors"
        );
        println!("vfs-child-restored");
    }
    #[test]
    fn changed_files_corrupt_manifests_wrong_identity_and_used_targets_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let mut vfs = Vfs::new(dir.path().to_owned());
        vfs.open("A:\\alpha.bin", "rb").unwrap();
        let mut component = ReadOnlyVfsComponent::capture(&mut vfs, ID).unwrap();
        let mut fresh = Vfs::new(dir.path().to_owned());
        assert!(component.restore(&mut fresh, "wrong").is_err());
        component.files[0].position += 1;
        assert!(component.restore(&mut fresh, ID).is_err());
        component.files[0].position -= 1;
        std::fs::write(dir.path().join("A/alpha.bin"), b"12345678").unwrap();
        for _ in 0..32 {
            assert!(component.restore(&mut fresh, ID).is_err());
        }
        assert_eq!(fresh.inspection()["open_files"], 0);
        std::fs::write(dir.path().join("A/alpha.bin"), b"abcdefgh").unwrap();
        fresh.open("A:\\alpha.bin", "rb").unwrap();
        assert!(component.restore(&mut fresh, ID).is_err());
    }
    #[test]
    fn writers_searches_path_traversal_and_fabricated_live_resume_are_blocked() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let mut vfs = Vfs::new(dir.path().to_owned());
        let handle = vfs.open("A:\\alpha.bin", "r+").unwrap();
        assert!(ReadOnlyVfsComponent::capture(&mut vfs, ID).is_err());
        vfs.close(handle);
        vfs.find_register(1, dir.path().join("A"), vec![], "*".into(), 0);
        assert!(ReadOnlyVfsComponent::capture(&mut vfs, ID).is_err());
        vfs.find_close(1);
        vfs.open("A:\\alpha.bin", "rb").unwrap();
        let mut component = ReadOnlyVfsComponent::capture(&mut vfs, ID).unwrap();
        component.files[0].path = "../outside".into();
        component.manifest_sha256 = component.digest().unwrap();
        assert!(
            component
                .restore(&mut Vfs::new(dir.path().to_owned()), ID)
                .is_err()
        );
        component.files[0].path = "A/alpha.bin".into();
        component.live_resume = true;
        component.manifest_sha256 = component.digest().unwrap();
        assert!(
            component
                .restore(&mut Vfs::new(dir.path().to_owned()), ID)
                .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_resources_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        std::os::unix::fs::symlink(
            dir.path().join("A/alpha.bin"),
            dir.path().join("A/link.bin"),
        )
        .unwrap();
        let mut vfs = Vfs::new(dir.path().to_owned());
        vfs.open("A:\\link.bin", "rb").unwrap();
        assert!(ReadOnlyVfsComponent::capture(&mut vfs, ID).is_err());
    }
    #[test]
    fn replaced_open_descriptor_is_rejected_and_its_position_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let mut vfs = Vfs::new(dir.path().to_owned());
        let handle = vfs.open("A:\\alpha.bin", "rb").unwrap();
        vfs.seek_set(handle, 3).unwrap();
        std::fs::rename(
            dir.path().join("A/alpha.bin"),
            dir.path().join("A/original.bin"),
        )
        .unwrap();
        std::fs::write(dir.path().join("A/alpha.bin"), b"12345678").unwrap();
        assert!(ReadOnlyVfsComponent::capture(&mut vfs, ID).is_err());
        assert_eq!(vfs.tell(handle), Some(3));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn fifo_replacements_are_rejected_before_opening_for_capture_or_restore() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let mut vfs = Vfs::new(dir.path().to_owned());
        let handle = vfs.open("A:\\alpha.bin", "rb").unwrap();
        vfs.seek_set(handle, 3).unwrap();
        let component = ReadOnlyVfsComponent::capture(&mut vfs, ID).unwrap();
        let path = dir.path().join("A/alpha.bin");
        std::fs::rename(&path, dir.path().join("A/original.bin")).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        // Check admission first so a regression fails instead of hanging in
        // File::open waiting for an unrelated FIFO writer.
        assert!(jailed(dir.path(), "A/alpha.bin").is_err());
        assert!(ReadOnlyVfsComponent::capture(&mut vfs, ID).is_err());
        assert_eq!(vfs.tell(handle), Some(3));
        let mut fresh = Vfs::new(dir.path().to_owned());
        assert!(component.restore(&mut fresh, ID).is_err());
        assert_eq!(fresh.inspection()["open_files"], 0);
    }
}
