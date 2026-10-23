//! Durable mailbox requests and capability evidence, separate from HID I/O.
use ppl_device::remote::{Reply, Request};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub model: String,
    pub usb_pid: u16,
    pub firmware_build: Option<u32>,
    pub boot_version: Option<String>,
    pub serial_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub mailbox: bool,
    pub expressions: bool,
    pub program_calls: bool,
    pub live_upload: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Listener {
    pub schema_version: u32,
    pub device: Identity,
    pub program: String,
    pub session: String,
    pub source: String,
    pub source_sha256: String,
    pub uploaded: bool,
    #[serde(default)]
    pub heartbeat: Option<String>,
    pub capabilities: Capabilities,
    pub verification_requests: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journal {
    pub schema_version: u32,
    pub device: Identity,
    pub operation: String,
    pub request: Request,
    /// prepared: no write attempted; submitting: F7 may have reached device;
    /// completed: validated terminal response retrieved.
    pub state: String,
    pub upload_verified: bool,
    pub reply: Option<Reply>,
}

pub struct Store {
    pub dir: PathBuf,
}
impl Store {
    pub fn new(root: &Path, identity: &Identity) -> Self {
        Self {
            dir: root.join(&identity.serial_id),
        }
    }
    pub fn listener_path(&self) -> PathBuf {
        self.dir.join("listener.json")
    }
    pub fn request_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("request-{id}.json"))
    }
    pub fn read<T: serde::de::DeserializeOwned>(&self, path: &Path) -> std::io::Result<T> {
        serde_json::from_slice(&std::fs::read(path)?).map_err(std::io::Error::other)
    }
    pub fn save<T: Serialize>(&self, path: &Path, value: &T) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        // The device's exclusive OS lock serializes every store mutation.
        let temp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
        use std::io::Write;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        std::fs::rename(temp, path)?;
        #[cfg(unix)]
        std::fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }
    pub fn pending(&self) -> std::io::Result<Option<Journal>> {
        if !self.dir.exists() {
            return Ok(None);
        }
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with("request-") && name.ends_with(".json") {
                let request: Journal = self.read(&path)?;
                if request.state == "submitting" {
                    return Ok(Some(request));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journals_preserve_uncertainty_across_process_restarts() {
        let root = std::env::temp_dir().join(format!(
            "ppl-remote-store-{}-{}",
            std::process::id(),
            ppl_device::fresh_test_name()
        ));
        let identity = Identity {
            model: "mock".into(),
            usb_pid: 0x1541,
            firmware_build: Some(15157),
            boot_version: None,
            serial_id: "test".into(),
        };
        let store = Store::new(&root, &identity);
        let mut journal = Journal {
            schema_version: 1,
            device: identity.clone(),
            operation: "eval".into(),
            request: Request::new("ABC123", "2+2", false).unwrap(),
            state: "prepared".into(),
            upload_verified: false,
            reply: None,
        };
        let path = store.request_path(&journal.request.id);
        store.save(&path, &journal).unwrap();
        assert!(store.pending().unwrap().is_none());
        journal.state = "submitting".into();
        store.save(&path, &journal).unwrap();
        let reopened = Store::new(&root, &identity);
        assert_eq!(
            reopened.pending().unwrap().unwrap().request.id,
            journal.request.id
        );
        // A read-back verification alone is not a terminal execution record.
        journal.upload_verified = true;
        store.save(&path, &journal).unwrap();
        assert!(store.pending().unwrap().is_some());
        journal.state = "completed".into();
        store.save(&path, &journal).unwrap();
        assert!(store.pending().unwrap().is_none());
        std::fs::write(&path, b"corrupt").unwrap();
        assert!(store.pending().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
