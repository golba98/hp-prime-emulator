//! Single-writer Native profiles. Disk work never runs in an egui frame.
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

const MAX_PROFILE: u64 = 64 * 1024 * 1024;
#[derive(Clone)]
pub struct NativeProfile {
    inner: Arc<Inner>,
}
struct Inner {
    pending: Mutex<Value>,
    tx: mpsc::SyncSender<Message>,
    error: Mutex<Option<String>>,
}
enum Message {
    Changed,
    Flush(mpsc::Sender<Result<(), String>>),
}
impl NativeProfile {
    pub fn open(path: &Path) -> Result<(Self, Value), String> {
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join("profile.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "Native profile is already open in another instance".to_string())?;
        let current = path.join("state-v1.json");
        let previous = path.join("state-v1.previous.json");
        let state = if current.exists() {
            match read(&current) {
                Ok(v) => v,
                Err(e) if e.starts_with("Unsupported") => return Err(e),
                Err(e) => {
                    let recovered = read(&previous)
                        .map_err(|b| format!("Native profile: {e}; previous generation: {b}"))?;
                    let archive = path.join(format!(
                        "state-v1.corrupt-{}-{}.json",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ));
                    let bytes = fs::read(&current).map_err(|e| e.to_string())?;
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(archive)
                        .map_err(|e| e.to_string())?;
                    file.write_all(&bytes)
                        .and_then(|_| file.sync_all())
                        .map_err(|e| e.to_string())?;
                    recovered
                }
            }
        } else {
            json!({"version":1,"memory":null,"ui":null})
        };
        let (tx, rx) = mpsc::sync_channel(2);
        let inner = Arc::new(Inner {
            pending: Mutex::new(state.clone()),
            tx,
            error: Mutex::new(None),
        });
        let weak = Arc::downgrade(&inner);
        let directory = path.to_path_buf();
        thread::spawn(move || {
            let _lock = lock;
            while let Ok(message) = rx.recv() {
                let Some(inner) = weak.upgrade() else { break };
                let image = inner
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let result = write(&directory, &image);
                *inner.error.lock().unwrap_or_else(|e| e.into_inner()) =
                    result.as_ref().err().cloned();
                if let Message::Flush(reply) = message {
                    let _ = reply.send(result);
                }
            }
        });
        Ok((Self { inner }, state))
    }
    pub fn update(&self, key: &str, value: Value) {
        self.inner.pending.lock().unwrap_or_else(|e| e.into_inner())[key] = value;
        let _ = self.inner.tx.try_send(Message::Changed);
    }
    pub fn error(&self) -> Option<String> {
        self.inner
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn flush(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut message = Message::Flush(tx);
        loop {
            match self.inner.tx.try_send(message) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err("Profile writer stopped".into());
                }
                Err(mpsc::TrySendError::Full(returned)) => {
                    if std::time::Instant::now() >= deadline {
                        return Err("Profile writer did not accept flush".into());
                    }
                    message = returned;
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
        rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|_| "Profile writer did not finish".to_string())?
    }
}
fn read(path: &Path) -> Result<Value, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_PROFILE {
        return Err("Profile exceeds size limit".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if value["version"].as_u64() != Some(1) {
        return Err("Unsupported Native profile version; original file preserved".into());
    }
    if !value.is_object() || value.get("memory").is_none() || value.get("ui").is_none() {
        return Err("Incomplete Native profile".into());
    }
    if !value["memory"].is_null() {
        let mut memory = ppl_vm::SessionContext::default();
        memory.restore_memory(&value["memory"])?;
    }
    if let Some(tools) = value["ui"].get("tools") {
        let memory: crate::ide::native_tools::ToolMemory =
            serde_json::from_value(tools.clone()).map_err(|e| e.to_string())?;
        memory.validate()?;
    }
    Ok(value)
}
fn write(directory: &Path, value: &Value) -> Result<(), String> {
    let current = directory.join("state-v1.json");
    let temp = directory.join(format!("state-v1.{}.tmp", std::process::id()));
    let backup = directory.join("state-v1.previous.json");
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PROFILE {
        return Err("Native memory exceeds profile size limit".into());
    }
    let io = || -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if current.exists() && read(&current).is_ok() {
            let previous_temp = directory.join("state-v1.previous.tmp");
            fs::copy(&current, &previous_temp)?;
            // Windows FlushFileBuffers requires a writable file handle.
            OpenOptions::new()
                .write(true)
                .open(&previous_temp)?
                .sync_all()?;
            fs::rename(previous_temp, &backup)?;
        }
        fs::rename(&temp, &current)?;
        ppl_programs::delete_store::sync_dir(directory)?;
        Ok(())
    };
    io().map_err(|e| e.to_string())
}
pub fn default_directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PPL_NATIVE_PROFILE") {
        return Some(path.into());
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .map(|p| p.join("hp-prime-emulator/native"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_reopens_and_preserves_previous_generation() {
        let dir = std::env::temp_dir().join(format!("ppl-native-profile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        {
            let (p, _) = NativeProfile::open(&dir).unwrap();
            p.update("ui", json!({"history":["1+1"]}));
            p.flush().unwrap();
            p.update("ui", json!({"history":["1+1","2+2"]}));
            p.flush().unwrap();
            assert!(NativeProfile::open(&dir).is_err());
        }
        // Writer exits after its channel disconnects and releases the profile lock.
        let mut opened = None;
        for _ in 0..100 {
            match NativeProfile::open(&dir) {
                Ok(p) => {
                    opened = Some(p);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
        let (p, state) = opened.expect("profile lock released");
        assert_eq!(state["ui"]["history"].as_array().unwrap().len(), 2);
        p.flush().unwrap();
        drop(p);
    }
    #[test]
    fn future_profile_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("ppl-native-future-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let content = b"{\"version\":999,\"memory\":null,\"ui\":null}";
        fs::write(dir.join("state-v1.json"), content).unwrap();
        assert!(NativeProfile::open(&dir).is_err());
        assert_eq!(fs::read(dir.join("state-v1.json")).unwrap(), content);
    }

    #[test]
    fn repeated_profile_rotation_recovers_the_last_valid_generation() {
        let dir = std::env::temp_dir().join(format!(
            "ppl-native-rotation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        for generation in 1..=3 {
            write(
                &dir,
                &json!({"version":1,"memory":null,"ui":{"generation":generation}}),
            )
            .unwrap();
        }
        assert_eq!(
            read(&dir.join("state-v1.json")).unwrap()["ui"]["generation"],
            3
        );
        assert_eq!(
            read(&dir.join("state-v1.previous.json")).unwrap()["ui"]["generation"],
            2
        );
        fs::write(dir.join("state-v1.json"), b"incomplete write").unwrap();
        let (profile, recovered) = NativeProfile::open(&dir).unwrap();
        assert_eq!(recovered["ui"]["generation"], 2);
        profile.flush().unwrap();
        assert_eq!(read(&dir.join("state-v1.json")).unwrap(), recovered);
        assert!(fs::read_dir(&dir).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("state-v1.corrupt-")
                && fs::read(path).unwrap() == b"incomplete write"
        }));
    }
}
