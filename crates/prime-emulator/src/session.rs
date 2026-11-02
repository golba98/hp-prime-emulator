//! Long-lived local firmware session, with bounded transport and coherent LCD capture.
mod job;
mod repeat;
pub use job::{LOADER, LOADER_V1};

use crate::{
    display::LcdView,
    memory::GuestMemory,
    script::{ScriptConfig, ScriptDriver, Tick},
    services::{SharedState, TouchSample},
    trace::TraceHub,
};
use ppl_engine::session::{self, Command, Event, Lifecycle};
use std::{
    collections::{HashSet, VecDeque},
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Default)]
struct Output {
    events: VecDeque<Event>,
    frame: Option<(u64, u64, Vec<u32>)>,
    closed: bool,
}
struct Device {
    held: HashSet<u16>,
    delivered: HashSet<u16>,
    repeat: repeat::KeyRepeat,
    script: Option<ScriptDriver>,
    jobs: job::Jobs,
    /// Last lifecycle state reported, and the idle meter that ends Booting.
    lifecycle: Lifecycle,
    boot: job::IdleMeter,
    /// Unsupported service call sites already reported.
    unsupported_seen: usize,
    /// Key transitions not yet given to the firmware, and when the last one
    /// was: see [`KEY_FALLBACK`].
    keys: VecDeque<(u16, bool)>,
    last_key: Instant,
    /// A `PowerDown` in progress: when it began, and whether OFF was sent.
    power_down: Option<(Instant, bool)>,
    recovery: Option<Arc<crate::recovery::Recovery>>,
    saved_sleep: u64,
    save_result: Option<Result<(), String>>,
    save_worker: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}
/// The firmware's OFF: Shift, then ON.
const SHIFT_KEY: u16 = 0x8B;
/// How long a PowerDown waits for the firmware to stop a program, save
/// and switch off.
const POWER_DOWN_LIMIT: Duration = Duration::from_secs(30);
/// A queued key transition waits for the firmware to take the previous one
/// (as scripted keys do): the firmware drops keys that arrive in one burst
/// (typing `WAITING` at once left `WA`). A program that never asks for
/// events still gets its keys after this long.
const KEY_FALLBACK: Duration = Duration::from_millis(50);
struct Capture {
    last: Vec<u8>,
    scratch: Vec<u8>,
    generation: u64,
    next: Instant,
}
pub struct LiveSession {
    commands: Mutex<std::sync::mpsc::Receiver<Command>>,
    output: Arc<(Mutex<Output>, Condvar)>,
    device: Mutex<Device>,
    capture: Mutex<Capture>,
    started: Instant,
    shutdown: AtomicBool,
}
impl LiveSession {
    pub fn stdio() -> Arc<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel(128);
        let out = Arc::new((Mutex::new(Output::default()), Condvar::new()));
        let session = Arc::new(Self {
            commands: Mutex::new(rx),
            output: out.clone(),
            device: Mutex::new(Device {
                held: HashSet::new(),
                delivered: HashSet::new(),
                repeat: repeat::KeyRepeat::default(),
                script: None,
                jobs: job::Jobs::new(),
                lifecycle: Lifecycle::Booting,
                boot: job::IdleMeter::default(),
                unsupported_seen: 0,
                keys: VecDeque::new(),
                last_key: Instant::now(),
                power_down: None,
                recovery: None,
                saved_sleep: 0,
                save_result: None,
                save_worker: None,
            }),
            capture: Mutex::new(Capture {
                last: vec![0; 320 * 240 * 4],
                scratch: vec![0; 320 * 240 * 4],
                generation: 0,
                next: Instant::now(),
            }),
            started: Instant::now(),
            shutdown: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&session);
        std::thread::spawn(move || {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            while let Ok(command) = session::read_command(&mut input) {
                let stop = matches!(command, Command::Shutdown);
                if tx.send(command).is_err() || stop {
                    break;
                }
            }
            if let Some(s) = weak.upgrade() {
                s.shutdown.store(true, Ordering::Release);
            }
        });
        std::thread::spawn(move || {
            let stdout = io::stdout();
            let mut writer = io::BufWriter::new(stdout.lock());
            loop {
                let (events, frame, closed) = {
                    let mut state = out.0.lock().unwrap();
                    while state.events.is_empty() && state.frame.is_none() && !state.closed {
                        state = out.1.wait(state).unwrap();
                    }
                    (
                        state.events.drain(..).collect::<Vec<_>>(),
                        state.frame.take(),
                        state.closed,
                    )
                };
                for event in events {
                    if session::write_event(&mut writer, &event).is_err() {
                        return;
                    }
                }
                if let Some((g, t, p)) = frame {
                    if session::write_frame(&mut writer, g, t, &p).is_err() {
                        return;
                    }
                }
                if closed {
                    return;
                }
            }
        });
        session.event(Event::Hello {
            version: session::VERSION,
        });
        session
    }
    pub fn event(&self, event: Event) {
        let mut state = self.output.0.lock().unwrap();
        if state.events.len() < 256 {
            state.events.push_back(event);
        }
        self.output.1.notify_one();
    }
    pub fn close(&self, message: String) {
        self.event(Event::Closed { message });
        self.output.0.lock().unwrap().closed = true;
        self.output.1.notify_one();
    }
    pub fn tick(&self, mem: &Arc<GuestMemory>, shared: &Arc<SharedState>, trace: &TraceHub) {
        if self.shutdown.load(Ordering::Acquire) {
            shared.set_stopping();
            return;
        }
        let mut device = self.device.lock().unwrap();
        // Snapshot only at the firmware's completed OFF boundary. Keep ON and
        // all mutations queued until copying finishes; the UI thread never waits.
        if shared.sleeping() && device.recovery.is_some() && device.saved_sleep != shared.sleeps() {
            if let Some(worker) = &device.save_worker {
                match worker.try_recv() {
                    Ok(result) => {
                        device.saved_sleep = shared.sleeps();
                        self.event(Event::Status {
                            message: match &result {
                                Ok(()) => "Calculator recovery checkpoint saved".into(),
                                Err(e) => format!(
                                    "Recovery checkpoint failed; previous checkpoint retained: {e}"
                                ),
                            },
                        });
                        device.save_result = Some(result);
                        device.save_worker = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => return,
                    Err(_) => {
                        device.saved_sleep = shared.sleeps();
                        device.save_result = Some(Err("checkpoint worker stopped".into()));
                        device.save_worker = None;
                    }
                }
            } else {
                let store = device.recovery.as_ref().unwrap().clone();
                let mem = mem.clone();
                let shared = shared.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                device.save_worker = Some(rx);
                self.event(Event::Status {
                    message: "Saving calculator recovery checkpoint…".into(),
                });
                std::thread::spawn(move || {
                    let result = (|| {
                        let start = Instant::now();
                        let guard = loop {
                            if let Some(g) = mem.snapshot_gate.try_snapshot() {
                                break g;
                            }
                            if start.elapsed() > Duration::from_secs(5) {
                                return Err("could not freeze sleeping firmware".into());
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        };
                        shared.vfs.check_durable().map_err(|e| e.to_string())?;
                        let result = store.checkpoint().map_err(|e| e.to_string());
                        drop(guard);
                        result
                    })();
                    let _ = tx.send(result);
                });
                return;
            }
        }
        for command in self.commands.lock().unwrap().try_iter().take(128) {
            match command {
                Command::Key { code, down }
                    if (code <= 191 || code == 0xE047) && device.jobs.accepts_input() =>
                {
                    if down {
                        device.held.insert(code);
                    } else {
                        device.held.remove(&code);
                        device.repeat.release(code);
                    }
                    device.keys.push_back((code, down));
                }
                Command::Touch { status, x, y }
                    if matches!(status, 1 | 2 | 8)
                        && x < 320
                        && y < 240
                        && device.jobs.accepts_input() =>
                {
                    shared.push_touch(TouchSample { status, x, y })
                }
                Command::ReleaseAll => {
                    device.held.clear();
                    device.repeat.clear();
                    device.keys.clear();
                    shared.clear_input_events();
                    let held: Vec<u16> = device.delivered.drain().collect();
                    device
                        .keys
                        .extend(held.into_iter().map(|code| (code, false)));
                }
                Command::Shutdown => shared.set_stopping(),
                Command::PowerDown => {
                    device.repeat.clear();
                    if device.power_down.is_none() {
                        device.power_down = Some((Instant::now(), false));
                        // A program is stopped first, as Stop does.
                        if device.jobs.interrupt(shared, trace) {
                            device.lifecycle = Lifecycle::Stopping;
                            self.event(Event::Lifecycle {
                                state: Lifecycle::Stopping,
                            });
                        }
                        if let Some(request) = device.jobs.cancel(mem, shared) {
                            self.event(Event::Result {
                                request,
                                success: false,
                                message:
                                    "Firmware operation cancelled: the calculator is switching off"
                                        .into(),
                                check: None,
                            });
                        }
                        device.script = None;
                    }
                }
                Command::Script { keys } => match crate::keys::parse_keys(&keys) {
                    Ok(keys) => {
                        device.script = Some(ScriptDriver::new(ScriptConfig {
                            keys,
                            idle_observe_ms: 0,
                            settle_ms: 150,
                            ..Default::default()
                        }));
                    }
                    Err(message) => self.event(Event::Error { message }),
                },
                Command::Import {
                    request,
                    name,
                    source,
                } => {
                    match device.jobs.start(
                        job::Request {
                            request,
                            source,
                            entry: None,
                            arguments: String::new(),
                            run: false,
                            import: Some(name),
                        },
                        shared,
                        trace,
                    ) {
                        Ok(()) => self.event(Event::Phase {
                            phase: "compiling".into(),
                        }),
                        Err(message) => self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        }),
                    }
                }
                Command::Stop => {
                    device.script = None;
                    let interrupted = device.jobs.interrupt(shared, trace);
                    if interrupted {
                        device.lifecycle = Lifecycle::Stopping;
                        self.event(Event::Lifecycle {
                            state: Lifecycle::Stopping,
                        });
                    }
                    if let Some(request) = device.jobs.cancel(mem, shared) {
                        self.event(Event::Result {
                            request,
                            success: false,
                            message: "Firmware operation cancelled".into(),
                            check: None,
                        });
                        self.event(Event::Phase {
                            phase: "ready".into(),
                        });
                    }
                    if !interrupted {
                        shared.push_key_press(job::ON_KEY);
                    }
                }
                Command::LoadSource { request, source } => {
                    match device.jobs.start(
                        job::Request {
                            request,
                            source,
                            entry: None,
                            arguments: String::new(),
                            run: false,
                            import: None,
                        },
                        shared,
                        trace,
                    ) {
                        Ok(()) => self.event(Event::Phase {
                            phase: "compiling".into(),
                        }),
                        Err(message) => self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        }),
                    }
                }
                Command::FrameRequest => {
                    self.capture_forced(mem, shared);
                }
                Command::Compile { request, source } => {
                    match device.jobs.start(
                        job::Request {
                            request,
                            source,
                            entry: None,
                            arguments: String::new(),
                            run: false,
                            import: None,
                        },
                        shared,
                        trace,
                    ) {
                        Ok(()) => self.event(Event::Phase {
                            phase: "compiling".into(),
                        }),
                        Err(message) => self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        }),
                    }
                }
                Command::DirectCheck { request, source } => {
                    match device
                        .jobs
                        .start_direct(request, source, mem, shared, trace)
                    {
                        Ok(()) => self.event(Event::Phase {
                            phase: "compiling".into(),
                        }),
                        Err(message) => self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        }),
                    }
                }
                Command::Run {
                    request,
                    source,
                    entry,
                    arguments,
                } => {
                    match device.jobs.start(
                        job::Request {
                            request,
                            source,
                            entry,
                            arguments,
                            run: true,
                            import: None,
                        },
                        shared,
                        trace,
                    ) {
                        Ok(()) => self.event(Event::Phase {
                            phase: "compiling".into(),
                        }),
                        Err(message) => self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        }),
                    }
                }
                _ => self.event(Event::Error {
                    message: "Unsupported calculator input".into(),
                }),
            }
        }
        // One key transition at a time: once the previous event was taken
        // and the UI pump waits for the next one.
        if !device.keys.is_empty()
            && shared.input_events_pending() == 0
            && (shared.getevent_parked() || device.last_key.elapsed() >= KEY_FALLBACK)
        {
            let (code, down) = device.keys.pop_front().unwrap();
            shared.push_key_transition(code, down);
            if down {
                device.delivered.insert(code);
                // A tap may already have its release queued; never repeat it.
                if device.held.contains(&code) {
                    device.repeat.press(code, Instant::now());
                }
            } else {
                device.delivered.remove(&code);
            }
            device.last_key = Instant::now();
        }
        // Generate at most one repeat after consumption, never a catch-up burst.
        if device.keys.is_empty()
            && shared.input_events_pending() == 0
            && shared.getevent_parked()
            && shared.main_waiting()
            && device.jobs.active.is_none()
            && device.power_down.is_none()
            && !shared.sleeping()
        {
            if let Some((code, down)) = device.repeat.poll(Instant::now()) {
                shared.push_key_transition(code, down);
            }
        }
        for event in device.jobs.tick(mem, shared, trace) {
            self.event(event);
        }
        // Booting ends when the firmware UI first settles waiting for input;
        // Stopping ends when the interrupted invocation is closed; Sleeping
        // follows the firmware's SysPowerOff.
        let asleep = shared.sleeping();
        let current = device.lifecycle;
        let next = match current {
            _ if asleep => Lifecycle::Sleeping,
            Lifecycle::Booting if device.boot.settled(shared) => Lifecycle::Ready,
            Lifecycle::Stopping if !device.jobs.interrupting() => Lifecycle::Ready,
            Lifecycle::Sleeping => Lifecycle::Ready,
            other => other,
        };
        if next != device.lifecycle {
            device.lifecycle = next;
            self.event(Event::Lifecycle { state: next });
        }
        self.power_down(&mut device, shared);
        let Device {
            unsupported_seen, ..
        } = &mut *device;
        for (id, name, caller_pc) in trace.unsupported_sites_since(unsupported_seen) {
            self.event(Event::Unsupported {
                id,
                name,
                caller_pc,
            });
        }
        if let Some(script) = &mut device.script {
            trace.set_script_step(script.current_step());
            match script.tick(mem, shared) {
                Tick::Continue => {}
                Tick::Done => {
                    device.script = None;
                    self.event(Event::Status {
                        message: "Calculator ready".into(),
                    });
                }
                Tick::Failed(message) => {
                    device.script = None;
                    self.event(Event::Error { message });
                }
            }
        }
    }
    /// Advances a PowerDown: once no program runs, press the firmware's
    /// OFF; once it sleeps (its state saved), end the session.
    fn power_down(&self, device: &mut Device, shared: &SharedState) {
        let Some((since, off_sent)) = device.power_down else {
            return;
        };
        let end = |message: &str| {
            self.event(Event::Status {
                message: message.into(),
            });
            shared.set_stopping();
        };
        if shared.sleeping() {
            // SysPowerOff can begin during this tick, after the initial save
            // check. Let the next tick start/finish that checkpoint first.
            if device.recovery.is_some() && device.saved_sleep != shared.sleeps() {
                return;
            }
            if let Some(store) = &device.recovery {
                if matches!(device.save_result, Some(Ok(()))) {
                    if let Err(e) = store.clean_shutdown() {
                        return end(&format!(
                            "Checkpoint saved, but shutdown marker failed: {e}"
                        ));
                    }
                } else {
                    return end(
                        "Firmware switched off, but recovery checkpoint failed; previous saved generation retained",
                    );
                }
            }
            return end("The calculator saved its state and switched off");
        }
        if device.lifecycle == Lifecycle::Booting {
            // Nothing was used yet; the firmware has no state to save.
            return end("Shut down while booting");
        }
        if since.elapsed() >= POWER_DOWN_LIMIT {
            return end(
                "The firmware did not complete power-off; changes since the last OFF may be lost",
            );
        }
        if !off_sent
            && device.jobs.active.is_none()
            && !device.jobs.interrupting()
            && device.lifecycle == Lifecycle::Ready
        {
            device.held.clear();
            device.keys.extend([
                (SHIFT_KEY, true),
                (SHIFT_KEY, false),
                (crate::services::ON_KEY_CODE, true),
                (crate::services::ON_KEY_CODE, false),
            ]);
            device.power_down = Some((since, true));
        }
    }
    pub fn capture(&self, mem: &GuestMemory, shared: &SharedState) {
        let Ok(mut capture) = self.capture.try_lock() else {
            return;
        };
        let now = Instant::now();
        if now < capture.next {
            return;
        }
        capture.next = now + Duration::from_millis(16);
        let Some(_snapshot) = mem.snapshot_gate.try_snapshot() else {
            return;
        };
        let Ok(view) = LcdView::locate(mem, *shared.lcd_object.lock().unwrap()) else {
            return;
        };
        if mem.read_bytes(view.fb_va, &mut capture.scratch).is_err() {
            return;
        }
        if capture.generation != 0 && capture.scratch == capture.last {
            return;
        }
        let pixels = capture
            .scratch
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) & 0xffffff)
            .collect();
        let Capture { last, scratch, .. } = &mut *capture;
        std::mem::swap(last, scratch);
        capture.generation += 1;
        let frame = (
            capture.generation,
            self.started.elapsed().as_micros() as u64,
            pixels,
        );
        self.output.0.lock().unwrap().frame = Some(frame);
        self.output.1.notify_one();
    }
    pub fn capture_forced(&self, mem: &GuestMemory, shared: &SharedState) {
        let Ok(mut capture) = self.capture.try_lock() else {
            return;
        };
        let Some(_snapshot) = mem.snapshot_gate.try_snapshot() else {
            return;
        };
        let Ok(view) = LcdView::locate(mem, *shared.lcd_object.lock().unwrap()) else {
            return;
        };
        if mem.read_bytes(view.fb_va, &mut capture.scratch).is_err() {
            return;
        }
        let pixels = capture
            .scratch
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) & 0xffffff)
            .collect();
        let Capture { last, scratch, .. } = &mut *capture;
        std::mem::swap(last, scratch);
        capture.generation += 1;
        let frame = (
            capture.generation,
            self.started.elapsed().as_micros() as u64,
            pixels,
        );
        self.output.0.lock().unwrap().frame = Some(frame);
        self.output.1.notify_one();
    }
}
pub fn run(
    firmware: std::path::PathBuf,
    os_rom: Option<std::path::PathBuf>,
    data_dir: std::path::PathBuf,
) -> i32 {
    let session = LiveSession::stdio();
    // Separate helper lock also protects direct session callers. GUI's existing
    // supervisor session.lock remains unchanged for compatibility.
    let helper_lock = (|| -> std::io::Result<std::fs::File> {
        std::fs::create_dir_all(&data_dir)?;
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(data_dir.join("helper.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        Ok(lock)
    })();
    let _helper_lock = match helper_lock {
        Ok(lock) => lock,
        Err(e) => {
            session.close(format!("Calculator profile is unavailable: {e}"));
            return 1;
        }
    };
    let bootstrap = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut identity = std::fs::read(&firmware)?;
        if let Some(rom) = &os_rom {
            identity.extend(std::fs::read(rom)?);
        }
        let resources = data_dir.join("A/programs/misc/armfir.dat");
        if resources.exists() {
            identity.extend(crate::recovery::resource_hash(&resources)?.as_bytes());
        } else {
            identity.extend(b"no-armfir.dat");
        }
        let (recovery, restored) =
            crate::recovery::Recovery::begin(&data_dir, crate::sha256::hex(&identity))?;
        session.device.lock().unwrap().recovery = Some(Arc::new(recovery));
        if restored {
            session.event(Event::Status { message: "Recovered calculator data from the last intact storage checkpoint; interrupted files were archived".into() });
        }

        let dir = data_dir.join("C/DATA");
        std::fs::create_dir_all(&dir)?;
        for (name, source) in [
            ("ZZT_IDE", "EXPORT MAIN()\nBEGIN\n RETURN 0;\nEND;\n"),
            ("ZZT_LOADER", job::LOADER),
            ("ZZT_IMPORT", job::IMPORTER),
        ] {
            let path = dir.join(format!("{name}.hpprgm"));
            // The IDE's own first loader, recognised by its exact source, is
            // upgraded before the firmware enumerates the programs; any
            // other ZZT_LOADER stays as it is (and is refused by the jobs).
            if name == "ZZT_LOADER" && path.exists() {
                let v1 = crate::hpprgm::parse(&std::fs::read(&path)?)
                    .ok()
                    .and_then(|c| c.source().ok())
                    .is_some_and(|s| s.source == job::LOADER_V1);
                if v1 {
                    let pending = path.with_extension("pending");
                    std::fs::write(&pending, crate::hpprgm::build_program(source)?.to_bytes())?;
                    std::fs::File::open(&pending)?.sync_all()?;
                    std::fs::rename(&pending, &path)?;
                }
            }
            if !path.exists() {
                use std::io::Write;
                let bytes = crate::hpprgm::build_program(source)?.to_bytes();
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
        }
        Ok(())
    })();
    if let Err(e) = bootstrap {
        session.close(e.to_string());
        return 1;
    }
    session.event(Event::Lifecycle {
        state: Lifecycle::Booting,
    });
    session.event(Event::Status {
        message: "Booting local G1 firmware…".into(),
    });
    let cfg = crate::runner::RunConfig {
        firmware_path: firmware,
        os_rom_path: os_rom,
        data_dir,
        max_seconds: f64::MAX,
        max_instructions: u64::MAX,
        ui_timers: true,
        on_unsupported: crate::services::UnsupportedPolicy::ZeroReturn,
        trace: crate::trace::TraceConfig {
            compiler: true,
            runtime: true,
            fs_trace: true,
            verbosity: 0,
            jsonl_path: std::env::var_os("PRIME_SESSION_TRACE").map(std::path::PathBuf::from),
            ..Default::default()
        },
        ..Default::default()
    };
    match crate::runner::run_with_session(&cfg, Some(session.clone())) {
        Ok(summary) => {
            session.close(summary.report.evidence);
            0
        }
        Err(e) => {
            session.event(Event::Error {
                message: e.to_string(),
            });
            session.close(e.to_string());
            1
        }
    }
}
