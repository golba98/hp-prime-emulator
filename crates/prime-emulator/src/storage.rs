//! Host-side snapshots of a guest data directory (`--storage-diff`): what
//! the firmware created, changed or deleted during a run.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

/// Relative path (`/`-separated) → (size, sha256).
pub type Snapshot = BTreeMap<String, (u64, String)>;

/// Every regular file under `root`.
pub fn snapshot(root: &Path) -> Snapshot {
    let mut out = Snapshot::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(rel, (bytes.len() as u64, crate::sha256::hex(&bytes)));
            }
        }
    }
    out
}

/// Complete snapshot for transactional callers: unreadable directories/files
/// are errors, never evidence that unrelated files stayed unchanged.
pub fn try_snapshot(root: &Path) -> std::io::Result<Snapshot> {
    let mut out = Snapshot::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let ty = entry.file_type()?;
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let bytes = std::fs::read(&path)?;
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(rel, (bytes.len() as u64, crate::sha256::hex(&bytes)));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileChange {
    pub path: String,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
    pub sha256_after: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StorageDiff {
    pub created: Vec<FileChange>,
    pub modified: Vec<FileChange>,
    pub deleted: Vec<FileChange>,
}

pub fn diff(before: &Snapshot, after: &Snapshot) -> StorageDiff {
    let mut d = StorageDiff::default();
    for (path, (size, sha)) in after {
        match before.get(path) {
            None => d.created.push(FileChange {
                path: path.clone(),
                size_before: None,
                size_after: Some(*size),
                sha256_after: Some(sha.clone()),
            }),
            Some((old_size, old_sha)) if old_sha != sha => d.modified.push(FileChange {
                path: path.clone(),
                size_before: Some(*old_size),
                size_after: Some(*size),
                sha256_after: Some(sha.clone()),
            }),
            Some(_) => {}
        }
    }
    for (path, (size, _)) in before {
        if !after.contains_key(path) {
            d.deleted.push(FileChange {
                path: path.clone(),
                size_before: Some(*size),
                size_after: None,
                sha256_after: None,
            });
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_created_modified_and_deleted_files() {
        let dir = std::env::temp_dir().join(format!("prime-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("C/DATA")).unwrap();
        std::fs::write(dir.join("C/DATA/keep"), b"same").unwrap();
        std::fs::write(dir.join("C/DATA/change"), b"old").unwrap();
        std::fs::write(dir.join("C/DATA/gone"), b"x").unwrap();
        let before = snapshot(&dir);
        std::fs::write(dir.join("C/DATA/change"), b"newer").unwrap();
        std::fs::remove_file(dir.join("C/DATA/gone")).unwrap();
        std::fs::write(dir.join("C/DATA/new.hpprgm"), b"prog").unwrap();
        let d = diff(&before, &snapshot(&dir));
        let paths = |v: &[FileChange]| v.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&d.created), vec!["C/DATA/new.hpprgm"]);
        assert_eq!(paths(&d.modified), vec!["C/DATA/change"]);
        assert_eq!(d.modified[0].size_before, Some(3));
        assert_eq!(d.modified[0].size_after, Some(5));
        assert_eq!(paths(&d.deleted), vec!["C/DATA/gone"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
