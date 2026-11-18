//! One guest CPU thread: a Unicorn ARM engine over the shared guest memory,
//! with the SVC dispatcher and fault hooks attached.
//!
//! The run loop advances in bounded instruction slices (`uc_emu_start` with
//! an instruction count), so limits, heartbeat checks and stall detection
//! all run between slices and an unbounded `emu_start` can never wedge the
//! process.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use unicorn_engine::unicorn_const::{Arch, Mode, Prot};
use unicorn_engine::{ArmCpuModel, RegisterARM, Unicorn, uc_error};

use crate::memory::GuestMemory;
use crate::services::{self, Registry, SharedState, UnsupportedPolicy};
use crate::trace::TraceHub;

/// The firmware's spin/yield idiom, treated as a no-op that sets Z and
/// advances (ref: PrimU2 `Thread::ThreadProc`, word 0xEE17FF7E — an
/// `MRC p15, 0, r15, c7, c14, 3` that Unicorn rejects as invalid).
pub const FIRMWARE_YIELD_WORD: u32 = 0xEE17_FF7E;

#[derive(Debug, Clone)]
pub struct FaultReport {
    pub kind: String,
    pub pc: u32,
    pub addr: Option<u64>,
    pub regs: [u32; 16],
    pub cpsr: u32,
    pub thread_id: u32,
    /// Last executed PCs when instruction tracing was enabled.
    pub last_pcs: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Instruction budget exhausted.
    InstructionLimit,
    /// Wall-clock budget exhausted.
    TimeLimit,
    /// Strict unsupported-service policy fired.
    UnsupportedService { id: u32, caller_pc: u32 },
    /// Guest fault (memory, invalid instruction, other Unicorn error).
    Fault,
    /// Runner requested shutdown (e.g. heartbeat already proven).
    Requested,
}

/// Shared between the engine hooks and the run loop.
struct HookState {
    mem: Arc<GuestMemory>,
    registry: Arc<Registry>,
    shared: Arc<SharedState>,
    trace: Arc<TraceHub>,
    policy: UnsupportedPolicy,
    thread_id: u32,
    /// Guest write watchpoints (addr, size).
    watches: Vec<(u32, u32)>,
    pending: Mutex<Option<services::pending::PendingService>>,
    stop: Mutex<Option<StopReason>>,
    /// Set by the mem-invalid hook with fault details.
    fault: Mutex<Option<FaultReport>>,
    /// Active ISR sentinel (0 = none): fetch faults there are normal ISR
    /// returns, not guest faults.
    isr_sentinel: std::sync::atomic::AtomicU32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionImage {
    pub thread_id: u32,
    pub cpu: crate::cpu_state::ArmCpuImage,
    pub pending: Option<services::pending::ServiceImage>,
    pub instructions: u64,
    pub counted_exact: bool,
    pub sentinel_pc: Option<u32>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerImage {
    pub epoch: u64,
    pub image: Option<ExecutionImage>,
    pub error: Option<String>,
}

pub struct CpuThread {
    // Field order matters: the engine must drop before the Arcs that own
    // the host memory it maps. Rust drops fields in declaration order.
    engine: Unicorn<'static, ()>,
    _mem: Arc<GuestMemory>,
    _registry: Arc<Registry>,
    _shared: Arc<SharedState>,
    trace: Arc<TraceHub>,
    hooks: Arc<HookState>,
    trace_handle: Option<std::sync::Arc<TraceHub>>,
    pub thread_id: u32,
    pub instructions: u64,
    /// True when the last slice's instruction count is exact.
    pub counted_exact: bool,
    /// Optional sentinel LR: a fetch fault at this address is a normal
    /// return (used by the virtual IRQ dispatcher), not a guest fault.
    pub sentinel_pc: Option<u32>,
    stopped: Arc<AtomicBool>,
    /// The `--lcd-watch` framebuffer hook is installed on this engine.
    lcd_hooked: bool,
    _owner: crate::safepoint::OwnerLease,
}

impl CpuThread {
    /// Build a guest thread whose memory is the shared host-backed blocks.
    ///
    /// SAFETY (uc_mem_map_ptr): the host pointers belong to mmap blocks
    /// owned by the `Arc<GuestMemory>` kept in this struct (field order
    /// guarantees it outlives `engine`), and all mapped sizes equal the
    /// block sizes, so no guest access can escape the allocation.
    pub fn new(
        mem: Arc<GuestMemory>,
        registry: Arc<Registry>,
        shared: Arc<SharedState>,
        trace: Arc<TraceHub>,
        policy: UnsupportedPolicy,
        thread_id: u32,
        watches: Vec<(u32, u32)>,
    ) -> Result<CpuThread, uc_error> {
        let owner = shared.safe_points.register(thread_id);
        Self::new_owned(mem, registry, shared, trace, policy, owner, watches)
    }
    /// Production workers register before stack allocation. A coordinator can
    /// therefore either save a queued launch or wait for its complete first
    /// CPU boundary, without an untracked memory-writing construction window.
    pub(crate) fn new_owned(
        mem: Arc<GuestMemory>,
        registry: Arc<Registry>,
        shared: Arc<SharedState>,
        trace: Arc<TraceHub>,
        policy: UnsupportedPolicy,
        owner: crate::safepoint::OwnerLease,
        watches: Vec<(u32, u32)>,
    ) -> Result<CpuThread, uc_error> {
        if !owner.coordinates(&shared.safe_points) {
            return Err(uc_error::ARG);
        }
        let thread_id = owner.thread_id();
        if shared.scheduler.register(thread_id).is_none() {
            return Err(uc_error::RESOURCE);
        }
        let mut engine = Unicorn::new(Arch::ARM, Mode::ARM)?;
        // Unicorn defaults to Cortex-A15. G1's S3C2416 is ARM926EJ-S;
        // select its instruction/exception behavior before initializing CPU state.
        engine.ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)?;
        for b in mem.blocks() {
            // SAFETY: see doc comment above.
            unsafe {
                engine.mem_map_ptr(
                    b.vaddr as u64,
                    b.size as u64,
                    Prot::ALL,
                    b.host_ptr() as *mut _,
                )?;
            }
        }

        let hooks = Arc::new(HookState {
            mem: mem.clone(),
            registry: registry.clone(),
            shared: shared.clone(),
            trace: trace.clone(),
            policy,
            thread_id,
            watches,
            pending: Mutex::new(None),
            stop: Mutex::new(None),
            fault: Mutex::new(None),
            isr_sentinel: std::sync::atomic::AtomicU32::new(0),
        });

        // Optional per-instruction trace hook (diagnostics; heavy).
        let trace_handle = if trace.config().code_trace {
            {
                let t = trace.clone();
                engine.add_code_hook(
                    1,
                    0,
                    Box::new(move |_uc: &mut Unicorn<'_, ()>, addr: u64, _size: u32| {
                        t.note_code(addr as u32);
                    }),
                )?;
            }
            Some(trace.clone())
        } else {
            None
        };

        // Diagnostic register probes (`PRIME_PROBE_PCS=0xADDR,...`): log the
        // argument registers, LR and SP each time an address executes. For
        // reverse-engineering call contracts; never installed otherwise.
        if let Ok(list) = std::env::var("PRIME_PROBE_PCS") {
            for pc in list
                .split(',')
                .filter_map(|a| u32::from_str_radix(a.trim().trim_start_matches("0x"), 16).ok())
            {
                let t = trace.clone();
                engine.add_code_hook(
                    pc as u64,
                    pc as u64,
                    Box::new(move |uc: &mut Unicorn<'_, ()>, _addr: u64, _size: u32| {
                        let r = |x| uc.reg_read(x).unwrap_or(0) as u32;
                        t.probe(
                            pc,
                            thread_id,
                            [
                                r(RegisterARM::R0),
                                r(RegisterARM::R1),
                                r(RegisterARM::R2),
                                r(RegisterARM::R3),
                                r(RegisterARM::R4),
                                r(RegisterARM::LR),
                                r(RegisterARM::SP),
                            ],
                        );
                    }),
                )?;
            }
        }

        crate::clipboard::install(&mut engine, &mem, &shared.clipboard)?;
        crate::compiler::install(&mut engine, &mem, &trace, thread_id)?;
        crate::runtime::install(&mut engine, &mem, &shared, &trace, thread_id)?;

        // SVC dispatch hook.
        {
            let hs = hooks.clone();
            engine.add_intr_hook(Box::new(move |uc: &mut Unicorn<'_, ()>, _intno: u32| {
                let hs = &*hs;
                publish_thread(uc, &hs.trace, hs.thread_id, "svc_entry", None, false);
                match services::dispatch(
                    uc,
                    &hs.mem,
                    &hs.registry,
                    &hs.shared,
                    &hs.trace,
                    hs.policy,
                    hs.thread_id,
                ) {
                    Ok(services::DispatchOutcome::Deferred(pending)) => {
                        if hs.isr_sentinel.load(Ordering::Relaxed) != 0 {
                            let report = fault_report(
                                uc,
                                "blocking service in synchronous ISR",
                                hs.thread_id,
                            );
                            hs.trace.fault(report.kind.clone(), report.pc);
                            *hs.fault.lock().unwrap() = Some(report);
                            *hs.stop.lock().unwrap() = Some(StopReason::Fault);
                        } else {
                            hs.shared
                                .pending_services
                                .lock()
                                .unwrap()
                                .insert(hs.thread_id, pending.image());
                            *hs.pending.lock().unwrap() = Some(pending);
                        }
                        let _ = uc.emu_stop();
                    }
                    Ok(services::DispatchOutcome::Handled) => {
                        if hs.shared.stopping() {
                            *hs.stop.lock().unwrap() = Some(StopReason::Requested);
                            let _ = uc.emu_stop();
                        }
                    }
                    Ok(services::DispatchOutcome::Unsupported { id, caller_pc }) => {
                        // dispatch already recorded this invocation once.
                        if hs.policy == UnsupportedPolicy::Stop {
                            *hs.stop.lock().unwrap() =
                                Some(StopReason::UnsupportedService { id, caller_pc });
                            let _ = uc.emu_stop();
                        }
                    }
                    Err(e) => {
                        let mut report =
                            fault_report(uc, &format!("svc-dispatch: {e}"), hs.thread_id);
                        report.addr = Some(u64::from(e.va));
                        hs.trace
                            .fault(format!("SVC dispatch failed: {e}"), report.pc);
                        if let Some(runtime) = &hs.trace.runtime {
                            runtime.fault(&report);
                        }
                        *hs.stop.lock().unwrap() = Some(StopReason::Fault);
                        *hs.fault.lock().unwrap() = Some(report);
                        let _ = uc.emu_stop();
                    }
                }
            }))?;
        }

        // Guest write watchpoints (diagnostics; bounded logging).
        if !hooks.watches.is_empty() {
            let hs = hooks.clone();
            // Let Unicorn exclude unrelated writes before entering Rust. The
            // inner check preserves sparse and overlapping watch semantics.
            let start = hooks.watches.iter().map(|&(a, _)| a).min().unwrap();
            let end = hooks
                .watches
                .iter()
                .map(|&(a, n)| a.saturating_add(n.saturating_sub(1)))
                .max()
                .unwrap();
            engine.add_mem_hook(
                unicorn_engine::HookType::MEM_WRITE,
                u64::from(start),
                u64::from(end),
                Box::new(
                    move |uc: &mut Unicorn<'_, ()>,
                          _typ: unicorn_engine::MemType,
                          address: u64,
                          size: usize,
                          value: i64|
                          -> bool {
                        let hs = &*hs;
                        let addr = address as u32;
                        for &(wa, wsz) in &hs.watches {
                            if addr >= wa && addr < wa.wrapping_add(wsz) {
                                let pc = uc.reg_read(RegisterARM::PC).unwrap_or(0) as u32;
                                let lr = uc.reg_read(RegisterARM::LR).unwrap_or(0) as u32;
                                hs.trace.watch_hit(addr, size, value as u32, pc, lr);
                                break;
                            }
                        }
                        true // always let the write through
                    },
                ),
            )?;
        }

        // Memory-fault hook: record and stop.
        {
            let hs = hooks.clone();
            engine.add_mem_hook(
                unicorn_engine::HookType::MEM_INVALID,
                1,
                0,
                Box::new(
                    move |uc: &mut Unicorn<'_, ()>,
                          mem_type: unicorn_engine::MemType,
                          address: u64,
                          size: usize,
                          value: i64| -> bool {
                        let hs = &*hs;
                        let sentinel =
                            hs.isr_sentinel.load(Ordering::Relaxed) as u64;
                        if sentinel != 0 && address == sentinel {
                            return false; // normal ISR return; not a fault
                        }
                        let mut report = fault_report(uc, &format!("{mem_type:?}"), thread_id);
                        report.addr = Some(address);
                        if let Some(runtime) = &hs.trace.runtime { runtime.fault(&report); }
                        hs.trace.fault(
                            format!(
                                "invalid guest {mem_type:?} at {address:#010X} (size {size}, value {value:#X})"
                            ),
                            report.pc,
                        );
                        *hs.fault.lock().unwrap() = Some(report);
                        *hs.stop.lock().unwrap() = Some(StopReason::Fault);
                        let _ = uc.emu_stop();
                        false // do not service the access; propagate the fault
                    },
                ),
            )?;
        }

        Ok(CpuThread {
            engine,
            _mem: mem,
            _registry: registry,
            _shared: shared,
            trace,
            hooks,
            trace_handle,
            thread_id,
            instructions: 0,
            counted_exact: true,
            sentinel_pc: None,
            stopped: Arc::new(AtomicBool::new(false)),
            lcd_hooked: false,
            _owner: owner,
        })
    }

    pub fn set_entry(&mut self, pc: u32, sp: u32, arg: u32) -> Result<(), uc_error> {
        self.engine.reg_write(RegisterARM::PC, pc as u64)?;
        self.engine.reg_write(RegisterARM::SP, sp as u64)?;
        self.engine.reg_write(RegisterARM::R0, arg as u64)?;
        Ok(())
    }

    pub fn set_reg(&mut self, r: RegisterARM, value: u32) -> Result<(), uc_error> {
        self.engine.reg_write(r, value as u64)
    }

    pub fn set_lr(&mut self, lr: u32) -> Result<(), uc_error> {
        self.engine.reg_write(RegisterARM::LR, lr as u64)?;
        Ok(())
    }

    pub fn pc(&self) -> u32 {
        self.reg(RegisterARM::PC)
    }

    /// The address to resume at: Unicorn's ARM `emu_start(begin)` selects
    /// the instruction set from bit 0 of `begin`, so Thumb code must be
    /// resumed at `pc | 1` (CPSR.T), or it is decoded as ARM.
    fn resume_addr(&self) -> u64 {
        let thumb = self.reg(RegisterARM::CPSR) & (1 << 5) != 0;
        (self.pc() | u32::from(thumb)) as u64
    }

    pub fn sp(&self) -> u32 {
        self.reg(RegisterARM::SP)
    }

    pub fn reg(&self, r: RegisterARM) -> u32 {
        self.engine.reg_read(r).unwrap_or(0) as u32
    }

    pub fn take_stop_reason(&self) -> Option<StopReason> {
        self.hooks.stop.lock().unwrap().take()
    }

    pub fn request_stop(&mut self, reason: StopReason) {
        *self.hooks.stop.lock().unwrap() = Some(reason);
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.engine.emu_stop();
    }

    pub fn stop_requested(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    pub fn fault_report(&self) -> Option<FaultReport> {
        self.hooks.fault.lock().unwrap().take()
    }

    /// Drain accepted interrupt edges on the main owner. The front stays in
    /// shared, serializable state through run_isr's initial pause boundary.
    /// Completion and dequeue happen before the next owner acknowledgement.
    pub fn dispatch_pending_isrs(&mut self) {
        while !self._shared.stopping() {
            let Some((irq, handler)) = self._shared.pending_irqs.lock().unwrap().front().copied()
            else {
                break;
            };
            let result = self.run_isr(handler, irq);
            // A busy cooperative scheduler has not entered the ISR yet.
            if result == Err(uc_error::RESOURCE) {
                break;
            }
            let edge = self._shared.pending_irqs.lock().unwrap().pop_front();
            assert_eq!(
                edge,
                Some((irq, handler)),
                "IRQ dispatch requires one main owner"
            );
            if result.is_ok() {
                self._shared.note_irq_delivered(irq);
            }
        }
    }

    /// Execute one guest ISR synchronously on this engine with full
    /// register save/restore (deterministic IRQ delivery between slices —
    /// real hardware preempts one context; concurrent engines would race).
    /// The ISR returns via the unmapped sentinel LR.
    pub fn run_isr(&mut self, isr_va: u32, irq: u32) -> Result<(), uc_error> {
        self.owner_boundary();
        let Some(_scheduled) = self._shared.scheduler.try_enter(self.thread_id, true) else {
            return Err(uc_error::RESOURCE);
        };
        self._shared.safe_points.executing(self.thread_id);
        let memory = self._mem.clone();
        let _active = memory.snapshot_gate.enter();
        const SENTINEL: u32 = 0x9000_0000;
        const SAVED: [RegisterARM; 17] = [
            RegisterARM::R0,
            RegisterARM::R1,
            RegisterARM::R2,
            RegisterARM::R3,
            RegisterARM::R4,
            RegisterARM::R5,
            RegisterARM::R6,
            RegisterARM::R7,
            RegisterARM::R8,
            RegisterARM::R9,
            RegisterARM::R10,
            RegisterARM::R11,
            RegisterARM::R12,
            RegisterARM::SP,
            RegisterARM::LR,
            RegisterARM::PC,
            RegisterARM::CPSR,
        ];
        let saved: Vec<u64> = SAVED
            .iter()
            .map(|r| self.engine.reg_read(*r).unwrap_or(0))
            .collect();
        self.engine.reg_write(RegisterARM::R0, irq as u64)?;
        self.engine.reg_write(RegisterARM::LR, SENTINEL as u64)?;
        self.engine.reg_write(RegisterARM::PC, isr_va as u64)?;
        self.hooks.isr_sentinel.store(SENTINEL, Ordering::Relaxed);
        let mut guard = 0;
        let mut failure = None;
        loop {
            if self._shared.stopping() {
                failure = Some(uc_error::EXCEPTION);
                break;
            }
            let res = self.engine.emu_start(self.resume_addr(), 0, 0, 100_000);
            match res {
                Ok(()) => {
                    if self.hooks.stop.lock().unwrap().is_some() {
                        failure = Some(uc_error::EXCEPTION);
                        break;
                    }
                    self.instructions += 100_000;
                    if self.pc() == SENTINEL {
                        break;
                    }
                    guard += 1;
                    if guard > 100 {
                        self.trace.thread_event(format!(
                            "ISR {isr_va:#010X} (irq {irq}) did not return (pc {:#010X})",
                            self.pc()
                        ));
                        *self.hooks.stop.lock().unwrap() = Some(StopReason::Fault);
                        failure = Some(uc_error::EXCEPTION);
                        break;
                    }
                }
                Err(e) => {
                    self.instructions += 0;
                    if self.pc() == SENTINEL {
                        break; // BX LR to the sentinel
                    }
                    self.trace.fault(
                        format!("ISR {isr_va:#010X} (irq {irq}) error: {e}"),
                        self.pc(),
                    );
                    failure = Some(e);
                    break;
                }
            }
        }
        self.hooks.isr_sentinel.store(0, Ordering::Relaxed);
        for (r, v) in SAVED.iter().zip(&saved) {
            self.engine.reg_write(*r, *v)?;
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Run up to `slice` instructions, then return control to the scheduler.
    /// Returns the stop reason if the engine asked to stop.
    /// `--lcd-watch`: once `GetActiveLCD` has published the framebuffer
    /// range, hook guest stores *in that range only* (Unicorn skips the
    /// callback for every other store, so execution stays fast).
    fn ensure_lcd_hook(&mut self) -> Result<(), uc_error> {
        if self.lcd_hooked || !self._shared.lcd_watch.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some((start, end)) = self._shared.lcd_watch.range() else {
            return Ok(());
        };
        let shared = self._shared.clone();
        self.engine.add_mem_hook(
            unicorn_engine::HookType::MEM_WRITE,
            u64::from(start),
            u64::from(end) - 1,
            Box::new(
                move |uc: &mut Unicorn<'_, ()>,
                      _typ: unicorn_engine::MemType,
                      address: u64,
                      _size: usize,
                      _value: i64|
                      -> bool {
                    let pc = uc.reg_read(RegisterARM::PC).unwrap_or(0) as u32;
                    shared.lcd_watch.note_write(address as u32, pc);
                    true
                },
            ),
        )?;
        self.lcd_hooked = true;
        Ok(())
    }

    /// Capture the explicit production wait, without a host stack or callback.
    pub fn pending_service_image(&self) -> Option<services::pending::ServiceImage> {
        self.hooks
            .pending
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.image())
    }

    fn publish_owner_image(&mut self, epoch: u64) {
        let (image, error) = match self.execution_image() {
            Ok(image) => (Some(image), None),
            Err(error) => (None, Some(error)),
        };
        self._shared.owner_images.lock().unwrap().insert(
            self.thread_id,
            OwnerImage {
                epoch,
                image,
                error,
            },
        );
    }
    fn owner_boundary(&mut self) {
        let points = self._shared.safe_points.clone();
        points.boundary_with(self.thread_id, |epoch| self.publish_owner_image(epoch));
    }

    pub fn execution_image(&mut self) -> Result<ExecutionImage, String> {
        if self.hooks.stop.lock().unwrap().is_some()
            || self.hooks.isr_sentinel.load(Ordering::Relaxed) != 0
        {
            return Err("CPU image requires an ordinary execution boundary".into());
        }
        let memory = self._mem.clone();
        let _gate = memory.snapshot_gate.enter();
        Ok(ExecutionImage {
            thread_id: self.thread_id,
            cpu: crate::cpu_state::ArmCpuImage::capture(&mut self.engine)?,
            pending: self.pending_service_image(),
            instructions: self.instructions,
            counted_exact: self.counted_exact,
            sentinel_pc: self.sentinel_pc,
        })
    }
    pub fn restore_execution_image(
        &mut self,
        image: ExecutionImage,
        resources: &services::sync::image::RestoredSync,
    ) -> Result<(), String> {
        if image.thread_id != self.thread_id || self.hooks.pending.lock().unwrap().is_some() {
            return Err("Execution image belongs to another or already active owner".into());
        }
        self._mem
            .validate_range(
                image.cpu.gprs[15],
                if image.cpu.cpsr & 0x20 != 0 { 2 } else { 4 },
            )
            .map_err(|e| e.to_string())?;
        let previous = crate::cpu_state::ArmCpuImage::capture(&mut self.engine)?;
        image.cpu.restore(&mut self.engine)?;
        if let Some(pending) = image.pending {
            if let Err(error) = self.restore_pending_service(pending, resources) {
                previous.restore(&mut self.engine)?;
                return Err(error);
            }
        }
        self.instructions = image.instructions;
        self.counted_exact = image.counted_exact;
        self.sentinel_pc = image.sentinel_pc;
        Ok(())
    }

    /// Bind a restored production service after its CPU and memory were loaded.
    /// Every frame/ABI check precedes changes to either the owner or its queues.
    pub fn restore_pending_service(
        &mut self,
        image: services::pending::ServiceImage,
        resources: &services::sync::image::RestoredSync,
    ) -> Result<(), String> {
        let memory = self._mem.clone();
        let _gate = memory.snapshot_gate.enter();
        if self.hooks.pending.lock().unwrap().is_some() || image.owner != self.thread_id {
            return Err("Pending-service restore requires the matching unused CPU owner".into());
        }
        for (reg, expected) in [
            (RegisterARM::R0, image.args.r0),
            (RegisterARM::R1, image.args.r1),
            (RegisterARM::R2, image.args.r2),
            (RegisterARM::R3, image.args.r3),
            (RegisterARM::SP, image.args.sp),
        ] {
            if self.reg(reg) != expected {
                return Err("Saved service arguments disagree with the restored CPU".into());
            }
        }
        let pc = self.pc();
        let service = match image.wait {
            services::pending::Wait::Sleep { .. } => "OSSleep",
            services::pending::Wait::Input { .. } => "GetEvent",
            services::pending::Wait::PowerOff => "SysPowerOff",
            services::pending::Wait::Indicator { .. } => "DeviceIoControl",
            services::pending::Wait::Usb { .. } => "VirtualUsbReceive",
            services::pending::Wait::Event { .. } => "OSWaitForEvent",
            services::pending::Wait::Semaphore { .. } => "OSWaitForSemaphore",
            services::pending::Wait::CriticalSection { .. } => "OSEnterCriticalSection",
        };
        let instruction = pc.checked_sub(4).ok_or("Invalid pending service trap PC")?;
        let service_id = if service == "VirtualUsbReceive" {
            services::usb::RECEIVE_SVC
        } else {
            services::names::service_id_by_name(service).ok_or("Unknown pending service")?
        };
        let expected = 0xef000000 | service_id;
        if self._mem.read_u32(instruction).map_err(|e| e.to_string())? != expected
            || self
                ._mem
                .read_u32(image.args.sp)
                .map_err(|e| e.to_string())?
                != image.args.caller_pc
        {
            return Err(
                "Saved wait disagrees with the restored SVC instruction/return frame".into(),
            );
        }
        if let services::pending::Wait::Indicator { record } = image.wait {
            self._mem
                .validate_range(record, 40)
                .map_err(|e| e.to_string())?;
            if !self
                ._shared
                .indicator
                .lock()
                .unwrap()
                .active
                .as_ref()
                .is_some_and(|pattern| pattern.record == record && pattern.owner == self.thread_id)
            {
                return Err("Pending indicator requires its matching reconstructed pattern".into());
            }
        }
        if let services::pending::Wait::Usb { output } = image.wait {
            self._mem
                .validate_range(output, services::usb::RECEIVE_BYTES)
                .map_err(|e| e.to_string())?;
        }
        if let services::pending::Wait::Input { output } = image.wait {
            self._mem
                .validate_range(output, 124)
                .map_err(|e| e.to_string())?;
        }
        let mut pending = services::pending::PendingService::from_bound_image(image, resources)?;
        pending.attach_owner_state(&self._shared);
        self._shared
            .pending_services
            .lock()
            .unwrap()
            .insert(self.thread_id, pending.image());
        *self.hooks.pending.lock().unwrap() = Some(pending);
        Ok(())
    }

    pub fn run_slice(&mut self, slice: usize) -> Result<RunSliceOutcome, uc_error> {
        if let Some(pending) = self.hooks.pending.lock().unwrap().as_ref() {
            self._shared
                .pending_services
                .lock()
                .unwrap()
                .insert(self.thread_id, pending.image());
        }
        self.owner_boundary();
        let shared = self._shared.clone();
        if !shared
            .guest_thread_checkpoint_with(self.thread_id, |epoch| self.publish_owner_image(epoch))
        {
            return Ok(RunSliceOutcome::Stopped(StopReason::Requested));
        }
        let power_owner = self
            .hooks
            .pending
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|pending| {
                matches!(
                    pending.image().wait,
                    services::pending::Wait::PowerOff | services::pending::Wait::Indicator { .. }
                )
            });
        if !self._shared.power_execution_allowed(power_owner) {
            // An OFF-gated CPU cannot service a queued scheduler turn. Leaving
            // its request ready can starve the power-service owner after that
            // owner's quantum expires, even when ON has already been accepted.
            self._shared.scheduler.cancel_request(self.thread_id);
            std::thread::sleep(std::time::Duration::from_millis(1));
            return Ok(RunSliceOutcome::Progress);
        }
        let Some(scheduled) = self._shared.scheduler.enter_slice(self.thread_id) else {
            return Ok(RunSliceOutcome::Progress);
        };
        if self.hooks.pending.lock().unwrap().is_some() {
            let memory = self._mem.clone();
            let active = memory.snapshot_gate.enter();
            let mut slot = self.hooks.pending.lock().unwrap();
            let pending = slot.as_ref().unwrap();
            let redirected = pending
                .redirect(&mut self.engine, &memory, &self._shared)
                .unwrap_or_else(|error| {
                    self.trace
                        .fault(format!("guest call: {error}"), pending.args.caller_pc);
                    false
                });
            let result = if redirected {
                Ok(Some(0))
            } else {
                pending.poll(&memory, &self._shared, self.thread_id)
            };
            match result {
                Ok(Some(value)) => {
                    if !redirected {
                        self.engine.reg_write(RegisterARM::R0, u64::from(value))?;
                        self.engine
                            .reg_write(RegisterARM::SP, u64::from(pending.args.sp) + 8)?;
                        self.engine
                            .reg_write(RegisterARM::PC, u64::from(pending.args.caller_pc))?;
                    }
                    *slot = None;
                    self._shared
                        .pending_services
                        .lock()
                        .unwrap()
                        .remove(&self.thread_id);
                }
                Ok(None) => {
                    // No memory gate or native callback survives this bounded
                    // owner wait. Every next poll acknowledges pause requests.
                    drop(slot);
                    drop(active);
                    self._shared.scheduler.cancel_request(self.thread_id);
                    drop(scheduled);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    return Ok(RunSliceOutcome::Progress);
                }
                Err(error) => {
                    let report = fault_report(
                        &self.engine,
                        &format!("pending service: {error}"),
                        self.thread_id,
                    );
                    self.trace.fault(report.kind.clone(), report.pc);
                    *self.hooks.fault.lock().unwrap() = Some(report);
                    return Ok(RunSliceOutcome::Stopped(StopReason::Fault));
                }
            }
        }
        self._shared.safe_points.executing(self.thread_id);
        let memory = self._mem.clone();
        let _active = memory.snapshot_gate.enter();
        self.ensure_lcd_hook()?;
        if self._shared.stopping() {
            return Ok(RunSliceOutcome::Stopped(StopReason::Requested));
        }
        let res = self.engine.emu_start(self.resume_addr(), 0, 0, slice);
        // Exact instruction accounting: a slice that returns without a stop
        // reason ran exactly `slice` instructions; an interrupted slice ran
        // fewer (unknown), so we conservatively add nothing and flag it.
        if self.hooks.stop.lock().unwrap().is_none()
            && self.hooks.pending.lock().unwrap().is_none()
            && res.is_ok()
        {
            self.instructions += slice as u64;
            self.counted_exact = true;
        } else {
            self.counted_exact = false;
        }
        publish_thread(
            &self.engine,
            &self.trace,
            self.thread_id,
            "slice_return",
            Some(self.instructions),
            self.counted_exact,
        );
        match res {
            Ok(()) => {
                if let Some(reason) = self.hooks.stop.lock().unwrap().take() {
                    return Ok(RunSliceOutcome::Stopped(reason));
                }
                Ok(RunSliceOutcome::Progress)
            }
            Err(e) => {
                // The firmware's yield idiom: invalid instruction that the
                // reference emulator treats as no-op setting Z (see
                // FIRMWARE_YIELD_WORD doc).
                if e == uc_error::INSN_INVALID {
                    // On INSN_INVALID the PC still points at the faulting
                    // instruction; check the word there (reference behavior).
                    let cur = self.pc();
                    let mut word = [0u8; 4];
                    if self.engine.mem_read(cur as u64, &mut word).is_ok()
                        && u32::from_le_bytes(word) == FIRMWARE_YIELD_WORD
                    {
                        let mut cpsr = self.reg(RegisterARM::CPSR);
                        cpsr |= 1 << 30; // Z flag
                        let _ = self.engine.reg_write(RegisterARM::CPSR, cpsr as u64);
                        let _ = self.engine.reg_write(RegisterARM::PC, (cur + 4) as u64);
                        return Ok(RunSliceOutcome::Progress);
                    }
                }
                if e == uc_error::INSN_INVALID {
                    if let Some((module, import)) = self.hooks.shared.unresolved_import(self.pc()) {
                        // Dependencies may have loaded since this IAT was built.
                        // Rebind to their real export, retaining the caller's LR
                        // and argument registers; never answer the call with 0.
                        if let Some(target) = self
                            .hooks
                            .shared
                            .pe_resolve(&import.dll, import.name.clone())
                        {
                            self.hooks
                                .mem
                                .write_u32(import.iat, target)
                                .map_err(|_| uc_error::WRITE_UNMAPPED)?;
                            let cpsr = (self.reg(RegisterARM::CPSR) & !0x20) | ((target & 1) << 5);
                            self.engine.reg_write(RegisterARM::CPSR, u64::from(cpsr))?;
                            self.engine
                                .reg_write(RegisterARM::PC, u64::from(target & !1))?;
                            return Ok(RunSliceOutcome::Progress);
                        }
                        let report = fault_report(
                            &self.engine,
                            &format!("unresolved PE import {} in {module}", import.description()),
                            self.thread_id,
                        );
                        if let Some(runtime) = &self.trace.runtime {
                            runtime.fault(&report);
                        }
                        self.trace.fault(report.kind.clone(), report.pc);
                        *self.hooks.fault.lock().unwrap() = Some(report);
                        return Ok(RunSliceOutcome::Stopped(StopReason::Fault));
                    }
                }
                if let Some(sentinel) = self.sentinel_pc {
                    if self.pc() == sentinel {
                        return Ok(RunSliceOutcome::Stopped(StopReason::Requested));
                    }
                }
                let last_pcs = self
                    .trace_handle
                    .as_ref()
                    .map(|t| t.last_code(64))
                    .unwrap_or_default();
                let report = self.hooks.fault.lock().unwrap().clone().unwrap_or_else(|| {
                    fault_report_with(&self.engine, &format!("{e}"), self.thread_id, last_pcs)
                });
                if let Some(runtime) = &self.trace.runtime {
                    runtime.fault(&report);
                }
                *self.hooks.fault.lock().unwrap() = Some(report.clone());
                self.trace.fault(format!("unicorn error: {e}"), report.pc);
                Ok(RunSliceOutcome::Stopped(StopReason::Fault))
            }
        }
    }
}

#[derive(Debug)]
pub enum RunSliceOutcome {
    /// The slice ran; execution continues.
    Progress,
    /// The engine asked to stop (policy, fault, external request).
    Stopped(StopReason),
}

impl Drop for CpuThread {
    fn drop(&mut self) {
        self._shared.scheduler.cancel_request(self.thread_id);
        self._shared
            .owner_images
            .lock()
            .unwrap()
            .remove(&self.thread_id);
        self._shared
            .pending_services
            .lock()
            .unwrap()
            .remove(&self.thread_id);
    }
}

fn fault_report(uc: &Unicorn<'_, ()>, kind: &str, thread_id: u32) -> FaultReport {
    fault_report_with(uc, kind, thread_id, Vec::new())
}

fn fault_report_with(
    uc: &Unicorn<'_, ()>,
    kind: &str,
    thread_id: u32,
    last_pcs: Vec<u32>,
) -> FaultReport {
    let mut regs = [0u32; 16];
    for (i, r) in [
        RegisterARM::R0,
        RegisterARM::R1,
        RegisterARM::R2,
        RegisterARM::R3,
        RegisterARM::R4,
        RegisterARM::R5,
        RegisterARM::R6,
        RegisterARM::R7,
        RegisterARM::R8,
        RegisterARM::R9,
        RegisterARM::R10,
        RegisterARM::R11,
        RegisterARM::R12,
        RegisterARM::SP,
        RegisterARM::LR,
        RegisterARM::PC,
    ]
    .into_iter()
    .enumerate()
    {
        regs[i] = uc.reg_read(r).unwrap_or(0) as u32;
    }
    FaultReport {
        kind: kind.to_owned(),
        pc: regs[15],
        addr: None,
        regs,
        cpsr: uc.reg_read(RegisterARM::CPSR).unwrap_or(0) as u32,
        thread_id,
        last_pcs,
    }
}

/// Read registers exclusively on the engine owner. A failed register read
/// omits the observation rather than manufacturing zero-valued registers.
fn publish_thread(
    uc: &Unicorn<'_, ()>,
    trace: &TraceHub,
    thread_id: u32,
    boundary: &str,
    instructions: Option<u64>,
    exact: bool,
) {
    if !trace.thread_observation_enabled() {
        return;
    }
    let mut registers = [0; 16];
    for (i, r) in [
        RegisterARM::R0,
        RegisterARM::R1,
        RegisterARM::R2,
        RegisterARM::R3,
        RegisterARM::R4,
        RegisterARM::R5,
        RegisterARM::R6,
        RegisterARM::R7,
        RegisterARM::R8,
        RegisterARM::R9,
        RegisterARM::R10,
        RegisterARM::R11,
        RegisterARM::R12,
        RegisterARM::SP,
        RegisterARM::LR,
        RegisterARM::PC,
    ]
    .into_iter()
    .enumerate()
    {
        let Ok(value) = uc.reg_read(r) else {
            return;
        };
        registers[i] = value as u32;
    }
    let Ok(cpsr) = uc.reg_read(RegisterARM::CPSR) else {
        return;
    };
    trace.note_thread(crate::trace::ThreadObservation {
        thread_id,
        sequence: 0,
        observed_ms: 0.0,
        boundary: boundary.into(),
        registers,
        cpsr: cpsr as u32,
        instructions_lower_bound: instructions,
        last_slice_exact: exact,
    });
}

#[cfg(test)]
mod debug_observation_tests {
    use super::*;
    use crate::trace::TraceConfig;

    #[test]
    fn observations_copy_actual_owner_registers_without_resuming_guest() {
        let mut uc = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
        uc.mem_map(0x1000, 0x1000, Prot::ALL).unwrap();
        // MOV R0,#42; B . — one bounded instruction, no proprietary assets.
        uc.mem_write(0x1000, &[42, 0, 160, 227, 254, 255, 255, 234])
            .unwrap();
        uc.emu_start(0x1000, 0, 0, 1).unwrap();
        let trace = TraceHub::new(TraceConfig {
            svc_trace: true,
            verbosity: 0,
            ..Default::default()
        })
        .unwrap();
        publish_thread(&uc, &trace, 7, "slice_return", Some(1), true);
        let snapshots = trace.thread_observations();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].thread_id, 7);
        assert_eq!(snapshots[0].registers[0], 42);
        assert_eq!(snapshots[0].registers[15], 0x1004);
        assert_eq!(snapshots[0].instructions_lower_bound, Some(1));
        uc.reg_write(RegisterARM::R0, 99).unwrap();
        assert_eq!(snapshots[0].registers[0], 42);
        assert_eq!(uc.reg_read(RegisterARM::PC).unwrap(), 0x1004);
    }
}
