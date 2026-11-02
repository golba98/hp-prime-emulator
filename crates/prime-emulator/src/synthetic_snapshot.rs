//! Portable continuation for a deliberately small synthetic ARM926 guest.
//!
//! This owner has no firmware hooks, native callbacks, devices, MMU, FPU or guest
//! threads. Only whitelisted integer ARM/Thumb instructions execute. Its complete
//! architectural state is GPRs/CPSR and three fixed memory regions. This is not a
//! calculator-session snapshot and does not change firmware recovery admission.
use crate::sha256;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use unicorn_engine::{
    ArmCpuModel, RegisterARM, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

mod instructions;
mod store;
pub use store::{SnapshotSelection, SyntheticSnapshotStore};

pub const CODE_BASE: u32 = 0x1000_0000;
pub const DATA_BASE: u32 = 0x2000_0000;
pub const STACK_BASE: u32 = 0x3000_0000;
pub const REGION_BYTES: usize = 64 * 1024;
pub const MAX_SLICE: usize = 100_000;
const PAYLOAD_BYTES: usize = REGION_BYTES * 3;
const MAX_MANIFEST: usize = 64 * 1024;
const MAGIC: &[u8; 8] = b"HPUSYN01";
const KIND: &str = "prime.synthetic-arm926";
const VERSION: u32 = 1;
const REGS: [RegisterARM; 16] = [
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
];
type Result<T> = std::result::Result<T, String>;

/// The caller supplies the original self-authored program again during restore.
/// It is never inferred from an arbitrary snapshot or proprietary firmware image.
pub struct SyntheticProgram {
    code: Vec<u8>,
    thumb: bool,
    identity: String,
}
impl SyntheticProgram {
    pub fn new(code: &[u8], thumb: bool) -> Result<Self> {
        let alignment = if thumb { 2 } else { 4 };
        if code.is_empty() || code.len() > REGION_BYTES || code.len() % alignment != 0 {
            return Err("synthetic code must be nonempty, aligned and at most 64 KiB".into());
        }
        let identity =
            sha256::hex(&serde_json::to_vec(&(KIND, VERSION, thumb, code)).map_err(string)?);
        Ok(Self {
            code: code.to_vec(),
            thumb,
            identity,
        })
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuState {
    pub registers: [u32; 16],
    pub cpsr: u32,
    pub instructions: u64,
}
impl CpuState {
    fn validate(&self) -> Result<()> {
        // User mode, little endian, ARM/Thumb, NZCV and I/F only. No banked,
        // exception, Jazelle, saturation, endian or extension state is omitted.
        if self.cpsr & 0x1f != 0x10 || self.cpsr & !0xf000_00ff != 0 {
            return Err("synthetic snapshot supports only integer user-mode ARM/Thumb CPSR".into());
        }
        let alignment = if self.cpsr & 0x20 != 0 { 2 } else { 4 };
        if self.registers[15] % alignment != 0
            || !within(self.registers[15], alignment as usize, CODE_BASE)
        {
            return Err("synthetic PC is unaligned or outside executable code".into());
        }
        if self.registers[13] % 4 != 0
            || self.registers[13] < STACK_BASE
            || self.registers[13] > STACK_BASE + REGION_BYTES as u32
        {
            return Err("synthetic SP is unaligned or outside the stack".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    kind: String,
    version: u32,
    calculator_live_resume: bool,
    cpu: String,
    unicorn_version: u32,
    resource_identity: String,
    regions: [(u32, usize, String); 3],
    state: CpuState,
    payload_sha256: String,
    manifest_sha256: String,
}
impl Manifest {
    fn digest(&self) -> Result<String> {
        Ok(sha256::hex(
            &serde_json::to_vec(&(
                &self.kind,
                self.version,
                self.calculator_live_resume,
                &self.cpu,
                self.unicorn_version,
                &self.resource_identity,
                &self.regions,
                &self.state,
                &self.payload_sha256,
            ))
            .map_err(string)?,
        ))
    }
}
/// A complete snapshot only for the fixed synthetic machine contract above.
pub struct SyntheticSnapshot {
    manifest: Manifest,
    bytes: Vec<u8>,
}
impl SyntheticSnapshot {
    pub fn state(&self) -> &CpuState {
        &self.manifest.state
    }
    fn validate(&self, program: &SyntheticProgram) -> Result<()> {
        let m = &self.manifest;
        if m.kind != KIND
            || m.version != VERSION
            || m.calculator_live_resume
            || m.cpu != "ARM926"
            || m.unicorn_version != runtime_version()
        {
            return Err("incompatible synthetic snapshot kind/version/CPU/backend or calculator resume claim".into());
        }
        if m.resource_identity != program.identity || m.regions != regions() {
            return Err(
                "synthetic snapshot resource identity or fixed memory layout mismatch".into(),
            );
        }
        m.state.validate()?;
        if self.bytes.len() != PAYLOAD_BYTES
            || m.manifest_sha256 != m.digest()?
            || sha256::hex(&self.bytes) != m.payload_sha256
            || self.bytes[..program.code.len()] != program.code
            || self.bytes[program.code.len()..REGION_BYTES]
                .iter()
                .any(|&x| x != 0)
        {
            return Err("synthetic snapshot checksum, payload or original code mismatch".into());
        }
        Ok(())
    }
    /// Private stable directories required. No device/FIFO/symlink input is opened.
    pub fn load(path: &Path, program: &SyntheticProgram) -> Result<Self> {
        let meta = fs::symlink_metadata(path).map_err(string)?;
        if !meta.is_file() || meta.len() > (16 + MAX_MANIFEST + PAYLOAD_BYTES) as u64 {
            return Err("synthetic snapshot is not a bounded regular file".into());
        }
        let mut f = fs::File::open(path).map_err(string)?;
        let mut header = [0; 16];
        f.read_exact(&mut header).map_err(string)?;
        let len = u64::from_le_bytes(header[8..16].try_into().unwrap());
        if &header[..8] != MAGIC
            || len == 0
            || len > MAX_MANIFEST as u64
            || meta.len() != 16 + len + PAYLOAD_BYTES as u64
        {
            return Err("synthetic snapshot header/length invalid".into());
        }
        let mut json = vec![0; len as usize];
        f.read_exact(&mut json).map_err(string)?;
        let manifest = serde_json::from_slice(&json).map_err(string)?;
        let mut bytes = vec![0; PAYLOAD_BYTES];
        f.read_exact(&mut bytes).map_err(string)?;
        let mut tail = [0];
        if f.read(&mut tail).map_err(string)? != 0 {
            return Err("synthetic snapshot changed length during read".into());
        }
        let snapshot = Self { manifest, bytes };
        snapshot.validate(program)?;
        Ok(snapshot)
    }
    /// Atomic replacement: failed staging writes cannot overwrite the prior save.
    pub fn save(&self, path: &Path, program: &SyntheticProgram) -> Result<()> {
        self.validate(program)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut staging = tempfile::NamedTempFile::new_in(parent).map_err(string)?;
        let json = serde_json::to_vec(&self.manifest).map_err(string)?;
        if json.len() > MAX_MANIFEST {
            return Err("synthetic manifest exceeds bound".into());
        }
        staging.write_all(MAGIC).map_err(string)?;
        staging
            .write_all(&(json.len() as u64).to_le_bytes())
            .map_err(string)?;
        #[cfg(test)]
        tests::interrupt_staged_save(path)?;
        staging.write_all(&json).map_err(string)?;
        staging.write_all(&self.bytes).map_err(string)?;
        staging.as_file().sync_all().map_err(string)?;
        staging.persist(path).map_err(string)?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(string)?;
        Ok(())
    }
    /// All validation precedes constructing an entirely new machine owner.
    pub fn restore(&self, program: &SyntheticProgram) -> Result<SyntheticGuest> {
        self.validate(program)?;
        let mut guest = SyntheticGuest::new(program)?;
        for ((base, _, _), bytes) in regions().iter().zip(self.bytes.chunks_exact(REGION_BYTES)) {
            guest
                .engine
                .mem_write(u64::from(*base), bytes)
                .map_err(string)?;
        }
        guest
            .engine
            .reg_write(RegisterARM::CPSR, u64::from(self.manifest.state.cpsr))
            .map_err(string)?;
        for (reg, value) in REGS.iter().zip(self.manifest.state.registers) {
            guest
                .engine
                .reg_write(*reg, u64::from(value))
                .map_err(string)?;
        }
        guest.instructions = self.manifest.state.instructions;
        // Writing PC can select instruction-set state in some engine versions.
        guest
            .engine
            .reg_write(RegisterARM::CPSR, u64::from(self.manifest.state.cpsr))
            .map_err(string)?;
        if guest.state()? != self.manifest.state {
            return Err("synthetic restored register state differs".into());
        }
        Ok(guest)
    }
}

#[derive(Default)]
struct HookState {
    executed: usize,
    error: Option<String>,
}
/// Synchronous, uniquely owned synthetic CPU. `&mut self` is the checkpoint
/// barrier; no engine, shared backing store or callback escapes this owner.
pub struct SyntheticGuest {
    engine: Unicorn<'static, HookState>,
    identity: String,
    instructions: u64,
    failed: bool,
}
impl SyntheticGuest {
    pub fn new(program: &SyntheticProgram) -> Result<Self> {
        let mut engine =
            Unicorn::new_with_data(Arch::ARM, Mode::ARM, HookState::default()).map_err(string)?;
        engine
            .ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)
            .map_err(string)?;
        for (base, _, _) in regions() {
            let permissions = if base == CODE_BASE {
                Prot::READ | Prot::EXEC
            } else {
                Prot::READ | Prot::WRITE
            };
            engine
                .mem_map(u64::from(base), REGION_BYTES as u64, permissions)
                .map_err(string)?;
        }
        engine
            .mem_write(u64::from(CODE_BASE), &program.code)
            .map_err(string)?;
        engine
            .reg_write(RegisterARM::CPSR, if program.thumb { 0xf0 } else { 0xd0 })
            .map_err(string)?;
        engine
            .reg_write(RegisterARM::SP, u64::from(STACK_BASE) + REGION_BYTES as u64)
            .map_err(string)?;
        engine
            .reg_write(RegisterARM::PC, u64::from(CODE_BASE))
            .map_err(string)?;
        // Unicorn PC writes may clear T; apply the authoritative state last.
        engine
            .reg_write(RegisterARM::CPSR, if program.thumb { 0xf0 } else { 0xd0 })
            .map_err(string)?;
        engine
            .add_code_hook(1, 0, |uc, address, size| {
                let valid = (|| -> Result<()> {
                    let cpsr = uc.reg_read(RegisterARM::CPSR).map_err(string)? as u32;
                    if cpsr & 0x1f != 0x10 || cpsr & !0xf000_00ff != 0 {
                        return Err("synthetic guest left supported user state".into());
                    }
                    if address > u64::from(u32::MAX)
                        || !within(address as u32, size as usize, CODE_BASE)
                    {
                        return Err("synthetic execution escaped code region".into());
                    }
                    let mut bytes = [0; 4];
                    if size != 2 && size != 4 {
                        return Err("unsupported instruction length".into());
                    }
                    uc.mem_read(address, &mut bytes[..size as usize])
                        .map_err(string)?;
                    if !instructions::allowed(&bytes, cpsr & 0x20 != 0, size) {
                        return Err(format!("unsupported synthetic instruction at {address:#x}"));
                    }
                    Ok(())
                })();
                if let Err(error) = valid {
                    uc.get_data_mut().error = Some(error);
                    let _ = uc.emu_stop();
                } else {
                    uc.get_data_mut().executed += 1;
                }
            })
            .map_err(string)?;
        engine
            .add_intr_hook(|uc, number| {
                uc.get_data_mut().error =
                    Some(format!("synthetic exception {number} is unsupported"));
                let _ = uc.emu_stop();
            })
            .map_err(string)?;
        Ok(Self {
            engine,
            identity: program.identity.clone(),
            instructions: 0,
            failed: false,
        })
    }
    pub fn set_register(&mut self, index: usize, value: u32) -> Result<()> {
        if index >= 15 || self.failed {
            return Err("unsupported synthetic register or failed guest".into());
        }
        self.engine
            .reg_write(REGS[index], u64::from(value))
            .map_err(string)
    }
    pub fn read_word(&self, address: u32) -> Result<u32> {
        if !within(address, 4, DATA_BASE) && !within(address, 4, STACK_BASE) {
            return Err("synthetic read is outside data/stack".into());
        }
        let mut bytes = [0; 4];
        self.engine
            .mem_read(u64::from(address), &mut bytes)
            .map_err(string)?;
        Ok(u32::from_le_bytes(bytes))
    }
    pub fn state(&self) -> Result<CpuState> {
        let mut registers = [0; 16];
        for (reg, value) in REGS.iter().zip(registers.iter_mut()) {
            *value = self.engine.reg_read(*reg).map_err(string)? as u32;
        }
        Ok(CpuState {
            registers,
            cpsr: self.engine.reg_read(RegisterARM::CPSR).map_err(string)? as u32,
            instructions: self.instructions,
        })
    }
    /// At most 100,000 integer instructions and one second per slice. Any
    /// unsupported operation/fault/short execution poisons snapshot admission.
    pub fn run_slice(&mut self, count: usize) -> Result<()> {
        if self.failed || count == 0 || count > MAX_SLICE {
            return Err("synthetic slice unavailable or exceeds instruction bound".into());
        }
        let next_count = self
            .instructions
            .checked_add(count as u64)
            .ok_or("synthetic instruction counter overflow")?;
        let state = self.state()?;
        state.validate()?;
        let start = u64::from(state.registers[15] | ((state.cpsr >> 5) & 1));
        self.engine.get_data_mut().executed = 0;
        let result = self
            .engine
            .emu_start(start, 0, 1_000_000, count)
            .map_err(string);
        let executed = self.engine.get_data().executed;
        let failure = self
            .engine
            .get_data_mut()
            .error
            .take()
            .or_else(|| result.err())
            .or_else(|| {
                (executed != count).then(|| "synthetic slice timed out or stopped early".into())
            });
        if let Some(error) = failure {
            self.failed = true;
            return Err(error);
        }
        self.instructions = next_count;
        self.state()?.validate()?;
        Ok(())
    }
    pub fn capture(&mut self, program: &SyntheticProgram) -> Result<SyntheticSnapshot> {
        if self.failed || self.identity != program.identity {
            return Err("synthetic snapshot requires a healthy, matching exclusive owner".into());
        }
        let state = self.state()?;
        state.validate()?;
        let mut bytes = vec![0; PAYLOAD_BYTES];
        for ((base, _, _), bytes) in regions().iter().zip(bytes.chunks_exact_mut(REGION_BYTES)) {
            self.engine
                .mem_read(u64::from(*base), bytes)
                .map_err(string)?;
        }
        let mut manifest = Manifest {
            kind: KIND.into(),
            version: VERSION,
            calculator_live_resume: false,
            cpu: "ARM926".into(),
            unicorn_version: runtime_version(),
            resource_identity: self.identity.clone(),
            regions: regions(),
            state,
            payload_sha256: sha256::hex(&bytes),
            manifest_sha256: String::new(),
        };
        manifest.manifest_sha256 = manifest.digest()?;
        let snapshot = SyntheticSnapshot { manifest, bytes };
        snapshot.validate(program)?;
        Ok(snapshot)
    }
}
fn regions() -> [(u32, usize, String); 3] {
    [
        (CODE_BASE, REGION_BYTES, "rx".into()),
        (DATA_BASE, REGION_BYTES, "rw".into()),
        (STACK_BASE, REGION_BYTES, "rw".into()),
    ]
}
fn within(address: u32, length: usize, base: u32) -> bool {
    address >= base
        && address
            .checked_add(length as u32)
            .is_some_and(|end| end <= base + REGION_BYTES as u32)
}
fn runtime_version() -> u32 {
    // SAFETY: Unicorn explicitly permits null output pointers and returns its
    // packed runtime version. No engine context or pointer is persisted.
    unsafe { unicorn_engine::uc_version(std::ptr::null_mut(), std::ptr::null_mut()) }
}
fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
