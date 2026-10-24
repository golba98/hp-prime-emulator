//! PE32 (ARM, Windows CE 4.20) module loader for `_LoadLibraryA` & co.
//!
//! The firmware loads its platform DLLs (`SDKLIB.DLL`, `KRNLLIB.DLL`, ...)
//! through this service at boot. Conventions follow the reference
//! (PrimU2 `PELoader.cpp`): sections are mapped into guest memory, imports
//! are resolved against already-loaded modules, unresolved imports get a
//! return-zero Thumb stub, exports are recorded for `GetProcAddress`, and
//! base relocations are applied whenever the image cannot load at its
//! preferred base.

use std::collections::HashMap;

use crate::memory::GuestMemory;

/// IMAGE_FILE_MACHINE_ARM
const MACHINE_ARM: u16 = 0x01C0;
/// IMAGE_OPTIONAL_HEADER_MAGIC PE32
const MAGIC_PE32: u16 = 0x010B;
/// Data directory indices.
const DIR_EXPORT: usize = 0;
const DIR_IMPORT: usize = 1;
const DIR_BASERELOC: usize = 5;
/// IMAGE_REL_BASED_HIGHLOW
const REL_HIGHLOW: u16 = 3;

#[derive(Debug)]
pub struct PeError(pub String);

impl std::fmt::Display for PeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PE load failed: {}", self.0)
    }
}
impl std::error::Error for PeError {}

struct PeHeaders {
    image_base: u32,
    size_of_image: u32,
    size_of_headers: u32,
    entry_rva: u32,
    sections: Vec<PeSection>,
    export_dir: (u32, u32), // (rva, size)
    import_dir: (u32, u32),
    reloc_dir: (u32, u32),
}

struct PeSection {
    virtual_size: u32,
    virtual_address: u32,
    raw_size: u32,
    raw_ptr: u32,
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn parse_headers(b: &[u8]) -> Result<PeHeaders, PeError> {
    if b.len() < 0x40 || &b[..2] != b"MZ" {
        return Err(PeError("no MZ header".into()));
    }
    let e_lfanew = u32_at(b, 0x3C) as usize;
    if e_lfanew + 24 > b.len() || &b[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return Err(PeError("no PE signature".into()));
    }
    let coff = e_lfanew + 4;
    let machine = u16_at(b, coff);
    if machine != MACHINE_ARM {
        return Err(PeError(format!("machine {machine:#X} is not ARM")));
    }
    let nsections = u16_at(b, coff + 2) as usize;
    let opt_size = u16_at(b, coff + 16) as usize;
    let opt = coff + 20;
    if opt_size < 96 + 16 * 8 {
        return Err(PeError("optional header too small".into()));
    }
    if u16_at(b, opt) != MAGIC_PE32 {
        return Err(PeError(format!("magic {:#X} is not PE32", u16_at(b, opt))));
    }
    let entry_rva = u32_at(b, opt + 16);
    let image_base = u32_at(b, opt + 28);
    let size_of_image = u32_at(b, opt + 56);
    let size_of_headers = u32_at(b, opt + 60);
    let dd = opt + 96;
    let dir = |i: usize| (u32_at(b, dd + i * 8), u32_at(b, dd + i * 8 + 4));
    let mut sections = Vec::with_capacity(nsections);
    let sec0 = opt + opt_size;
    for i in 0..nsections {
        let s = sec0 + i * 40;
        if s + 40 > b.len() {
            return Err(PeError("section table out of range".into()));
        }
        sections.push(PeSection {
            virtual_size: u32_at(b, s + 8),
            virtual_address: u32_at(b, s + 12),
            raw_size: u32_at(b, s + 16),
            raw_ptr: u32_at(b, s + 20),
        });
    }
    Ok(PeHeaders {
        image_base,
        size_of_image,
        size_of_headers,
        entry_rva,
        sections,
        export_dir: dir(DIR_EXPORT),
        import_dir: dir(DIR_IMPORT),
        reloc_dir: dir(DIR_BASERELOC),
    })
}

/// A loaded guest PE module.
pub struct PeModule {
    pub name: String,
    pub guest_path: String,
    pub base: u32,
    pub size: u32,
    pub entry_rva: u32,
    pub exports_by_name: HashMap<String, u32>,
    pub exports_by_ordinal: HashMap<u32, u32>,
}

/// Load `bytes` into `mem` as a guest module. `resolve` maps an imported
/// symbol (dll, name/ordinal) to a guest VA (or None → return-zero stub
/// allocated by the caller through `stub_alloc`).
pub struct LoadCtx<'a> {
    pub name: &'a str,
    pub guest_path: &'a str,
    pub alloc: &'a dyn Fn(usize) -> u32,
    pub alloc_at_preferred: &'a dyn Fn(u32, usize) -> Option<u32>,
    pub resolve: &'a dyn Fn(&str, ImportName) -> Option<u32>,
    pub stub_alloc: &'a dyn Fn() -> u32,
}

#[allow(clippy::too_many_arguments)]
pub fn load_module(
    mem: &GuestMemory,
    bytes: &[u8],
    ctx: &LoadCtx<'_>,
) -> Result<PeModule, PeError> {
    let LoadCtx {
        name,
        guest_path,
        alloc,
        alloc_at_preferred,
        resolve,
        stub_alloc,
    } = *ctx;
    let h = parse_headers(bytes)?;
    if h.size_of_image == 0 {
        return Err(PeError("SizeOfImage is zero".into()));
    }
    let rounded = crate::memory::page_round(h.size_of_image as usize);
    let base = choose_base(
        h.image_base,
        rounded,
        has_relocations(&h),
        alloc_at_preferred,
        alloc,
    );
    if base == 0 {
        return Err(PeError("arena allocation failed".into()));
    }
    // Headers then sections.
    let headers_bytes = (h.size_of_headers as usize).min(bytes.len());
    mem.write_bytes(base, &bytes[..headers_bytes])
        .map_err(|e| PeError(format!("headers: {e}")))?;
    for s in &h.sections {
        if s.raw_ptr as usize >= bytes.len() {
            continue;
        }
        let avail = bytes.len() - s.raw_ptr as usize;
        let n = (s.raw_size as usize).min(avail);
        let vs = (s.virtual_size as usize).max(n);
        if vs > 0 {
            // Ensure the tail of the section (beyond raw data) is zero.
            mem.write_bytes(base + s.virtual_address, &vec![0u8; vs])
                .map_err(|e| PeError(format!("zero section: {e}")))?;
        }
        if n > 0 {
            mem.write_bytes(
                base + s.virtual_address,
                &bytes[s.raw_ptr as usize..s.raw_ptr as usize + n],
            )
            .map_err(|e| PeError(format!("copy section: {e}")))?;
        }
    }
    // Base relocations when we could not use the preferred base.
    if base != h.image_base {
        apply_relocations(mem, bytes, &h, base)?;
    }
    // Exports.
    let mut exports_by_name = HashMap::new();
    let mut exports_by_ordinal = HashMap::new();
    if h.export_dir.0 != 0 {
        let rva_of = |rva: u32| -> Option<usize> { rva_to_offset(&h, bytes, rva) };
        if let Some(off) = rva_of(h.export_dir.0) {
            let n_funcs = u32_at(bytes, off + 20);
            let n_names = u32_at(bytes, off + 24);
            let addr_tab = u32_at(bytes, off + 28);
            let name_tab = u32_at(bytes, off + 32);
            let ord_tab = u32_at(bytes, off + 36);
            let ordinal_base = u32_at(bytes, off + 16);
            for i in 0..n_funcs {
                let row = rva_of(addr_tab + i * 4)
                    .map(|o| u32_at(bytes, o))
                    .unwrap_or(0);
                if row != 0 {
                    exports_by_ordinal.insert(ordinal_base + i, base + row);
                }
            }
            for i in 0..n_names {
                let (Some(name_off), Some(fn_idx)) = (
                    rva_of(name_tab + i * 4)
                        .map(|o| u32_at(bytes, o))
                        .and_then(|nrva| rva_to_offset(&h, bytes, nrva)),
                    rva_of(ord_tab + i * 2).map(|o| u16_at(bytes, o) as u32),
                ) else {
                    continue;
                };
                let end = bytes[name_off..]
                    .iter()
                    .position(|&c| c == 0)
                    .map(|p| name_off + p)
                    .unwrap_or(bytes.len());
                let fname = String::from_utf8_lossy(&bytes[name_off..end]).into_owned();
                let row = rva_of(addr_tab + fn_idx * 4)
                    .map(|o| u32_at(bytes, o))
                    .unwrap_or(0);
                if row != 0 {
                    exports_by_name.insert(fname, base + row);
                }
            }
        }
    }
    // Imports: resolve each thunk and rewrite the IAT in guest memory.
    if h.import_dir.0 != 0 {
        let mut desc_rva = h.import_dir.0;
        while let Some(off) = rva_to_offset(&h, bytes, desc_rva) {
            let first_thunk = u32_at(bytes, off + 16);
            let name_rva = u32_at(bytes, off + 12);
            if first_thunk == 0 && name_rva == 0 {
                break; // terminating null descriptor
            }
            let dll = rva_to_offset(&h, bytes, name_rva)
                .and_then(|o| cstr_at(bytes, o))
                .unwrap_or_default();
            let mut i = 0u32;
            while let Some(thunk_off) = rva_to_offset(&h, bytes, first_thunk + i * 4) {
                let thunk = u32_at(bytes, thunk_off);
                if thunk == 0 {
                    break;
                }
                let import = if thunk & 0x8000_0000 != 0 {
                    ImportName::Ordinal(thunk & 0xFFFF)
                } else {
                    let fn_name = rva_to_offset(&h, bytes, thunk + 2)
                        .and_then(|o| cstr_at(bytes, o))
                        .unwrap_or_default();
                    ImportName::Name(fn_name)
                };
                let resolved = resolve(&dll, import).unwrap_or_else(stub_alloc);
                mem.write_u32(base + first_thunk + i * 4, resolved)
                    .map_err(|e| PeError(format!("IAT write: {e}")))?;
                i += 1;
            }
            desc_rva += 20;
        }
    }
    Ok(PeModule {
        name: name.to_owned(),
        guest_path: guest_path.to_owned(),
        base,
        size: h.size_of_image,
        entry_rva: h.entry_rva,
        exports_by_name,
        exports_by_ordinal,
    })
}

/// One imported symbol.
pub enum ImportName {
    Name(String),
    Ordinal(u32),
}

fn rva_to_offset(h: &PeHeaders, bytes: &[u8], rva: u32) -> Option<usize> {
    if rva < h.size_of_headers {
        return Some(rva as usize);
    }
    for s in &h.sections {
        if rva >= s.virtual_address && rva < s.virtual_address + s.virtual_size.max(s.raw_size) {
            let off = s.raw_ptr + (rva - s.virtual_address);
            if (off as usize) < bytes.len() {
                return Some(off as usize);
            }
        }
    }
    None
}

fn cstr_at(bytes: &[u8], off: usize) -> Option<String> {
    let end = bytes[off..].iter().position(|&c| c == 0)? + off;
    Some(String::from_utf8_lossy(&bytes[off..end]).into_owned())
}

fn apply_relocations(
    mem: &GuestMemory,
    bytes: &[u8],
    h: &PeHeaders,
    base: u32,
) -> Result<(), PeError> {
    let delta = base.wrapping_sub(h.image_base);
    let (rva, size) = h.reloc_dir;
    if rva == 0 || size == 0 {
        return Err(PeError(
            "image needs relocation but has no .reloc directory".into(),
        ));
    }
    let mut off = rva_to_offset(h, bytes, rva).ok_or(PeError("reloc rva out of range".into()))?;
    let end = off + size as usize;
    while off + 8 <= end && off < bytes.len() {
        let page_rva = u32_at(bytes, off);
        let block_size = u32_at(bytes, off + 4) as usize;
        if block_size < 8 || off + block_size > bytes.len() {
            break;
        }
        let n = (block_size - 8) / 2;
        for i in 0..n {
            let entry = u16_at(bytes, off + 8 + i * 2);
            let typ = entry >> 12;
            let eoff = (entry & 0x0FFF) as u32;
            if typ == REL_HIGHLOW {
                let va = base + page_rva + eoff;
                let cur = mem
                    .read_u32(va)
                    .map_err(|e| PeError(format!("reloc read: {e}")))?;
                mem.write_u32(va, cur.wrapping_add(delta))
                    .map_err(|e| PeError(format!("reloc write: {e}")))?;
            }
        }
        off += block_size;
    }
    Ok(())
}

/// Inspect a PE file's load layout without loading it (used by the boot
/// routine to pre-map preferred image bases).
pub struct PeProbe {
    pub image_base: u32,
    pub size_of_image: u32,
    /// The image carries base relocations and can load anywhere.
    pub relocatable: bool,
}

pub fn probe_headers(bytes: &[u8]) -> Result<PeProbe, PeError> {
    let h = parse_headers(bytes)?;
    Ok(PeProbe {
        image_base: h.image_base,
        size_of_image: h.size_of_image,
        relocatable: has_relocations(&h),
    })
}

fn has_relocations(h: &PeHeaders) -> bool {
    h.reloc_dir.0 != 0 && h.reloc_dir.1 != 0
}

/// Where an image goes. A fixed-base image (no .reloc, like the firmware's
/// SDKLIB.DLL) must sit at its preferred base, which the boot routine
/// pre-maps; `alloc_at_preferred` returns it only when a mapped block
/// starts exactly there. A relocatable image is carved out of the heap
/// arena and relocated, so it can never land on a fixed-base image that
/// shares its preferred base (several platform DLLs prefer 0x10000000).
pub fn choose_base(
    image_base: u32,
    size: usize,
    relocatable: bool,
    alloc_at_preferred: &dyn Fn(u32, usize) -> Option<u32>,
    alloc: &dyn Fn(usize) -> u32,
) -> u32 {
    if relocatable {
        return alloc(size);
    }
    match alloc_at_preferred(image_base, size) {
        Some(b) if b == image_base => b,
        _ => alloc(size),
    }
}

/// The DLLs to pre-map at their preferred bases: fixed-base images only
/// (relocatable ones load anywhere), first come first served by name.
/// Returns indices into `probes`.
pub fn premap_plan(probes: &[(String, PeProbe)]) -> Vec<usize> {
    let mut taken: Vec<(u32, u32)> = Vec::new();
    let mut out = Vec::new();
    for (i, (_, p)) in probes.iter().enumerate() {
        if p.relocatable {
            continue;
        }
        let start = p.image_base;
        let end = start.saturating_add(crate::memory::page_round(p.size_of_image as usize) as u32);
        if taken.iter().any(|&(a, b)| start < b && a < end) {
            continue;
        }
        taken.push((start, end));
        out.push(i);
    }
    out
}

#[cfg(test)]
mod base_tests {
    use super::*;

    fn probe(base: u32, size: u32, relocatable: bool) -> PeProbe {
        PeProbe {
            image_base: base,
            size_of_image: size,
            relocatable,
        }
    }

    #[test]
    fn premap_keeps_fixed_base_images_only() {
        // The firmware's A:\WINDOW\SYSTEM: three DLLs prefer 0x10000000,
        // only SDKLIB is fixed-base (no .reloc).
        let probes = vec![
            ("COREDLL.DLL".to_string(), probe(0x1000_0000, 0x8D000, true)),
            ("KRNLLIB.DLL".to_string(), probe(0x3180_0000, 0x3000, false)),
            ("MD5DLL.DLL".to_string(), probe(0x1000_0000, 0x5000, true)),
            ("SDKLIB.DLL".to_string(), probe(0x1000_0000, 0xA000, false)),
        ];
        let names: Vec<&str> = premap_plan(&probes)
            .into_iter()
            .map(|i| probes[i].0.as_str())
            .collect();
        assert_eq!(names, vec!["KRNLLIB.DLL", "SDKLIB.DLL"]);
        // Two fixed-base images on the same base: the first one wins.
        let clash = vec![
            ("A.DLL".to_string(), probe(0x1000_0000, 0x1000, false)),
            ("B.DLL".to_string(), probe(0x1000_0800, 0x1000, false)),
        ];
        assert_eq!(premap_plan(&clash), vec![0]);
    }

    #[test]
    fn base_choice_respects_fixed_and_relocatable_images() {
        let arena = |_: usize| 0x2050_0000;
        let mapped_at = |b: u32| move |p: u32, _: usize| (p == b).then_some(b);
        // Fixed-base image whose base is pre-mapped: loads there.
        assert_eq!(
            choose_base(0x1000_0000, 0xA000, false, &mapped_at(0x1000_0000), &arena),
            0x1000_0000
        );
        // A block that merely covers the base (starts elsewhere) is not the
        // preferred base: fall back to the arena (and fail on relocation).
        let covering = |_: u32, _: usize| Some(0x0FFF_0000);
        assert_eq!(
            choose_base(0x1000_0000, 0xA000, false, &covering, &arena),
            0x2050_0000
        );
        // Relocatable image: always the arena, even if its base is mapped.
        assert_eq!(
            choose_base(0x1000_0000, 0x5000, true, &mapped_at(0x1000_0000), &arena),
            0x2050_0000
        );
    }
}
