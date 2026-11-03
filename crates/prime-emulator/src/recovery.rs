//! Durable firmware-OFF checkpoints. Never serializes live CPU/host state.
use ppl_engine::checkpoint::{
    CheckpointKind, CheckpointReceipt, CheckpointSafePoint, RECEIPT_VERSION,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    firmware: String,
    trigger: String,
    files: BTreeMap<String, String>,
    directories: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Restore {
    generation: String,
    work: String,
    drives: Vec<String>,
}
pub struct Recovery {
    root: PathBuf,
    firmware: String,
}
/// Opt-in limits for explicit persistence checkpoints. Existing automatic OFF
/// generations and interrupted-restore archives are never silently pruned.
#[derive(Debug, Clone)]
pub struct CheckpointPolicy {
    pub max_checkpoint_bytes: u64,
    /// Counts regular-file bytes throughout `.recovery`, including archives
    /// and interrupted writes. Filesystem block/metadata overhead is excluded.
    pub max_recovery_bytes: u64,
    pub max_entries: usize,
    /// At least two intact compatible generations must remain.
    pub keep_generations: usize,
}
impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            max_checkpoint_bytes: 256 * 1024 * 1024,
            max_recovery_bytes: 1024 * 1024 * 1024,
            max_entries: 100_000,
            keep_generations: 4,
        }
    }
}
struct CopyBudget {
    bytes: u64,
    entries: usize,
}
impl CopyBudget {
    fn unlimited() -> Self {
        Self {
            bytes: u64::MAX,
            entries: usize::MAX,
        }
    }
    fn entry(&mut self) -> io::Result<()> {
        self.entries = self
            .entries
            .checked_sub(1)
            .ok_or_else(|| invalid("checkpoint entry budget exceeded"))?;
        Ok(())
    }
}
fn invalid(s: impl Into<String>) -> io::Error {
    io::Error::other(s.into())
}
fn sync(p: &Path) -> io::Result<()> {
    fs::File::open(p)?.sync_all()
}
fn write_new(p: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)?;
    f.write_all(bytes)?;
    f.sync_all()
}
fn file_hash(source: &Path, target: Option<&Path>) -> io::Result<String> {
    file_hash_limit(source, target, u64::MAX).map(|(hash, _)| hash)
}
fn file_hash_limit(source: &Path, target: Option<&Path>, limit: u64) -> io::Result<(String, u64)> {
    let mut input = fs::File::open(source)?;
    let mut output = target
        .map(|p| fs::OpenOptions::new().write(true).create_new(true).open(p))
        .transpose()?;
    let mut hash = crate::sha256::Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .filter(|bytes| *bytes <= limit)
            .ok_or_else(|| invalid("checkpoint byte budget exceeded"))?;
        hash.update(&buffer[..count]);
        if let Some(f) = output.as_mut() {
            f.write_all(&buffer[..count])?;
        }
    }
    if let Some(f) = output {
        f.sync_all()?;
    }
    Ok((hash.finalize_hex(), bytes))
}
pub(crate) fn resource_hash(source: &Path) -> io::Result<String> {
    file_hash(source, None)
}

fn relative(s: &str) -> bool {
    !s.is_empty()
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn drives(root: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for e in fs::read_dir(root)? {
        let e = e?;
        let n = e.file_name().to_string_lossy().into_owned();
        if n.len() == 1 && n.as_bytes()[0].is_ascii_uppercase() {
            if !e.file_type()?.is_dir() {
                return Err(invalid("guest drive is not a directory"));
            }
            out.push(n);
        }
    }
    out.sort();
    Ok(out)
}
fn copy_tree(
    source: &Path,
    dest: &Path,
    relative: &str,
    manifest: &mut Manifest,
) -> io::Result<()> {
    copy_tree_bounded(
        source,
        dest,
        relative,
        manifest,
        &mut CopyBudget::unlimited(),
    )
}
fn copy_tree_bounded(
    source: &Path,
    dest: &Path,
    relative: &str,
    manifest: &mut Manifest,
    budget: &mut CopyBudget,
) -> io::Result<()> {
    budget.entry()?;
    if !fs::symlink_metadata(source)?.is_dir() {
        return Err(invalid("checkpoint refuses symbolic links"));
    }
    fs::create_dir(dest)?;
    manifest.directories.push(relative.into());
    for e in fs::read_dir(source)? {
        let e = e?;
        let name = e
            .file_name()
            .into_string()
            .map_err(|_| invalid("non UTF-8 storage path"))?;
        let rel = format!("{relative}/{name}");
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_tree_bounded(&e.path(), &dest.join(name), &rel, manifest, budget)?;
        } else if ty.is_file() {
            budget.entry()?;
            let (hash, bytes) = file_hash_limit(&e.path(), Some(&dest.join(name)), budget.bytes)?;
            budget.bytes -= bytes;
            manifest.files.insert(rel, hash);
        } else {
            return Err(invalid(
                "checkpoint refuses symbolic links and special files",
            ));
        }
    }
    sync(dest)
}
fn tree_bytes(root: &Path, entries: &mut usize) -> io::Result<u64> {
    *entries = entries
        .checked_sub(1)
        .ok_or_else(|| invalid("recovery entry budget exceeded"))?;
    let meta = fs::symlink_metadata(root)?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Err(invalid(
            "recovery budget scan refuses symbolic links and special files",
        ));
    }
    let mut bytes = 0u64;
    for e in fs::read_dir(root)? {
        bytes = bytes
            .checked_add(tree_bytes(&e?.path(), entries)?)
            .ok_or_else(|| invalid("recovery byte count overflow"))?;
    }
    Ok(bytes)
}
impl Recovery {
    pub fn resource_identity(&self) -> &str {
        &self.firmware
    }

    /// Called before bootstrap mutations and while holding the helper lock.
    pub fn begin(root: &Path, firmware: String) -> io::Result<(Self, bool)> {
        fs::create_dir_all(root)?;
        let r = Self {
            root: root.to_owned(),
            firmware,
        };
        fs::create_dir_all(r.base())?;
        if fs::symlink_metadata(r.base())?.file_type().is_symlink() {
            return Err(invalid("recovery directory is a symbolic link"));
        }
        sync(root)?;
        let dirty = r.base().join("unclean");
        let recovering = dirty.exists();
        if r.base().join("restore.json").exists() {
            r.finish_restore()?;
        } else if recovering {
            r.restore_latest()?;
        }
        if !recovering {
            r.checkpoint_as("startup_baseline")?;
        }
        if !dirty.exists() {
            write_new(&dirty, b"session active\n")?;
            sync(&r.base())?;
        }
        Ok((r, recovering))
    }
    fn base(&self) -> PathBuf {
        self.root.join(".recovery")
    }
    /// Caller guarantees completed firmware OFF and excludes all guest mutation.
    pub fn checkpoint(&self) -> io::Result<()> {
        self.checkpoint_receipt().map(|_| ())
    }
    /// Same safe-point contract as [`Self::checkpoint`], with an exact receipt.
    /// This does not prune existing recovery evidence.
    pub fn checkpoint_receipt(&self) -> io::Result<CheckpointReceipt> {
        self.publish("firmware_off", None, &mut || Ok(()))
    }
    /// Explicit, opt-in bounded checkpoint. Old compatible generations may be
    /// pruned only after a new generation has been durably published. Restore
    /// archives, interrupted writes and incompatible/corrupt evidence remain.
    pub fn checkpoint_bounded(&self, policy: &CheckpointPolicy) -> io::Result<CheckpointReceipt> {
        if policy.keep_generations < 2 || policy.max_entries == 0 {
            return Err(invalid(
                "checkpoint policy must retain at least two generations and permit entries",
            ));
        }
        if self.base().join("restore.json").exists() {
            return Err(invalid("cannot checkpoint during an unfinished restore"));
        }
        let receipt = self.publish("firmware_off", Some(policy), &mut || Ok(()))?;
        // Failure to prune is not a failed save: the receipt already identifies
        // a durable generation. Keep the extra evidence rather than misreporting
        // the successfully committed checkpoint as absent.
        let _ = self.prune(policy.keep_generations);
        Ok(receipt)
    }
    fn checkpoint_as(&self, trigger: &str) -> io::Result<()> {
        self.publish(trigger, None, &mut || Ok(())).map(|_| ())
    }
    fn publish(
        &self,
        trigger: &str,
        policy: Option<&CheckpointPolicy>,
        before_publish: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<CheckpointReceipt> {
        let mut budget = CopyBudget::unlimited();
        let mut used = 0;
        if let Some(policy) = policy {
            let mut entries = policy.max_entries;
            used = tree_bytes(&self.base(), &mut entries)?;
            let available = policy.max_recovery_bytes.checked_sub(used).ok_or_else(|| {
                invalid("recovery disk budget already exceeded; existing data retained")
            })?;
            budget = CopyBudget {
                bytes: policy.max_checkpoint_bytes.min(available),
                entries: policy.max_entries,
            };
        }
        let initial_budget = budget.bytes;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(invalid_time)?
            .as_nanos();
        let id = self.next_id_at(now)?;
        let name = format!("generation-{id:039}");
        let pending = self.base().join(format!("{name}.pending"));
        fs::create_dir(&pending)?;
        let result = (|| {
            let mut m = Manifest {
                version: 1,
                firmware: self.firmware.clone(),
                trigger: trigger.into(),
                files: BTreeMap::new(),
                directories: Vec::new(),
            };
            for d in drives(&self.root)? {
                copy_tree_bounded(
                    &self.root.join(&d),
                    &pending.join(&d),
                    &d,
                    &mut m,
                    &mut budget,
                )?;
            }
            let payload_bytes = initial_budget - budget.bytes;
            let bytes = serde_json::to_vec(&m)?;
            let manifest_sha256 = crate::sha256::hex(&bytes);
            if let Some(policy) = policy {
                let size = used
                    .checked_add(payload_bytes)
                    .and_then(|n| n.checked_add(bytes.len() as u64 + manifest_sha256.len() as u64))
                    .ok_or_else(|| invalid("recovery byte count overflow"))?;
                if size > policy.max_recovery_bytes {
                    return Err(invalid(
                        "recovery disk budget exceeded; previous generations retained",
                    ));
                }
            }
            write_new(&pending.join("manifest.json"), &bytes)?;
            write_new(&pending.join("manifest.sha256"), manifest_sha256.as_bytes())?;
            sync(&pending)?;
            // Test fault injection occurs after every payload/manifest fsync but
            // before the atomic publication boundary.
            before_publish()?;
            fs::rename(&pending, self.base().join(&name))?;
            sync(&self.base())?;
            Ok(CheckpointReceipt {
                version: RECEIPT_VERSION,
                kind: CheckpointKind::FirmwarePersistence,
                safe_point: CheckpointSafePoint::FirmwareOff,
                generation: name,
                firmware: self.firmware.clone(),
                manifest_sha256,
                bytes: payload_bytes,
                files: m.files.len() as u64,
                restores_cpu: false,
                restores_framebuffer: false,
            })
        })();
        if result.is_err() && pending.exists() {
            // Only this call's newly created staging directory is removed.
            // A killed process leaves .pending, which restore ignores.
            let _ = fs::remove_dir_all(&pending);
        }
        result
    }
    fn prune(&self, retain: usize) -> io::Result<()> {
        if self.base().join("restore.json").exists() {
            return Err(invalid("cannot prune during an unfinished restore"));
        }
        let mut compatible = Vec::new();
        for e in fs::read_dir(self.base())? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name
                .strip_prefix("generation-")
                .is_some_and(|id| id.len() == 39 && id.bytes().all(|c| c.is_ascii_digit()))
                && self.validate(&e.path()).is_ok()
            {
                compatible.push(e.path());
            }
        }
        compatible.sort();
        let remove = compatible.len().saturating_sub(retain.max(2));
        for path in compatible.into_iter().take(remove) {
            fs::remove_dir_all(path)?;
        }
        sync(&self.base())
    }
    // The helper storage lock serializes writers. Wall time alone cannot
    // order OFF generations when the host clock moves backwards.
    fn next_id_at(&self, now: u128) -> io::Result<u128> {
        let mut next = now;
        for entry in fs::read_dir(self.base())? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if let Some(id) = name
                .strip_prefix("generation-")
                .and_then(|n| n.strip_suffix(".pending").or(Some(n)))
                .and_then(|n| n.parse::<u128>().ok())
            {
                next = next.max(
                    id.checked_add(1)
                        .ok_or_else(|| invalid("recovery generation counter exhausted"))?,
                );
            }
        }
        Ok(next)
    }
    pub fn clean_shutdown(&self) -> io::Result<()> {
        fs::remove_file(self.base().join("unclean"))?;
        sync(&self.base())
    }
    fn validate(&self, dir: &Path) -> io::Result<Manifest> {
        if !fs::symlink_metadata(dir)?.is_dir() {
            return Err(invalid("invalid generation directory"));
        }
        let bytes = fs::read(dir.join("manifest.json"))?;
        if fs::read_to_string(dir.join("manifest.sha256"))? != crate::sha256::hex(&bytes) {
            return Err(invalid("manifest checksum mismatch"));
        }
        let m: Manifest = serde_json::from_slice(&bytes)?;
        if m.version != 1 || m.firmware != self.firmware {
            return Err(invalid("incompatible recovery generation"));
        }
        for path in m.directories.iter().chain(m.files.keys()) {
            if !relative(path) {
                return Err(invalid("invalid recovery path"));
            }
            let first = path.split('/').next().unwrap();
            if first.len() != 1 || !first.as_bytes()[0].is_ascii_uppercase() {
                return Err(invalid("invalid recovery drive"));
            }
            // Check every component; a parent symlink is unsafe too.
            let mut p = dir.to_owned();
            for c in Path::new(path).components() {
                p.push(c);
                if fs::symlink_metadata(&p)?.file_type().is_symlink() {
                    return Err(invalid("recovery symlink"));
                }
            }
        }
        for (p, hash) in &m.files {
            if file_hash(&dir.join(p), None)? != *hash {
                return Err(invalid("recovery checksum mismatch"));
            }
        }
        for d in &m.directories {
            if !dir.join(d).is_dir() {
                return Err(invalid("missing recovery directory"));
            }
        }
        Ok(m)
    }
    fn restore_latest(&self) -> io::Result<()> {
        let mut candidates = fs::read_dir(self.base())?.collect::<Result<Vec<_>, _>>()?;
        candidates.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for e in candidates {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n.starts_with("generation-") || n.ends_with(".pending") || !e.file_type()?.is_dir()
            {
                continue;
            }
            if let Ok(m) = self.validate(&e.path()) {
                let mut ds = drives(&self.root)?;
                for d in &m.directories {
                    if d.len() == 1 && !ds.contains(d) {
                        ds.push(d.clone());
                    }
                }
                let work_name = format!(
                    "restore-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(invalid_time)?
                        .as_nanos()
                );
                let work = self.base().join(&work_name);
                fs::create_dir(&work)?;
                fs::create_dir(work.join("new"))?;
                fs::create_dir(work.join("archive"))?;
                let mut copied = Manifest {
                    version: 1,
                    firmware: self.firmware.clone(),
                    trigger: "restore".into(),
                    files: BTreeMap::new(),
                    directories: Vec::new(),
                };
                for d in m.directories.iter().filter(|d| d.len() == 1) {
                    copy_tree(&e.path().join(d), &work.join("new").join(d), d, &mut copied)?;
                }
                if copied.files != m.files {
                    return Err(invalid("checkpoint contains unrecorded files"));
                }
                sync(&work.join("new"))?;
                sync(&work)?;
                write_new(
                    &self.base().join("restore.json"),
                    &serde_json::to_vec(&Restore {
                        generation: n,
                        work: work_name,
                        drives: ds,
                    })?,
                )?;
                sync(&self.base())?;
                return self.finish_restore();
            }
        }
        Err(invalid(format!(
            "Unclean calculator shutdown: no intact compatible checkpoint; original data preserved at {}",
            self.root.display()
        )))
    }
    fn finish_restore(&self) -> io::Result<()> {
        let plan: Restore = serde_json::from_slice(&fs::read(self.base().join("restore.json"))?)?;
        if !relative(&plan.generation)
            || !plan.generation.starts_with("generation-")
            || plan.generation.contains('/')
        {
            return Err(invalid("invalid restore journal"));
        }
        let manifest = self.validate(&self.base().join(&plan.generation))?;
        if !relative(&plan.work) || !plan.work.starts_with("restore-") || plan.work.contains('/') {
            return Err(invalid("invalid restore work directory"));
        }
        let work = self.base().join(plan.work);
        // Validate every staged (or already installed) drive before moving
        // any current drive. Interrupted copies must never become authoritative.
        let mut seen = std::collections::BTreeSet::new();
        for d in &plan.drives {
            if d.len() != 1 || !d.as_bytes()[0].is_ascii_uppercase() || !seen.insert(d) {
                return Err(invalid("invalid or duplicate restore drive"));
            }
            if manifest.directories.contains(d) {
                let staged = work.join("new").join(d);
                let source = if staged.exists() {
                    staged
                } else {
                    self.root.join(d)
                };
                validate_drive(&source, d, &manifest)?;
            }
        }
        for d in plan.drives {
            if d.len() != 1 || !d.as_bytes()[0].is_ascii_uppercase() {
                return Err(invalid("invalid restore drive"));
            }
            let target = self.root.join(&d);
            let archive = work.join("archive").join(&d);
            let new = work.join("new").join(&d);
            if new.exists() {
                if target.exists() {
                    if archive.exists() {
                        return Err(invalid("ambiguous interrupted restore; data preserved"));
                    }
                    fs::rename(&target, &archive)?;
                    sync(&work.join("archive"))?;
                    sync(&self.root)?;
                }
                fs::rename(&new, &target)?;
                sync(&work.join("new"))?;
                sync(&self.root)?;
            } else if !self.base().join(&plan.generation).join(&d).exists() && target.exists() {
                if archive.exists() {
                    return Err(invalid("ambiguous extra drive during restore"));
                }
                fs::rename(&target, &archive)?;
                sync(&work.join("archive"))?;
                sync(&self.root)?;
            } else if !target.exists() && self.base().join(&plan.generation).join(&d).exists() {
                return Err(invalid("missing staged restore drive"));
            }
        }
        fs::remove_file(self.base().join("restore.json"))?;
        sync(&self.base())
    }
}
fn validate_drive(source: &Path, drive: &str, manifest: &Manifest) -> io::Result<()> {
    fn scan(
        p: &Path,
        rel: &str,
        files: &mut BTreeMap<String, String>,
        dirs: &mut Vec<String>,
    ) -> io::Result<()> {
        if !fs::symlink_metadata(p)?.is_dir() {
            return Err(invalid("invalid staged directory"));
        }
        dirs.push(rel.into());
        for entry in fs::read_dir(p)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("non UTF-8 staged path"))?;
            let rel = format!("{rel}/{name}");
            let ty = entry.file_type()?;
            if ty.is_dir() {
                scan(&entry.path(), &rel, files, dirs)?;
            } else if ty.is_file() {
                files.insert(rel, file_hash(&entry.path(), None)?);
            } else {
                return Err(invalid("staged symbolic link or special file"));
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    let mut dirs = Vec::new();
    scan(source, drive, &mut files, &mut dirs)?;
    let prefix = format!("{drive}/");
    let expected_files = manifest
        .files
        .iter()
        .filter(|(p, _)| p.starts_with(&prefix))
        .map(|(p, h)| (p.clone(), h.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut expected_dirs = manifest
        .directories
        .iter()
        .filter(|p| *p == drive || p.starts_with(&prefix))
        .cloned()
        .collect::<Vec<_>>();
    dirs.sort();
    expected_dirs.sort();
    if files != expected_files || dirs != expected_dirs {
        return Err(invalid("staged recovery drive integrity mismatch"));
    }
    Ok(())
}
fn invalid_time(e: std::time::SystemTimeError) -> io::Error {
    invalid(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Recovery) {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("C/DATA")).unwrap();
        fs::write(d.path().join("C/DATA/value"), b"saved").unwrap();
        let (r, _) = Recovery::begin(d.path(), "fw".into()).unwrap();
        (d, r)
    }
    fn generations(r: &Recovery) -> Vec<PathBuf> {
        let mut out = fs::read_dir(r.base())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.join("manifest.json").is_file() && !p.to_string_lossy().ends_with(".pending")
            })
            .collect::<Vec<_>>();
        out.sort();
        out
    }
    #[test]
    fn receipt_identifies_the_exact_durable_partial_checkpoint() {
        let (_d, r) = setup();
        let receipt = r.checkpoint_receipt().unwrap();
        let dir = r.base().join(&receipt.generation);
        assert!(r.validate(&dir).is_ok());
        assert_eq!(
            receipt.manifest_sha256,
            crate::sha256::hex(&fs::read(dir.join("manifest.json")).unwrap())
        );
        assert_eq!(receipt.bytes, 5);
        assert_eq!(receipt.files, 1);
        assert_eq!(receipt.firmware, "fw");
        assert!(!receipt.restores_cpu);
        assert!(!receipt.restores_framebuffer);
    }
    #[test]
    fn failed_publish_retains_previous_generation_and_original_data() {
        let (d, r) = setup();
        let before = generations(&r);
        fs::write(d.path().join("C/DATA/value"), b"changed").unwrap();
        let result = r.publish("firmware_off", None, &mut || {
            Err(invalid("injected interrupted publication"))
        });
        assert!(result.is_err());
        assert_eq!(generations(&r), before);
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"changed");
        Recovery::begin(d.path(), "fw".into()).unwrap();
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
    }
    #[test]
    fn payload_and_entry_limits_preserve_known_good_save() {
        let (d, r) = setup();
        let before = generations(&r);
        let mut policy = CheckpointPolicy {
            max_checkpoint_bytes: 4,
            ..Default::default()
        };
        assert!(r.checkpoint_bounded(&policy).is_err());
        assert_eq!(generations(&r), before);
        policy.max_checkpoint_bytes = 100;
        policy.max_entries = 1;
        assert!(r.checkpoint_bounded(&policy).is_err());
        assert_eq!(generations(&r), before);
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
    }
    #[test]
    fn total_budget_includes_archives_and_abandoned_pending_data() {
        let (_d, r) = setup();
        fs::create_dir(r.base().join("restore-preserved")).unwrap();
        fs::write(r.base().join("restore-preserved/archive"), vec![0u8; 4096]).unwrap();
        fs::create_dir(r.base().join("generation-999.pending")).unwrap();
        fs::write(
            r.base().join("generation-999.pending/incomplete"),
            vec![0u8; 4096],
        )
        .unwrap();
        let policy = CheckpointPolicy {
            max_recovery_bytes: 8192,
            ..Default::default()
        };
        assert!(r.checkpoint_bounded(&policy).is_err());
        assert!(r.base().join("restore-preserved/archive").exists());
        assert!(r.base().join("generation-999.pending/incomplete").exists());
        assert_eq!(generations(&r).len(), 1);
    }
    #[test]
    fn retention_preserves_two_valid_generations_and_unrelated_evidence() {
        let (_d, r) = setup();
        for _ in 0..4 {
            r.checkpoint().unwrap();
        }
        let bad = r
            .base()
            .join("generation-000000000000000000000000000000000000001");
        fs::create_dir(&bad).unwrap();
        fs::write(bad.join("manifest.json"), b"bad retained evidence").unwrap();
        fs::create_dir(r.base().join("restore-preserved")).unwrap();
        let policy = CheckpointPolicy {
            keep_generations: 2,
            ..Default::default()
        };
        let saved = r.checkpoint_bounded(&policy).unwrap();
        let valid = generations(&r)
            .into_iter()
            .filter(|p| r.validate(p).is_ok())
            .collect::<Vec<_>>();
        assert_eq!(valid.len(), 2);
        assert!(r.base().join(saved.generation).exists());
        assert!(bad.join("manifest.json").exists());
        assert!(r.base().join("restore-preserved").exists());
        let invalid_policy = CheckpointPolicy {
            keep_generations: 1,
            ..Default::default()
        };
        assert!(r.checkpoint_bounded(&invalid_policy).is_err());
    }
    #[test]
    fn unfinished_restore_prevents_bounded_save_and_retention() {
        let (_d, r) = setup();
        fs::write(r.base().join("restore.json"), b"unfinished journal").unwrap();
        assert!(r.checkpoint_bounded(&CheckpointPolicy::default()).is_err());
        assert!(r.prune(2).is_err());
        assert_eq!(generations(&r).len(), 1);
    }
    #[test]
    fn generation_order_survives_clock_rollback_and_pending_copy() {
        let (_d, r) = setup();
        let next = r.next_id_at(0).unwrap();
        fs::create_dir(r.base().join(format!("generation-{next:039}.pending"))).unwrap();
        assert_eq!(r.next_id_at(0).unwrap(), next + 1);
        assert_eq!(r.next_id_at(next).unwrap(), next + 1);
        fs::create_dir(r.base().join(format!("generation-{:039}", u128::MAX))).unwrap();
        assert!(r.next_id_at(0).is_err());
    }
    #[test]
    fn corrupt_staged_drive_is_rejected_before_original_moves() {
        let (d, r) = setup();
        let generation = fs::read_dir(r.base())
            .unwrap()
            .map(|e| e.unwrap())
            .find(|e| e.path().join("manifest.json").is_file())
            .unwrap();
        let m = r.validate(&generation.path()).unwrap();
        let work = r.base().join("restore-corrupt");
        fs::create_dir_all(work.join("new")).unwrap();
        fs::create_dir(work.join("archive")).unwrap();
        let mut copied = Manifest {
            version: 1,
            firmware: "fw".into(),
            trigger: "test".into(),
            files: BTreeMap::new(),
            directories: vec![],
        };
        copy_tree(
            &generation.path().join("C"),
            &work.join("new/C"),
            "C",
            &mut copied,
        )
        .unwrap();
        fs::write(work.join("new/C/DATA/value"), b"truncated").unwrap();
        assert!(validate_drive(&work.join("new/C"), "C", &m).is_err());
        write_new(
            &r.base().join("restore.json"),
            &serde_json::to_vec(&Restore {
                generation: generation.file_name().into_string().unwrap(),
                work: "restore-corrupt".into(),
                drives: vec!["C".into()],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(r.finish_restore().is_err());
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
        assert!(!work.join("archive/C").exists());
    }
    #[test]
    fn crash_recovers_saved_data_and_preserves_interrupted_files() {
        let (d, r) = setup();
        r.checkpoint().unwrap();
        fs::write(d.path().join("C/DATA/value"), b"unsaved").unwrap();
        let (_, restored) = Recovery::begin(d.path(), "fw".into()).unwrap();
        assert!(restored);
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
        assert!(
            fs::read_dir(r.base()).unwrap().any(|e| e
                .unwrap()
                .path()
                .join("archive/C/DATA/value")
                .exists())
        );
    }
    #[test]
    fn corrupt_newest_falls_back_and_pending_is_ignored() {
        let (d, r) = setup();
        r.checkpoint().unwrap();
        fs::write(d.path().join("C/DATA/value"), b"new").unwrap();
        r.checkpoint().unwrap();
        let mut dirs = fs::read_dir(r.base())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("manifest.json").exists())
            .collect::<Vec<_>>();
        dirs.sort();
        fs::write(dirs.last().unwrap().join("C/DATA/value"), b"corrupt").unwrap();
        fs::create_dir(r.base().join("generation-999.pending")).unwrap();
        Recovery::begin(d.path(), "fw".into()).unwrap();
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
    }
    #[test]
    fn incompatible_or_missing_checkpoint_never_resets_data() {
        let (d, r) = setup();
        r.checkpoint().unwrap();
        assert!(Recovery::begin(d.path(), "other".into()).is_err());
        assert!(d.path().join("C/DATA/value").exists());
    }
    #[test]
    fn clean_shutdown_keeps_current_data() {
        let (d, r) = setup();
        r.checkpoint().unwrap();
        r.clean_shutdown().unwrap();
        let (_, restored) = Recovery::begin(d.path(), "fw".into()).unwrap();
        assert!(!restored);
    }
    #[test]
    fn repeated_crashes_keep_distinct_archives() {
        let (d, r) = setup();
        r.checkpoint().unwrap();
        for _ in 0..2 {
            fs::write(d.path().join("C/DATA/value"), b"unsaved").unwrap();
            Recovery::begin(d.path(), "fw".into()).unwrap();
            assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
        }
        assert_eq!(
            fs::read_dir(r.base())
                .unwrap()
                .filter(|e| e.as_ref().unwrap().path().join("archive").is_dir())
                .count(),
            2
        );
    }
    #[test]
    fn restore_resumes_after_original_drive_was_archived() {
        let (d, r) = setup();
        let generation = fs::read_dir(r.base())
            .unwrap()
            .map(|e| e.unwrap())
            .find(|e| e.path().join("manifest.json").is_file())
            .unwrap();
        let work = r.base().join("restore-test");
        fs::create_dir(&work).unwrap();
        fs::create_dir(work.join("new")).unwrap();
        fs::create_dir(work.join("archive")).unwrap();
        let mut m = r.validate(&generation.path()).unwrap();
        m.files.clear();
        m.directories.clear();
        copy_tree(
            &generation.path().join("C"),
            &work.join("new/C"),
            "C",
            &mut m,
        )
        .unwrap();
        fs::rename(d.path().join("C"), work.join("archive/C")).unwrap();
        write_new(
            &r.base().join("restore.json"),
            &serde_json::to_vec(&Restore {
                generation: generation.file_name().into_string().unwrap(),
                work: "restore-test".into(),
                drives: vec!["C".into()],
            })
            .unwrap(),
        )
        .unwrap();
        Recovery::begin(d.path(), "fw".into()).unwrap();
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
        assert!(!r.base().join("restore.json").exists());
        assert!(work.join("archive/C/DATA/value").exists());
    }
    #[test]
    fn corrupted_manifest_never_replaces_working_drives() {
        let (d, r) = setup();
        for e in fs::read_dir(r.base()).unwrap() {
            let p = e.unwrap().path().join("manifest.json");
            if p.exists() {
                fs::write(p, b"{}").unwrap();
            }
        }
        assert!(Recovery::begin(d.path(), "fw".into()).is_err());
        assert_eq!(fs::read(d.path().join("C/DATA/value")).unwrap(), b"saved");
    }
    #[cfg(unix)]
    #[test]
    fn symlink_is_never_copied() {
        let (d, r) = setup();
        std::os::unix::fs::symlink("/etc/passwd", d.path().join("C/link")).unwrap();
        assert!(r.checkpoint().is_err());
    }
}
