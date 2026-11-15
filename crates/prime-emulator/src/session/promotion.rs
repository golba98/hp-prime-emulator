//! Crash-recoverable installation of admitted live-image drive directories.
//! Interrupted installation rolls back to the exact old trees before retrying;
//! old and rejected trees remain archived. No calculator file is deleted.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    work: String,
    drives: Vec<(String, bool)>,
}
pub(crate) struct Promotion {
    profile: PathBuf,
    plan: Plan,
    finished: bool,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn sync(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(err)?;
    }
    let _ = path;
    Ok(())
}
fn directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path).map_err(err)?.is_dir() {
        return Err("Live restore requires real directories".into());
    }
    Ok(())
}
fn drives(root: &Path) -> Result<Vec<String>> {
    directory(root)?;
    let mut names = Vec::new();
    for entry in fs::read_dir(root).map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
            directory(&entry.path())?;
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}
impl Promotion {
    pub(crate) fn prepare(profile: &Path, staged: tempfile::TempDir) -> Result<Self> {
        let base = profile.join(".live");
        directory(profile)?;
        directory(&base)?;
        if base.join("restore.json").exists() {
            return Err("An interrupted live restore must be rolled back first".into());
        }
        let mut names = drives(profile)?;
        names.extend(drives(staged.path())?);
        names.sort();
        names.dedup();
        let work = tempfile::Builder::new()
            .prefix("restore-")
            .tempdir_in(&base)
            .map_err(err)?;
        let plan = Plan {
            version: 1,
            work: work
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            drives: names
                .into_iter()
                .map(|name| {
                    let existed = profile.join(&name).exists();
                    (name, existed)
                })
                .collect(),
        };
        fs::create_dir(work.path().join("archive")).map_err(err)?;
        fs::create_dir(work.path().join("rejected")).map_err(err)?;
        fs::rename(staged.path(), work.path().join("new")).map_err(err)?;
        sync(work.path())?;
        let body = serde_json::to_vec(&plan).map_err(err)?;
        let mut file = tempfile::NamedTempFile::new_in(&base).map_err(err)?;
        file.write_all(&body).map_err(err)?;
        file.as_file().sync_all().map_err(err)?;
        // Work becomes journal-owned before its pointer is published.
        let _ = work.keep();
        file.persist(base.join("restore.json")).map_err(err)?;
        sync(&base)?;
        Ok(Self {
            profile: profile.into(),
            plan,
            finished: false,
        })
    }
    pub(crate) fn install(&self) -> Result<()> {
        self.install_with(|_| Ok(()))
    }
    fn install_with(&self, mut after_drive: impl FnMut(&str) -> Result<()>) -> Result<()> {
        let work = self.profile.join(".live").join(&self.plan.work);
        for (name, existed) in &self.plan.drives {
            let target = self.profile.join(name);
            if *existed {
                directory(&target)?;
                fs::rename(&target, work.join("archive").join(name)).map_err(err)?;
                sync(&work.join("archive"))?;
                sync(&self.profile)?;
            }
            let new = work.join("new").join(name);
            if new.exists() {
                directory(&new)?;
                fs::rename(new, &target).map_err(err)?;
                sync(&work.join("new"))?;
                sync(&self.profile)?;
            }
            after_drive(name)?;
        }
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<()> {
        fs::remove_file(self.profile.join(".live/restore.json")).map_err(err)?;
        sync(&self.profile.join(".live"))?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for Promotion {
    fn drop(&mut self) {
        if !self.finished {
            let _ = rollback_interrupted(&self.profile);
        }
    }
}
pub(crate) fn rollback_interrupted(profile: &Path) -> Result<bool> {
    let base = profile.join(".live");
    if !base.exists() {
        return Ok(false);
    }
    directory(&base)?;
    let journal = base.join("restore.json");
    if !journal.exists() {
        return Ok(false);
    }
    let meta = fs::symlink_metadata(&journal).map_err(err)?;
    if !meta.is_file() || meta.len() > 65536 {
        return Err("Invalid live restore journal file".into());
    }
    let plan: Plan = serde_json::from_slice(&fs::read(&journal).map_err(err)?).map_err(err)?;
    if plan.version != 1
        || !plan.work.starts_with("restore-")
        || plan.work.len() > 128
        || plan
            .work
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && b != b'-')
        || plan.drives.len() > 26
        || plan.drives.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || plan
            .drives
            .iter()
            .any(|(name, _)| name.len() != 1 || !name.as_bytes()[0].is_ascii_uppercase())
    {
        return Err("Invalid live restore journal schema/paths".into());
    }
    let work = base.join(&plan.work);
    for path in [
        &work,
        &work.join("archive"),
        &work.join("new"),
        &work.join("rejected"),
    ] {
        directory(path)?;
    }
    for (name, existed) in plan.drives {
        let target = profile.join(&name);
        let old = work.join("archive").join(&name);
        let rejected = work.join("rejected").join(&name);
        if old.exists() || !existed {
            if target.exists() {
                directory(&target)?;
                if rejected.exists() {
                    return Err("Ambiguous rollback; all trees preserved".into());
                }
                fs::rename(&target, &rejected).map_err(err)?;
                sync(&work.join("rejected"))?;
                sync(profile)?;
            }
            if existed {
                directory(&old)?;
                fs::rename(old, &target).map_err(err)?;
                sync(&work.join("archive"))?;
                sync(profile)?;
            }
        } else if !target.exists() {
            return Err("Missing original drive during rollback; trees preserved".into());
        }
    }
    fs::remove_file(journal).map_err(err)?;
    sync(&base)?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_drive_promotion_rolls_back_before_a_second_install() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".live")).unwrap();
        for drive in ["A", "C", "D"] {
            fs::create_dir(root.path().join(drive)).unwrap();
            fs::write(root.path().join(drive).join("old"), drive).unwrap();
        }
        let staged = tempfile::tempdir_in(root.path()).unwrap();
        for drive in ["A", "C", "E"] {
            fs::create_dir(staged.path().join(drive)).unwrap();
            fs::write(staged.path().join(drive).join("new"), drive).unwrap();
        }
        let promotion = Promotion::prepare(root.path(), staged).unwrap();
        assert!(
            promotion
                .install_with(|name| if name == "C" {
                    Err("interrupted".into())
                } else {
                    Ok(())
                })
                .is_err()
        );
        // Model abrupt death: no Drop is available to repair it.
        std::mem::forget(promotion);
        assert!(rollback_interrupted(root.path()).unwrap());
        for drive in ["A", "C", "D"] {
            assert_eq!(
                fs::read(root.path().join(drive).join("old")).unwrap(),
                drive.as_bytes()
            );
        }
        assert!(!root.path().join("E").exists());
        assert!(!rollback_interrupted(root.path()).unwrap());
        let staged = tempfile::tempdir_in(root.path()).unwrap();
        fs::create_dir(staged.path().join("C")).unwrap();
        fs::write(staged.path().join("C/new"), b"saved").unwrap();
        let promotion = Promotion::prepare(root.path(), staged).unwrap();
        promotion.install().unwrap();
        promotion.finish().unwrap();
        assert_eq!(fs::read(root.path().join("C/new")).unwrap(), b"saved");
        assert!(!root.path().join("A").exists());
        assert!(!root.path().join("D").exists());
    }
}
