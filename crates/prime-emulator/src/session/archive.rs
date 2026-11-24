//! Atomic two-generation publication of complete live-machine archives.
//! Incomplete generations are never candidates. Each bounded part is checked
//! before the caller constructs resources or changes a calculator drive.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const ABI: u32 = 1;
const MAX_MANIFEST: u64 = 65536;
const PARTS: [(&str, u64); 3] = [
    ("memory.gz", 600 * 1024 * 1024),
    ("storage.bin", 530 * 1024 * 1024),
    ("state.json", 128 * 1024 * 1024),
];
type Result<T> = std::result::Result<T, String>;
#[derive(Clone)]
pub(super) struct Store {
    root: PathBuf,
    identity: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    abi: u32,
    identity: String,
    parts: BTreeMap<String, Part>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    generation: String,
    sha256: String,
}
pub(super) struct Loaded {
    pub path: PathBuf,
    pub previous: bool,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn sync(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(error)?;
    }
    let _ = path;
    Ok(())
}
fn regular(path: &Path, max: u64) -> Result<fs::File> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max {
        return Err(format!(
            "Live archive part is not a bounded regular file: {}",
            path.display()
        ));
    }
    let file = fs::File::open(path).map_err(error)?;
    // Reject replacement by a different file between metadata/open.
    if (same_file::Handle::from_path(path).map_err(error)?
        != same_file::Handle::from_file(file.try_clone().map_err(error)?).map_err(error)?)
        || file.metadata().map_err(error)?.len() != metadata.len()
    {
        return Err("Live archive part changed during opening".into());
    }
    Ok(file)
}
fn bytes(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut file = regular(path, max)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > max {
        return Err("Live archive part grew beyond budget".into());
    }
    Ok(bytes)
}
fn digest(path: &Path, max: u64) -> Result<Part> {
    use sha2::Digest;
    let mut file = regular(path, max)?;
    let mut count = 0u64;
    let mut hash = sha2::Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or("Live archive byte count overflow")?;
        if count > max {
            return Err("Live archive part exceeds budget".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok(Part {
        bytes: count,
        sha256: format!("{:x}", hash.finalize()),
    })
}
fn atomic(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("Live archive has no parent")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    temp.write_all(data).map_err(error)?;
    temp.as_file().sync_all().map_err(error)?;
    temp.persist(path).map_err(error)?;
    sync(parent)
}
impl Store {
    pub fn new(profile: &Path, identity: &str) -> Result<Self> {
        if identity.is_empty() || identity.len() > 256 {
            return Err("Invalid live archive resource identity".into());
        }
        let root = profile.join(".live");
        fs::create_dir_all(&root).map_err(error)?;
        if !fs::symlink_metadata(&root).map_err(error)?.is_dir() {
            return Err("Live archive root is not a real directory".into());
        }
        sync(profile)?;
        Ok(Self {
            root,
            identity: identity.into(),
        })
    }
    pub fn stage(&self) -> Result<tempfile::TempDir> {
        tempfile::Builder::new()
            .prefix("pending-")
            .tempdir_in(&self.root)
            .map_err(error)
    }
    fn validate(&self, pointer: &[u8]) -> Result<PathBuf> {
        let pointer: Pointer = serde_json::from_slice(pointer).map_err(error)?;
        if !pointer.generation.starts_with("generation-")
            || pointer.generation.len() > 128
            || pointer
                .generation
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && b != b'-')
        {
            return Err("Invalid live archive generation pointer".into());
        }
        let generation = self.root.join(pointer.generation);
        if !fs::symlink_metadata(&generation).map_err(error)?.is_dir() {
            return Err("Live generation is not a real directory".into());
        }
        let body = bytes(&generation.join("manifest.json"), MAX_MANIFEST)?;
        if crate::sha256::hex(&body) != pointer.sha256 {
            return Err("Live manifest checksum mismatch".into());
        }
        let manifest: Manifest = serde_json::from_slice(&body).map_err(error)?;
        if manifest.abi != ABI
            || manifest.identity != self.identity
            || manifest.parts.len() != PARTS.len()
        {
            return Err("Live archive version/resource identity mismatch".into());
        }
        for (name, max) in PARTS {
            let expected = manifest
                .parts
                .get(name)
                .ok_or("Live archive lacks a required machine part")?;
            let observed = digest(&generation.join(name), max)?;
            if expected.bytes != observed.bytes || expected.sha256 != observed.sha256 {
                return Err(format!("Live archive checksum mismatch: {name}"));
            }
        }
        Ok(generation)
    }
    pub fn load(&self) -> Result<Loaded> {
        let mut errors = vec![];
        for (name, previous) in [("current.json", false), ("previous.json", true)] {
            match bytes(&self.root.join(name), MAX_MANIFEST).and_then(|body| self.validate(&body)) {
                Ok(path) => return Ok(Loaded { path, previous }),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        Err(format!(
            "No intact compatible live archive; calculator data preserved: {}",
            errors.join("; ")
        ))
    }
    pub fn publish(&self, stage: tempfile::TempDir) -> Result<PathBuf> {
        self.publish_with(stage, || Ok(()))
    }
    fn publish_with(
        &self,
        stage: tempfile::TempDir,
        before_pointer: impl FnOnce() -> Result<()>,
    ) -> Result<PathBuf> {
        if stage.path().parent() != Some(self.root.as_path()) {
            return Err("Live publication staging owner mismatch".into());
        }
        let mut parts = BTreeMap::new();
        for (name, max) in PARTS {
            let path = stage.path().join(name);
            let part = digest(&path, max)?;
            regular(&path, max)?.sync_all().map_err(error)?;
            parts.insert(name.into(), part);
        }
        let manifest = serde_json::to_vec(&Manifest {
            abi: ABI,
            identity: self.identity.clone(),
            parts,
        })
        .map_err(error)?;
        atomic(&stage.path().join("manifest.json"), &manifest)?;
        sync(stage.path())?;
        let suffix = stage
            .path()
            .file_name()
            .ok_or("Invalid staging directory")?
            .to_string_lossy()
            .trim_start_matches("pending-")
            .to_owned();
        let generation = format!("generation-{suffix}");
        let destination = self.root.join(&generation);
        if destination.exists() {
            return Err("Live generation already exists".into());
        }
        fs::rename(stage.path(), &destination).map_err(error)?;
        // TempDir drops its old, now nonexistent path. The sealed generation is
        // retained even when pointer publication is interrupted.
        sync(&self.root)?;
        before_pointer()?;
        let pointer = serde_json::to_vec(&Pointer {
            generation,
            sha256: crate::sha256::hex(&manifest),
        })
        .map_err(error)?;
        if let Ok(old) = bytes(&self.root.join("current.json"), MAX_MANIFEST)
            && self.validate(&old).is_ok()
        {
            atomic(&self.root.join("previous.json"), &old)?;
        }
        atomic(&self.root.join("current.json"), &pointer)?;
        Ok(destination)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn staged(store: &Store, marker: u8) -> tempfile::TempDir {
        let stage = store.stage().unwrap();
        for (name, _) in PARTS {
            fs::write(stage.path().join(name), [marker; 19]).unwrap();
        }
        stage
    }
    #[test]
    fn interrupted_and_corrupted_publication_keep_previous_intact_machine_generation() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path(), "assets").unwrap();
        let first = store.publish(staged(&store, 1)).unwrap();
        assert!(
            store
                .publish_with(staged(&store, 2), || Err("cancelled".into()))
                .is_err()
        );
        assert_eq!(store.load().unwrap().path, first);
        let second = store.publish(staged(&store, 3)).unwrap();
        assert_eq!(store.load().unwrap().path, second);
        fs::write(second.join("memory.gz"), b"corrupted").unwrap();
        let fallback = store.load().unwrap();
        assert!(fallback.previous);
        assert_eq!(fallback.path, first);
        assert!(
            Store::new(root.path(), "different-assets")
                .unwrap()
                .load()
                .is_err()
        );
        assert_eq!(fs::read(first.join("memory.gz")).unwrap(), [1; 19]);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_parts_and_generation_escape_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path(), "assets").unwrap();
        let generation = store.publish(staged(&store, 1)).unwrap();
        fs::remove_file(generation.join("memory.gz")).unwrap();
        std::os::unix::fs::symlink(root.path().join("outside"), generation.join("memory.gz"))
            .unwrap();
        assert!(store.load().is_err());
        atomic(
            &store.root.join("current.json"),
            br#"{"generation":"generation-../escape","sha256":"x"}"#,
        )
        .unwrap();
        assert!(store.load().is_err());
    }
}
