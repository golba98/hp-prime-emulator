//! Firmware loading: the real HP Prime G1 `armfir.elf` (ELF32, ARM, EABI5)
//! plus the static guest memory map verified during Phase 1
//! (docs/emulator/FIRMWARE_ARCHITECTURE.md, docs/emulator/PRIMU2_SOURCE_AUDIT.md).

use std::fs;
use std::path::Path;

use goblin::Object;
use goblin::elf::program_header::{PF_R, PF_W, PF_X, PT_LOAD};

/// Static guest regions (Phase 1-verified; see PRIMU2_SOURCE_AUDIT.md §2).
/// Names and sizes follow the reference emulator's `common.h`.
pub mod regions {
    pub const NULL_PAGE: u32 = 0x0000_0000;
    pub const NULL_PAGE_SIZE: usize = 0x1000;

    pub const STACK: u32 = 0x1000_0000;
    pub const HEAP: u32 = 0x2000_0000;
    /// PrimU2 pre-maps 0x10000000 bytes; the header comment there says
    /// "32MB" — the actual constant is 256 MiB and that is what boots.
    pub const HEAP_SIZE: usize = 0x1000_0000;

    pub const OS_ROM: u32 = 0x3000_0000;
    pub const OS_ROM_MAX: usize = 0x10_0000; // 1 MiB, the shipped image size

    pub const EXCEPTION_PAGE: u32 = 0x31FF_F000;
    pub const EXCEPTION_PAGE_SIZE: usize = 0x1000;

    pub const INTC: u32 = 0x4A00_0000; // S3C24xx interrupt controller
    pub const INTC_SIZE: usize = 0x20;

    pub const LCD_REGS: u32 = 0x4C80_0000; // LCD register block (PrimU2 maps .. 0x4E000000)
    pub const LCD_REGS_SIZE: usize = 0x4E00_0000 - 0x4C80_0000;

    pub const PWM_TIMERS: u32 = 0x5100_0000; // S3C2416 timer block (TCFG0..TCNTO4)
    pub const PWM_TIMERS_SIZE: usize = 0x1000;

    pub const RTC: u32 = 0x5700_0000;
    pub const RTC_SIZE: usize = 0x1000;
}

#[derive(Debug)]
pub struct FirmwareError(pub String);

impl std::fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "firmware load failed: {}", self.0)
    }
}
impl std::error::Error for FirmwareError {}

/// One loadable segment of the firmware image.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    /// File offset of the segment contents (p_offset).
    pub offset: u64,
    pub vaddr: u32,
    pub file_size: usize,
    pub mem_size: usize,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

/// A parsed firmware ELF.
pub struct ElfImage {
    pub entry: u32,
    pub segments: Vec<Segment>,
    pub file_size: u64,
    pub bytes: Vec<u8>,
}

impl ElfImage {
    /// Parse and validate the firmware ELF. Rejects anything that is not an
    /// ELF32 little-endian ARM executable — the real firmware is exactly
    /// that (Phase 1, readelf + goblin verification).
    pub fn load(path: &Path) -> Result<ElfImage, FirmwareError> {
        let bytes =
            fs::read(path).map_err(|e| FirmwareError(format!("read {}: {e}", path.display())))?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<ElfImage, FirmwareError> {
        let obj = Object::parse(&bytes).map_err(|e| FirmwareError(format!("parse: {e}")))?;
        let elf = match obj {
            Object::Elf(elf) => elf,
            _ => return Err(FirmwareError("not an ELF file".into())),
        };
        if elf.header.e_ident[goblin::elf::header::EI_CLASS..][0] != goblin::elf::header::ELFCLASS32
        {
            return Err(FirmwareError("expected ELF32 (ELFCLASS64 found)".into()));
        }
        if elf.header.e_machine != goblin::elf::header::EM_ARM {
            return Err(FirmwareError(format!(
                "expected EM_ARM, found machine {}",
                elf.header.e_machine
            )));
        }
        if elf.header.e_type != goblin::elf::header::ET_EXEC {
            return Err(FirmwareError(format!(
                "expected ET_EXEC, found type {}",
                elf.header.e_type
            )));
        }
        let entry = elf.entry as u32;
        let mut segments = Vec::new();
        for ph in &elf.program_headers {
            if ph.p_type != PT_LOAD {
                continue;
            }
            let vaddr = ph.p_vaddr as u32;
            let file_size = ph.p_filesz as usize;
            let mem_size = ph.p_memsz as usize;
            if mem_size < file_size {
                return Err(FirmwareError(format!(
                    "PT_LOAD at {vaddr:#X}: memsz {mem_size} < filesz {file_size}"
                )));
            }
            let va_end = (vaddr as u64) + (mem_size as u64);
            if va_end > u32::MAX as u64 {
                return Err(FirmwareError(format!(
                    "PT_LOAD at {vaddr:#X} end {va_end:#X} overflows 32-bit space"
                )));
            }
            if ph.p_offset.saturating_add(file_size as u64) > bytes.len() as u64 {
                return Err(FirmwareError(format!(
                    "PT_LOAD at {vaddr:#X}: file range [{:#X}, {:#X}) exceeds file size {:#X}",
                    ph.p_offset,
                    ph.p_offset + file_size as u64,
                    bytes.len()
                )));
            }
            segments.push(Segment {
                offset: ph.p_offset,
                vaddr,
                file_size,
                mem_size,
                readable: ph.p_flags & PF_R != 0,
                writable: ph.p_flags & PF_W != 0,
                executable: ph.p_flags & PF_X != 0,
            });
        }
        if segments.is_empty() {
            return Err(FirmwareError("no PT_LOAD segments".into()));
        }
        Ok(ElfImage {
            entry,
            segments,
            file_size: bytes.len() as u64,
            bytes,
        })
    }

    /// Raw file bytes of one segment (bounds-checked at parse time).
    pub fn segment_bytes(&self, seg: &Segment) -> &[u8] {
        let start = seg.offset as usize;
        &self.bytes[start..start + seg.file_size]
    }
}

/// The OS ROM image (`PRIME_OS.ROM`): flat 1 MiB image with a 24-byte header
/// (ripem `osrom2elf` format, Phase 1-verified on the 20260909 image):
/// `e_entry, pad[4], e_size_copy, e_loadaddr, e_size, ident "V5J\0" "2416"`.
pub struct OsRom {
    pub entry: u32,
    pub load_addr: u32,
    pub size: u32,
    pub bytes: Vec<u8>,
}

impl OsRom {
    pub fn load(path: &Path) -> Result<OsRom, FirmwareError> {
        let bytes =
            fs::read(path).map_err(|e| FirmwareError(format!("read {}: {e}", path.display())))?;
        if bytes.len() < 24 {
            return Err(FirmwareError(
                "OS ROM smaller than its 24-byte header".into(),
            ));
        }
        let le = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let entry = le(0);
        let load_addr = le(12);
        let size = le(16);
        let ident = &bytes[20..28];
        if ident != b"V5J\x002416" {
            return Err(FirmwareError(format!(
                "OS ROM ident mismatch: {:?} (expected \"V5J\\0\" \"2416\")",
                String::from_utf8_lossy(ident)
            )));
        }
        Ok(OsRom {
            entry,
            load_addr,
            size,
            bytes,
        })
    }
}
