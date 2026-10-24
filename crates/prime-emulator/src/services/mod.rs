//! OS service registry and SVC dispatch.
//!
//! The firmware calls platform services through 12-byte stubs in its own
//! image (`push {r0}; push {lr}; svc #(0x10000+N)`), reached by plain `bl`.
//! Verified against the 20260909 `armfir.elf` (stub table at `0x307FBBE4`)
//! and matching the reference emulator's dispatcher:
//!
//! - at SVC time: `[SP]` = caller return address, `[SP+4]` = saved r0,
//!   caller stack arguments start at `[SP+8]`; arguments r0..r3 are in
//!   registers;
//! - on return: `R0` = result, `SP += 8`, `PC` = the caller return address.
//!
//! An unimplemented service is *never* answered with an invented success
//! value: it is recorded (id, caller, registers) and handled according to
//! the configured [`UnsupportedPolicy`].

pub mod fs;
pub mod fsops;
pub mod handlers;
pub mod names;
pub mod pe;
pub mod sync;
pub mod system2;
pub mod threading;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use unicorn_engine::{RegisterARM, Unicorn};

use crate::memory::{GuestMemory, MemResult, MemoryError};
use crate::trace::TraceHub;

pub use names::FIRST_SERVICE_ID;
pub use names::LAST_SERVICE_ID;

/// Guest-visible register arguments captured at SVC time.
#[derive(Debug, Clone, Copy)]
pub struct GuestArgs {
    pub r0: u32,
    pub r1: u32,
    pub r2: u32,
    pub r3: u32,
    pub sp: u32,
    /// Return address the stub pushed (`[sp]`), i.e. the call site + 4.
    pub caller_pc: u32,
    /// Two guest-stack words above the stub frame (`[sp+8]`, `[sp+12]`).
    /// These are the caller's first saved frame words — e.g. for the
    /// wait_event shim (0x30693404) they are the saved event handle and
    /// timeout. Diagnostic only; NOT return addresses in general.
    pub stack_slots: [u32; 2],
}

impl GuestArgs {
    /// Caller-provided stack argument `i` (0-based): `[sp + 8 + 4*i]`.
    /// The +8 skips the stub's pushed return address and saved r0.
    pub fn stack_arg(&self, mem: &GuestMemory, i: usize) -> MemResult<u32> {
        mem.read_u32(self.sp + 8 + (i as u32) * 4)
    }
}

/// Host-side error escaping a handler: guest memory was invalid while
/// marshalling. This is a fault of the guest state, not a guest error code.
#[derive(Debug)]
pub struct HandlerFault(pub MemoryError);

/// A single service invocation context.
pub struct SvcCtx<'a, 'e> {
    pub uc: &'a mut Unicorn<'e, ()>,
    pub mem: &'a GuestMemory,
    pub args: GuestArgs,
    pub shared: &'a SharedState,
    pub trace: &'a TraceHub,
    /// Emulator-side thread identity of the engine that trapped.
    pub thread_id: u32,
}

impl SvcCtx<'_, '_> {
    pub fn stack_arg(&self, i: usize) -> Result<u32, HandlerFault> {
        self.args.stack_arg(self.mem, i).map_err(HandlerFault)
    }

    pub fn guest(&self, va: u32) -> Result<u32, HandlerFault> {
        self.mem.read_u32(va).map_err(HandlerFault)
    }

    pub fn guest_str(&self, va: u32) -> Result<Option<String>, HandlerFault> {
        self.mem.read_cstr_bounded(va, 4096).map_err(HandlerFault)
    }

    pub fn guest_wstr(&self, va: u32) -> Result<Option<String>, HandlerFault> {
        self.mem.read_wstr_bounded(va, 4096).map_err(HandlerFault)
    }

    /// Allocate a guest heap block and copy a NUL-terminated string into it.
    /// Returns the guest pointer.
    pub fn heap_cstr(&self, s: &str) -> Result<u32, HandlerFault> {
        let va = self.mem.heap_alloc(s.len() + 1).map_err(HandlerFault)?;
        self.mem.write_cstr(va, s).map_err(HandlerFault)?;
        Ok(va)
    }

    /// Block the calling guest thread like the real OS would, but bounded so
    /// a headless run cannot hang forever without diagnostics.
    pub fn bounded_sleep(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Implementation state of a service, reported in the run report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceStatus {
    /// Handler implements the verified platform behavior.
    Implemented,
    /// Handler covers the firmware's observed path only; documented gaps.
    PartiallyImplemented,
    /// Deliberately not implemented (policy-relevant: never silently 0).
    Unsupported,
    /// No handler written yet; discovery order decides.
    NotInvestigated,
}

impl ServiceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceStatus::Implemented => "implemented",
            ServiceStatus::PartiallyImplemented => "partial",
            ServiceStatus::Unsupported => "unsupported",
            ServiceStatus::NotInvestigated => "not-investigated",
        }
    }
}

pub type HandlerFn = fn(&mut SvcCtx<'_, '_>) -> Result<u32, HandlerFault>;

pub struct ServiceEntry {
    pub id: u32,
    pub name: Option<&'static str>,
    pub status: ServiceStatus,
    pub handler: Option<HandlerFn>,
    /// Number of times this service was dispatched (host-side counter).
    pub dispatches: AtomicU64,
}

/// The complete 742-entry service table.
pub struct Registry {
    entries: Vec<ServiceEntry>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Registry {
        let entries = names::SERVICE_NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| ServiceEntry {
                id: FIRST_SERVICE_ID + i as u32,
                name: *name,
                status: ServiceStatus::NotInvestigated,
                handler: None,
                dispatches: AtomicU64::new(0),
            })
            .collect();
        Registry { entries }
    }

    fn idx(id: u32) -> Option<usize> {
        let i = id.checked_sub(FIRST_SERVICE_ID)? as usize;
        (i < names::SERVICE_COUNT).then_some(i)
    }

    pub fn entry(&self, id: u32) -> Option<&ServiceEntry> {
        Self::idx(id).map(|i| &self.entries[i])
    }

    pub fn name(&self, id: u32) -> Option<&'static str> {
        self.entry(id).and_then(|e| e.name)
    }

    /// Register a handler by verified service name. Panics at boot if the
    /// name is not in the verified table — a typo must fail loudly.
    pub fn register_by_name(&mut self, name: &str, status: ServiceStatus, handler: HandlerFn) {
        let id = names::service_id_by_name(name)
            .unwrap_or_else(|| panic!("service name '{name}' is not in the verified SDKLIB table"));
        self.register_by_id(id, status, handler);
    }

    pub fn register_by_id(&mut self, id: u32, status: ServiceStatus, handler: HandlerFn) {
        let i = Self::idx(id).expect("service id outside verified range");
        self.entries[i].status = status;
        self.entries[i].handler = Some(handler);
    }

    /// Mark a service as deliberately unsupported (distinct from
    /// not-investigated): it will be reported as a known gap.
    pub fn mark_unsupported_by_name(&mut self, name: &str) {
        let id = names::service_id_by_name(name)
            .unwrap_or_else(|| panic!("service name '{name}' is not in the verified SDKLIB table"));
        let i = Self::idx(id).unwrap();
        self.entries[i].status = ServiceStatus::Unsupported;
    }

    pub fn entries(&self) -> &[ServiceEntry] {
        &self.entries
    }

    /// Summary of every service that was dispatched at least once.
    pub fn used_services(&self) -> Vec<(u32, Option<&'static str>, &'static str, u64)> {
        self.entries
            .iter()
            .filter(|e| e.dispatches.load(Ordering::Relaxed) > 0)
            .map(|e| {
                (
                    e.id,
                    e.name,
                    e.status.as_str(),
                    e.dispatches.load(Ordering::Relaxed),
                )
            })
            .collect()
    }
}

/// What to do when the guest calls a service with no handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedPolicy {
    /// Stop emulation at the first unsupported service (strict).
    Stop,
    /// Log it, return 0, continue (diagnostic mode; matches how the firmware
    /// tolerates absent optional services). Never reported as success.
    ZeroReturn,
}

/// Host-side state shared by all service handlers and guest threads.
pub struct SharedState {
    /// Root directory serving the guest's drive letters (`A:\` etc.).
    pub data_dir: PathBuf,
    /// Host-side input event queue consumed by the GetEvent service.
    /// Entries are (status, key_code0) pairs in the verified UIMultipressEvent
    /// semantics: status 16 = key down, 0x100000 = key up.
    pub input_events: Mutex<std::collections::VecDeque<InputEvent>>,
    /// Set by the runner when limits are reached so blocking handlers can
    /// unwind instead of hanging the run loop.
    pub stopping: std::sync::atomic::AtomicBool,
    /// Guest-visible path of the running firmware image (the path the
    /// firmware was loaded from, in guest drive-letter form).
    pub firmware_guest_path: String,
    /// Count of GetEvent batches delivered with real input events.
    input_delivered: AtomicU64,
    /// Thread that most recently delivered an input batch (heartbeat
    /// evidence: the pump thread identity).
    input_thread: std::sync::atomic::AtomicU32,
    /// Most recent GetEvent call context: [caller_pc, [sp+8], [sp+12]].
    pub last_getevent_chain: Mutex<[u32; 3]>,
    /// Event-type filter state behind SetEventType/GetEventType. The UI
    /// pump reads it, ORs in the types it wants (observed: 0x20), and
    /// writes it back before entering GetEvent. GetEvent currently
    /// delivers all queued input batches regardless of this filter
    /// (documented divergence; the observed pump path only ORs bits in).
    pub event_type_filter: std::sync::atomic::AtomicU32,
    /// Singleton zeroed "running program" record returned by
    /// ProgramIsRunningA/W (reference behavior: one 0x250-byte struct).
    pub program_record: Mutex<Option<u32>>,
    /// Guest virtual filesystem over the host data directory.
    pub vfs: fs::Vfs,
    /// Live sync objects keyed by guest descriptor address.
    pub sync_objects: Mutex<sync::SyncMap>,
    /// Virtual devices keyed by handle (ref: CreateFile registers names).
    pub devices: Mutex<HashMap<u32, String>>,
    next_device: AtomicU64,
    /// Factory (FRP) blob, in-memory only (ref persists to a file).
    pub frp_data: Mutex<Vec<u8>>,
    /// LCD object guest VA singleton.
    pub lcd_object: Mutex<Option<u32>>,
    /// Optional framebuffer write statistics (`--lcd-watch`, diagnostics).
    pub lcd_watch: LcdWatch,
    /// True while GetEvent is waiting with an empty input queue: the UI
    /// pump has finished the previous batch and is idle (guest-observable
    /// synchronization point for key scripts).
    getevent_parked: AtomicBool,
    /// GetEvent invocations (pump entries).
    getevent_entries: AtomicU64,
    /// True while the main guest thread (tid 0) is blocked in
    /// OSWaitForEvent: it has finished handling the messages the pump
    /// forwarded to it.
    main_waiting: AtomicBool,
    /// Pending non-input UI events (timer events), delivered by GetEvent
    /// before queued input — PrimU2's `special_event` slot, as a small
    /// bounded queue so a SYS_TIMER and a DEVICE_TIMER due together are
    /// both delivered.
    system_events: Mutex<std::collections::VecDeque<u32>>,
    /// Timer/system events delivered by GetEvent.
    system_events_delivered: AtomicU64,
    /// DEVICE_TIMER events delivered by GetEvent (the ~1 s UI commit tick).
    device_timer_delivered: AtomicU64,
    /// Pending touch samples, one delivered per GetEvent call as a
    /// `UI_EVENT_TYPE_TOUCH` event (PrimU2 `TouchUpdate` semantics).
    touch_events: Mutex<std::collections::VecDeque<TouchSample>>,
    touch_delivered: AtomicU64,
    /// Spawner installed by the runner; creates real guest threads.
    pub thread_spawner: Mutex<Option<ThreadSpawner>>,
    /// Host threads that requested guest-thread exit.
    pub threads_exiting: Mutex<std::collections::HashSet<std::thread::ThreadId>>,
    /// Live spawned-thread handles for the final report.
    pub thread_handles: Mutex<HashMap<u32, std::thread::JoinHandle<()>>>,
    /// Per-thread instruction counters (liveness evidence).
    pub thread_instructions: Mutex<HashMap<u32, u64>>,
    /// Registered guest ISRs (irq -> guest entry), delivered by the
    /// runner's virtual IRQ dispatcher.
    pub isrs: Mutex<HashMap<u32, u32>>,
    /// Count of delivered virtual IRQs per irq number.
    pub irq_delivered: Mutex<HashMap<u32, u64>>,
    /// Loaded guest PE modules.
    pub pe_modules: Mutex<Vec<pe::PeModule>>,
    /// Import-stub block (base, next free offset): return-zero Thumb stubs
    /// for unresolved imports (reference behavior).
    pub stub_block: Mutex<Option<(u32, u32)>>,
    /// Guest thread records (tid -> entry point), for OSCreateThread & co.
    pub threads: Mutex<HashMap<u32, ThreadRecord>>,
    /// Next guest thread id (matches reference: first thread is 0).
    next_thread_id: AtomicU64,
}

/// Framebuffer write statistics for `--lcd-watch` (diagnostics only).
/// `GetActiveLCD` publishes the buffer range when it creates the object;
/// each engine then installs a write hook limited to that range.
#[derive(Debug, Default)]
pub struct LcdWatch {
    pub enabled: AtomicBool,
    /// `start << 32 | end` of the framebuffer (0 = not yet known).
    range: AtomicU64,
    writes: AtomicU64,
    /// Guest PCs that stored into the framebuffer (bounded).
    pcs: Mutex<HashMap<u32, u64>>,
}

impl LcdWatch {
    const MAX_PCS: usize = 256;

    pub fn set_range(&self, start: u32, len: u32) {
        let end = start.saturating_add(len);
        self.range
            .store((u64::from(start) << 32) | u64::from(end), Ordering::Release);
    }

    /// Hot path (every guest store while enabled): a range check, then a
    /// counter and a bounded histogram update for framebuffer stores.
    pub fn note_write(&self, addr: u32, pc: u32) {
        let r = self.range.load(Ordering::Relaxed);
        let (start, end) = ((r >> 32) as u32, r as u32);
        if addr < start || addr >= end {
            return;
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        let mut pcs = self.pcs.lock().unwrap();
        if pcs.len() < Self::MAX_PCS || pcs.contains_key(&pc) {
            *pcs.entry(pc).or_insert(0) += 1;
        }
    }

    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    /// `[start, end)` of the framebuffer once known.
    pub fn range(&self) -> Option<(u32, u32)> {
        let r = self.range.load(Ordering::Acquire);
        (r != 0).then_some(((r >> 32) as u32, r as u32))
    }

    /// Writer PCs by descending store count.
    pub fn top_pcs(&self, n: usize) -> Vec<(u32, u64)> {
        let mut v: Vec<(u32, u64)> = self
            .pcs
            .lock()
            .unwrap()
            .iter()
            .map(|(&k, &c)| (k, c))
            .collect();
        v.sort_by_key(|&(pc, c)| (std::cmp::Reverse(c), pc));
        v.truncate(n);
        v
    }

    pub fn reset_pcs(&self) {
        self.pcs.lock().unwrap().clear();
    }
}

/// One touch sample (finger 0). `status` is UI_EVENT_TYPE_TOUCH_BEGIN /
/// _MOVE / _END; the pump reads x at entry+6 and y at entry+8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchSample {
    pub status: u32,
    pub x: u16,
    pub y: u16,
}

/// One synthetic multipress event, matching PrimU2's `UIMultipressEvent`
/// key-press semantics (CommandServer.cpp: "key <code> <down|up|press>").
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    /// UI_EVENT_TYPE_KEY (16) or UI_EVENT_TYPE_KEY_UP (0x100000).
    pub status: u32,
    pub key_code0: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct ThreadRecord {
    pub entry: u32,
    pub arg: u32,
    pub stack_size: usize,
}

/// Runner hook that creates a real guest thread (engine + native thread).
pub type ThreadSpawner = std::sync::Arc<dyn Fn(u32, u32, u32, usize) + Send + Sync>;

/// Verified UI event type constants (PrimU2 ui.h `ui_event_type_e`).
pub const UI_EVENT_TYPE_KEY: u32 = 16;
pub const UI_EVENT_TYPE_KEY_UP: u32 = 0x100000;
pub const UI_EVENT_TYPE_KEY_BATCH: u32 = 0x100010;
/// System event (`event+8` carries the subtype); PrimU2 ui.h.
pub const UI_EVENT_TYPE_SYSTEM: u32 = 0x100;
/// Periodic UI timer events (PrimU2 ui.h; raised by its 60 Hz window
/// timer every 11 and 61 ticks). The firmware's pump subscribes to
/// DEVICE_TIMER (`SetEventType(GetEventType() | 0x20)`) and dispatches
/// both (pump at 0x3060708C: 0x20 → 0x306099C0/0x3060A2A4, 0x4000 →
/// 0x306098A8).
pub const UI_EVENT_TYPE_DEVICE_TIMER: u32 = 0x20;
pub const UI_EVENT_TYPE_SYS_TIMER: u32 = 0x4000;
/// Touch event and its per-finger statuses (PrimU2 ui.h; the pump's 0xF
/// branch at 0x30607180 handles statuses 1/2 and 8).
pub const UI_EVENT_TYPE_TOUCH: u32 = 0xF;
pub const UI_EVENT_TYPE_TOUCH_BEGIN: u32 = 1;
pub const UI_EVENT_TYPE_TOUCH_MOVE: u32 = 2;
pub const UI_EVENT_TYPE_TOUCH_END: u32 = 8;

impl SharedState {
    pub fn new(data_dir: PathBuf, firmware_guest_path: String) -> SharedState {
        SharedState {
            firmware_guest_path,
            input_delivered: AtomicU64::new(0),
            input_thread: std::sync::atomic::AtomicU32::new(u32::MAX),
            last_getevent_chain: Mutex::new([0; 3]),
            program_record: Mutex::new(None),
            event_type_filter: std::sync::atomic::AtomicU32::new(0),
            vfs: fs::Vfs::new(data_dir.clone()),
            sync_objects: Mutex::new(HashMap::new()),
            devices: Mutex::new(HashMap::new()),
            next_device: AtomicU64::new(1),
            frp_data: Mutex::new(Vec::new()),
            lcd_object: Mutex::new(None),
            lcd_watch: LcdWatch::default(),
            getevent_parked: AtomicBool::new(false),
            getevent_entries: AtomicU64::new(0),
            main_waiting: AtomicBool::new(false),
            system_events: Mutex::new(std::collections::VecDeque::new()),
            system_events_delivered: AtomicU64::new(0),
            device_timer_delivered: AtomicU64::new(0),
            touch_events: Mutex::new(std::collections::VecDeque::new()),
            touch_delivered: AtomicU64::new(0),
            thread_spawner: Mutex::new(None),
            threads_exiting: Mutex::new(std::collections::HashSet::new()),
            thread_handles: Mutex::new(HashMap::new()),
            thread_instructions: Mutex::new(HashMap::new()),
            pe_modules: Mutex::new(Vec::new()),
            isrs: Mutex::new(HashMap::new()),
            irq_delivered: Mutex::new(HashMap::new()),
            stub_block: Mutex::new(None),
            data_dir,
            input_events: Mutex::new(std::collections::VecDeque::new()),
            threads: Mutex::new(HashMap::new()),
            next_thread_id: AtomicU64::new(1),
            stopping: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn alloc_thread_id(&self) -> u32 {
        self.next_thread_id.fetch_add(1, Ordering::Relaxed) as u32
    }

    /// Enqueue a key press (down + up), the reference emulator's "press".
    pub fn note_input_delivered(&self, thread_id: u32) {
        self.input_delivered.fetch_add(1, Ordering::Relaxed);
        self.input_thread.store(thread_id, Ordering::Relaxed);
    }

    pub fn input_thread(&self) -> u32 {
        self.input_thread.load(Ordering::Relaxed)
    }

    pub fn note_getevent_chain(&self, chain: [u32; 3]) {
        *self.last_getevent_chain.lock().unwrap() = chain;
    }

    pub fn getevent_chain(&self) -> [u32; 3] {
        *self.last_getevent_chain.lock().unwrap()
    }

    pub fn input_delivered(&self) -> u32 {
        self.input_delivered.load(Ordering::Relaxed) as u32
    }

    /// GetEvent is idle-waiting with nothing queued.
    pub fn getevent_parked(&self) -> bool {
        self.getevent_parked.load(Ordering::Acquire)
    }

    pub fn set_getevent_parked(&self, parked: bool) {
        self.getevent_parked.store(parked, Ordering::Release);
    }

    pub fn note_getevent_entry(&self) {
        self.getevent_entries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn main_waiting(&self) -> bool {
        self.main_waiting.load(Ordering::Acquire)
    }

    pub fn set_main_waiting(&self, waiting: bool) {
        self.main_waiting.store(waiting, Ordering::Release);
    }

    pub fn getevent_entries(&self) -> u64 {
        self.getevent_entries.load(Ordering::Relaxed)
    }

    /// Queues a timer/system UI event (`UI_EVENT_TYPE_SYS_TIMER`,
    /// `UI_EVENT_TYPE_DEVICE_TIMER`, ...). A type already pending is not
    /// queued twice (the guest sees one tick per pump round, as with the
    /// reference's single slot).
    pub fn push_system_event(&self, event_type: u32) {
        let mut q = self.system_events.lock().unwrap();
        if !q.contains(&event_type) && q.len() < 8 {
            q.push_back(event_type);
        }
    }

    pub fn pop_system_event(&self) -> Option<u32> {
        let ev = self.system_events.lock().unwrap().pop_front();
        if let Some(k) = ev {
            self.system_events_delivered.fetch_add(1, Ordering::Relaxed);
            if k == UI_EVENT_TYPE_DEVICE_TIMER {
                self.device_timer_delivered.fetch_add(1, Ordering::Relaxed);
            }
        }
        ev
    }

    pub fn system_events_delivered(&self) -> u64 {
        self.system_events_delivered.load(Ordering::Relaxed)
    }

    /// DEVICE_TIMER events handed to the guest by GetEvent. The firmware
    /// commits some UI state only on this ~1 s tick (M4: the first-run
    /// wizard's page transition needs one after a tap), so scripted input
    /// uses it as a guest-observed settle signal.
    pub fn device_timer_delivered(&self) -> u64 {
        self.device_timer_delivered.load(Ordering::Relaxed)
    }

    pub fn push_touch(&self, sample: TouchSample) {
        self.touch_events.lock().unwrap().push_back(sample);
    }

    pub fn pop_touch(&self) -> Option<TouchSample> {
        let t = self.touch_events.lock().unwrap().pop_front();
        if t.is_some() {
            self.touch_delivered.fetch_add(1, Ordering::Relaxed);
        }
        t
    }

    pub fn touch_pending(&self) -> usize {
        self.touch_events.lock().unwrap().len()
    }

    pub fn touch_delivered(&self) -> u64 {
        self.touch_delivered.load(Ordering::Relaxed)
    }

    /// Install the runner's guest-thread spawner.
    pub fn set_thread_spawner(&self, spawner: ThreadSpawner) {
        *self.thread_spawner.lock().unwrap() = Some(spawner);
    }

    /// Spawn a guest thread via the installed spawner (runner hook).
    pub fn spawn_thread(&self, tid: u32, func: u32, arg: u32, stack_size: usize) {
        let spawner = self.thread_spawner.lock().unwrap().clone();
        if let Some(sp) = spawner {
            sp(tid, func, arg, stack_size);
        }
    }

    pub fn mark_thread_exiting(&self) {
        self.threads_exiting
            .lock()
            .unwrap()
            .insert(std::thread::current().id());
    }

    pub fn thread_is_exiting(&self) -> bool {
        self.threads_exiting
            .lock()
            .unwrap()
            .contains(&std::thread::current().id())
    }

    pub fn register_thread_handle(&self, tid: u32, h: std::thread::JoinHandle<()>) {
        self.thread_handles.lock().unwrap().insert(tid, h);
    }

    pub fn note_thread_instructions(&self, tid: u32, n: u64) {
        self.thread_instructions.lock().unwrap().insert(tid, n);
    }

    /// Allocate a return-zero Thumb stub (MOVS r0,#0; BX LR) from the heap
    /// arena and return its address with the Thumb bit set (reference
    /// behavior for unresolved imports).
    pub fn alloc_import_stub(&self, mem: &crate::memory::GuestMemory) -> Option<u32> {
        let va = mem.heap_alloc(4).ok()?;
        mem.write_bytes(va, &[0x00, 0x20, 0x70, 0x47]).ok()?;
        Some(va | 1)
    }

    /// Look up a loaded module by its base address (LoadLibrary's handle).
    pub fn pe_module_by_base(&self, base: u32) -> Option<pe::PeModule> {
        self.pe_modules
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.base == base)
            .map(|m| pe::PeModule {
                name: m.name.clone(),
                guest_path: m.guest_path.clone(),
                base: m.base,
                size: m.size,
                entry_rva: m.entry_rva,
                exports_by_name: m.exports_by_name.clone(),
                exports_by_ordinal: m.exports_by_ordinal.clone(),
            })
    }

    pub fn pe_module_by_name(&self, name: &str) -> Option<u32> {
        let lower = name.to_lowercase();
        self.pe_modules
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.name.to_lowercase() == lower)
            .map(|m| m.base)
    }

    /// Resolve an imported symbol across all loaded modules.
    pub fn pe_resolve(&self, dll: &str, import: pe::ImportName) -> Option<u32> {
        let modules = self.pe_modules.lock().unwrap();
        // The firmware resolves SDKLIB imports against the OS — which here
        // is the SVC layer, so cross-DLL imports resolve only between the
        // guest PE modules.
        let lower = dll.to_lowercase();
        let in_dll = modules.iter().find(|m| m.name.to_lowercase() == lower);
        let search: Vec<&pe::PeModule> = if lower.is_empty() {
            modules.iter().collect()
        } else {
            in_dll.into_iter().collect()
        };
        match &import {
            pe::ImportName::Name(n) => search
                .iter()
                .find_map(|m| m.exports_by_name.get(n).copied()),
            pe::ImportName::Ordinal(o) => search
                .iter()
                .find_map(|m| m.exports_by_ordinal.get(o).copied()),
        }
    }

    pub fn add_pe_module(&self, m: pe::PeModule) {
        self.pe_modules.lock().unwrap().push(m);
    }

    /// Register / replace a guest ISR (ref: InterruptInitialize).
    pub fn register_isr(&self, irq: u32, isr_va: u32) {
        self.isrs.lock().unwrap().insert(irq, isr_va);
    }

    pub fn unregister_isr(&self, irq: u32) {
        self.isrs.lock().unwrap().remove(&irq);
    }

    /// Snapshot of ISRs due for delivery (the dispatcher calls this each
    /// tick; delivered IRQs are counted).
    pub fn take_due_isrs(&self) -> Vec<(u32, u32)> {
        self.isrs
            .lock()
            .unwrap()
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect()
    }

    pub fn note_irq_delivered(&self, irq: u32) {
        *self.irq_delivered.lock().unwrap().entry(irq).or_insert(0) += 1;
    }

    pub fn irq_delivery_counts(&self) -> Vec<(u32, u64)> {
        let mut v: Vec<(u32, u64)> = self
            .irq_delivered
            .lock()
            .unwrap()
            .iter()
            .map(|(&k, &c)| (k, c))
            .collect();
        v.sort();
        v
    }

    pub fn register_device(&self, name: &str) -> u32 {
        let h = self.next_device.fetch_add(1, Ordering::Relaxed) as u32;
        self.devices.lock().unwrap().insert(h, name.to_owned());
        h
    }

    pub fn unregister_device(&self, handle: u32) -> bool {
        self.devices.lock().unwrap().remove(&handle).is_some()
    }

    pub fn device_name(&self, handle: u32) -> Option<String> {
        self.devices.lock().unwrap().get(&handle).cloned()
    }

    pub fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }

    pub fn set_stopping(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }

    pub fn firmware_guest_path(&self) -> &str {
        &self.firmware_guest_path
    }

    pub fn push_key_press(&self, key_code: u16) {
        let mut q = self.input_events.lock().unwrap();
        q.push_back(InputEvent {
            status: UI_EVENT_TYPE_KEY,
            key_code0: key_code,
        });
        q.push_back(InputEvent {
            status: UI_EVENT_TYPE_KEY_UP,
            key_code0: key_code,
        });
    }

    pub fn pop_input_events(&self, max: usize) -> Vec<InputEvent> {
        let mut q = self.input_events.lock().unwrap();
        let n = q.len().min(max);
        q.drain(..n).collect()
    }

    pub fn input_events_pending(&self) -> usize {
        self.input_events.lock().unwrap().len()
    }
}

/// Result of dispatching one SVC.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// Handler ran; result written to r0.
    Handled,
    /// No handler; policy applied (ZeroReturn wrote 0, Stop set the flag).
    Unsupported { id: u32, caller_pc: u32 },
}

/// Dispatch one intercepted SVC. Applies the verified return convention.
pub fn dispatch(
    uc: &mut Unicorn<'_, ()>,
    mem: &GuestMemory,
    registry: &Registry,
    shared: &SharedState,
    trace: &TraceHub,
    _policy: UnsupportedPolicy,
    thread_id: u32,
) -> Result<DispatchOutcome, MemoryError> {
    let pc = reg(uc, RegisterARM::PC);
    let sp = reg(uc, RegisterARM::SP);
    if pc < 4 {
        return Err(MemoryError {
            va: pc,
            msg: "SVC with PC < 4".into(),
        });
    }
    let imm = mem.read_u32(pc - 4)?;
    let svc_id = imm & 0xFFFFF;
    let caller_pc = mem.read_u32(sp)?;
    let stack_slots = [
        mem.read_u32(sp.wrapping_add(8)).unwrap_or(0),
        mem.read_u32(sp.wrapping_add(12)).unwrap_or(0),
    ];
    let args = GuestArgs {
        r0: reg(uc, RegisterARM::R0),
        r1: reg(uc, RegisterARM::R1),
        r2: reg(uc, RegisterARM::R2),
        r3: reg(uc, RegisterARM::R3),
        sp,
        caller_pc,
        stack_slots,
    };
    let entry = registry.entry(svc_id);
    let supported = entry.is_some_and(|e| e.handler.is_some());
    if let Some(e) = entry {
        e.dispatches.fetch_add(1, Ordering::Relaxed);
    }
    trace.svc(
        svc_id,
        entry.and_then(|e| e.name),
        caller_pc,
        args.r0,
        supported,
        thread_id,
    );

    let handler = entry.and_then(|e| e.handler);
    let result = match handler {
        Some(handler) => {
            let mut ctx = SvcCtx {
                uc,
                mem,
                args,
                shared,
                trace,
                thread_id,
            };
            match handler(&mut ctx) {
                Ok(v) => v,
                Err(HandlerFault(e)) => {
                    let (live, blocks, largest_free) = mem.heap_stats();
                    trace.fault(
                        format!(
                            "service {svc_id:#07X} handler faulted: {e} \
                             (heap live {live}/{} bytes in {blocks} blocks, largest free {largest_free})",
                            mem.heap_size()
                        ),
                        caller_pc,
                    );
                    0
                }
            }
        }
        // No handler: r0 = 0 (never a fabricated success — the caller is
        // told the service is unsupported via the outcome / trace).
        None => 0,
    };

    // Verified return convention: r0 = result, sp += 8, pc = [sp].
    uc.reg_write(RegisterARM::R0, result as u64)
        .map_err(|e| MemoryError {
            va: sp,
            msg: format!("write r0: {e}"),
        })?;
    uc.reg_write(RegisterARM::SP, (sp + 8) as u64)
        .map_err(|e| MemoryError {
            va: sp,
            msg: format!("write sp: {e}"),
        })?;
    uc.reg_write(RegisterARM::PC, caller_pc as u64)
        .map_err(|e| MemoryError {
            va: sp,
            msg: format!("write pc: {e}"),
        })?;
    if handler.is_none() {
        return Ok(DispatchOutcome::Unsupported {
            id: svc_id,
            caller_pc,
        });
    }
    Ok(DispatchOutcome::Handled)
}

fn reg(uc: &Unicorn<'_, ()>, r: RegisterARM) -> u32 {
    uc.reg_read(r).expect("unicorn register read") as u32
}
