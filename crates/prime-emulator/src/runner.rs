//! Emulator orchestration: firmware load, memory setup, CPU start, the
//! limit-bounded run loop, heartbeat detection, and the run report.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::display::{Frame, LcdView};
use crate::engine::{CpuThread, FaultReport, StopReason};
use crate::firmware::{ElfImage, OsRom, regions};
use crate::memory::GuestMemory;
use crate::script::{CaptureRecord, ScriptConfig, ScriptDriver, ScriptReport, Tick};
use crate::services::pe;
use crate::services::{Registry, SharedState, UnsupportedPolicy, handlers, names};
use crate::sha256::Sha256;
use crate::trace::{TraceConfig, TraceHub};

/// Startup stack for the main thread. Reference: `MEM_STACK_SIZE` (4 MiB),
/// allocated from the dynamic heap (PrimU2 `Thread::Thread`).
pub const MAIN_STACK_SIZE: usize = 0x40_0000;

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub firmware_path: PathBuf,
    /// Optional `PRIME_OS.ROM`; mapped inert (reference behavior) when given,
    /// mapped zeroed when not.
    pub os_rom_path: Option<PathBuf>,
    /// Host directory serving the guest drive letters.
    pub data_dir: PathBuf,
    /// Guest-visible path of the firmware image (drive-letter form).
    pub firmware_guest_path: String,
    /// Instructions per `emu_start` slice.
    pub slice_instructions: usize,
    /// Total instruction budget.
    pub max_instructions: u64,
    /// Wall-clock budget in seconds.
    pub max_seconds: f64,
    pub on_unsupported: UnsupportedPolicy,
    /// Minimum GetEvent-with-input returns before declaring a heartbeat.
    pub heartbeat_min_events: u32,
    /// Inject one key press every N ms (None = no injection).
    pub inject_key: Option<(u16, u64)>,
    /// Virtual IRQ tick interval in ms (large = effectively disabled).
    pub irq_tick_ms: u64,
    /// Guest write watchpoints (addr, size) for diagnostics.
    pub watches: Vec<(u32, u32)>,
    pub trace: TraceConfig,
    /// Scripted keys (M4): run until the script completes instead of
    /// stopping at the heartbeat.
    pub script: Option<ScriptConfig>,
    /// Save the final framebuffer here (PNG).
    pub screenshot: Option<PathBuf>,
    /// Count framebuffer stores and their guest PCs (slows execution).
    pub lcd_watch: bool,
    /// Snapshot the data directory before and after the run and report
    /// what the firmware created, changed or deleted.
    pub storage_diff: bool,
    /// Deliver the periodic UI timer events through GetEvent (reference
    /// cadence); the interactive UI needs them. `false` reproduces the M3
    /// heartbeat runs exactly (input-only GetEvent).
    pub ui_timers: bool,
    pub program: Option<crate::runprog::Plan>,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            firmware_path: PathBuf::new(),
            os_rom_path: None,
            data_dir: PathBuf::from("prime_data"),
            firmware_guest_path: "A:\\programs\\misc\\armfir.elf".to_owned(),
            slice_instructions: 200_000,
            max_instructions: 20_000_000,
            max_seconds: 30.0,
            on_unsupported: UnsupportedPolicy::Stop,
            heartbeat_min_events: 3,
            inject_key: None,
            irq_tick_ms: 10,
            watches: Vec::new(),
            trace: TraceConfig::default(),
            script: None,
            screenshot: None,
            lcd_watch: false,
            storage_diff: false,
            // Off by default (M3 behaviour); key scripts turn it on.
            ui_timers: false,
            program: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ThreadInfo {
    pub tid: u32,
    pub instructions: u64,
}

#[derive(Debug, Serialize)]
pub struct CallSiteInfo {
    pub id: u32,
    pub name: Option<String>,
    pub caller_pc: u32,
    pub calls: u64,
}

#[derive(Debug, Serialize)]
pub struct RunReport {
    pub firmware: String,
    pub firmware_sha256: Option<String>,
    pub entry: u32,
    pub segments: Vec<SegmentInfo>,
    pub initial_sp: u32,
    pub outcome: String,
    /// Human-readable evidence for the outcome; empty on config errors.
    pub evidence: String,
    pub instructions: u64,
    pub elapsed_secs: f64,
    pub last_pc: u32,
    pub svc_calls_total: u64,
    pub used_services: Vec<UsedService>,
    pub unsupported_hits: Vec<UnsupportedHit>,
    pub fault: Option<FaultJson>,
    pub heartbeat: Option<HeartbeatJson>,
    /// Per-guest-thread instruction counters (liveness evidence).
    pub threads: Vec<ThreadInfo>,
    /// Virtual IRQ deliveries per IRQ number.
    pub irq_delivered: Vec<(u32, u64)>,
    /// Top repeating SVC call sites (heartbeat / stall analysis).
    pub top_call_sites: Vec<CallSiteInfo>,
    /// Independent M4 outcomes (none implies another).
    pub outcomes: Outcomes,
    /// Framebuffer, captures and scripted-input evidence.
    pub display: DisplayReport,
    /// `--storage-diff`: files the run created, changed or deleted.
    pub storage: Option<crate::storage::StorageDiff>,
    /// `--trace-fs`: every guest file-service call (bounded).
    pub fs_events: Vec<serde_json::Value>,
    pub fs_events_dropped: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler: Option<crate::compiler::CompilerTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<crate::runtime::RuntimeTrace>,
}

/// Independent outcomes: `None` = not attempted in this run.
#[derive(Debug, Serialize)]
pub struct Outcomes {
    /// The firmware ran without a guest fault or strict-policy stop.
    pub firmware_execution: bool,
    /// The framebuffer was located and read.
    pub framebuffer_capture: Option<bool>,
    /// Every scripted key was delivered to the guest by GetEvent.
    pub input_delivery: Option<bool>,
    /// Every scripted key settled with a screen change outside the idle
    /// mask.
    pub input_to_display: Option<bool>,
    /// Set only by the M4-C verifier (comparing runs), never by a run.
    pub calculation: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
pub struct DisplayReport {
    pub view: Option<LcdView>,
    /// Why the framebuffer could not be read, if it could not.
    pub error: Option<String>,
    pub final_capture: Option<CaptureRecord>,
    pub script: Option<ScriptReport>,
    /// `--lcd-watch`: framebuffer stores and top writer PCs (whole run).
    pub fb_writes: Option<u64>,
    pub fb_writer_pcs: Vec<(u32, u64)>,
}

#[derive(Debug, Serialize)]
pub struct SegmentInfo {
    pub vaddr: u32,
    pub file_size: usize,
    pub mem_size: usize,
    pub flags: String,
}

#[derive(Debug, Serialize)]
pub struct UsedService {
    pub id: u32,
    pub name: Option<String>,
    pub status: String,
    pub calls: u64,
}

#[derive(Debug, Serialize)]
pub struct UnsupportedHit {
    pub id: u32,
    pub name: Option<String>,
    pub caller_pc: u32,
    pub calls: u64,
}

#[derive(Debug, Serialize)]
pub struct FaultJson {
    pub kind: String,
    pub pc: u32,
    pub regs: [u32; 16],
    pub cpsr: u32,
    pub last_pcs: Vec<u32>,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatJson {
    pub service: String,
    pub caller_pc: u32,
    /// Caller's first two saved stack words at GetEvent time
    /// (diagnostic; see GuestArgs::stack_slots).
    pub stack_slots: [u32; 2],
    /// Thread that services the event loop.
    pub thread_id: u32,
    pub events_delivered: u32,
    /// SVC invocations of the event service from the pump site.
    pub pump_reentries: u64,
    pub instructions: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    Heartbeat,
    LimitReached,
    UnsupportedStop,
    Fault,
    /// Every scripted key was delivered and settled.
    ScriptComplete,
    /// Runtime controller finished; its report carries the program outcome.
    ProgramComplete,
    /// A scripted key or the boot gate timed out.
    ScriptFailed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Heartbeat => "heartbeat",
            Outcome::LimitReached => "limit-reached",
            Outcome::UnsupportedStop => "unsupported-service-stop",
            Outcome::Fault => "fault",
            Outcome::ScriptComplete => "script-complete",
            Outcome::ProgramComplete => "program-session-complete",
            Outcome::ScriptFailed => "script-failed",
        }
    }
}

/// The device thread's view of the key script's end.
struct DeviceThread {
    handle: std::thread::JoinHandle<Option<(ScriptReport, Option<Frame>)>>,
    result: Arc<std::sync::Mutex<Option<Tick>>>,
}

/// Runs the device-side event producers until the session stops: UI timer
/// events (reference cadence), the `--inject-key` probe, and the key
/// script. They only use the shared input queues and read guest memory.
fn spawn_device_thread(
    cfg: &RunConfig,
    mem: Arc<GuestMemory>,
    shared: Arc<SharedState>,
    trace: Arc<TraceHub>,
    session: Option<Arc<crate::session::LiveSession>>,
) -> DeviceThread {
    let result = Arc::new(std::sync::Mutex::new(None));
    let out = result.clone();
    let ui_timers = cfg.ui_timers;
    let inject_key = cfg.inject_key;
    let mut driver = cfg
        .program
        .is_none()
        .then(|| cfg.script.clone().map(ScriptDriver::new))
        .flatten();
    let mut program = cfg.program.clone().map(crate::runprog::Driver::new);
    let handle = std::thread::Builder::new()
        .name("device".into())
        .spawn(move || {
            let (mut sys, mut dev, mut inject) = (Instant::now(), Instant::now(), Instant::now());
            let mut script_running = driver.is_some();
            while !shared.stopping() {
                if let Some(s) = &session {
                    s.tick(&mem, &shared, &trace);
                    s.capture(&mem, &shared);
                }
                if ui_timers {
                    // PrimU2's 60 Hz window timer raises SYS_TIMER every 11
                    // ticks and DEVICE_TIMER every 61.
                    if sys.elapsed() >= SYS_TIMER_PERIOD {
                        shared.push_system_event(crate::services::UI_EVENT_TYPE_SYS_TIMER);
                        sys = Instant::now();
                    }
                    if dev.elapsed() >= DEVICE_TIMER_PERIOD {
                        shared.push_system_event(crate::services::UI_EVENT_TYPE_DEVICE_TIMER);
                        dev = Instant::now();
                    }
                }
                if let Some((key, interval_ms)) = inject_key {
                    if inject.elapsed() >= Duration::from_millis(interval_ms) {
                        shared.push_key_press(key);
                        inject = Instant::now();
                    }
                }
                if let Some(p) = program.as_mut() {
                    let t = p.tick(&mem, &shared, &trace);
                    trace.set_script_step(p.current_step());
                    if t != Tick::Continue {
                        *out.lock().unwrap() = Some(t);
                        shared.set_stopping();
                        break;
                    }
                }
                if script_running {
                    if let Some(d) = driver.as_mut() {
                        let t = d.tick(&mem, &shared);
                        trace.set_script_step(d.current_step());
                        if t != Tick::Continue {
                            script_running = false;
                            *out.lock().unwrap() = Some(t);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let mut evidence = program
                .and_then(|p| p.check_evidence())
                .or_else(|| driver.map(|d| (d.report().clone(), d.last_frame().cloned())));
            // A Check checkpoint is useful for blocked navigation, but is
            // not the program's final runtime framebuffer.
            if trace
                .runtime
                .as_ref()
                .is_some_and(|observer| !observer.snapshot().invocations.is_empty())
            {
                if let Some((_, frame)) = &mut evidence {
                    *frame = None;
                }
            }
            evidence
        })
        .expect("spawn device thread");
    DeviceThread { handle, result }
}

/// 11 and 61 ticks of a 60 Hz timer (PrimU2 LCD.cpp WM_TIMER).
const SYS_TIMER_PERIOD: Duration = Duration::from_micros(11 * 1_000_000 / 60);
const DEVICE_TIMER_PERIOD: Duration = Duration::from_micros(61 * 1_000_000 / 60);

pub struct RunSummary {
    pub report: RunReport,
    pub outcome: Outcome,
}

fn program_completion_evidence(runtime: Option<crate::runtime::RuntimeTrace>) -> String {
    match runtime {
        Some(runtime) => format!(
            "program controller stopped: {:?}: {}",
            runtime.terminal,
            runtime
                .reason
                .as_deref()
                .unwrap_or("no terminal reason recorded")
        ),
        None => "program controller stopped without runtime evidence".into(),
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn stopped_program_reports_navigation_timeout_without_claiming_key_completion() {
        let runtime = crate::runtime::RuntimeTrace {
            terminal: Some(crate::runtime::ExecutionStatus::TimedOut),
            reason: Some("checking UI: key 1 not settled".into()),
            ..Default::default()
        };
        let evidence = program_completion_evidence(Some(runtime));
        assert!(evidence.contains("TimedOut"));
        assert!(evidence.contains("checking UI: key 1 not settled"));
        assert!(!evidence.contains("delivered and settled"));
    }
}

pub fn run(cfg: &RunConfig) -> Result<RunSummary, Box<dyn std::error::Error>> {
    run_with_session(cfg, None)
}

pub fn run_with_session(
    cfg: &RunConfig,
    session: Option<Arc<crate::session::LiveSession>>,
) -> Result<RunSummary, Box<dyn std::error::Error>> {
    let trace = Arc::new(TraceHub::new(cfg.trace.clone())?);
    // Before the firmware touches anything (`--storage-diff`).
    let storage_before = cfg
        .storage_diff
        .then(|| crate::storage::snapshot(&cfg.data_dir));

    // ── Stage 1: firmware load ────────────────────────────────────────
    trace.stage("load", format!("firmware {}", cfg.firmware_path.display()));
    let elf = ElfImage::load(&cfg.firmware_path)?;
    let hash = sha256_file(&cfg.firmware_path);
    if (cfg.trace.compiler || cfg.trace.runtime)
        && hash.as_deref() != Some(crate::compiler::FIRMWARE_SHA256)
    {
        return Err("unsupported firmware for compiler observation".into());
    }
    trace.stage(
        "load",
        format!(
            "ELF32 ARM ok: entry {:#010X}, {} PT_LOAD segment(s), {} bytes",
            elf.entry,
            elf.segments.len(),
            elf.file_size
        ),
    );

    // ── Stage 2: guest memory ────────────────────────────────────────
    let mut mem = GuestMemory::new();
    // Null/trap page (ref maps 0x0 with 16 bytes; page granularity here).
    mem.map("null", regions::NULL_PAGE, regions::NULL_PAGE_SIZE)?;
    // Dynamic heap, pre-mapped (ref: 0x20000000, 256 MiB).
    mem.map("heap", regions::HEAP, regions::HEAP_SIZE)?;
    mem.set_heap(regions::HEAP, regions::HEAP_SIZE);
    // Kernel ROM: inert data (ref maps PRIME_OS.ROM at 0x30000000 RWX and
    // never enters it).
    mem.map("os-rom", regions::OS_ROM, regions::OS_ROM_MAX)?;
    if let Some(rom_path) = &cfg.os_rom_path {
        match OsRom::load(rom_path) {
            Ok(rom) => {
                if rom.load_addr != regions::OS_ROM {
                    trace.info(format!(
                        "OS ROM header load address {:#010X} differs from reference map {:#010X}; using reference map",
                        rom.load_addr, regions::OS_ROM
                    ));
                }
                let n = rom.bytes.len().min(regions::OS_ROM_MAX);
                mem.write_bytes(regions::OS_ROM, &rom.bytes[..n])?;
                trace.stage(
                    "load",
                    format!("OS ROM mapped inert: {n} bytes, entry {:#010X}", rom.entry),
                );
            }
            Err(e) => {
                trace.info(format!("OS ROM ignored: {e}"));
            }
        }
    } else {
        trace.info("no OS ROM supplied; 0x30000000 mapped zeroed (inert, reference behavior)");
    }
    // Exception/vector page (ref maps 0x31FFF000).
    mem.map(
        "exception-page",
        regions::EXCEPTION_PAGE,
        regions::EXCEPTION_PAGE_SIZE,
    )?;
    // Firmware segments.
    let mut seg_infos = Vec::new();
    for (i, seg) in elf.segments.iter().enumerate() {
        let mut flags = String::new();
        if seg.readable {
            flags.push('r');
        }
        if seg.writable {
            flags.push('w');
        }
        if seg.executable {
            flags.push('x');
        }
        mem.map(mem::block_name(i), seg.vaddr, seg.mem_size)?;
        mem.write_bytes(seg.vaddr, elf.segment_bytes(seg))?;
        trace.map(&format!("elf-seg-{i}"), seg.vaddr, seg.mem_size);
        seg_infos.push(SegmentInfo {
            vaddr: seg.vaddr,
            file_size: seg.file_size,
            mem_size: seg.mem_size,
            flags,
        });
    }
    // Pre-map platform DLLs at their preferred image bases so fixed-base
    // WinCE images (no .reloc) load exactly where the firmware expects.
    {
        let sys_dir = cfg.data_dir.join("A/WINDOW/SYSTEM");
        let mut entries: Vec<_> = std::fs::read_dir(&sys_dir)
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        let probes: Vec<(String, pe::PeProbe)> = entries
            .iter()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("dll"))
            })
            .filter_map(|e| {
                let bytes = std::fs::read(e.path()).ok()?;
                let probe = pe::probe_headers(&bytes).ok()?;
                Some((e.file_name().to_string_lossy().to_uppercase(), probe))
            })
            .collect();
        for i in pe::premap_plan(&probes) {
            let (name, p) = &probes[i];
            let size = crate::memory::page_round(p.size_of_image as usize);
            if mem.covering_block(p.image_base, size).is_none()
                && mem.map("pe-prefixed", p.image_base, size).is_ok()
            {
                trace.map(&format!("pe-{name}"), p.image_base, size);
            }
        }
    }

    // LCD register block (ref maps 0x4C800000..0x4E000000).
    mem.map("lcd-regs", regions::LCD_REGS, regions::LCD_REGS_SIZE)?;
    // S3C2416 PWM timer block (ref maps 0x51000000, 0x44 bytes used).
    mem.map("pwm-timers", regions::PWM_TIMERS, regions::PWM_TIMERS_SIZE)?;
    // RTC block (ref maps 0x57000000, 0x100).
    mem.map("rtc", regions::RTC, regions::RTC_SIZE)?;

    let mem = Arc::new(mem);

    // ── Stage 3: services ────────────────────────────────────────────
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    let registry = Arc::new(registry);
    let shared = Arc::new(SharedState::new(
        cfg.data_dir.clone(),
        cfg.firmware_guest_path.clone(),
    ));
    // Must be set before any engine is built (engines install the hook).
    shared
        .lcd_watch
        .enabled
        .store(cfg.lcd_watch, std::sync::atomic::Ordering::Relaxed);

    // Main thread stack: heap-allocated like the reference (top of block).
    let stack_va = mem.heap_alloc(MAIN_STACK_SIZE)?;
    let sp = stack_va + MAIN_STACK_SIZE as u32;
    trace.stage(
        "cpu",
        format!(
            "main thread: entry {:#010X}, sp {:#010X} (stack at {:#010X})",
            elf.entry, sp, stack_va
        ),
    );

    // Virtual IRQ delivery: pending ticks are delivered synchronously
    // between main-thread slices by executing the registered guest ISR on
    // the main engine with full register save/restore. This matches real
    // hardware far better than a concurrent ISR engine, whose writes raced
    // the running main thread (observed as torn callback pointers during
    // M2 diagnosis).
    let irq_last = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now()));

    // Guest-thread spawner: a new native thread + own Unicorn engine over
    // the shared host-backed blocks (reference model, PrimU2 Thread.cpp).
    {
        let mem_s = mem.clone();
        let reg_s = registry.clone();
        let trace_s = trace.clone();
        let shared_s = shared.clone();
        let policy = cfg.on_unsupported;
        // Guest threads park inside blocking services (GetEvent waits for
        // input); a slice only completes after this many guest instructions,
        // so use a fine granularity for spawned threads or their progress
        // is never reported. 10k keeps per-thread counters current.
        let slice = cfg.slice_instructions.min(10_000);
        let spawner: crate::services::ThreadSpawner =
            std::sync::Arc::new(move |tid: u32, func: u32, arg: u32, stack_size: usize| {
                let mem = mem_s.clone();
                let registry = reg_s.clone();
                let trace = trace_s.clone();
                let shared = shared_s.clone();
                let handle = std::thread::spawn(move || {
                    let stack_va = mem.heap_alloc(stack_size).unwrap_or(0);
                    if stack_va == 0 {
                        trace.thread_event(format!("thread {tid}: stack allocation failed"));
                        return;
                    }
                    let sp = stack_va + stack_size as u32;
                    let Ok(mut cpu) = CpuThread::new(
                        mem.clone(),
                        registry.clone(),
                        shared.clone(),
                        trace.clone(),
                        policy,
                        tid,
                        Vec::new(),
                    ) else {
                        trace.thread_event(format!("thread {tid}: engine creation failed"));
                        return;
                    };
                    // Function pointer bit 0 selects Thumb entry (AAPCS
                    // interworking convention).
                    let _ = cpu.set_entry(func, sp, arg);
                    trace.thread_event(format!(
                        "thread {tid} running: pc {func:#010X} sp {sp:#010X}"
                    ));
                    while !shared.stopping() && !shared.thread_is_exiting() {
                        match cpu.run_slice(slice) {
                            Ok(crate::engine::RunSliceOutcome::Progress) => {
                                shared.note_thread_instructions(tid, cpu.instructions);
                            }
                            Ok(crate::engine::RunSliceOutcome::Stopped(reason)) => {
                                trace.thread_event(format!(
                                    "thread {tid} stopped: {reason:?} at pc {:#010X}",
                                    cpu.pc()
                                ));
                                break;
                            }
                            Err(e) => {
                                trace.thread_event(format!("thread {tid} engine error: {e}"));
                                break;
                            }
                        }
                    }
                    shared.note_thread_instructions(tid, cpu.instructions);
                    if trace.runtime.as_ref().is_some_and(|r| {
                        r.terminal_status()
                            == Some(crate::runtime::ExecutionStatus::GuestExecutionFault)
                    }) {
                        shared.set_stopping();
                    }
                    trace.thread_event(format!(
                        "thread {tid} finished: {} instructions, pc {:#010X}",
                        cpu.instructions,
                        cpu.pc()
                    ));
                });
                shared_s.register_thread_handle(tid, handle);
            });
        shared.set_thread_spawner(spawner);
    }

    // ── Stage 4: execution ───────────────────────────────────────────
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry.clone(),
        shared.clone(),
        trace.clone(),
        cfg.on_unsupported,
        0,
        cfg.watches.clone(),
    )?;
    cpu.set_entry(elf.entry, sp, 0)?;
    trace.stage("cpu", "starting execution (ARM state at ELF entry)");

    let start = Instant::now();
    let mut heartbeat_events: Option<HeartbeatJson> = None;
    let outcome: Outcome;
    let evidence: String;
    let mut fault_json: Option<FaultJson> = None;
    // Device-side producers (UI timers, key injection, key script) run on
    // their own host thread: the main guest thread blocks in long
    // OSWaitForEvent calls that only the UI pump ends, and the pump only
    // runs when GetEvent has something to deliver.
    let device = spawn_device_thread(
        cfg,
        mem.clone(),
        shared.clone(),
        trace.clone(),
        session.clone(),
    );

    let get_event_id = names::service_id_by_name("GetEvent").unwrap();

    loop {
        // Deliver virtual IRQ ticks between slices (deterministic order).
        {
            let mut last = irq_last.lock().unwrap();
            if last.elapsed() >= Duration::from_millis(cfg.irq_tick_ms) {
                *last = Instant::now();
                let due = shared.take_due_isrs();
                for (irq, isr_va) in due {
                    if shared.stopping() {
                        break;
                    }
                    let _ = cpu.run_isr(isr_va, irq);
                    shared.note_irq_delivered(irq);
                }
            }
        }
        match cpu.run_slice(cfg.slice_instructions)? {
            crate::engine::RunSliceOutcome::Progress => {}
            crate::engine::RunSliceOutcome::Stopped(reason) => {
                match reason {
                    StopReason::UnsupportedService { id, caller_pc } => {
                        outcome = Outcome::UnsupportedStop;
                        evidence = format!(
                            "strict policy stopped at unimplemented service {id:#07X} ({}) called from {caller_pc:#010X}",
                            registry.name(id).unwrap_or("unknown"),
                        );
                    }
                    StopReason::Fault => {
                        outcome = Outcome::Fault;
                        if let Some(f) = cpu.fault_report() {
                            evidence = format!("{} at pc={:#010X}", f.kind, f.pc);
                            fault_json = Some(f.to_json());
                        } else {
                            evidence = "guest fault (no report)".into();
                        }
                    }
                    StopReason::Requested if cfg.program.is_some() => {
                        outcome = Outcome::ProgramComplete;
                        evidence = "program controller requested shutdown".into();
                    }
                    other => {
                        evidence = format!("stopped: {other:?}");
                        outcome = Outcome::LimitReached;
                    }
                }
                break;
            }
        }

        if let Some(s) = &session {
            s.capture(&mem, &shared);
        }

        // Heartbeat probe: count GetEvent calls that returned with a real
        // batch. The handler bumps the total; we require the guest to have
        // re-entered it from the same loop, which call-site counters show.
        let delivered = shared.input_delivered();
        if delivered >= cfg.heartbeat_min_events {
            let sites = trace.call_sites();
            let getev = sites
                .iter()
                .filter(|((id, _), _)| *id == get_event_id)
                .max_by_key(|(_, c)| *c);
            let chain = shared.getevent_chain();
            if let Some(((_, caller_pc), calls)) = getev {
                if *calls >= cfg.heartbeat_min_events as u64 && heartbeat_events.is_none() {
                    trace.heartbeat(format!(
                        "firmware event loop verified: GetEvent serviced {delivered} injected key events, re-entered from caller {caller_pc:#010X} {calls} times, no faults"
                    ));
                    heartbeat_events = Some(HeartbeatJson {
                        service: "GetEvent".into(),
                        caller_pc: *caller_pc,
                        stack_slots: [chain[1], chain[2]],
                        thread_id: shared.input_thread(),
                        events_delivered: delivered,
                        pump_reentries: *calls,
                        instructions: cpu.instructions,
                    });
                    if cfg.script.is_none() && cfg.program.is_none() && session.is_none() {
                        outcome = Outcome::Heartbeat;
                        evidence = format!(
                            "firmware event loop verified: GetEvent serviced {delivered} injected key events, re-entered from caller {caller_pc:#010X} {calls} times, no faults"
                        );
                        break;
                    }
                }
            }
        }

        // Scripted input (M4) finished on the device thread?
        let finished = device.result.lock().unwrap().take();
        match finished {
            None | Some(Tick::Continue) => {}
            Some(Tick::Done) => {
                outcome = if cfg.program.is_some() {
                    Outcome::ProgramComplete
                } else {
                    Outcome::ScriptComplete
                };
                evidence = if cfg.program.is_some() {
                    program_completion_evidence(trace.runtime.as_ref().map(|o| o.snapshot()))
                } else {
                    format!(
                        "key script completed: {} item(s) delivered and settled",
                        cfg.script.as_ref().map_or(0, |s| s.keys.len())
                    )
                };
                break;
            }
            Some(Tick::Failed(m)) => {
                outcome = Outcome::ScriptFailed;
                evidence = format!("key script failed: {m}");
                break;
            }
        }

        if shared.thread_is_exiting() {
            outcome = Outcome::LimitReached;
            evidence = "main guest thread called OSExitThread".to_owned();
            break;
        }
        if cpu.instructions >= cfg.max_instructions {
            outcome = Outcome::LimitReached;
            evidence = format!("instruction limit reached ({})", cfg.max_instructions);
            break;
        }
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= cfg.max_seconds {
            outcome = Outcome::LimitReached;
            evidence = format!("time limit reached ({:.1}s)", cfg.max_seconds);
            break;
        }
    }

    shared.set_stopping();
    let _ = cpu.take_stop_reason();
    let script = device.handle.join().unwrap_or(None);
    let evidence = if cfg.program.is_some() && outcome == Outcome::ProgramComplete {
        program_completion_evidence(trace.runtime.as_ref().map(|o| o.snapshot()))
    } else {
        evidence
    };

    // ── Report ────────────────────────────────────────────────────────
    let svc_calls_total: u64 = trace.svc_totals().iter().map(|(_, c)| *c).sum();
    let implemented: std::collections::HashSet<u32> = registry
        .entries()
        .iter()
        .filter(|e| e.handler.is_some())
        .map(|e| e.id)
        .collect();
    let used_services = registry
        .used_services()
        .into_iter()
        .map(|(id, name, status, calls)| UsedService {
            id,
            name: name.map(str::to_owned),
            status: status.to_owned(),
            calls,
        })
        .collect();
    let unsupported_hits = trace
        .call_sites()
        .into_iter()
        .filter(|((id, _), _)| !implemented.contains(id))
        .map(|((id, caller_pc), calls)| UnsupportedHit {
            id,
            name: names::service_name(id).map(str::to_owned),
            caller_pc,
            calls,
        })
        .collect();

    trace.stage("done", format!("outcome: {}", outcome.as_str()));
    let threads = {
        let mut t: Vec<ThreadInfo> = shared
            .thread_instructions
            .lock()
            .unwrap()
            .iter()
            .map(|(&tid, &n)| ThreadInfo {
                tid,
                instructions: n,
            })
            .collect();
        t.push(ThreadInfo {
            tid: 0,
            instructions: cpu.instructions,
        });
        t.sort_by_key(|t| t.tid);
        t
    };
    let top_call_sites = trace
        .call_sites()
        .into_iter()
        .take(20)
        .map(|((id, caller_pc), calls)| CallSiteInfo {
            id,
            name: names::service_name(id).map(str::to_owned),
            caller_pc,
            calls,
        })
        .collect();
    let display = display_report(cfg, &mem, &shared, script.as_ref());
    let (fs_events, fs_events_dropped) = trace.fs_log();
    let script_report = display.script.as_ref();
    let outcomes = Outcomes {
        firmware_execution: !matches!(outcome, Outcome::Fault | Outcome::UnsupportedStop)
            && cpu.instructions > 0,
        framebuffer_capture: (cfg.script.is_some() || cfg.screenshot.is_some())
            .then(|| display.final_capture.is_some()),
        input_delivery: script_report.map(|s| {
            s.keys.len() == cfg.script.as_ref().map_or(0, |c| c.keys.len())
                && s.keys.iter().all(|k| k.delivered_ms.is_some())
        }),
        // Every real input changed the screen lastingly; IDLE controls must
        // not have.
        input_to_display: script_report.map(|s| {
            s.keys.len() == cfg.script.as_ref().map_or(0, |c| c.keys.len())
                && s.keys.iter().all(|k| {
                    let idle = k.code == 0 && k.name == "IDLE";
                    (k.stable_change.changed > 0) != idle
                })
        }),
        calculation: None,
    };
    let report = RunReport {
        firmware: cfg.firmware_path.display().to_string(),
        firmware_sha256: hash,
        entry: elf.entry,
        segments: seg_infos,
        initial_sp: sp,
        outcome: outcome.as_str().to_owned(),
        evidence: evidence.clone(),
        instructions: cpu.instructions,
        elapsed_secs: start.elapsed().as_secs_f64(),
        last_pc: cpu.pc(),
        svc_calls_total,
        used_services,
        unsupported_hits,
        fault: fault_json,
        heartbeat: heartbeat_events,
        threads,
        irq_delivered: shared.irq_delivery_counts(),
        top_call_sites,
        outcomes,
        display,
        storage: storage_before
            .as_ref()
            .map(|b| crate::storage::diff(b, &crate::storage::snapshot(&cfg.data_dir))),
        fs_events,
        fs_events_dropped,
        compiler: trace.compiler.as_ref().map(|o| o.snapshot()),
        runtime: trace.runtime.as_ref().map(|o| o.snapshot()),
    };
    Ok(RunSummary { report, outcome })
}

/// Final framebuffer capture (and screenshot), plus script evidence.
fn display_report(
    cfg: &RunConfig,
    mem: &GuestMemory,
    shared: &SharedState,
    script: Option<&(ScriptReport, Option<Frame>)>,
) -> DisplayReport {
    let mut out = DisplayReport {
        script: script.map(|(report, _)| report.clone()),
        ..DisplayReport::default()
    };
    if cfg.lcd_watch {
        out.fb_writes = Some(shared.lcd_watch.writes());
        out.fb_writer_pcs = shared.lcd_watch.top_pcs(16);
    }
    if cfg.script.is_none() && cfg.screenshot.is_none() {
        return out;
    }
    let lcd = *shared.lcd_object.lock().unwrap();
    let view = match LcdView::locate(mem, lcd) {
        Ok(v) => v,
        Err(e) => {
            out.error = Some(e.to_string());
            return out;
        }
    };
    out.view = Some(view);
    // Prefer the script driver's last settled frame: it is the last guest
    // state the settlement gate verified. A fresh read here can catch the
    // firmware's idle history repaint mid-flight (M4 observed a partially
    // clipped entry when the run stopped between the settled capture and
    // this read).
    let frame = match script.and_then(|(_, frame)| frame.as_ref()) {
        Some(f) => f.clone(),
        None => match view.capture(mem) {
            Ok(f) => f,
            Err(e) => {
                out.error = Some(e.to_string());
                return out;
            }
        },
    };
    let png = cfg
        .screenshot
        .as_ref()
        .and_then(|p| match frame.save_png(p) {
            Ok(()) => Some(p.display().to_string()),
            Err(e) => {
                out.error = Some(format!("screenshot not written: {e}"));
                None
            }
        });
    out.final_capture = Some(CaptureRecord {
        checkpoint: "final".into(),
        elapsed_ms: 0,
        sha256: frame.sha256(),
        unique_colors: frame.unique_colors(),
        changed_vs_previous: script
            .and_then(|(_, frame)| frame.as_ref())
            .map(|prev| frame.diff(prev, None)),
        png,
    });
    out
}

mod mem {
    pub fn block_name(i: usize) -> &'static str {
        match i {
            0 => "elf-seg-0",
            1 => "elf-seg-1",
            2 => "elf-seg-2",
            _ => "elf-seg",
        }
    }
}

impl FaultReport {
    pub fn to_json(&self) -> FaultJson {
        FaultJson {
            kind: self.kind.clone(),
            pc: self.pc,
            regs: self.regs,
            cpsr: self.cpsr,
            last_pcs: self.last_pcs.clone(),
        }
    }
}

fn sha256_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize_hex())
}
