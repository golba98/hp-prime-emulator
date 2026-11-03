//! Two published generations for an exclusively owned synthetic snapshot path.
//! Loading reports recovery explicitly and never promotes a file automatically.
use super::{Result, SyntheticProgram, SyntheticSnapshot};
use std::path::{Path, PathBuf};

/// Recovery provenance is explicit; a previous generation also retains the
/// diagnostic explaining why the current generation was not selected.
pub enum SnapshotSelection {
    Current(SyntheticSnapshot),
    Previous {
        snapshot: SyntheticSnapshot,
        current_error: String,
    },
}
impl SnapshotSelection {
    pub fn snapshot(&self) -> &SyntheticSnapshot {
        match self {
            Self::Current(snapshot) | Self::Previous { snapshot, .. } => snapshot,
        }
    }
}

/// Caller must exclusively own this path and its private stable directory.
/// At most two generations are published: `path` and `path.previous`.
/// Atomic staging files left by killed writers are never recovery candidates.
pub struct SyntheticSnapshotStore {
    current: PathBuf,
    previous: PathBuf,
}
impl SyntheticSnapshotStore {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let current = path.into();
        let mut previous_name = current
            .file_name()
            .ok_or("synthetic store requires a snapshot filename")?
            .to_os_string();
        previous_name.push(".previous");
        let previous = current.with_file_name(previous_name);
        Ok(Self { current, previous })
    }
    pub fn current_path(&self) -> &Path {
        &self.current
    }
    pub fn previous_path(&self) -> &Path {
        &self.previous
    }

    /// Both candidates must pass the complete original-program and backend
    /// validation. A wrong-program snapshot is never accepted as a fallback.
    pub fn load(&self, program: &SyntheticProgram) -> Result<SnapshotSelection> {
        match SyntheticSnapshot::load(&self.current, program) {
            Ok(snapshot) => Ok(SnapshotSelection::Current(snapshot)),
            Err(current_error) => match SyntheticSnapshot::load(&self.previous, program) {
                Ok(snapshot) => Ok(SnapshotSelection::Previous {
                    snapshot,
                    current_error,
                }),
                Err(previous_error) => Err(format!(
                    "no compatible synthetic snapshot; current: {current_error}; previous: {previous_error}"
                )),
            },
        }
    }

    /// Validate the new snapshot before touching either generation. Only a fully
    /// validated compatible current snapshot may replace the previous one. A
    /// corrupt/missing/incompatible current file cannot overwrite a good backup.
    /// Each publication is individually atomic and durable; this is not a
    /// transaction spanning two renames. Failure may leave equal generations.
    pub fn save(&mut self, snapshot: &SyntheticSnapshot, program: &SyntheticProgram) -> Result<()> {
        snapshot.validate(program)?;
        if let Ok(current) = SyntheticSnapshot::load(&self.current, program) {
            // Repeated saves of identical state must not erase a useful older
            // generation. Both digests were fully checked by load/validate.
            if current.manifest.manifest_sha256 == snapshot.manifest.manifest_sha256 {
                return Ok(());
            }
            current.save(&self.previous, program)?;
            #[cfg(test)]
            super::tests::interrupt_rotated_save(&self.current)?;
        }
        snapshot.save(&self.current, program)
    }
}
