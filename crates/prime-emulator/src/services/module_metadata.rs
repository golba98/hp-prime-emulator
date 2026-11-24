//! G1 module metadata, from executable PRIME_OS.ROM controls.
//!
//! GetApplicationHeadInfoA/W clear 132 bytes only after opening the file. The
//! OS probes PE, ELF and FM/FI modules. ELF has no metadata/resource callback;
//! FM/FI exposes its own header as BIN/ROMSPC; PE obtains that named resource
//! and reports the optional-header checksum and header-plus-section raw size.
//! These fields are not host file timestamps or an invented application version.
use super::{HandlerFault, SvcCtx, fs::Vfs};

const OUTPUT_BYTES: usize = 132;
const READ_BUDGET: usize = 8 * 1024 * 1024;
trait ReadAt {
    fn len(&self) -> u32;
    fn read(&mut self, at: u32, len: usize) -> Option<Vec<u8>>;
}
struct File<'a> {
    vfs: &'a Vfs,
    handle: u32,
    len: u32,
    budget: usize,
}
impl ReadAt for File<'_> {
    fn len(&self) -> u32 {
        self.len
    }
    fn read(&mut self, at: u32, len: usize) -> Option<Vec<u8>> {
        if u64::from(at).checked_add(len as u64)? > u64::from(self.len) || len > self.budget {
            return None;
        }
        self.budget -= len;
        let mut bytes = vec![0; len];
        let mut done = 0;
        while done < len {
            let count =
                self.vfs
                    .read_at(self.handle, u64::from(at) + done as u64, &mut bytes[done..])?;
            if count == 0 {
                return None;
            }
            done += count;
        }
        Some(bytes)
    }
}
impl Drop for File<'_> {
    fn drop(&mut self) {
        self.vfs.close(self.handle);
    }
}
fn short(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn word(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn put_word(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn header(bytes: &[u8], output: &mut [u8; OUTPUT_BYTES]) {
    if matches!(short(bytes, 0), Some(0x464d | 0x4649)) {
        output[..2].copy_from_slice(&bytes[..2]);
        if let Some(kind) = short(bytes, 4) {
            output[4..6].copy_from_slice(&kind.to_le_bytes());
        }
    }
}
struct Pe {
    optional: Vec<u8>,
    sections: Vec<u8>,
}
impl Pe {
    fn open(file: &mut impl ReadAt, dos: &[u8]) -> Option<Self> {
        if short(dos, 0)? != 0x5a4d {
            return None;
        }
        // The G1 probes the ARM COFF record after e_lfanew+4; it does not
        // independently validate the PE signature in this metadata path.
        let nt = word(dos, 0x3c)?;
        let coff = file.read(nt.checked_add(4)?, 20)?;
        if !matches!(short(&coff, 0)?, 0x1c0 | 0x1c2) {
            return None;
        }
        let optional = file.read(nt.checked_add(24)?, 224)?;
        if short(&optional, 0)? != 0x10b {
            return None;
        }
        // Original callbacks locate sections at NT+0xf8 (the PE32 layout).
        let sections = file.read(
            nt.checked_add(0xf8)?,
            usize::from(short(&coff, 2)?).checked_mul(40)?,
        )?;
        Some(Self { optional, sections })
    }
    fn metadata(&self, file: &mut impl ReadAt, out: &mut [u8; OUTPUT_BYTES]) -> Option<()> {
        let header_size = word(&self.optional, 60)?;
        // The original metadata callback first reads SizeOfHeaders bytes.
        // A truncated header does not commit even a valid checksum.
        if header_size > file.len() {
            return None;
        }
        let mut raw_size = header_size;
        for section in self.sections.chunks_exact(40) {
            raw_size = raw_size.wrapping_add(word(section, 16)?);
        }
        put_word(out, 8, word(&self.optional, 64)?);
        put_word(out, 12, raw_size);
        Some(())
    }
    fn resource(&self, file: &mut impl ReadAt) -> Option<(u32, u32)> {
        if word(&self.optional, 92)? <= 2 || word(&self.optional, 116)? == 0 {
            return None;
        }
        let rva = word(&self.optional, 112)?;
        let mut root = None;
        for section in self.sections.chunks_exact(40) {
            let start = word(section, 12)?;
            if start <= rva && rva < start.checked_add(word(section, 16)?)? {
                root = Some(word(section, 20)?);
                break;
            }
        }
        // Match the OS's module callback: the tree begins at this section's
        // raw offset; data RVAs use the resource-directory RVA as their base.
        let root = root?;
        let mut found = None;
        for ty in directory_entries(file, root, 0, "BIN")? {
            for name in directory_entries(file, root, ty & 0x7fff_ffff, "ROMSPC")? {
                let read_entry = |file: &mut dyn ReadAt| -> Option<(u32, u32)> {
                    let lang = root.checked_add(name & 0x7fff_ffff)?.checked_add(16)?;
                    let first = file.read(lang, 8)?;
                    let data = root.checked_add(word(&first, 4)?)?;
                    let entry = file.read(data, 16)?;
                    let at = word(&entry, 0)?.checked_sub(rva)?.checked_add(root)?;
                    if at >= file.len() {
                        return None;
                    }
                    Some((at, word(&entry, 4)?.min(file.len() - at)))
                };
                if let Some(value) = read_entry(file) {
                    found = Some(value);
                }
            }
        }
        found
    }
}
fn directory_entries(
    file: &mut impl ReadAt,
    root: u32,
    relative: u32,
    requested: &str,
) -> Option<Vec<u32>> {
    let at = root.checked_add(relative)?;
    let dir = file.read(at, 16)?;
    let count = u32::from(short(&dir, 12)?) + u32::from(short(&dir, 14)?);
    if count > 65535 {
        return None;
    }
    let entries = file.read(at.checked_add(16)?, count as usize * 8)?;
    let mut found = Vec::new();
    let requested = requested.encode_utf16().collect::<Vec<_>>();
    for entry in entries.chunks_exact(8) {
        let key = word(entry, 0)?;
        if key & 0x8000_0000 == 0 {
            continue;
        }
        let name_at = root.checked_add(key & 0x7fff_ffff)?;
        let size = usize::from(short(&file.read(name_at, 2)?, 0)?);
        let raw = file.read(name_at.checked_add(2)?, size.checked_mul(2)?)?;
        let name = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        // G1 compares the stored UTF-16 length; later matches overwrite.
        if requested.starts_with(&name) {
            found.push(word(entry, 4)?);
        }
    }
    Some(found)
}
fn metadata(file: &mut impl ReadAt) -> [u8; OUTPUT_BYTES] {
    let mut out = [0; OUTPUT_BYTES];
    let Some(dos) = file.read(0, 64) else {
        return out;
    };
    if short(&dos, 0) == Some(0x5a4d) {
        if let Some(pe) = Pe::open(file, &dos) {
            if let Some((at, size)) = pe.resource(file) {
                if let Some(bytes) = file.read(at, (size as usize).min(128)) {
                    header(&bytes, &mut out);
                }
            }
            let _ = pe.metadata(file, &mut out);
        }
    } else if dos.starts_with(b"\x7fELF") {
        // The installed ELF module has no resource or metadata callbacks.
    } else if matches!(short(&dos, 0), Some(0x464d | 0x4649)) {
        if let Some(bytes) = file.read(0, 128) {
            header(&bytes, &mut out);
            if let Some(value) = short(&bytes, 6) {
                put_word(&mut out, 8, u32::from(value));
            }
        }
    }
    out
}
fn get(ctx: &mut SvcCtx, wide: bool) -> Result<u32, HandlerFault> {
    let path = if wide {
        ctx.guest_wstr(ctx.args.r0)?
    } else {
        ctx.guest_str(ctx.args.r0)?
    }
    .unwrap_or_default();
    let Some(handle) = ctx.shared.vfs.open(&path, "rb") else {
        return Ok(u32::MAX);
    };
    let mut file = File {
        vfs: &ctx.shared.vfs,
        handle,
        len: ctx.shared.vfs.filesize(handle).unwrap_or(0) as u32,
        budget: READ_BUDGET,
    };
    let output = metadata(&mut file);
    ctx.mem
        .write_bytes(ctx.args.r1, &output)
        .map_err(HandlerFault)?;
    Ok(0)
}
pub fn ansi(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    get(ctx, false)
}
pub fn wide(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    get(ctx, true)
}

/// The installed OS implements these legacy Win32 resource exports as ARM
/// `mov r0,#0; bx lr`. They do not inspect pointers or write output buffers.
/// This is distinct from the real module-resource parser used above.
pub fn legacy_resource_absent(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(0)
}

#[cfg(test)]
#[path = "module_metadata_tests.rs"]
mod tests;
