//! Long-lived local firmware session, with bounded transport and coherent LCD capture.
mod job;

use crate::{
    display::LcdView,
    memory::GuestMemory,
    script::{ScriptConfig, ScriptDriver, Tick},
    services::{SharedState, TouchSample},
    trace::TraceHub,
};
use ppl_engine::session::{self, Command, Event};
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
    script: Option<ScriptDriver>,
    jobs: job::Jobs,
}
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
                script: None,
                jobs: job::Jobs::new(),
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
    pub fn tick(&self, mem: &GuestMemory, shared: &SharedState, trace: &TraceHub) {
        if self.shutdown.load(Ordering::Acquire) {
            shared.set_stopping();
            return;
        }
        let mut device = self.device.lock().unwrap();
        for command in self.commands.lock().unwrap().try_iter().take(128) {
            match command {
                Command::Key { code, down }
                    if (code <= 191 || code == 0xE047) && device.jobs.accepts_input() =>
                {
                    if down {
                        device.held.insert(code);
                    } else {
                        device.held.remove(&code);
                    }
                    shared.push_key_transition(code, down);
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
                    for code in device.held.drain() {
                        shared.push_key_transition(code, false);
                    }
                }
                Command::Shutdown => shared.set_stopping(),
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
                Command::Stop => {
                    device.script = None;
                    if let Some(request) = device.jobs.cancel() {
                        self.event(Event::Result {
                            request,
                            success: false,
                            message: "Firmware operation cancelled".into(),
                        });
                        self.event(Event::Phase {
                            phase: "ready".into(),
                        });
                    }
                    shared.push_key_press(0x83);
                }
                Command::LoadSource { request, source } => {
                    match device.jobs.start(
                        job::Request {
                            request,
                            source,
                            entry: None,
                            arguments: String::new(),
                            run: false,
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
                        }),
                    }
                }
                _ => self.event(Event::Error {
                    message: "Unsupported calculator input".into(),
                }),
            }
        }
        for event in device.jobs.tick(mem, shared, trace) {
            self.event(event);
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
    let bootstrap = (|| -> Result<(), Box<dyn std::error::Error>> {
        let dir = data_dir.join("C/DATA");
        std::fs::create_dir_all(&dir)?;
        for (name, source) in [
            ("ZZT_IDE", "EXPORT MAIN()\nBEGIN\n RETURN 0;\nEND;\n"),
            ("ZZT_LOADER", job::LOADER),
        ] {
            let path = dir.join(format!("{name}.hpprgm"));
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
