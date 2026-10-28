//! Managed local firmware helper. All startup, IO and shutdown run off the UI thread.
use ppl_engine::session::{self, Command as Input, Event, Packet};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
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
    tx: SyncSender<Input>,
    rx: Receiver<Event>,
    pub latest: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
    pub alive: Arc<AtomicBool>,
}
impl FirmwareClient {
    pub fn start(settings: FirmwareSettings, ctx: egui::Context) -> Self {
        let (tx, commands) = mpsc::sync_channel(128);
        let (events, rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let frames = latest.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let living = alive.clone();
        std::thread::spawn(move || {
            let notify = |event| {
                let _ = events.send(event);
                ctx.request_repaint();
            };
            if let Err(message) = supervise(
                settings,
                commands,
                events.clone(),
                frames,
                stopping,
                ctx.clone(),
            ) {
                notify(Event::Error { message });
            }
            living.store(false, Ordering::Release);
            notify(Event::Closed {
                message: "Local firmware session closed".into(),
            });
        });
        Self {
            tx,
            rx,
            latest,
            stop,
            alive,
        }
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
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.tx.try_send(Input::Shutdown);
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
fn supervise(
    settings: FirmwareSettings,
    commands: Receiver<Input>,
    events: mpsc::Sender<Event>,
    frames: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
    ctx: egui::Context,
) -> Result<(), String> {
    let runner = ppl_engine::firmware::discover_runner(
        configured(&settings.runner, "PPL_FIRMWARE_RUNNER").as_deref(),
    )?;
    let firmware=configured(&settings.firmware,"PPL_FIRMWARE").ok_or("Choose the G1 armfir.elf and factory A directory in Settings, or configure PPL_FIRMWARE and PPL_FACTORY_A")?;
    let factory = configured(&settings.factory_a, "PPL_FACTORY_A")
        .ok_or("Choose the factory A directory in Settings, or set PPL_FACTORY_A")?;
    if !firmware.is_file() || !factory.join("programs/misc/armfir.dat").is_file() {
        return Err("The firmware ELF or factory resources are missing".into());
    }
    let explicit_profile = !settings.profile.trim().is_empty()
        || std::env::var_os("PPL_FIRMWARE_PROFILE").is_some()
        || std::env::var_os("PPL_PROFILE").is_some();
    let base_profile = profile_path(&settings)?;

    let (profile, lock) = if explicit_profile {
        std::fs::create_dir_all(&base_profile).map_err(|e| e.to_string())?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(base_profile.join("session.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
            format!(
                "Calculator profile at '{}' is locked by another running session",
                base_profile.display()
            )
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
        chosen.ok_or_else(|| "All calculator profiles (primary and secondary 2-8) are currently locked by other sessions".to_string())?
    };

    seed_profile(
        &profile,
        &factory,
        configured("", "PPL_FIRMWARE_INIT_DATA").as_deref(),
    )?;
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
    let mut child = command.spawn().map_err(|e| e.to_string())?;
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
                    let _ = events.send(Event::Error {
                        message: "Incompatible local firmware helper; rebuild the workspace".into(),
                    });
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
            break;
        }
    }
    let _ = session::write_command(&mut stdin, &Input::Shutdown);
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(2);
    while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    drop(lock);
    if let Some(e) = failure {
        Err(e)
    } else {
        Ok(())
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
