//! Portable kernel resources shared by the real firmware CPU owners.
//! RAM/storage and managed call/job state belong to the enclosing machine.
//! Capture requires a current coordinated pause; reconstruction creates a new
//! owner and never mutates an existing live session on failed admission.
use super::*;
use crate::{
    clock::{ClockImage, RtcImage},
    engine::ExecutionImage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

const MAGIC: &[u8; 8] = b"HPKERN01";
const MAX_BYTES: usize = 32 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerImage {
    pub system: Duration,
    pub device: Duration,
    pub irq: Duration,
}
impl TimerImage {
    pub fn at(now: Duration) -> Self {
        Self {
            system: now,
            device: now,
            irq: now,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PowerImage {
    sleeping: bool,
    wake: bool,
    swallow_on_up: bool,
    sleeps: u64,
    source: SleepSource,
    auto: power::Image,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WatchImage {
    enabled: bool,
    range: u64,
    writes: u64,
    pcs: BTreeMap<u32, u64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelImage {
    version: u32,
    identity: String,
    firmware_guest_path: String,
    clock: ClockImage,
    rtc: RtcImage,
    power: PowerImage,
    scheduler: scheduler::Image,
    timers: TimerImage,
    environment: ppl_engine::environment::EnvironmentConfig,
    backlight: backlight::Backlight,
    indicator: indicator::Indicator,
    usb: usb::Image,
    clipboard: crate::clipboard::Image,
    #[serde(default)]
    text_calls: crate::guest_call::text::MailboxImage,
    lcd_watch: WatchImage,
    devices: BTreeMap<u32, String>,
    next_device: u64,
    lcd: Option<u32>,
    program: Option<u32>,
    input: VecDeque<InputEvent>,
    touch: VecDeque<TouchSample>,
    system: VecDeque<SystemEvent>,
    input_delivered: u64,
    input_thread: u32,
    last_getevent_chain: [u32; 3],
    event_filter: u32,
    getevent_parked: bool,
    getevent_entries: u64,
    main_waiting: bool,
    main_wait_entries: u64,
    system_delivered: u64,
    device_delivered: u64,
    touch_delivered: u64,
    threads: BTreeMap<u32, ThreadRecord>,
    controls: BTreeMap<u32, ThreadControl>,
    stacks: BTreeMap<u32, u32>,
    next_thread: u64,
    instructions: BTreeMap<u32, u64>,
    isrs: BTreeMap<u32, u32>,
    disabled_isrs: BTreeSet<u32>,
    irq_delivered: BTreeMap<u32, u64>,
    pending_irqs: VecDeque<(u32, u32)>,
    modules: Vec<pe::PeModule>,
    stub_block: Option<(u32, u32)>,
    sync: sync::image::SyncImage,
    vfs: fs::image::VfsImage,
    owners: BTreeMap<u32, ExecutionImage>,
}
pub struct RestoredKernel {
    pub shared: SharedState,
    /// Retain leases until all CPU waits have been rebound, including closed
    /// descriptors whose final reference is the blocked owner's continuation.
    pub synchronization: sync::image::RestoredSync,
    pub owners: BTreeMap<u32, ExecutionImage>,
    /// Keep both clocks frozen until CPU and session reconstruction completes.
    pub startup: Vec<crate::clock::Freeze>,
    /// Launches queued before capture but not yet admitted to guest RAM/CPU.
    /// Completed workers are excluded and must never be restarted.
    pub pending_starts: Vec<(u32, ThreadRecord)>,
}
impl KernelImage {
    pub fn owner_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.owners.keys().copied()
    }
    pub fn capture(
        shared: &SharedState,
        mem: &GuestMemory,
        pause: &crate::safepoint::PauseLease,
        identity: &str,
    ) -> Result<Self> {
        let gate = mem
            .snapshot_gate
            .try_snapshot()
            .ok_or("Kernel capture has active memory writers")?;
        Self::capture_while_guarded(shared, mem, pause, &gate, identity)
    }
    pub fn capture_while_guarded(
        shared: &SharedState,
        mem: &GuestMemory,
        pause: &crate::safepoint::PauseLease,
        gate: &crate::memory::snapshot::Snapshot<'_>,
        identity: &str,
    ) -> Result<Self> {
        if !gate.guards(&mem.snapshot_gate)
            || !pause.coordinates(&shared.safe_points)
            || !pause.is_current()
            || mem.snapshot_gate.counts() != (0, 0)
            || shared.stopping()
        {
            return Err(
                "Kernel capture requires a current pause with no native continuations".into(),
            );
        }
        if shared.thread_failure.lock().unwrap().is_some() {
            return Err("Cannot resume a failed guest CPU".into());
        }
        let mut owners = BTreeMap::new();
        let images = shared.owner_images.lock().unwrap();
        for owner in shared
            .safe_points
            .observations()
            .iter()
            .filter(|o| o.phase != ppl_engine::inspection::OwnerPhase::Exited)
        {
            let published = images
                .get(&owner.thread_id)
                .ok_or("Missing CPU-owner image")?;
            if published.epoch != pause.epoch()
                || owner.acknowledged_epoch != pause.epoch()
                || published.error.is_some()
            {
                return Err("CPU-owner image does not acknowledge this capture epoch".into());
            }
            owners.insert(
                owner.thread_id,
                published
                    .image
                    .clone()
                    .ok_or("CPU owner did not publish its execution state")?,
            );
        }
        drop(images);
        let power = shared.power.lock().unwrap();
        let image = Self {
            version: 1,
            identity: identity.into(),
            firmware_guest_path: shared.firmware_guest_path.clone(),
            clock: shared.clock.image()?,
            rtc: shared.rtc.image()?,
            power: PowerImage {
                sleeping: power.sleeping,
                wake: power.wake,
                swallow_on_up: power.swallow_on_up,
                sleeps: power.sleeps,
                source: power.source,
                auto: power.auto.image(),
            },
            scheduler: shared.scheduler.image()?,
            timers: shared.timers.lock().unwrap().clone(),
            environment: shared.environment.lock().unwrap().clone(),
            backlight: shared.backlight.lock().unwrap().clone(),
            indicator: shared.indicator.lock().unwrap().clone(),
            usb: shared.usb.image(),
            clipboard: shared.clipboard.image(),
            text_calls: shared.text_calls.image(),
            lcd_watch: WatchImage {
                enabled: shared.lcd_watch.enabled.load(Ordering::Acquire),
                range: shared.lcd_watch.range.load(Ordering::Acquire),
                writes: shared.lcd_watch.writes.load(Ordering::Acquire),
                pcs: shared
                    .lcd_watch
                    .pcs
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(&k, &v)| (k, v))
                    .collect(),
            },
            devices: shared
                .devices
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, v)| (k, v.clone()))
                .collect(),
            next_device: shared.next_device.load(Ordering::Acquire),
            lcd: *shared.lcd_object.lock().unwrap(),
            program: *shared.program_record.lock().unwrap(),
            input: shared.input_events.lock().unwrap().clone(),
            touch: shared.touch_events.lock().unwrap().clone(),
            system: shared.system_events.lock().unwrap().clone(),
            input_delivered: shared.input_delivered.load(Ordering::Acquire),
            input_thread: shared.input_thread.load(Ordering::Acquire),
            last_getevent_chain: *shared.last_getevent_chain.lock().unwrap(),
            event_filter: shared.event_type_filter.load(Ordering::Acquire),
            getevent_parked: shared.getevent_parked(),
            getevent_entries: shared.getevent_entries(),
            main_waiting: shared.main_waiting(),
            main_wait_entries: shared.main_wait_entries(),
            system_delivered: shared.system_events_delivered(),
            device_delivered: shared.device_timer_delivered(),
            touch_delivered: shared.touch_delivered(),
            threads: shared
                .threads
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            controls: shared
                .thread_control
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            stacks: shared
                .thread_stacks
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            next_thread: shared.next_thread_id.load(Ordering::Acquire),
            instructions: shared
                .thread_instructions
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            isrs: shared
                .isrs
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            disabled_isrs: shared
                .disabled_isrs
                .lock()
                .unwrap()
                .iter()
                .copied()
                .collect(),
            irq_delivered: shared
                .irq_delivered
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            pending_irqs: shared.pending_irqs.lock().unwrap().clone(),
            modules: shared.pe_modules.lock().unwrap().clone(),
            stub_block: *shared.stub_block.lock().unwrap(),
            sync: shared.sync_objects.lock().unwrap().image()?,
            vfs: fs::image::VfsImage::capture(&shared.vfs, mem, identity)?,
            owners,
        };
        drop(power);
        if !pause.is_current() {
            return Err("CPU pause expired during kernel capture".into());
        }
        image.validate(mem, identity)?;
        if !pause.is_current() {
            return Err("CPU pause expired during kernel image admission".into());
        }
        Ok(image)
    }
    fn validate(&self, mem: &GuestMemory, identity: &str) -> Result<()> {
        if self.version != 1
            || identity.is_empty()
            || identity.len() > 256
            || self.identity != identity
            || self.firmware_guest_path.len() > 4096
            || self.threads.len() > 128
            || self.owners.len() > 64
            || self.modules.len() > 128
            || self.devices.len() > 4096
            || self.input.len() > 4096
            || self.touch.len() > 4096
            || self.system.len() > 4096
            || self.next_thread > u64::from(u32::MAX) + 1
            || self.next_thread == 0
            || self.next_device > u64::from(u32::MAX) + 1
            || self.next_device == 0
        {
            return Err("Invalid kernel image identity, version or resource bounds".into());
        }
        self.environment.validate()?;
        let now = Duration::from_nanos(self.clock.elapsed_ns);
        if self.timers.system > now
            || self.timers.device > now
            || self.timers.irq > now
            || self.backlight.last_step > now
            || self.backlight.brightness > 3
            || !matches!(self.backlight.mode, 0 | 4)
        {
            return Err(
                "Kernel clock, power or peripheral state disagrees with its time base".into(),
            );
        }
        let scheduler = self.scheduler.restore()?;
        for (&id, record) in &self.threads {
            if id == 0
                || u64::from(id) >= self.next_thread
                || record.stack_size < 4096
                || record.stack_size > mem.heap_size()
                || !self.controls.contains_key(&id)
            {
                return Err("Invalid guest thread launch/control record".into());
            }
            mem.validate_range(record.entry & !1, 4)
                .map_err(|e| e.to_string())?;
        }
        if self
            .controls
            .keys()
            .any(|id| !self.threads.contains_key(id))
        {
            return Err("Thread control lacks its original launch record".into());
        }
        for (&id, owner) in &self.owners {
            if owner.thread_id != id
                || scheduler.priority(id).is_none()
                || (id != 0 && !self.stacks.contains_key(&id))
                || (id != 0
                    && !self
                        .controls
                        .get(&id)
                        .is_some_and(|c| c.started && !c.terminated && !c.completed))
            {
                return Err("Execution owner disagrees with its scheduler/thread record".into());
            }
        }
        for (&id, &address) in &self.stacks {
            let size = self
                .threads
                .get(&id)
                .ok_or("Worker stack has no thread record")?
                .stack_size;
            if !self.owners.contains_key(&id) || mem.heap_size_of(address).is_none_or(|n| n < size)
            {
                return Err("Worker stack has no live owner/allocation".into());
            }
        }
        for (&handle, name) in &self.devices {
            if handle == 0
                || u64::from(handle) >= self.next_device
                || !["ARCH", "BAT", "USB", "USBHOST", "INDICATOR", "BACK"]
                    .contains(&name.trim_start_matches(r"\\?\"))
            {
                return Err("Device handle/name is not an implemented driver".into());
            }
        }
        if let Some(va) = self.lcd {
            mem.validate_range(va, system2::LCD_STRUCT_SIZE)
                .map_err(|e| e.to_string())?;
        }
        if let Some(va) = self.program {
            mem.validate_range(va, 0x250).map_err(|e| e.to_string())?;
        }
        for &handler in self.isrs.values() {
            mem.validate_range(handler & !1, 4)
                .map_err(|e| e.to_string())?;
        }
        if self
            .disabled_isrs
            .iter()
            .any(|irq| !self.isrs.contains_key(irq))
        {
            return Err("IRQ mask has no registered handler".into());
        }
        if self.pending_irqs.len() > 65536
            || self
                .pending_irqs
                .iter()
                .map(|(irq, _)| *irq)
                .collect::<BTreeSet<_>>()
                .len()
                != self.pending_irqs.len()
        {
            return Err("Pending IRQ queue exceeds bounds or repeats an edge".into());
        }
        for &(_, handler) in &self.pending_irqs {
            mem.validate_range(handler & !1, 4)
                .map_err(|e| e.to_string())?;
        }
        if let Some(pattern) = &self.indicator.active {
            if pattern.colors.is_empty() || pattern.colors.len() > 1024 * 1024 || pattern.period >= 60
                || !self.owners.get(&pattern.owner).and_then(|o| o.pending.as_ref()).is_some_and(|p| matches!(p.wait, pending::Wait::Indicator { record } if record == pattern.record))
                || !self.power.sleeping || self.power.source != SleepSource::Indicator
            { return Err("Indicator pattern lacks its suspended execution owner".into()); }
            mem.validate_range(pattern.record, 40)
                .map_err(|e| e.to_string())?;
        } else if self.power.sleeping && self.power.source == SleepSource::Indicator {
            return Err("Indicator sleep has no pattern".into());
        }
        for module in &self.modules {
            if module.size == 0
                || module.entry_rva >= module.size
                || module.name.len() > 4096
                || module.guest_path.len() > 4096
                || module.exports_by_name.len() > 65536
                || module.exports_by_ordinal.len() > 65536
            {
                return Err("Invalid loaded-module image".into());
            }
            mem.validate_range(module.base, module.size as usize)
                .map_err(|e| e.to_string())?;
            if module.unresolved_imports.len() > 65536 {
                return Err("Too many unresolved PE imports".into());
            }
            for (trap, import) in &module.unresolved_imports {
                if trap % 4 != 0
                    || mem.heap_size_of(*trap).is_none_or(|size| size < 4)
                    || mem.read_u32(*trap).ok() != Some(pe::IMPORT_TRAP)
                    || import.dll.is_empty()
                    || import.dll.len() > 4096
                    || import.description().len() > 8192
                    || import.iat < module.base
                    || import
                        .iat
                        .checked_add(4)
                        .is_none_or(|end| end > module.base.saturating_add(module.size))
                {
                    return Err("Invalid named PE import trap".into());
                }
                mem.validate_range(import.iat, 4)
                    .map_err(|e| e.to_string())?;
            }
            for address in module
                .exports_by_name
                .values()
                .chain(module.exports_by_ordinal.values())
            {
                mem.validate_range(*address & !1, 2)
                    .map_err(|e| e.to_string())?;
            }
        }
        if let Some((base, next)) = self.stub_block {
            if next > 4096 {
                return Err("Import-stub cursor exceeds its allocation".into());
            }
            mem.validate_range(base, 4096).map_err(|e| e.to_string())?;
        }
        self.usb.restore(mem)?;
        self.clipboard.restore()?;
        self.text_calls.restore()?;
        let expected_range = self
            .lcd
            .map(|lcd| {
                let base = lcd + system2::LCD_BUFFER_OFF;
                (u64::from(base) << 32) | u64::from(base.saturating_add(320 * 240 * 4))
            })
            .unwrap_or(0);
        if self.lcd_watch.range != expected_range
            || self.lcd_watch.pcs.len() > LcdWatch::MAX_PCS
            || self
                .lcd_watch
                .pcs
                .values()
                .map(|n| u128::from(*n))
                .sum::<u128>()
                > u128::from(self.lcd_watch.writes)
        {
            return Err(
                "LCD-write observation image disagrees with its framebuffer/counters".into(),
            );
        }
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if body.len() > MAX_BYTES {
            return Err("Kernel image exceeds byte budget".into());
        }
        let mut bytes = Vec::with_capacity(80 + body.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(crate::sha256::hex(&body).as_bytes());
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 80
            || bytes.len() > MAX_BYTES + 80
            || &bytes[..8] != MAGIC
            || u64::from_le_bytes(bytes[8..16].try_into().unwrap()) != (bytes.len() - 80) as u64
            || crate::sha256::hex(&bytes[80..]).as_bytes() != &bytes[16..80]
        {
            return Err("Kernel image header, length or checksum mismatch".into());
        }
        serde_json::from_slice(&bytes[80..]).map_err(|e| e.to_string())
    }
    /// `data_dir` must be the admitted matching storage archive, not the live
    /// old profile. All reconstruction occurs in this new, unpublished owner.
    pub fn restore(
        &self,
        mem: Arc<GuestMemory>,
        data_dir: PathBuf,
        identity: &str,
    ) -> Result<RestoredKernel> {
        self.validate(&mem, identity)?;
        let clock = Arc::new(crate::clock::Clock::restore(self.clock.clone())?);
        let rtc = Arc::new(crate::clock::Rtc::restore(self.rtc.clone())?);
        let mut shared = SharedState::with_clocks(
            data_dir,
            self.firmware_guest_path.clone(),
            clock.clone(),
            rtc,
        );
        let startup = vec![clock.freeze(None), shared.rtc.clock.freeze(None)];
        shared.environment = Mutex::new(self.environment.clone());
        shared
            .vfs
            .set_capacity(self.environment.storage_capacity_bytes)?;
        shared.scheduler = Arc::new(self.scheduler.restore()?);
        *shared.power.lock().unwrap() = Power {
            sleeping: self.power.sleeping,
            wake: self.power.wake,
            swallow_on_up: self.power.swallow_on_up,
            sleeps: self.power.sleeps,
            source: self.power.source,
            auto: self.power.auto.restore(clock.clone())?,
            clock_pause: self.power.sleeping.then(|| clock.freeze(None)),
        };
        shared.timers = Mutex::new(self.timers.clone());
        shared.backlight = Mutex::new(self.backlight.clone());
        shared.indicator = Mutex::new(self.indicator.clone());
        shared.usb = self.usb.restore(&mem)?;
        shared.clipboard = Arc::new(self.clipboard.restore()?);
        shared.text_calls = Arc::new(self.text_calls.restore()?);
        shared
            .lcd_watch
            .enabled
            .store(self.lcd_watch.enabled, Ordering::Release);
        shared
            .lcd_watch
            .range
            .store(self.lcd_watch.range, Ordering::Release);
        shared
            .lcd_watch
            .writes
            .store(self.lcd_watch.writes, Ordering::Release);
        *shared.lcd_watch.pcs.lock().unwrap() = self.lcd_watch.pcs.clone().into_iter().collect();
        shared.devices = Mutex::new(self.devices.clone().into_iter().collect());
        shared
            .next_device
            .store(self.next_device, Ordering::Release);
        *shared.lcd_object.lock().unwrap() = self.lcd;
        if let Some(lcd) = self.lcd {
            shared
                .lcd_watch
                .set_range(lcd + system2::LCD_BUFFER_OFF, 320 * 240 * 4);
        }
        *shared.program_record.lock().unwrap() = self.program;
        shared.input_events = Mutex::new(self.input.clone());
        shared.touch_events = Mutex::new(self.touch.clone());
        shared.system_events = Mutex::new(self.system.clone());
        shared
            .input_delivered
            .store(self.input_delivered, Ordering::Release);
        shared
            .input_thread
            .store(self.input_thread, Ordering::Release);
        *shared.last_getevent_chain.lock().unwrap() = self.last_getevent_chain;
        shared
            .event_type_filter
            .store(self.event_filter, Ordering::Release);
        shared
            .getevent_parked
            .store(self.getevent_parked, Ordering::Release);
        shared
            .getevent_entries
            .store(self.getevent_entries, Ordering::Release);
        shared
            .main_waiting
            .store(self.main_waiting, Ordering::Release);
        shared
            .main_wait_entries
            .store(self.main_wait_entries, Ordering::Release);
        shared
            .system_events_delivered
            .store(self.system_delivered, Ordering::Release);
        shared
            .device_timer_delivered
            .store(self.device_delivered, Ordering::Release);
        shared
            .touch_delivered
            .store(self.touch_delivered, Ordering::Release);
        shared.threads = Mutex::new(self.threads.clone().into_iter().collect());
        shared.thread_control = Mutex::new(self.controls.clone().into_iter().collect());
        shared.thread_stacks = Mutex::new(self.stacks.clone().into_iter().collect());
        shared
            .next_thread_id
            .store(self.next_thread, Ordering::Release);
        shared.thread_instructions = Mutex::new(self.instructions.clone().into_iter().collect());
        shared.isrs = Mutex::new(self.isrs.clone().into_iter().collect());
        shared.disabled_isrs = Mutex::new(self.disabled_isrs.clone().into_iter().collect());
        shared.irq_delivered = Mutex::new(self.irq_delivered.clone().into_iter().collect());
        shared.pending_irqs = Mutex::new(self.pending_irqs.clone());
        shared.pe_modules = Mutex::new(self.modules.clone());
        *shared.stub_block.lock().unwrap() = self.stub_block;
        self.vfs.restore(&mut shared.vfs, &mem, identity)?;
        let mut synchronization = self.sync.restore(mem)?;
        synchronization.install_handles(&shared)?;
        Ok(RestoredKernel {
            shared,
            synchronization,
            owners: self.owners.clone(),
            startup,
            pending_starts: self
                .threads
                .iter()
                .filter_map(|(&id, &record)| {
                    let control = self.controls.get(&id)?;
                    (control.started
                        && !control.terminated
                        && !control.completed
                        && !self.owners.contains_key(&id))
                    .then_some((id, record))
                })
                .collect(),
        })
    }
}
