//! Durable deletion operations and device/type/content-bound creation receipts.
//! No HID traffic belongs here. All writes are synced before mutation proceeds.
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ppl_device::{Backup, ControlObject, DeleteTarget, DeviceError, FileType, NamePolicy};
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, DeviceError>;
fn disk(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Hid(format!("deletion record: {e}"))
}
pub fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        // No portable directory-fsync equivalent; durability is left to the OS.
        let _ = path;
    }
    Ok(())
}
fn durable_directory(path: &Path) -> io::Result<()> {
    let missing: Vec<_> = path
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty() && !p.exists())
        .collect();
    std::fs::create_dir_all(path)?;
    for dir in missing {
        sync_dir(dir)?;
        if let Some(parent) = dir.parent() {
            sync_dir(if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            })?;
        }
    }
    Ok(())
}
fn immutable(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    sync_dir(path.parent().unwrap())
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path.parent().unwrap();
    durable_directory(parent)?;
    let temp = parent.join(format!(".{}.tmp", ppl_device::fresh_test_name()));
    immutable(&temp, &serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&temp, path)?;
    sync_dir(parent)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    schema_version: u32,
    pub device_serial_id: String,
    pub ty: String,
    pub name: String,
    pub sha256: String,
    pub created_utc: String,
    pub active: bool,
}
fn receipt_path(root: &Path, serial: &str, ty: FileType, name: &str) -> Result<PathBuf> {
    ppl_device::validate_test_name(name)?;
    if serial.len() != 16 || !serial.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(DeviceError::Refused(
            "invalid device identity for ownership receipt".into(),
        ));
    }
    Ok(root
        .join("ownership")
        .join(serial)
        .join(format!("{}-{name}.json", ty.label())))
}
pub fn record_creation(
    root: &Path,
    serial: &str,
    ty: FileType,
    name: &str,
    data: &[u8],
) -> Result<()> {
    let receipt = Receipt {
        schema_version: 1,
        device_serial_id: serial.into(),
        ty: ty.label().into(),
        name: name.into(),
        sha256: ppl_compat::sha256_hex(data),
        created_utc: ppl_compat::utc_now(),
        active: true,
    };
    atomic_json(&receipt_path(root, serial, ty, name)?, &receipt).map_err(disk)
}
pub fn owns(root: &Path, serial: &str, b: &Backup) -> Result<bool> {
    let path = receipt_path(root, serial, b.ty, &b.name)?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(disk(e)),
    };
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(disk)?;
    Ok(receipt.schema_version == 1
        && receipt.active
        && receipt.device_serial_id == serial
        && receipt.ty == b.ty.label()
        && receipt.name == b.name
        && receipt.sha256 == ppl_compat::sha256_hex(&b.data))
}
pub fn retire(root: &Path, serial: &str, b: &Backup) -> Result<()> {
    if !owns(root, serial, b)? {
        return Ok(());
    }
    let path = receipt_path(root, serial, b.ty, &b.name)?;
    let mut receipt: Receipt =
        serde_json::from_slice(&std::fs::read(&path).map_err(disk)?).map_err(disk)?;
    receipt.active = false;
    atomic_json(&path, &receipt).map_err(disk)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub ty: String,
    pub sha256: String,
    pub bytes: usize,
    pub status: String,
}
impl Target {
    pub fn target(&self, policy: NamePolicy) -> Result<DeleteTarget> {
        let ty = FileType::parse(&self.ty)
            .filter(|t| matches!(t, FileType::Program | FileType::Note))
            .ok_or_else(|| DeviceError::Refused("invalid deletion record type".into()))?;
        match policy {
            NamePolicy::Test => ppl_device::validate_test_name(&self.name)?,
            NamePolicy::User => ppl_device::validate_object_name(ty, &self.name)?,
        }
        Ok(DeleteTarget {
            ty,
            name: self.name.clone(),
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub ty: String,
    pub sha256: String,
    pub text: String,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub schema_version: u32,
    pub id: String,
    pub device_serial_id: String,
    pub usb_pid: u16,
    pub firmware_build: Option<u32>,
    pub boot_version: Option<String>,
    pub control: String,
    pub state: String,
    pub mechanism: String,
    pub created_utc: String,
    pub updated_utc: String,
    pub targets: Vec<Target>,
    pub helper: Option<String>,
    pub helper_note: Option<String>,
    pub helper_source: Option<String>,
    pub helper_uploaded: bool,
    pub completion: Option<String>,
    pub artifacts: Vec<Artifact>,
    pub cleanup_state: String,
    pub runs_dir: PathBuf,
    pub error: Option<String>,
    /// `"test"` (disposable ZZT_ objects, receipt-authorizable) or `"user"`
    /// (ordinary objects, typed confirmation only). Records written before
    /// this field existed are `test` records.
    #[serde(default)]
    pub class: String,
    /// How deletion was authorized (`creation_receipt` / `exact_typed_confirmation`).
    #[serde(default)]
    pub authorization: Option<String>,
    /// When the deletion was requested (UTC).
    #[serde(default)]
    pub requested_utc: Option<String>,
    /// Empty for legacy records: absence of transport evidence is unknown.
    #[serde(default)]
    pub submission: Vec<ppl_device::delete::SubmissionState>,
}
impl Operation {
    pub fn policy(&self) -> NamePolicy {
        if self.class == "user" {
            NamePolicy::User
        } else {
            NamePolicy::Test
        }
    }
    pub fn confirmed(&self) -> bool {
        !self.targets.is_empty() && self.targets.iter().all(|t| t.status == "confirmed_deleted")
    }
    pub fn targets(&self) -> Result<Vec<DeleteTarget>> {
        let policy = self.policy();
        self.targets.iter().map(|t| t.target(policy)).collect()
    }
    pub fn control(&self) -> Result<ControlObject> {
        ControlObject::parse(&self.control).map_err(disk)
    }
}
pub struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn new(root: &Path) -> Self {
        Self { root: root.into() }
    }
    /// Held across journal read/modify/write, independently of the HID lock.
    pub fn lock(&self) -> Result<std::fs::File> {
        durable_directory(&self.root).map_err(disk)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("operations.lock"))
            .map_err(disk)?;
        fs2::FileExt::try_lock_exclusive(&file)
            .map_err(|e| DeviceError::Busy(format!("operation journal is locked: {e}")))?;
        Ok(file)
    }
    pub fn directory(&self, id: &str) -> Result<PathBuf> {
        ppl_device::validate_test_name(id)?;
        Ok(self.root.join("operations").join(id))
    }
    pub fn create(&self, op: &Operation, backups: &[Backup]) -> Result<()> {
        let dir = self.directory(&op.id)?;
        durable_directory(dir.parent().unwrap()).map_err(disk)?;
        std::fs::create_dir(&dir).map_err(disk)?;
        sync_dir(dir.parent().unwrap()).map_err(disk)?;
        for b in backups {
            immutable(
                &dir.join(format!("{}-{}.bin", b.ty.label(), b.name)),
                &b.data,
            )
            .map_err(disk)?;
        }
        self.save(op)
    }
    pub fn save(&self, op: &Operation) -> Result<()> {
        let dir = self.directory(&op.id)?;
        let path = dir.join("operation.json");
        if let Ok(bytes) = std::fs::read(&path) {
            let old: serde_json::Value = serde_json::from_slice(&bytes).map_err(disk)?;
            if old.get("submission").is_none() && !dir.join("record-v1-original.json").exists() {
                immutable(&dir.join("record-v1-original.json"), &bytes).map_err(disk)?;
            }
        }
        atomic_json(&path, op).map_err(disk)?;
        // Keep the existing audit stream, but never ignore a failed append.
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("journal.jsonl"))
            .map_err(disk)?;
        let line = serde_json::to_vec(&serde_json::json!({"id":op.id,"phase":op.state,"mechanism":op.mechanism,"at_utc":op.updated_utc,"device_serial_id":op.device_serial_id,"targets":op.targets,"submission":op.submission,"cleanup_state":op.cleanup_state,"error":op.error})).map_err(disk)?;
        f.write_all(&line)
            .and_then(|_| f.write_all(b"\n"))
            .and_then(|_| f.sync_all())
            .map_err(disk)?;
        sync_dir(&self.root).map_err(disk)
    }
    pub fn load(&self, id: &str) -> Result<Operation> {
        let op: Operation = serde_json::from_slice(
            &std::fs::read(self.directory(id)?.join("operation.json")).map_err(disk)?,
        )
        .map_err(disk)?;
        if op.schema_version != 1 || op.id != id || op.targets.is_empty() {
            return Err(DeviceError::Refused(
                "invalid deletion operation record".into(),
            ));
        }
        if !matches!(op.class.as_str(), "" | "test" | "user") {
            return Err(DeviceError::Refused("unknown deletion record class".into()));
        }
        if op.class == "user"
            && (op.targets.len() != 1
                || op.helper.is_some()
                || op.helper_source.is_some()
                || op.mechanism != "usb_delete")
        {
            return Err(DeviceError::Refused(
                "ordinary-object records are single-target USB deletions".into(),
            ));
        }
        let targets = op.targets()?;
        let control = op.control()?;
        for t in targets {
            if t.name.eq_ignore_ascii_case("ZZT_CTRL") || t.name.eq_ignore_ascii_case(&control.name)
            {
                return Err(DeviceError::Refused(
                    "protected target in operation record".into(),
                ));
            }
        }
        if op.targets.len() > ppl_device::MAX_TARGETS {
            return Err(DeviceError::Refused(
                "too many recorded deletion targets".into(),
            ));
        }
        if let Some(helper) = &op.helper {
            if helper != &op.id
                || op.helper_note.as_deref() != Some(ppl_device::result_note_name(helper).as_str())
            {
                return Err(DeviceError::Refused(
                    "helper identity differs from the operation".into(),
                ));
            }
        }
        for artifact in &op.artifacts {
            let helper = op.helper.as_deref();
            let authorized = (artifact.ty == "program"
                && Some(artifact.name.as_str()) == helper
                && op.helper_source.is_some())
                || (artifact.ty == "note"
                    && op.helper_note.as_deref() == Some(artifact.name.as_str()));
            if !authorized
                || artifact.sha256.len() != 64
                || !artifact.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(DeviceError::Refused(
                    "unrelated or invalid artifact in deletion record".into(),
                ));
            }
        }
        Ok(op)
    }
    pub fn backups(&self, op: &Operation) -> Result<Vec<Backup>> {
        op.targets
            .iter()
            .map(|t| {
                let target = t.target(op.policy())?;
                let data = std::fs::read(
                    self.directory(&op.id)?
                        .join(format!("{}-{}.bin", t.ty, t.name)),
                )
                .map_err(disk)?;
                if data.len() != t.bytes || ppl_compat::sha256_hex(&data) != t.sha256 {
                    return Err(DeviceError::Refused(
                        "deletion backup changed or is corrupt".into(),
                    ));
                }
                Ok(Backup {
                    ty: target.ty,
                    name: target.name,
                    data,
                    crc_ok: true,
                })
            })
            .collect()
    }
    pub fn reject_pending(&self, serial: &str, targets: &[DeleteTarget]) -> Result<()> {
        self.reject_other_pending(serial, targets, None)
    }
    pub fn reject_other_pending(
        &self,
        serial: &str,
        targets: &[DeleteTarget],
        except: Option<&str>,
    ) -> Result<()> {
        let upload_entries = match std::fs::read_dir(self.root.join("uploads")) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(disk(e)),
        };
        if let Some(entries) = upload_entries {
            for entry in entries {
                let entry = entry.map_err(disk)?;
                if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let record: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(entry.path()).map_err(disk)?)
                        .map_err(disk)?;
                if record["device"]["serial_id"] == serial
                    && record["name"].as_str().is_some_and(|name| {
                        targets
                            .iter()
                            .any(|t| t.ty == FileType::Program && t.name.eq_ignore_ascii_case(name))
                    })
                    && !matches!(
                        record["state"].as_str(),
                        Some("upload_verified" | "not_sent" | "verified_absent")
                    )
                {
                    return Err(DeviceError::Refused("unresolved upload blocks this device/program; perform read-only upload recovery".into()));
                }
            }
        }
        let entries = match std::fs::read_dir(self.root.join("operations")) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(disk(e)),
        };
        for entry in entries {
            let entry = entry.map_err(disk)?;
            if !entry.file_type().map_err(disk)?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let op = self.load(&id)?;
            if except == Some(id.as_str()) {
                continue;
            }
            if op.device_serial_id != serial
                || matches!(op.state.as_str(), "completed" | "not_submitted")
                || (op.class == "user" && op.confirmed())
            {
                continue;
            }
            if op.targets.iter().any(|old| {
                targets
                    .iter()
                    .any(|t| t.name.eq_ignore_ascii_case(&old.name) && t.ty.label() == old.ty)
            }) {
                return Err(DeviceError::Refused(format!(
                    "unresolved operation {}; use delete verify --operation-id {}",
                    op.id, op.id
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_requires_device_type_exact_name_content_and_active_receipt() {
        let root = std::env::temp_dir().join(ppl_device::fresh_test_name());
        let b = Backup {
            ty: FileType::Program,
            name: "ZZT_TEST".into(),
            data: b"source".to_vec(),
            crc_ok: true,
        };
        record_creation(&root, "0123456789abcdef", b.ty, &b.name, &b.data).unwrap();
        assert!(owns(&root, "0123456789abcdef", &b).unwrap());
        assert!(!owns(&root, "1123456789abcdef", &b).unwrap());
        assert!(
            !owns(
                &root,
                "0123456789abcdef",
                &Backup {
                    ty: FileType::Note,
                    ..b.clone()
                }
            )
            .unwrap()
        );
        assert!(
            !owns(
                &root,
                "0123456789abcdef",
                &Backup {
                    name: "ZZT_Test".into(),
                    ..b.clone()
                }
            )
            .unwrap()
        );
        assert!(
            !owns(
                &root,
                "0123456789abcdef",
                &Backup {
                    data: b"changed".to_vec(),
                    ..b.clone()
                }
            )
            .unwrap()
        );
        retire(&root, "0123456789abcdef", &b).unwrap();
        assert!(!owns(&root, "0123456789abcdef", &b).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
