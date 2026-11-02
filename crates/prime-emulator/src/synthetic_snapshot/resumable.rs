//! Reconstructable services for self-authored synthetic ARM code only.
//!
//! SVC0x190001 waits on one logical event port; SVC0x190002 receives an event.
//! These are deliberately separate from the calculator firmware ABI. A real
//! Unicorn SVC trap stops execution; all completion happens after its callback
//! has returned. Only explicit data is saved, never a native stack or callback.
use super::*;

const SERVICE_KIND: &str = "prime.synthetic-resumable-services";
const SERVICE_MAGIC: &[u8; 8] = b"HPUSVC01";
const WAIT: u32 = 0x190001;
const RECEIVE: u32 = 0x190002;
const MAX_TICK: u64 = 1 << 48;
const MAX_TIMEOUT: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitTicket {
    sequence: u64,
    binding_epoch: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticEvent {
    pub kind: u32,
    pub value: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceStop {
    Runnable,
    Waiting(WaitTicket),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    ticket: u64,
    operation: u32,
    return_pc: u32,
    arguments: [u32; 2],
    started_at: u64,
    deadline: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Services {
    resource_identity: String,
    binding_epoch: u64,
    logical_time: u64,
    next_ticket: u64,
    completed: u64,
    pending: Option<Pending>,
    queued: Option<SyntheticEvent>,
}
impl Services {
    fn validate(&self, program: &SyntheticProgram, cpu: &CpuState) -> Result<()> {
        if program.thumb || cpu.cpsr & 0x20 != 0 {
            return Err("resumable synthetic services require ARM code and CPU state".into());
        }
        if self.binding_epoch == 0
            || self.resource_identity.is_empty()
            || self.resource_identity.len() > 128
            || self.logical_time >= MAX_TICK
            || self.next_ticket
                != self
                    .completed
                    .checked_add(if self.pending.is_some() { 2 } else { 1 })
                    .ok_or("service counter overflow")?
        {
            return Err("invalid synthetic service identity/time/counters".into());
        }
        if let Some(pending) = &self.pending {
            if pending.ticket != self.next_ticket - 1
                || pending.return_pc != cpu.registers[15]
                || pending.arguments != [cpu.registers[0], cpu.registers[1]]
                || pending.deadline <= self.logical_time
                || pending.deadline >= MAX_TICK
                || pending.deadline - self.logical_time > u64::from(MAX_TIMEOUT)
                || self.queued.is_some()
            {
                return Err(
                    "pending synthetic operation disagrees with CPU, timer or queue".into(),
                );
            }
            let offset = pending
                .return_pc
                .checked_sub(CODE_BASE + 4)
                .ok_or("invalid service return PC")? as usize;
            let expected = 0xef00_0000 | pending.operation;
            if !matches!(pending.operation, WAIT | RECEIVE)
                || program.code.get(offset..offset + 4) != Some(expected.to_le_bytes().as_slice())
            {
                return Err("unsupported pending synthetic SVC or changed instruction".into());
            }
            let timeout = check_arguments(pending.operation, pending.arguments)?;
            if pending.started_at > self.logical_time
                || pending.started_at.checked_add(u64::from(timeout)) != Some(pending.deadline)
            {
                return Err("pending service deadline disagrees with its saved timer".into());
            }
        }
        Ok(())
    }
}
fn check_arguments(operation: u32, arguments: [u32; 2]) -> Result<u32> {
    let timeout = match operation {
        WAIT if arguments[1] == 1 => arguments[0],
        RECEIVE if arguments[0] % 4 == 0 && within(arguments[0], 8, DATA_BASE) => arguments[1],
        _ => return Err("unsupported synthetic service arguments/resource".into()),
    };
    if timeout == 0 || timeout > MAX_TIMEOUT {
        return Err("synthetic wait timeout must be1..1000000 logical ticks".into());
    }
    Ok(timeout)
}

/// One CPU owner, one logical event port, one bounded pending operation.
/// The caller supplies the logical resource identity and a new binding epoch.
/// This owner creates no OS resources and never restores a saved native handle.
pub struct ResumableGuest {
    guest: SyntheticGuest,
    services: Services,
}
impl ResumableGuest {
    pub fn new(
        program: &SyntheticProgram,
        binding: &str,
        logical_time: u64,
        binding_epoch: u64,
    ) -> Result<Self> {
        if program.thumb {
            return Err("resumable synthetic services currently require ARM code".into());
        }
        let mut guest = SyntheticGuest::new(program)?;
        guest.engine.get_data_mut().services = true;
        let services = Services {
            resource_identity: binding.into(),
            binding_epoch,
            logical_time,
            next_ticket: 1,
            completed: 0,
            pending: None,
            queued: None,
        };
        services.validate(program, &guest.state()?)?;
        Ok(Self { guest, services })
    }
    pub fn set_register(&mut self, index: usize, value: u32) -> Result<()> {
        if self.services.pending.is_some() {
            return Err("cannot edit a pending service context".into());
        }
        self.guest.set_register(index, value)
    }
    pub fn state(&self) -> Result<CpuState> {
        self.guest.state()
    }
    pub fn read_word(&self, address: u32) -> Result<u32> {
        self.guest.read_word(address)
    }
    pub fn pending(&self) -> Option<WaitTicket> {
        self.services.pending.as_ref().map(|p| WaitTicket {
            sequence: p.ticket,
            binding_epoch: self.services.binding_epoch,
        })
    }
    pub fn run_slice(&mut self, count: usize) -> Result<ServiceStop> {
        if count == 0 || count > MAX_SLICE {
            return Err("synthetic service slice exceeds bound".into());
        }
        if let Some(ticket) = self.pending() {
            return Ok(ServiceStop::Waiting(ticket));
        }
        let Some(trap) = self.guest.execute(count, true)? else {
            return Ok(ServiceStop::Runnable);
        };
        let result = (|| {
            let timeout = check_arguments(trap.number, trap.arguments)?;
            let deadline = self
                .services
                .logical_time
                .checked_add(u64::from(timeout))
                .filter(|t| *t < MAX_TICK)
                .ok_or("service deadline overflow")?;
            let ticket = self.services.next_ticket;
            self.services.next_ticket = self
                .services
                .next_ticket
                .checked_add(1)
                .ok_or("service ticket overflow")?;
            self.services.pending = Some(Pending {
                ticket,
                operation: trap.number,
                return_pc: trap.return_pc,
                arguments: trap.arguments,
                started_at: self.services.logical_time,
                deadline,
            });
            if let Some(event) = self.services.queued.take() {
                self.finish(Some(event))?;
                return Ok(ServiceStop::Runnable);
            }
            Ok(ServiceStop::Waiting(
                self.pending().ok_or("service ticket missing")?,
            ))
        })();
        if result.is_err() {
            self.guest.failed = true;
        }
        result
    }
    /// Ticket matching prevents stale or duplicate completion. Virtual time must
    /// be explicitly reconstructed before delivery; host wall time is irrelevant.
    pub fn deliver(
        &mut self,
        ticket: WaitTicket,
        event: SyntheticEvent,
        logical_time: u64,
    ) -> Result<()> {
        if self.guest.failed
            || self.pending() != Some(ticket)
            || logical_time != self.services.logical_time
        {
            return Err("stale/double synthetic completion or timer disagreement".into());
        }
        self.finish(Some(event))
    }
    pub fn advance_time(&mut self, logical_time: u64) -> Result<bool> {
        if self.guest.failed
            || logical_time < self.services.logical_time
            || logical_time >= MAX_TICK
        {
            return Err("invalid synthetic logical time".into());
        }
        self.services.logical_time = logical_time;
        if self
            .services
            .pending
            .as_ref()
            .is_some_and(|p| p.deadline <= logical_time)
        {
            self.finish(None)?;
            return Ok(true);
        }
        Ok(false)
    }
    fn finish(&mut self, event: Option<SyntheticEvent>) -> Result<()> {
        let result = self.finish_inner(event);
        if result.is_err() {
            self.guest.failed = true;
        }
        result
    }
    fn finish_inner(&mut self, event: Option<SyntheticEvent>) -> Result<()> {
        let pending = self
            .services
            .pending
            .as_ref()
            .ok_or("no pending synthetic service")?;
        if let Some(event) = event {
            if pending.operation == RECEIVE {
                let data = [event.kind.to_le_bytes(), event.value.to_le_bytes()].concat();
                self.guest
                    .engine
                    .mem_write(u64::from(pending.arguments[0]), &data)
                    .map_err(string)?;
            } else {
                self.services.queued = Some(event);
            }
        }
        self.guest
            .engine
            .reg_write(RegisterARM::R0, u64::from(event.is_some()))
            .map_err(string)?;
        self.services.completed += 1;
        self.services.pending = None;
        Ok(())
    }
    pub fn capture(&mut self, program: &SyntheticProgram) -> Result<ResumableSnapshot> {
        let cpu = self.guest.capture(program)?;
        let snapshot = ResumableSnapshot {
            cpu,
            services: self.services.clone(),
        };
        snapshot.validate(program)?;
        Ok(snapshot)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    kind: String,
    version: u32,
    calculator_live_resume: bool,
    cpu: Manifest,
    services: Services,
    checksum: String,
}
impl Envelope {
    fn digest(&self) -> Result<String> {
        Ok(sha256::hex(
            &serde_json::to_vec(&(
                &self.kind,
                self.version,
                self.calculator_live_resume,
                &self.cpu,
                &self.services,
            ))
            .map_err(string)?,
        ))
    }
}
/// A separate format; the M18 CPU-only v1 snapshot contract is unchanged.
pub struct ResumableSnapshot {
    cpu: SyntheticSnapshot,
    services: Services,
}
impl ResumableSnapshot {
    fn validate(&self, program: &SyntheticProgram) -> Result<()> {
        self.cpu.validate(program)?;
        self.services.validate(program, self.cpu.state())
    }
    pub fn pending(&self) -> Option<WaitTicket> {
        self.services.pending.as_ref().map(|p| WaitTicket {
            sequence: p.ticket,
            binding_epoch: self.services.binding_epoch,
        })
    }
    pub fn logical_time(&self) -> u64 {
        self.services.logical_time
    }
    pub fn binding_epoch(&self) -> u64 {
        self.services.binding_epoch
    }
    pub fn restore(
        &self,
        program: &SyntheticProgram,
        binding: &str,
        logical_time: u64,
        binding_epoch: u64,
    ) -> Result<ResumableGuest> {
        self.validate(program)?;
        if binding != self.services.resource_identity
            || logical_time != self.services.logical_time
            || binding_epoch <= self.services.binding_epoch
        {
            return Err("synthetic resource rebinding failed or timer state disagrees".into());
        }
        let mut guest = self.cpu.restore(program)?;
        #[cfg(test)]
        tests::interrupt_restore()?;
        guest.engine.get_data_mut().services = true;
        let mut services = self.services.clone();
        services.binding_epoch = binding_epoch;
        Ok(ResumableGuest { guest, services })
    }
    pub fn save(&self, path: &Path, program: &SyntheticProgram) -> Result<()> {
        self.validate(program)?;
        let mut envelope = Envelope {
            kind: SERVICE_KIND.into(),
            version: 1,
            calculator_live_resume: false,
            cpu: self.cpu.manifest.clone(),
            services: self.services.clone(),
            checksum: String::new(),
        };
        envelope.checksum = envelope.digest()?;
        let json = serde_json::to_vec(&envelope).map_err(string)?;
        if json.len() > MAX_MANIFEST {
            return Err("synthetic service manifest exceeds bound".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut staging = tempfile::NamedTempFile::new_in(parent).map_err(string)?;
        staging.write_all(SERVICE_MAGIC).map_err(string)?;
        staging
            .write_all(&(json.len() as u64).to_le_bytes())
            .map_err(string)?;
        #[cfg(test)]
        tests::interrupt_save(path)?;
        staging.write_all(&json).map_err(string)?;
        staging.write_all(&self.cpu.bytes).map_err(string)?;
        staging.as_file().sync_all().map_err(string)?;
        staging.persist(path).map_err(string)?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(string)?;
        Ok(())
    }
    pub fn load(path: &Path, program: &SyntheticProgram) -> Result<Self> {
        let metadata = fs::symlink_metadata(path).map_err(string)?;
        if !metadata.is_file() || metadata.len() > (16 + MAX_MANIFEST + PAYLOAD_BYTES) as u64 {
            return Err("synthetic service snapshot is not a bounded regular file".into());
        }
        let mut file = fs::File::open(path).map_err(string)?;
        let mut header = [0; 16];
        file.read_exact(&mut header).map_err(string)?;
        let length = u64::from_le_bytes(header[8..].try_into().unwrap());
        if &header[..8] != SERVICE_MAGIC
            || length == 0
            || length > MAX_MANIFEST as u64
            || metadata.len() != 16 + length + PAYLOAD_BYTES as u64
        {
            return Err("synthetic service snapshot header/length invalid".into());
        }
        let mut json = vec![0; length as usize];
        file.read_exact(&mut json).map_err(string)?;
        let envelope: Envelope = serde_json::from_slice(&json).map_err(string)?;
        if envelope.kind != SERVICE_KIND
            || envelope.version != 1
            || envelope.calculator_live_resume
            || envelope.checksum != envelope.digest()?
        {
            return Err("synthetic service snapshot kind/version/checksum invalid".into());
        }
        let mut bytes = vec![0; PAYLOAD_BYTES];
        file.read_exact(&mut bytes).map_err(string)?;
        let mut tail = [0];
        if file.read(&mut tail).map_err(string)? != 0 {
            return Err("service snapshot changed length during read".into());
        }
        let snapshot = Self {
            cpu: SyntheticSnapshot {
                manifest: envelope.cpu,
                bytes,
            },
            services: envelope.services,
        };
        snapshot.validate(program)?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests;
