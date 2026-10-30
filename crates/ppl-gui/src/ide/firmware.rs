//! Managed local firmware helper. All startup, IO and shutdown run off the UI thread.
use ppl_engine::session::{self, Command as Input, Event, Packet};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
/// How long ending a session waits: the firmware's OFF (save, then sleep;
/// the helper gives up after 30 s) and the helper's exit.
pub const SHUTDOWN_LIMIT: Duration = Duration::from_secs(40);
/// Identifies each session started by this process, so the IDE can tell a
/// replacement session's frames from the previous one's.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
/// Why a session ended before or during startup.
enum SessionError {
    /// Assets, runner, profile lock or helper version: cannot start here.
    Unavailable(String),
    /// The session started and then failed.
    Failed(String),
}
impl From<String> for SessionError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FirmwareSettings {
    pub firmware: String,
    pub factory_a: String,
    pub runner: String,
    pub profile: String,
}
#[derive(Debug, Clone)]
pub struct Frame {
    pub generation: u64,
    pub micros: u64,
    pub pixels: Arc<[u32]>,
}
pub struct FirmwareClient {
    pub session_id: u64,
    tx: SyncSender<Input>,
    rx: Receiver<Event>,
    pub latest: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
    pub alive: Arc<AtomicBool>,
    /// Why the session could not start, once known.
    pub unavailable: Arc<Mutex<Option<String>>>,
    /// The helper's process id, which is also its process group; 0 until
    /// it is spawned and after it was reaped.
    child: Arc<AtomicU32>,
    worker: Option<JoinHandle<()>>,
    done: Option<Receiver<()>>,
}
impl FirmwareClient {
    #[cfg(test)]
    pub(crate) fn with_test_frame(frame: Frame) -> Self {
        Self::for_test(Some(frame)).0
    }
    /// A client with no helper: the test plays the session, sending its
    /// events and reading the commands the IDE sends.
    #[cfg(test)]
    pub(crate) fn for_test(frame: Option<Frame>) -> (Self, mpsc::Sender<Event>, Receiver<Input>) {
        let (tx, commands) = mpsc::sync_channel(128);
        let (events, rx) = mpsc::channel();
        let client = Self {
            session_id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            tx,
            rx,
            latest: Arc::new(Mutex::new(frame)),
            stop: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(true)),
            unavailable: Arc::new(Mutex::new(None)),
            child: Arc::new(AtomicU32::new(0)),
            worker: None,
            done: None,
        };
        (client, events, commands)
    }

    /// Starts a session. A `previous` session is shut down first on the
    /// worker thread (it holds the calculator profile's lock), so the UI
    /// never waits for it.
    pub fn start(
        settings: FirmwareSettings,
        ctx: egui::Context,
        previous: Option<FirmwareClient>,
    ) -> Self {
        let (tx, commands) = mpsc::sync_channel(128);
        let (events, rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let frames = latest.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let living = alive.clone();
        let unavailable = Arc::new(Mutex::new(None));
        let reason = unavailable.clone();
        let child = Arc::new(AtomicU32::new(0));
        let child_pid = child.clone();
        let (finished, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let notify = |event| {
                let _ = events.send(event);
                ctx.request_repaint();
            };
            if let Some(previous) = previous {
                previous.shutdown(SHUTDOWN_LIMIT);
            }
            let result = supervise(
                settings,
                commands,
                events.clone(),
                frames,
                stopping,
                ctx.clone(),
                child_pid,
                reason.clone(),
            );
            match result {
                Ok(()) => {}
                Err(SessionError::Unavailable(message)) => {
                    *reason.lock().unwrap() = Some(message.clone());
                    notify(Event::Error { message });
                }
                Err(SessionError::Failed(message)) => notify(Event::Error { message }),
            }
            living.store(false, Ordering::Release);
            notify(Event::Closed {
                message: "Local firmware session closed".into(),
            });
            let _ = finished.send(());
        });
        Self {
            session_id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            tx,
            rx,
            latest,
            stop,
            alive,
            unavailable,
            child,
            worker: Some(worker),
            done: Some(done),
        }
    }
    /// Ends the session and waits up to `limit` for its helper to exit; a
    /// helper that does not is killed with its process group. Blocking:
    /// call from a worker thread, or once when the IDE closes.
    pub fn shutdown(mut self, limit: Duration) {
        self.stop.store(true, Ordering::Release);
        let Some(done) = self.done.take() else {
            return;
        };
        if done.recv_timeout(limit).is_err() {
            ppl_engine::firmware::kill_process_group(self.child.load(Ordering::Acquire));
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            let _ = worker.join();
        }
    }
    /// Kills the helper's process group at once: a battery pull, for
    /// crash-recovery tests (automation only).
    pub fn kill_helper(&self) {
        ppl_engine::firmware::kill_process_group(self.child.load(Ordering::Acquire));
    }
    /// Asks the session to end without waiting for it.
    pub fn request_shutdown(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn send(&self, command: Input) {
        if self.tx.try_send(command).is_err() {
            self.stop.store(true, Ordering::Release);
        }
    }
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}
impl Drop for FirmwareClient {
    /// The worker powers the firmware down (its OFF saves) and ends it.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
fn configured(value: &str, variable: &str) -> Option<PathBuf> {
    if !value.trim().is_empty() {
        return Some(PathBuf::from(value));
    }
    if let Some(val) = std::env::var_os(variable) {
        return Some(PathBuf::from(val));
    }
    // The CLI's config file, so `ppl show` and `ppl run` share assets.
    let config = ppl_engine::firmware_options::AssetPaths::config_file()
        .and_then(|f| ppl_engine::firmware_options::AssetPaths::from_config(&f).ok());
    if let Some(mut c) = config {
        let value = match variable {
            "PPL_FIRMWARE" => c.firmware.take(),
            "PPL_FACTORY_A" => c.factory_a.take(),
            "PPL_FIRMWARE_RUNNER" => c.runner.take(),
            "PPL_OS_ROM" => c.os_rom.take(),
            "PPL_FIRMWARE_INIT_DATA" => c.init_data.take(),
            _ => None,
        };
        if value.is_some() {
            return value;
        }
    }
    let fallback = match variable {
        "PPL_FIRMWARE" => "PRIME_TEST_FIRMWARE",
        "PPL_FACTORY_A" => "PRIME_TEST_FACTORY_A",
        "PPL_FIRMWARE_RUNNER" => "PRIME_TEST_RUNNER",
        "PPL_OS_ROM" => "PRIME_TEST_OS_ROM",
        _ => return None,
    };
    std::env::var_os(fallback).map(PathBuf::from)
}
fn profile_path(settings: &FirmwareSettings) -> Result<PathBuf, String> {
    if !settings.profile.trim().is_empty() {
        return Ok(PathBuf::from(&settings.profile));
    }
    if let Some(p) =
        std::env::var_os("PPL_FIRMWARE_PROFILE").or_else(|| std::env::var_os("PPL_PROFILE"))
    {
        return Ok(PathBuf::from(p));
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("No data directory; configure a firmware profile")?;
    let legacy = data.join("hp-prime-emulator/calculator-g1");
    if legacy.join("A").exists() {
        return Ok(legacy);
    }
    Ok(data.join("hp-prime-emulator/ide-session"))
}
/// Clears the calculator's memory to its factory state, as a reset does:
/// the profile (all of its storage) is moved aside to a timestamped backup
/// folder next to it, which is never deleted, and the next start seeds a
/// new one. The firmware's own memory-clear key combination is read by
/// the G1's boot code, which is not emulated, so this is done on the host.
/// Refused while a session holds the profile.
pub fn reset_profile(settings: &FirmwareSettings) -> Result<PathBuf, String> {
    let profile = profile_path(settings)?;
    if !profile.join("A").exists() {
        return Err("The calculator has no stored memory to reset".into());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join("session.lock"))
        .map_err(|e| e.to_string())?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| "The calculator is in use; shut it down first".to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = profile.file_name().unwrap_or_default().to_string_lossy();
    let backup = profile.with_file_name(format!("{name}.backup-{stamp}"));
    if backup.exists() {
        return Err(format!("{} already exists", backup.display()));
    }
    drop(lock);
    std::fs::rename(&profile, &backup).map_err(|e| e.to_string())?;
    Ok(backup)
}
/// Files an interrupted write left in the calculator's storage (a pending
/// install or an importer temporary). They are reported, never deleted.
pub fn leftover_files(profile: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if name.ends_with(".pending") {
                    out.push(path);
                } else {
                    walk(&path, out);
                }
            } else if name.ends_with(".pending") || name.starts_with(".prime-import-") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for drive in ["A", "C"] {
        walk(&profile.join(drive), &mut out);
    }
    out
}
/// Seeds a new calculator profile; an existing profile is never changed.
/// Initialized data (drives A: and C: after the firmware's first-run setup)
/// starts at Home; factory A alone stops at the first-run wizard. `A/` is
/// installed last, so an interrupted seed is completed on the next start.
fn seed_profile(profile: &Path, factory: &Path, initialized: Option<&Path>) -> Result<(), String> {
    if profile.join("A").exists() {
        return Ok(());
    }
    let initialized = initialized.filter(|d| d.join("A").is_dir() && d.join("C").is_dir());
    let sources: Vec<(PathBuf, &str)> = match initialized {
        Some(data) => vec![(data.join("C"), "C"), (data.join("A"), "A")],
        None => vec![(factory.to_path_buf(), "A")],
    };
    for (source, drive) in sources {
        if profile.join(drive).exists() {
            continue;
        }
        let temporary = profile.join(format!("{drive}.pending"));
        let _ = std::fs::remove_dir_all(&temporary);
        copy_tree(&source, &temporary)?;
        std::fs::rename(temporary, profile.join(drive)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = target.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn supervise(
    settings: FirmwareSettings,
    commands: Receiver<Input>,
    events: mpsc::Sender<Event>,
    frames: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
    ctx: egui::Context,
    child_pid: Arc<AtomicU32>,
    unavailable: Arc<Mutex<Option<String>>>,
) -> Result<(), SessionError> {
    let unavailable_err = SessionError::Unavailable;
    let runner = ppl_engine::firmware::discover_runner(
        configured(&settings.runner, "PPL_FIRMWARE_RUNNER").as_deref(),
    )
    .map_err(unavailable_err)?;
    let firmware = configured(&settings.firmware, "PPL_FIRMWARE").ok_or_else(|| {
        unavailable_err("Choose the G1 armfir.elf and factory A directory in Settings, or configure PPL_FIRMWARE and PPL_FACTORY_A".into())
    })?;
    let factory = configured(&settings.factory_a, "PPL_FACTORY_A").ok_or_else(|| {
        unavailable_err("Choose the factory A directory in Settings, or set PPL_FACTORY_A".into())
    })?;
    if !firmware.is_file() || !factory.join("programs/misc/armfir.dat").is_file() {
        return Err(unavailable_err(
            "The firmware ELF or factory resources are missing".into(),
        ));
    }
    let explicit_profile = !settings.profile.trim().is_empty()
        || std::env::var_os("PPL_FIRMWARE_PROFILE").is_some()
        || std::env::var_os("PPL_PROFILE").is_some();
    let base_profile = profile_path(&settings).map_err(unavailable_err)?;

    let (profile, lock) = if explicit_profile {
        std::fs::create_dir_all(&base_profile).map_err(|e| unavailable_err(e.to_string()))?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(base_profile.join("session.lock"))
            .map_err(|e| unavailable_err(e.to_string()))?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
            unavailable_err(format!(
                "Calculator profile at '{}' is locked by another running session",
                base_profile.display()
            ))
        })?;
        (base_profile, file)
    } else {
        let mut chosen = None;
        let candidates = std::iter::once(base_profile.clone()).chain((2..=8).map(|i| {
            let file_name = format!(
                "{}-{}",
                base_profile.file_name().unwrap().to_string_lossy(),
                i
            );
            base_profile.with_file_name(file_name)
        }));
        for cand in candidates {
            if std::fs::create_dir_all(&cand).is_ok() {
                if let Ok(file) = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(cand.join("session.lock"))
                {
                    if fs2::FileExt::try_lock_exclusive(&file).is_ok() {
                        chosen = Some((cand, file));
                        break;
                    }
                }
            }
        }
        chosen.ok_or_else(|| unavailable_err("All calculator profiles (primary and secondary 2-8) are currently locked by other sessions".to_string()))?
    };

    seed_profile(
        &profile,
        &factory,
        configured("", "PPL_FIRMWARE_INIT_DATA").as_deref(),
    )
    .map_err(unavailable_err)?;
    let leftovers = leftover_files(&profile);
    if !leftovers.is_empty() {
        let _ = events.send(Event::Status {
            message: format!(
                "Calculator storage: {} unfinished file(s) from an interrupted write, kept as they are: {}",
                leftovers.len(),
                leftovers
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut command = Command::new(runner);
    command
        .arg("session")
        .arg("--firmware")
        .arg(firmware)
        .arg("--data-dir")
        .arg(&profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            std::fs::File::create(profile.join("session.log")).map_err(|e| e.to_string())?,
        ));
    if let Some(rom) = configured("", "PPL_OS_ROM") {
        command.arg("--os-rom").arg(rom);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| unavailable_err(e.to_string()))?;
    child_pid.store(child.id(), Ordering::Release);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let reader_stop = stop.clone();
    let reader_ctx = ctx.clone();
    let reader = std::thread::spawn(move || {
        loop {
            match session::read_packet(&mut stdout) {
                Ok(Packet::Frame {
                    generation,
                    micros,
                    pixels,
                }) => {
                    let mut lock = frames.lock().unwrap();
                    if lock.as_ref().is_none_or(|f| generation > f.generation) {
                        *lock = Some(Frame {
                            generation,
                            micros,
                            pixels: pixels.into(),
                        });
                    }
                }
                Ok(Packet::Event(Event::Hello { version })) if version != session::VERSION => {
                    let message = format!(
                        "Incompatible local firmware helper (protocol {version}, IDE {}); \
                         rebuild or reinstall prime-emulator",
                        session::VERSION
                    );
                    *unavailable.lock().unwrap() = Some(message.clone());
                    let _ = events.send(Event::Error { message });
                    reader_stop.store(true, Ordering::Release);
                }
                Ok(Packet::Event(event)) => {
                    let _ = events.send(event);
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::UnexpectedEof {
                        let _ = events.send(Event::Error {
                            message: e.to_string(),
                        });
                    }
                    break;
                }
            }
            reader_ctx.request_repaint();
        }
        reader_stop.store(true, Ordering::Release);
        reader_ctx.request_repaint();
    });
    let mut failure = None;
    while !stop.load(Ordering::Acquire) {
        match commands.recv_timeout(Duration::from_millis(25)) {
            Ok(input) => {
                if let Err(e) = session::write_command(&mut stdin, &input) {
                    failure = Some(e.to_string());
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            if failure.is_none() && !stop.load(Ordering::Acquire) {
                failure = Some("The local firmware helper exited unexpectedly".into());
            }
            break;
        }
    }
    // The firmware's own OFF saves its state, then the helper exits. Only
    // a helper that does not (stuck, or already failing) is ended hard.
    let exited = |child: &mut std::process::Child, limit: Duration| {
        let deadline = Instant::now() + limit;
        while child.try_wait().ok().flatten().is_none() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    };
    let powered_down = failure.is_none()
        && session::write_command(&mut stdin, &Input::PowerDown).is_ok()
        && exited(&mut child, SHUTDOWN_LIMIT - Duration::from_secs(5));
    if !powered_down {
        let _ = session::write_command(&mut stdin, &Input::Shutdown);
        exited(&mut child, Duration::from_secs(2));
    }
    drop(stdin);
    // The helper's whole process group: it may have started children.
    ppl_engine::firmware::kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    child_pid.store(0, Ordering::Release);
    let _ = reader.join();
    drop(lock);
    match failure {
        Some(e) => Err(SessionError::Failed(e)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_configured_fallbacks() {
        assert_eq!(
            configured("/custom/path", "PPL_FIRMWARE"),
            Some(PathBuf::from("/custom/path"))
        );
    }

    #[test]
    fn new_profiles_start_from_initialized_data_and_existing_ones_are_kept() {
        let root = std::env::temp_dir().join(format!("ide-test-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |p: PathBuf, text: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        let factory = root.join("factory");
        write(factory.join("FIRSTRUN.INI"), "factory");
        let data = root.join("data");
        write(data.join("A/FIRSTRUN.INI"), "initialized");
        write(data.join("C/DATA/settings"), "home");
        let fresh = root.join("fresh");
        seed_profile(&fresh, &factory, Some(&data)).unwrap();
        assert_eq!(
            std::fs::read_to_string(fresh.join("A/FIRSTRUN.INI")).unwrap(),
            "initialized"
        );
        assert!(fresh.join("C/DATA/settings").is_file());
        // Without initialized data: factory A, as before.
        let plain = root.join("plain");
        seed_profile(&plain, &factory, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(plain.join("A/FIRSTRUN.INI")).unwrap(),
            "factory"
        );
        assert!(!plain.join("C").exists());
        // An existing profile (the user's calculator) is never touched.
        write(fresh.join("C/DATA/MYAPP.hpprgm"), "user program");
        seed_profile(&fresh, &factory, Some(&data)).unwrap();
        assert!(fresh.join("C/DATA/MYAPP.hpprgm").is_file());
        assert!(!fresh.join("A.pending").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_memory_reset_keeps_the_old_memory_and_respects_a_running_session() {
        let root = std::env::temp_dir().join(format!("ide-test-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("calc");
        std::fs::create_dir_all(profile.join("C/DATA")).unwrap();
        std::fs::create_dir_all(profile.join("A")).unwrap();
        std::fs::write(profile.join("C/DATA/MINE.hpprgm"), "user program").unwrap();
        let settings = FirmwareSettings {
            profile: profile.display().to_string(),
            ..Default::default()
        };
        // A running session holds the lock: nothing moves.
        let held = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(profile.join("session.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&held).unwrap();
        assert!(reset_profile(&settings).is_err());
        assert!(profile.join("C/DATA/MINE.hpprgm").is_file());
        drop(held);
        let backup = reset_profile(&settings).unwrap();
        assert!(!profile.exists(), "the next start seeds a fresh profile");
        assert_eq!(
            std::fs::read_to_string(backup.join("C/DATA/MINE.hpprgm")).unwrap(),
            "user program"
        );
        assert!(reset_profile(&settings).is_err(), "nothing left to reset");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn interrupted_writes_are_found_and_ordinary_files_are_not() {
        let root = std::env::temp_dir().join(format!("ide-test-leftover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("C/DATA");
        std::fs::create_dir_all(data.join("&Function.hpappdir")).unwrap();
        std::fs::create_dir_all(root.join("A")).unwrap();
        for f in [
            "A.hpprgm",
            ".prime-import-1",
            "&Function.hpappdir/ZZT_IDE_SOURCE.pending",
        ] {
            std::fs::write(data.join(f), "x").unwrap();
        }
        let mut found = leftover_files(&root);
        found.sort();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|p| !p.ends_with("A.hpprgm")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_profile_path_custom() {
        let settings = FirmwareSettings {
            profile: "/custom/profile".into(),
            ..Default::default()
        };
        assert_eq!(
            profile_path(&settings).unwrap(),
            PathBuf::from("/custom/profile")
        );
    }

    #[test]
    fn test_session_lock_mutual_exclusion_and_secondary_candidate() {
        let base_profile =
            std::env::temp_dir().join(format!("ide-test-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_profile);
        std::fs::create_dir_all(&base_profile).unwrap();

        // Lock primary session
        let lock1 = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(base_profile.join("session.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&lock1).is_ok());

        // Second lock on primary fails
        let lock2 = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(base_profile.join("session.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&lock2).is_err());

        // Probe secondary candidate
        let cand2 = std::env::temp_dir().join(format!("ide-test-session-{}-2", std::process::id()));
        let _ = std::fs::remove_dir_all(&cand2);
        std::fs::create_dir_all(&cand2).unwrap();
        let lock_cand2 = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(cand2.join("session.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&lock_cand2).is_ok());

        // Clean up locks
        drop(lock1);
        assert!(fs2::FileExt::try_lock_exclusive(&lock2).is_ok());
        drop(lock2);
        drop(lock_cand2);
        let _ = std::fs::remove_dir_all(&base_profile);
        let _ = std::fs::remove_dir_all(&cand2);
    }
}
