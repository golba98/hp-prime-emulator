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
use unicorn_engine::{RegisterARM, Unicorn, uc_error};

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
    stop: Mutex<Option<StopReason>>,
    /// Set by the mem-invalid hook with fault details.
    fault: Mutex<Option<FaultReport>>,
    /// Active ISR sentinel (0 = none): fetch faults there are normal ISR
    /// returns, not guest faults.
    isr_sentinel: std::sync::atomic::AtomicU32,
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
        let mut engine = Unicorn::new(Arch::ARM, Mode::ARM)?;
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

        crate::compiler::install(&mut engine, &mem, &trace, thread_id)?;
        crate::runtime::install(&mut engine, &mem, &shared, &trace, thread_id)?;

        // SVC dispatch hook.
        {
            let hs = hooks.clone();
            engine.add_intr_hook(Box::new(move |uc: &mut Unicorn<'_, ()>, _intno: u32| {
                let hs = &*hs;
                match services::dispatch(
                    uc,
                    &hs.mem,
                    &hs.registry,
                    &hs.shared,
                    &hs.trace,
                    hs.policy,
                    hs.thread_id,
                ) {
                    Ok(services::DispatchOutcome::Handled) => {
                        if hs.shared.stopping() {
                            *hs.stop.lock().unwrap() = Some(StopReason::Requested);
                            let _ = uc.emu_stop();
                        }
                    }
                    Ok(services::DispatchOutcome::Unsupported { id, caller_pc }) => {
                        hs.trace
                            .svc(id, hs.registry.name(id), caller_pc, 0, false, hs.thread_id);
                        if hs.policy == UnsupportedPolicy::Stop {
                            *hs.stop.lock().unwrap() =
                                Some(StopReason::UnsupportedService { id, caller_pc });
                            let _ = uc.emu_stop();
                        }
                    }
                    Err(e) => {
                        hs.trace.fault(format!("SVC dispatch failed: {e}"), 0);
                        *hs.stop.lock().unwrap() = Some(StopReason::Fault);
                        *hs.fault.lock().unwrap() = Some(FaultReport {
                            kind: format!("svc-dispatch: {e}"),
                            pc: 0,
                            addr: None,
                            regs: [0; 16],
                            cpsr: 0,
                            thread_id: 0,
                            last_pcs: Vec::new(),
                        });
                        let _ = uc.emu_stop();
                    }
                }
            }))?;
        }

        // Guest write watchpoints (diagnostics; bounded logging).
        if !hooks.watches.is_empty() {
            let hs = hooks.clone();
            // A universal-range write hook; each hit is filtered against
            // the configured watch list. Only installed when watches exist.
            engine.add_mem_hook(
                unicorn_engine::HookType::MEM_WRITE,
                1,
                0,
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
                                hs.trace.watch_hit(addr, size, value as u32, pc);
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
                        let report = fault_report(uc, &format!("{mem_type:?}"), thread_id);
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

    /// Execute one guest ISR synchronously on this engine with full
    /// register save/restore (deterministic IRQ delivery between slices —
    /// real hardware preempts one context; concurrent engines would race).
    /// The ISR returns via the unmapped sentinel LR.
    pub fn run_isr(&mut self, isr_va: u32, irq: u32) -> Result<(), uc_error> {
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
        loop {
            let res = self.engine.emu_start(self.resume_addr(), 0, 0, 100_000);
            match res {
                Ok(()) => {
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
                    break;
                }
            }
        }
        self.hooks.isr_sentinel.store(0, Ordering::Relaxed);
        for (r, v) in SAVED.iter().zip(&saved) {
            self.engine.reg_write(*r, *v)?;
        }
        Ok(())
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

    pub fn run_slice(&mut self, slice: usize) -> Result<RunSliceOutcome, uc_error> {
        self.ensure_lcd_hook()?;
        if self._shared.stopping() {
            return Ok(RunSliceOutcome::Stopped(StopReason::Requested));
        }
        let res = self.engine.emu_start(self.resume_addr(), 0, 0, slice);
        // Exact instruction accounting: a slice that returns without a stop
        // reason ran exactly `slice` instructions; an interrupted slice ran
        // fewer (unknown), so we conservatively add nothing and flag it.
        if self.hooks.stop.lock().unwrap().is_none() && res.is_ok() {
            self.instructions += slice as u64;
            self.counted_exact = true;
        } else {
            self.counted_exact = false;
        }
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
                let report =
                    fault_report_with(&self.engine, &format!("{e}"), self.thread_id, last_pcs);
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
