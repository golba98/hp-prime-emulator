//! Long-lived local firmware session, with bounded transport and coherent LCD capture.
mod job;
mod readiness;
mod repeat;
mod typing;
pub use job::{LOADER, LOADER_V1, LOADER_V2};

use crate::{
    display::LcdView,
    memory::GuestMemory,
    script::{ScriptConfig, ScriptDriver, Tick},
    services::{SharedState, TouchSample},
    trace::TraceHub,
};
use ppl_engine::checkpoint::{CheckpointReceipt, CheckpointResult};
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
impl Output {
    /// Reserve replies and lifecycle evidence; only optional notifications may be coalesced.
    fn enqueue(&mut self, event: Event) -> bool {
        let optional = |e: &Event| {
            matches!(
                e,
                Event::Status { .. } | Event::Phase { .. } | Event::Unsupported { .. }
            )
        };
        if self.closed {
            return false;
        }
        if self.events.len() >= 256 {
            if optional(&event) {
                return true;
            }
            if let Some(index) = self.events.iter().position(optional) {
                self.events.remove(index);
            }
        }
        if self.events.len() >= 512 {
            self.events.push_back(Event::Error {
                message: "Mandatory session event queue overflow; session stopped".into(),
            });
            self.events.push_back(Event::Closed {
                message: "Session output consumer did not drain mandatory replies".into(),
            });
            self.closed = true;
            return false;
        }
        self.events.push_back(event);
        true
    }
}
struct Device {
    held: HashSet<u16>,
    scenario_touch: Option<(u64, u16, u16)>,
    input_cleanup: Option<(u64, job::IdleMeter, Instant)>,
    delivered: HashSet<u16>,
    repeat: repeat::KeyRepeat,
    typing: typing::Typing,
    typing_ack: typing::UiAcknowledgement,
    script: Option<ScriptDriver>,
    jobs: job::Jobs,
    /// Last lifecycle state reported, and observed event-loop boot readiness.
    lifecycle: Lifecycle,
    boot: readiness::BootReadiness,
    /// Unsupported service call sites already reported.
    unsupported_seen: usize,
    /// Key transitions not yet given to the firmware, and when the last one
    /// was: see [`KEY_FALLBACK`].
    keys: VecDeque<(u16, bool)>,
    last_key: Instant,
    /// A `PowerDown` in progress: when it began, and whether OFF was sent.
    power_down: Option<(Instant, bool)>,
    power_sequence: Option<crate::checkpoint::OffSequence>,
    checkpoint: Option<crate::checkpoint::Checkpoint>,
    checkpoint_idle: job::IdleMeter,
    checkpoint_wait_entry: u64,
    recovery: Option<Arc<crate::recovery::Recovery>>,
    saved_sleep: Option<u64>,
    save_result: Option<Result<CheckpointReceipt, String>>,
    save_worker: Option<std::sync::mpsc::Receiver<Result<CheckpointReceipt, String>>>,
}
/// A key-up is not consumed merely because it left Device.keys. Preserve releases
/// for canceled transitions already queued to GetEvent, including pending key-ups.
fn release_input(device: &mut Device, shared: &SharedState) {
    if let Some((_, x, y)) = device.scenario_touch.take() {
        shared.push_touch(TouchSample { status: 8, x, y });
    }
    let mut releases: HashSet<u16> = shared
        .pop_input_events(usize::MAX)
        .into_iter()
        .map(|e| e.key_code0)
        .collect();
    releases.extend(device.keys.drain(..).map(|(code, _)| code));
    releases.extend(device.held.drain());
    releases.extend(device.delivered.drain());
    device.repeat.clear();
    device.typing.clear();
    device
        .keys
        .extend(releases.into_iter().map(|code| (code, false)));
}
fn poll_input_cleanup(device: &mut Device, shared: &SharedState) -> Option<Event> {
    let done = device
        .input_cleanup
        .as_mut()
        .is_some_and(|(_, meter, _)| meter.settled(shared));
    let failed = device
        .input_cleanup
        .as_ref()
        .is_some_and(|(_, _, started)| {
            shared.sleeping()
                || device.power_down.is_some()
                || started.elapsed() >= Duration::from_secs(30)
        });
    if failed || (done && device.keys.is_empty()) {
        let (request, _, _) = device.input_cleanup.take()?;
        Some(Event::Result {
            request,
            success: !failed,
            message: if failed {
                "Input release did not reach an awake idle boundary before its deadline".into()
            } else {
                "Input releases consumed and firmware idle".into()
            },
            check: None,
        })
    } else {
        None
    }
}
fn track_touch(touch: &mut Option<(u64, u16, u16)>, run: u64, status: u32, x: u16, y: u16) {
    if status == 8 {
        *touch = None;
    } else {
        *touch = Some((run, x, y));
    }
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
    writer: Mutex<Option<std::thread::JoinHandle<()>>>,
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
                scenario_touch: None,
                input_cleanup: None,
                delivered: HashSet::new(),
                repeat: repeat::KeyRepeat::default(),
                typing: typing::Typing::default(),
                typing_ack: typing::UiAcknowledgement::default(),
                script: None,
                jobs: job::Jobs::new(),
                lifecycle: Lifecycle::Booting,
                boot: readiness::BootReadiness::default(),
                unsupported_seen: 0,
                keys: VecDeque::new(),
                last_key: Instant::now(),
                power_down: None,
                power_sequence: None,
                checkpoint: None,
                checkpoint_idle: job::IdleMeter::default(),
                checkpoint_wait_entry: 0,
                recovery: None,
                saved_sleep: None,
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
            writer: Mutex::new(None),
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
        let writer = std::thread::spawn(move || {
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
        *session.writer.lock().unwrap() = Some(writer);
        session.event(Event::Hello {
            version: session::VERSION,
        });
        session
    }
    pub fn event(&self, event: Event) {
        let mut state = self.output.0.lock().unwrap();
        if !state.enqueue(event) {
            self.shutdown.store(true, Ordering::Release);
        }
        self.output.1.notify_one();
    }
    pub fn close(&self, message: String) {
        {
            let mut output = self.output.0.lock().unwrap();
            // Terminal delivery must not be dropped at the ordinary event
            // queue limit, nor separated from closing the writer's queue.
            output.events.push_back(Event::Closed { message });
            output.closed = true;
            self.output.1.notify_one();
        }
        // main calls process::exit after run_stdio returns. A detached writer
        // could otherwise lose Closed (and the last save status). The reader
        // owns pipe backpressure; broken pipes end the writer normally.
        if let Some(writer) = self.writer.lock().unwrap().take() {
            let _ = writer.join();
        }
    }
    pub fn tick(&self, mem: &Arc<GuestMemory>, shared: &Arc<SharedState>, trace: &TraceHub) {
        if self.shutdown.load(Ordering::Acquire) {
            shared.set_stopping();
            return;
        }
        let mut device = self.device.lock().unwrap();
        // Snapshot only at the firmware's completed OFF boundary. Keep ON and
        // all mutations queued until copying finishes; the UI thread never waits.
        if shared.sleeping()
            && device.recovery.is_some()
            && device.saved_sleep != Some(shared.sleeps())
        {
            if let Some(worker) = &device.save_worker {
                match worker.try_recv() {
                    Ok(result) => {
                        device.saved_sleep = Some(shared.sleeps());
                        self.event(Event::Status {
                            message: match &result {
                                Ok(_) => "Calculator recovery checkpoint saved".into(),
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
                        device.saved_sleep = Some(shared.sleeps());
                        device.save_result = Some(Err("checkpoint worker stopped".into()));
                        device.save_worker = None;
                    }
                }
            } else {
                let store = device.recovery.as_ref().unwrap().clone();
                let mem = mem.clone();
                let shared = shared.clone();
                let explicit = device.checkpoint.is_some();
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
                        let result = if explicit {
                            store.checkpoint_bounded(&crate::recovery::CheckpointPolicy::default())
                        } else {
                            store.checkpoint_receipt()
                        }
                        .map_err(|e| e.to_string());
                        drop(guard);
                        result
                    })();
                    let _ = tx.send(result);
                });
                return;
            }
        }
        for command in self.commands.lock().unwrap().try_iter().take(128) {
            // Once OFF begins, admit no new calculator mutations until the
            // exact sleep generation has been saved. Shutdown remains an
            // immediate escape; frame requests do not mutate calculator state.
            if device.checkpoint.is_some()
                && !matches!(
                    &command,
                    Command::Shutdown | Command::FrameRequest | Command::Inspect { .. }
                )
            {
                let message = "A firmware persistence checkpoint is in progress".to_string();
                match command {
                    Command::Checkpoint { id, .. } => self.event(Event::Checkpoint {
                        id,
                        result: CheckpointResult::Deferred { reason: message },
                    }),
                    Command::Run { request, .. }
                    | Command::Compile { request, .. }
                    | Command::DirectCheck { request, .. }
                    | Command::LoadSource { request, .. }
                    | Command::Import { request, .. }
                    | Command::StoreProgram { request, .. }
                    | Command::RunStored { request, .. }
                    | Command::ScenarioInput { request, .. }
                    | Command::ReleaseInput { request, .. } => self.event(Event::Result {
                        request,
                        success: false,
                        message,
                        check: None,
                    }),
                    _ => self.event(Event::Status { message }),
                }
                continue;
            }
            if matches!(
                &command,
                Command::Run { .. }
                    | Command::Compile { .. }
                    | Command::DirectCheck { .. }
                    | Command::LoadSource { .. }
                    | Command::Import { .. }
                    | Command::StoreProgram { .. }
                    | Command::RunStored { .. }
                    | Command::Script { .. }
            ) {
                device.typing.clear();
            }
            if trace.compiler.is_none() || trace.runtime.is_none() {
                let request = match &command {
                    Command::Run { request, .. }
                    | Command::Compile { request, .. }
                    | Command::DirectCheck { request, .. }
                    | Command::LoadSource { request, .. }
                    | Command::Import { request, .. }
                    | Command::StoreProgram { request, .. }
                    | Command::RunStored { request, .. } => Some(*request),
                    _ => None,
                };
                if let Some(request) = request {
                    self.event(Event::Result { request, success:false, message:"This firmware supports calculator input/display/power, but has no verified compiler/runtime observation profile for managed Check/Run/Import".into(), check:None });
                    continue;
                }
            }
            match command {
                Command::Inspect { request } => self.event(Event::Inspection {
                    request,
                    report: Box::new(self.inspect(&device, mem, shared, trace)),
                }),
                Command::Text { text, paste }
                    if device.jobs.accepts_input()
                        && device.script.is_none()
                        && device.power_down.is_none()
                        && !shared.sleeping() =>
                {
                    device.jobs.invalidate_prepared();
                    if let Err(message) = if paste {
                        device.typing.paste(&text)
                    } else {
                        device.typing.text(&text)
                    } {
                        self.event(Event::Status { message });
                    }
                }
                Command::Text { .. } => self.event(Event::Status {
                    message:
                        "Calculator typing is unavailable while firmware is busy or switched off"
                            .into(),
                }),
                Command::Key { code, down }
                    if (code <= 191 || code == 0xE047) && device.jobs.accepts_input() =>
                {
                    device.jobs.invalidate_prepared();
                    if code == 0x01 && down && !device.typing.is_empty() {
                        device.typing.clear();
                        device.keys.clear();
                        shared.clear_input_events();
                        let delivered: Vec<_> = device.delivered.drain().collect();
                        device
                            .keys
                            .extend(delivered.into_iter().map(|code| (code, false)));
                    }
                    if down {
                        device.held.insert(code);
                    } else {
                        device.held.remove(&code);
                        device.repeat.release(code);
                    }
                    if device.typing.is_empty() {
                        device.typing.clear();
                        device.keys.push_back((code, down));
                    } else {
                        device.typing.key(code, down);
                    }
                }
                Command::Touch { status, x, y }
                    if matches!(status, 1 | 2 | 8)
                        && x < 320
                        && y < 240
                        && device.jobs.accepts_input() =>
                {
                    device.jobs.invalidate_prepared();
                    device.typing.clear();
                    shared.push_touch(TouchSample { status, x, y })
                }
                Command::ReleaseAll => {
                    release_input(&mut device, shared);
                }
                Command::ReleaseInput { request } => {
                    if device.input_cleanup.is_some()
                        || shared.sleeping()
                        || device.power_down.is_some()
                    {
                        self.event(Event::Result {
                            request,
                            success: false,
                            message: "Input release requires an awake calculator without shutdown or another pending release".into(),
                            check: None,
                        });
                    } else {
                        release_input(&mut device, shared);
                        device.input_cleanup =
                            Some((request, job::IdleMeter::default(), Instant::now()));
                    }
                }
                Command::Shutdown => shared.set_stopping(),
                Command::Checkpoint { id, wake } => {
                    let admission = crate::checkpoint::Admission {
                        ready: device.lifecycle == Lifecycle::Ready,
                        sleeping: shared.sleeping(),
                        recovery_available: device.recovery.is_some(),
                        program_active: device.jobs.active.is_some() || device.jobs.interrupting(),
                        guest_call_active: shared.guest_calls.busy(),
                        script_active: device.script.is_some(),
                        input_pending: device.input_cleanup.is_some()
                            || device.scenario_touch.is_some()
                            || !device.keys.is_empty()
                            || !device.typing.is_empty()
                            || shared.input_events_pending() != 0
                            || shared.touch_pending() != 0,
                        keys_held: !device.held.is_empty() || !device.delivered.is_empty(),
                        shutdown_active: device.power_down.is_some(),
                    };
                    match crate::checkpoint::Checkpoint::begin(id, wake, &admission) {
                        Ok(checkpoint) => {
                            device.jobs.invalidate_prepared();
                            device.repeat.clear();
                            device.checkpoint_idle.clear();
                            device.checkpoint_wait_entry = shared.main_wait_entries();
                            if shared.sleeping() {
                                // Even an already sleeping calculator gets a
                                // new bounded generation for this request.
                                device.saved_sleep = None;
                                device.save_result = None;
                            }
                            device.checkpoint = Some(checkpoint);
                        }
                        Err(result) => self.event(Event::Checkpoint { id, result }),
                    }
                }
                Command::PowerDown => {
                    device.typing.clear();
                    device.jobs.invalidate_prepared();
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
                        device.jobs.invalidate_prepared();
                        device.script = Some(ScriptDriver::new(ScriptConfig {
                            keys,
                            idle_observe_ms: 0,
                            settle_ms: 150,
                            ..Default::default()
                        }));
                    }
                    Err(message) => self.event(Event::Error { message }),
                },
                Command::ScenarioInput {
                    request,
                    run_request,
                    input,
                } => {
                    let running = device.jobs.running_request(run_request, trace);
                    let accepted = match input {
                        session::CalculatorInput::Key { physical, down } => {
                            if let Some(&code) =
                                session::PHYSICAL_DEVICE_CODES.get(physical as usize)
                            {
                                if running || (!down && device.held.contains(&code)) {
                                    if down {
                                        device.held.insert(code);
                                    } else {
                                        device.held.remove(&code);
                                    }
                                    device.keys.push_back((code, down));
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }
                        session::CalculatorInput::Touch { status, x, y }
                            if [1, 2, 8].contains(&status)
                                && x < 320
                                && y < 240
                                && (running
                                    || (status == 8
                                        && device
                                            .scenario_touch
                                            .is_some_and(|t| t.0 == run_request))) =>
                        {
                            track_touch(&mut device.scenario_touch, run_request, status, x, y);
                            shared.push_touch(TouchSample { status, x, y });
                            true
                        }
                        _ => false,
                    };
                    self.event(Event::Result {
                        request,
                        success: accepted,
                        message: if accepted {
                            "queued".into()
                        } else {
                            "Input is invalid or its program is no longer running; nothing queued"
                                .into()
                        },
                        check: None,
                    });
                }
                Command::StoreProgram {
                    request,
                    name,
                    source,
                    expected_previous_sha256,
                } => {
                    if let Err(message) = device.jobs.store_named(
                        request,
                        name,
                        source,
                        expected_previous_sha256,
                        shared,
                        trace,
                    ) {
                        self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        });
                    }
                }
                Command::RunStored {
                    request,
                    name,
                    source_sha256,
                    entry,
                    arguments,
                } => {
                    if let Err(message) = device.jobs.run_stored(
                        request,
                        name,
                        source_sha256,
                        entry,
                        arguments,
                        shared,
                        trace,
                    ) {
                        self.event(Event::Result {
                            request,
                            success: false,
                            message,
                            check: None,
                        });
                    }
                }
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
                    device.typing.clear();
                    device.jobs.invalidate_prepared();
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
        // Translate only once all preceding physical events were consumed.
        // The guest owns letter case and templates; observe its status under
        // the same snapshot gate as LCD capture, never host-side mode flags.
        if device.keys.is_empty()
            && !device.typing.is_empty()
            && shared.input_events_pending() == 0
            && shared.getevent_parked()
            && (!device.typing.observes_modifiers()
                || device
                    .typing_ack
                    .ready(shared.main_ui_waiting(), shared.main_wait_entries()))
            && device.jobs.accepts_input()
            && device.script.is_none()
        {
            if let Some(_snapshot) = mem.snapshot_gate.try_snapshot() {
                // Recheck under the memory gate: another UI quantum may have
                // resumed between the outer check and coherent capture.
                if !device.typing.observes_modifiers()
                    || device
                        .typing_ack
                        .ready(shared.main_ui_waiting(), shared.main_wait_entries())
                {
                    if let Ok(view) = LcdView::locate(mem, *shared.lcd_object.lock().unwrap()) {
                        let mut status = [0u8; 320 * 8 * 4];
                        if mem.read_bytes(view.fb_va, &mut status).is_ok() {
                            match device.typing.next(typing::observe(&status)) {
                                Ok(keys) => device.keys.extend(keys),
                                Err(message) => self.event(Event::Status { message }),
                            }
                        }
                    }
                }
            }
        }
        // One key transition at a time: once the previous event was taken
        // and the UI pump waits for the next one.
        if !device.keys.is_empty()
            && shared.input_events_pending() == 0
            && (shared.getevent_parked() || device.last_key.elapsed() >= KEY_FALLBACK)
        {
            let (code, down) = device.keys.pop_front().unwrap();
            if down {
                device.typing_ack.pressed(shared.main_wait_entries());
                if code == SHIFT_KEY {
                    if let Some(sequence) = &mut device.power_sequence {
                        sequence.shift_delivered(shared.main_wait_entries());
                    }
                }
            }
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
        if let Some(event) = poll_input_cleanup(&mut device, shared) {
            self.event(event);
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
        // Boot readiness proves completed event-loop round trips, independently
        // of the sustained idle share required for safe checkpoint admission;
        // Stopping ends when the interrupted invocation is closed; Sleeping
        // follows the firmware's SysPowerOff.
        let asleep = shared.sleeping();
        let current = device.lifecycle;
        let next = match current {
            _ if asleep => Lifecycle::Sleeping,
            Lifecycle::Booting
                if device.boot.observe(readiness::Sample {
                    valid_frame: self.capture.try_lock().is_ok_and(|c| c.generation != 0),
                    idle: job::IdleMeter::idle_now(shared),
                    main_waits: shared.main_wait_entries(),
                    pump_entries: shared.getevent_entries(),
                    timer_deliveries: shared.system_events_delivered(),
                }) =>
            {
                Lifecycle::Ready
            }
            Lifecycle::Stopping if !device.jobs.interrupting() => Lifecycle::Ready,
            Lifecycle::Sleeping => Lifecycle::Ready,
            other => other,
        };
        if next != device.lifecycle {
            device.lifecycle = next;
            self.event(Event::Lifecycle { state: next });
        }
        self.advance_checkpoint(&mut device, shared);
        self.power_down(&mut device, shared);
        self.advance_power_sequence(&mut device, mem, shared);
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
    fn inspect(
        &self,
        device: &Device,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> ppl_engine::inspection::SessionInspection {
        use ppl_engine::inspection::{SessionInspection, SnapshotReadiness};
        let pause = shared
            .safe_points
            .pause(crate::memory::snapshot::REQUEST_HOLD);

        let mut blockers = vec![
            "complete_cpu_context_export_unimplemented".into(),
            "host_resource_reconstruction_incomplete".into(),
        ];
        if let Err(reason) = &pause {
            blockers.push(reason.clone());
        }
        let gate = mem.snapshot_gate.try_snapshot();
        let (active, parked) = mem.snapshot_gate.counts();
        if gate.is_none() {
            blockers.push("active_guest_memory_writers".into());
        }
        if parked != 0 {
            blockers.push("parked_native_callback_continuations".into());
        }
        if device.jobs.active.is_some() {
            blockers.push("managed_program_operation_active".into());
        }
        if shared.input_events_pending() != 0
            || shared.touch_pending() != 0
            || !device.held.is_empty()
            || device.scenario_touch.is_some()
        {
            blockers.push("input_pending_or_held".into());
        }
        let owners = shared.safe_points.observations();
        let resources = serde_json::json!({"vfs":shared.vfs.inspection(),"sync_objects":shared.sync_objects.try_lock().ok().map(|m|m.len()),"guest_call_busy":shared.guest_calls.inspected_busy(),"main_waiting":shared.main_waiting(),"main_ui_idle":shared.main_ui_waiting(),"power_sequence":device.power_sequence.as_ref().map(|s|s.phase()),"typing_pending":device.typing.pending_len(),"typing_last_keydown_main_wait":device.typing_ack.baseline(),"main_wait_generation":shared.main_wait_entries(),"pump_parked":shared.getevent_parked(),"pump_entries":shared.getevent_entries(),"timers_delivered":shared.system_events_delivered(),"input_pending":shared.input_events_pending(),"touch_pending":shared.touch_pending()});
        let diagnostics = trace.diagnostic_bundle();
        let paused = pause.as_ref().is_ok_and(|lease| lease.is_current());
        if pause.is_ok() && !paused {
            blockers.push("owner_pause_expired_during_inspection".into());
        }
        let report = SessionInspection {
            version: ppl_engine::inspection::VERSION,
            resource_identity: device
                .recovery
                .as_ref()
                .map(|r| r.resource_identity().to_owned()),
            lifecycle: device.lifecycle,
            readiness: SnapshotReadiness {
                memory_gate_acquired: gate.is_some(),
                owners_paused: paused,
                active_engines: active,
                parked_native_callbacks: parked,
                live_restore_ready: false,
                blockers,
            },
            owners,
            resources,
            diagnostics,
        };
        drop(gate);
        drop(pause);
        report
    }
    fn advance_checkpoint(&self, device: &mut Device, shared: &SharedState) {
        let saved = (device.saved_sleep == Some(shared.sleeps()))
            .then_some(device.save_result.as_ref())
            .flatten();
        let Some(checkpoint) = &mut device.checkpoint else {
            return;
        };
        let id = checkpoint.id;
        let idle = shared.main_wait_entries() > device.checkpoint_wait_entry
            && device.checkpoint_idle.settled(shared);
        match checkpoint.poll(shared.sleeping(), idle, saved) {
            Some(crate::checkpoint::Action::PowerOff) => {
                device.power_sequence = Some(crate::checkpoint::OffSequence::new(false));
            }
            Some(crate::checkpoint::Action::Finished { mut result, wake }) => {
                if let CheckpointResult::Failed { message } = &mut result {
                    message.push_str(&format!(
                        " (checkpoint stage={}, off_sent={}, power sequence={:?}, main waits {}→{}, main waiting={}, main UI blocked={}, input pump parked={}, queued input={}, local keys={}, delivered keys={:?}, touch={}, sleeping={}, sleep generation={}, saved sleep={:?}, save worker={}, save result={:?}, idle={})",
                        if checkpoint.off_sent() {"awaiting_firmware_sleep_or_durable_save"} else {"awaiting_strict_idle_admission"},
                        checkpoint.off_sent(),
                        device.power_sequence.as_ref().map(|s|s.phase()),
                        device.checkpoint_wait_entry, shared.main_wait_entries(),
                        shared.main_waiting(), shared.main_ui_waiting(),shared.getevent_parked(),
                        shared.input_events_pending(), device.keys.len(),device.delivered, shared.touch_pending(),
                        shared.sleeping(),shared.sleeps(),device.saved_sleep,device.save_worker.is_some(),
                        device.save_result.as_ref().map(|r|r.as_ref().map(|_|"saved").map_err(String::as_str)),
                        device.checkpoint_idle.diagnostic(),
                    ));
                }
                device.power_sequence = None;
                if matches!(result, CheckpointResult::Failed { .. }) {
                    // An expired transaction must not deliver a queued ON later.
                    // Admission excluded unrelated input throughout this request.
                    release_input(device, shared);
                }
                if wake {
                    device.keys.extend([
                        (crate::services::ON_KEY_CODE, true),
                        (crate::services::ON_KEY_CODE, false),
                    ]);
                }
                device.checkpoint = None;
                self.event(Event::Checkpoint { id, result });
            }
            None => {}
        }
    }
    fn advance_power_sequence(&self, device: &mut Device, mem: &GuestMemory, shared: &SharedState) {
        if device.power_sequence.is_none()
            || !device.keys.is_empty()
            || shared.input_events_pending() != 0
            || !shared.getevent_parked()
            || !shared.main_ui_waiting()
        {
            return;
        }
        let Some(_snapshot) = mem.snapshot_gate.try_snapshot() else {
            return;
        };
        if !shared.main_ui_waiting()
            || shared.input_events_pending() != 0
            || !shared.getevent_parked()
        {
            return;
        }
        let Ok(view) = LcdView::locate(mem, *shared.lcd_object.lock().unwrap()) else {
            return;
        };
        let mut status = [0u8; 320 * 8 * 4];
        if mem.read_bytes(view.fb_va, &mut status).is_err() {
            return;
        }
        // Shift's cyan glyph is independently observed even while Alpha is
        // active. Never clear Alpha or guess an unrecognized screen neutral.
        let shift = typing::observe(&status).map(|m| m.shift);
        let action = device
            .power_sequence
            .as_mut()
            .unwrap()
            .advance(shift, shared.main_wait_entries());
        match action {
            Some(crate::checkpoint::OffAction::Shift) => {
                device.keys.extend([(SHIFT_KEY, true), (SHIFT_KEY, false)])
            }
            Some(crate::checkpoint::OffAction::On) => device.keys.extend([
                (crate::services::ON_KEY_CODE, true),
                (crate::services::ON_KEY_CODE, false),
            ]),
            Some(crate::checkpoint::OffAction::Unsupported) => {
                device.power_sequence = None;
                if let Some(checkpoint) = device.checkpoint.take() {
                    self.event(Event::Checkpoint{id:checkpoint.id,result:CheckpointResult::Deferred{reason:"This firmware screen has no verified Shift indicator. Return to Home, then retry; no OFF keys were sent".into()}});
                }
            }
            None => {}
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
            if device.recovery.is_some() && device.saved_sleep != Some(shared.sleeps()) {
                return;
            }
            if let Some(store) = &device.recovery {
                if matches!(device.save_result, Some(Ok(_))) {
                    if let Err(e) = store.clean_shutdown() {
                        return end(&format!(
                            "Checkpoint saved, but shutdown marker failed: {e}"
                        ));
                    }
                } else {
                    return end(&format!(
                        "Firmware switched off, but recovery checkpoint failed; previous saved generation retained: {}",
                        device
                            .save_result
                            .as_ref()
                            .and_then(|r| r.as_ref().err())
                            .map(String::as_str)
                            .unwrap_or("no checkpoint result"),
                    ));
                }
            }
            return end("The calculator saved its state and switched off");
        }
        if device.lifecycle == Lifecycle::Booting {
            // Nothing was used yet; the firmware has no state to save.
            return end("Shut down while booting");
        }
        if since.elapsed() >= POWER_DOWN_LIMIT {
            device.power_sequence = None;
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
            device.power_sequence = Some(crate::checkpoint::OffSequence::new(true));
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
            ("ZZT_NAMED", job::NAMED_LOADER),
        ] {
            let path = dir.join(format!("{name}.hpprgm"));
            // The IDE's own first loader, recognised by its exact source, is
            // upgraded before the firmware enumerates the programs; any
            // other ZZT_LOADER stays as it is (and is refused by the jobs).
            if name == "ZZT_LOADER" && path.exists() {
                let v1 = crate::hpprgm::parse(&std::fs::read(&path)?)
                    .ok()
                    .and_then(|c| c.source().ok())
                    .is_some_and(|s| s.source == job::LOADER_V1 || s.source == job::LOADER_V2);
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
    let observed = crate::recovery::resource_hash(&firmware)
        .is_ok_and(|h| h == crate::compiler::FIRMWARE_SHA256);
    session.event(Event::Capabilities {
        managed_programs: observed,
        reason: (!observed).then(|| {
            "Calculator-only firmware: no verified compiler/runtime observation profile".into()
        }),
    });
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
            compiler: observed,
            runtime: observed,
            svc_trace: std::env::var_os("PRIME_SESSION_TRACE").is_some(),
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

#[cfg(test)]
mod output_delivery_tests {
    use super::*;
    fn test_device() -> Device {
        Device {
            held: HashSet::new(),
            scenario_touch: None,
            input_cleanup: None,
            delivered: HashSet::new(),
            repeat: repeat::KeyRepeat::default(),
            typing: typing::Typing::default(),
            typing_ack: typing::UiAcknowledgement::default(),
            script: None,
            jobs: job::Jobs::new(),
            lifecycle: Lifecycle::Ready,
            boot: readiness::BootReadiness::default(),
            unsupported_seen: 0,
            keys: VecDeque::new(),
            last_key: Instant::now(),
            power_down: None,
            power_sequence: None,
            checkpoint: None,
            checkpoint_idle: job::IdleMeter::default(),
            checkpoint_wait_entry: 0,
            recovery: None,
            saved_sleep: None,
            save_result: None,
            save_worker: None,
        }
    }

    #[test]
    fn input_cleanup_fails_once_on_sleep_or_expiry_without_sticking_admission() {
        for asleep in [true, false] {
            let shared = SharedState::new("/tmp/unused-m16-input".into(), "A:test".into());
            let mut device = test_device();
            let started = if asleep {
                Instant::now()
            } else {
                Instant::now() - Duration::from_secs(31)
            };
            device.input_cleanup = Some((77, job::IdleMeter::default(), started));
            if asleep {
                shared.enter_sleep();
            }
            assert!(matches!(
                poll_input_cleanup(&mut device, &shared),
                Some(Event::Result {
                    request: 77,
                    success: false,
                    ..
                })
            ));
            assert!(device.input_cleanup.is_none());
            assert!(poll_input_cleanup(&mut device, &shared).is_none());
        }
    }
    #[test]
    fn automatic_touch_release_uses_latest_drag_position() {
        let shared = SharedState::new("/tmp/unused-m16-input".into(), "A:test".into());
        let mut device = test_device();
        track_touch(&mut device.scenario_touch, 9, 1, 20, 30);
        track_touch(&mut device.scenario_touch, 9, 2, 100, 50);
        release_input(&mut device, &shared);
        let up = shared.pop_touch().unwrap();
        assert_eq!((up.status, up.x, up.y), (8, 100, 50));
        assert!(device.scenario_touch.is_none());
    }

    #[test]
    fn focus_release_preserves_a_guest_queued_key_up() {
        let shared = SharedState::new("/tmp/unused-m16-input".into(), "A:\\test".into());
        let mut device = test_device();
        // The up already left Device.keys, so delivered no longer contains it.
        shared.push_key_transition(0x0d, false);
        release_input(&mut device, &shared);
        assert_eq!(
            device.keys.into_iter().collect::<Vec<_>>(),
            vec![(0x0d, false)]
        );
        assert_eq!(shared.input_events_pending(), 0);
    }
    #[test]
    fn saturated_status_queue_preserves_correlated_terminal_results() {
        let mut output = Output::default();
        for _ in 0..1024 {
            assert!(output.enqueue(Event::Status {
                message: "busy".into()
            }));
        }
        assert!(output.enqueue(Event::Result {
            request: 91,
            success: true,
            message: "done".into(),
            check: None
        }));
        assert!(
            output
                .events
                .iter()
                .any(|e| matches!(e, Event::Result { request: 91, .. }))
        );
        assert!(output.events.len() <= 256);
    }
    #[test]
    fn mandatory_overflow_fails_closed_instead_of_silently_losing_replies() {
        let mut output = Output::default();
        for request in 0..512 {
            assert!(output.enqueue(Event::Result {
                request,
                success: true,
                message: "done".into(),
                check: None
            }));
        }
        assert!(!output.enqueue(Event::Result {
            request: 513,
            success: true,
            message: "done".into(),
            check: None
        }));
        assert!(output.closed);
        assert!(matches!(output.events.back(), Some(Event::Closed { .. })));
    }
}
