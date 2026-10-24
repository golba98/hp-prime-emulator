//! Filesystem service handlers over the [`Vfs`](super::fs::Vfs), plus the
//! loader file-descriptor API and INI access. Conventions verified against
//! the reference (`svc_filesystem.cpp`, `svc_system.cpp`).

use serde_json::json;

use super::{HandlerFault, SvcCtx};

/// `--trace-fs`: records one file-service call with the caller context.
fn fs_trace(ctx: &SvcCtx, op: &str, detail: serde_json::Value) {
    if ctx.trace.fs_enabled() {
        ctx.trace.fs(op, ctx.thread_id, ctx.args.caller_pc, detail);
    }
}

/// Host path behind a VFS handle, for trace output.
fn handle_path(ctx: &SvcCtx, handle: u32) -> String {
    if !ctx.trace.fs_enabled() {
        return String::new();
    }
    let root = ctx.shared.vfs.root().to_path_buf();
    ctx.shared
        .vfs
        .host_path_of(handle)
        .map(|p| {
            p.strip_prefix(&root)
                .map(|r| r.display().to_string())
                .unwrap_or_else(|_| p.display().to_string())
        })
        .unwrap_or_default()
}

/// Host path a guest path maps to, relative to the data root (trace).
fn host_of(ctx: &SvcCtx, guest: &str) -> String {
    if !ctx.trace.fs_enabled() {
        return String::new();
    }
    let root = ctx.shared.vfs.root().to_path_buf();
    ctx.shared
        .vfs
        .to_host(guest)
        .map(|p| {
            p.strip_prefix(&root)
                .map(|r| r.display().to_string())
                .unwrap_or_else(|_| p.display().to_string())
        })
        .unwrap_or_else(|| "<outside data root>".into())
}

/// ref: loader_file_descriptor_t (0x20 bytes): cart(VFS handle), parent_fd,
/// subfile_base, size, subfile_offset, unk16, unk18, unk1c.
const LOADER_FD_SIZE: usize = 0x20;

/// Guest-visible `loader_file_descriptor_t` (PrimU2 svc_system.cpp), 0x20
/// bytes: the firmware owns the pointer, so the layout must match.
mod fd {
    pub const CART: u32 = 0;
    /// Parent *descriptor* of a sub-file; 0 for a file opened directly.
    pub const PARENT: u32 = 4;
    pub const SUBFILE_BASE: u32 = 8;
    pub const SIZE: u32 = 12;
    pub const OFFSET: u32 = 16;
    /// `unk_0x14`: 1 for sub-files.
    pub const SUB_FLAG: u32 = 20;
}

// ── wide/ANSI fopen family ─────────────────────────────────────────────

/// ref: `__wfopen(r0 = wide path, r1 = wide mode)` → VFS handle (0 = fail).
pub fn w_fopen(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let path = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    let mode = ctx.guest_wstr(ctx.args.r1)?.unwrap_or_default();
    let handle = ctx.shared.vfs.open(&path, &mode);
    fs_trace(
        ctx,
        "wfopen",
        json!({ "path": path, "host": host_of(ctx, &path), "mode": mode, "handle": handle.unwrap_or(0) }),
    );
    match handle {
        Some(h) => Ok(h),
        None => {
            ctx.trace
                .thread_event(format!("wfopen: '{path}' mode '{mode}' failed"));
            Ok(0)
        }
    }
}

/// ref: `_fread(dest = r0, size = r1, count = r2, handle = r3)` → bytes read.
pub fn fread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let dest = ctx.args.r0;
    let total = ctx.args.r1 as usize * ctx.args.r2 as usize;
    let handle = ctx.args.r3;
    if total == 0 {
        return Ok(0);
    }
    let mut buf = vec![0u8; total];
    let n = ctx.shared.vfs.read(handle, &mut buf).unwrap_or(0);
    if n > 0 {
        ctx.mem.write_bytes(dest, &buf[..n]).map_err(HandlerFault)?;
    }
    fs_trace(
        ctx,
        "fread",
        json!({ "handle": handle, "host": handle_path(ctx, handle), "requested": total, "read": n }),
    );
    Ok(n as u32)
}

/// ref: `_fwrite(src = r0, size = r1, count = r2, handle = r3)` → written.
pub fn fwrite(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let src = ctx.args.r0;
    let total = ctx.args.r1 as usize * ctx.args.r2 as usize;
    let handle = ctx.args.r3;
    let buf = ctx.mem.read_bytes_alloc(src, total).map_err(HandlerFault)?;
    let n = ctx.shared.vfs.write(handle, &buf).unwrap_or(0);
    fs_trace(
        ctx,
        "fwrite",
        json!({ "handle": handle, "host": handle_path(ctx, handle), "requested": total, "written": n, "src": src }),
    );
    Ok(n as u32)
}

/// ref: `_fclose(handle = r0)` → 1/0.
pub fn fclose(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let host = handle_path(ctx, ctx.args.r0);
    let ok = ctx.shared.vfs.close(ctx.args.r0);
    fs_trace(
        ctx,
        "fclose",
        json!({ "handle": ctx.args.r0, "host": host, "ok": ok }),
    );
    Ok(ok as u32)
}

/// ref: `__fseek(handle = r0, offset = r1, whence = r2)` → 0/‑1.
pub fn fseek(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let handle = ctx.args.r0;
    let offset = ctx.args.r1;
    let whence = ctx.args.r2;
    let target = match whence {
        0 => offset as u64,
        1 => ctx.shared.vfs.tell(handle).unwrap_or(0) + offset as u64,
        2 => ctx
            .shared
            .vfs
            .filesize(handle)
            .unwrap_or(0)
            .wrapping_add(offset as u64),
        _ => return Ok(u32::MAX),
    };
    if ctx.shared.vfs.seek_set(handle, target).is_some() {
        Ok(0)
    } else {
        Ok(u32::MAX)
    }
}

/// ref: `__ftell(handle = r0)` → position.
pub fn ftell(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(ctx.shared.vfs.tell(ctx.args.r0).unwrap_or(0) as u32)
}

/// ref: `_filesize(handle = r0)` → size.
pub fn filesize(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(ctx.shared.vfs.filesize(ctx.args.r0).unwrap_or(0) as u32)
}

// ── directory ops ─────────────────────────────────────────────────────

/// ref: `_wmkdir(r0 = wide path)`; `_wrmdir`, `_wchdir` analogous.
pub fn w_mkdir(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let path = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    let ok = ctx.shared.vfs.mkdir(&path);
    fs_trace(
        ctx,
        "wmkdir",
        json!({ "path": path, "host": host_of(ctx, &path), "ok": ok }),
    );
    Ok(ok as u32)
}

pub fn w_rmdir(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let path = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    let ok = ctx.shared.vfs.rmdir(&path);
    fs_trace(
        ctx,
        "wrmdir",
        json!({ "path": path, "host": host_of(ctx, &path), "ok": ok }),
    );
    Ok(ok as u32)
}

/// ref: `__wremove(r0 = wide path)` deletes a guest file.
pub fn w_remove(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let path = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    let deleted = ctx
        .shared
        .vfs
        .to_host(&path)
        .map(|h| ctx.shared.vfs.resolve_case(&h))
        .and_then(|h| std::fs::remove_file(h).ok())
        .is_some();
    fs_trace(
        ctx,
        "wremove",
        json!({ "path": path, "host": host_of(ctx, &path), "ok": deleted }),
    );
    Ok(deleted as u32)
}

pub fn w_chdir(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let path = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    let ok = ctx.shared.vfs.chdir(&path);
    fs_trace(
        ctx,
        "wchdir",
        json!({ "path": path, "host": host_of(ctx, &path), "ok": ok }),
    );
    Ok(ok as u32)
}

// ── directory search (find_context_t, ref layout) ─────────────────────
// +0 host handle, +4 0xdeadbeef, +8 lfn ptr, +12 sfn ptr, +16 sfn ptr,
// +20 size, +24 mtime, +28 btime, +32 atime, +36 attrib_mask, +37 attrib.

const FC_LFN: u32 = 8;
const FC_SFN: u32 = 12;
const FC_SFN2: u32 = 16;
const FC_SIZE: u32 = 20;
const FC_MTIME: u32 = 24;
const FC_BTIME: u32 = 28;
const FC_ATIME: u32 = 32;
const FC_ATTRMASK: u32 = 36;
const FC_ATTRIB: u32 = 37;

/// ref: `_wfindfirst(pattern = r0, ctx = r1, attrib_mask = r2)` fills the
/// guest context and returns the first `_wfindnext` result.
pub fn w_findfirst(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let pattern = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    let guest_ctx = ctx.args.r1;
    let attrib_mask = ctx.args.r2;
    if guest_ctx == 0 {
        return Ok(u32::MAX);
    }
    let Some((handle, host_dir, names, pat)) = ctx.shared.vfs.find_first(&pattern, attrib_mask)
    else {
        fs_trace(
            ctx,
            "wfindfirst",
            json!({ "pattern": pattern, "host": host_of(ctx, &pattern), "mask": attrib_mask, "found": false }),
        );
        return Ok(u32::MAX);
    };
    fs_trace(
        ctx,
        "wfindfirst",
        json!({ "pattern": pattern, "host": host_of(ctx, &pattern), "mask": attrib_mask, "found": true, "candidates": names.len() }),
    );
    ctx.shared
        .vfs
        .find_register(handle, host_dir, names, pat, attrib_mask);
    ctx.mem.write_u32(guest_ctx, handle).map_err(HandlerFault)?;
    ctx.mem
        .write_u32(guest_ctx + 4, 0xdead_beef)
        .map_err(HandlerFault)?;
    w_findnext_inner(ctx, guest_ctx)
}

/// ref: `_wfindnext(ctx = r0)` → 0 found / -1 end.
pub fn w_findnext(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let guest_ctx = ctx.args.r0;
    if guest_ctx == 0 {
        return Ok(u32::MAX);
    }
    w_findnext_inner(ctx, guest_ctx)
}

fn w_findnext_inner(ctx: &mut SvcCtx, guest_ctx: u32) -> Result<u32, HandlerFault> {
    let handle = ctx.mem.read_u32(guest_ctx).map_err(HandlerFault)?;
    // Release the previous name strings (ref behavior).
    let (prev_sfn, prev_lfn) = ctx.shared.vfs.find_strings(handle);
    if prev_sfn != 0 {
        let _ = ctx.mem.heap_free(prev_sfn);
    }
    if prev_lfn != 0 {
        let _ = ctx.mem.heap_free(prev_lfn);
    }
    let Some(name) = ctx.shared.vfs.find_next(handle) else {
        fs_trace(ctx, "wfindnext", json!({ "handle": handle, "name": null }));
        return Ok(u32::MAX);
    };
    fs_trace(ctx, "wfindnext", json!({ "handle": handle, "name": name }));
    // SFN: uppercase 8.3-ish; LFN: full name as UTF-16LE.
    let sfn = sfn_of(&name);
    let sfn_va = ctx.mem.heap_alloc(sfn.len() + 1).map_err(HandlerFault)?;
    ctx.mem.write_cstr(sfn_va, &sfn).map_err(HandlerFault)?;
    let lfn_va = ctx
        .mem
        .heap_alloc(name.len() * 2 + 2)
        .map_err(HandlerFault)?;
    write_utf16z(ctx, lfn_va, &name)?;
    ctx.shared.vfs.find_set_strings(handle, sfn_va, lfn_va);

    let attrib_mask = ctx.shared.vfs.find_attrib_mask(handle);
    let meta = ctx.shared.vfs.stat_entry(handle, &name);
    ctx.mem
        .write_u32(guest_ctx + FC_LFN, lfn_va)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(guest_ctx + FC_SFN, sfn_va)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(guest_ctx + FC_SFN2, sfn_va)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(guest_ctx + FC_SIZE, meta.map(|m| m.0).unwrap_or(0))
        .map_err(HandlerFault)?;
    let dt = meta.map(|m| m.1).unwrap_or(0);
    ctx.mem
        .write_u32(guest_ctx + FC_MTIME, dt)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(guest_ctx + FC_BTIME, dt)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(guest_ctx + FC_ATIME, dt)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u8(guest_ctx + FC_ATTRMASK, attrib_mask as u8)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u8(guest_ctx + FC_ATTRIB, meta.map(|m| m.2).unwrap_or(0x20))
        .map_err(HandlerFault)?;
    Ok(0)
}

/// ref: `_findclose(ctx = r0)` → 0.
pub fn findclose(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let guest_ctx = ctx.args.r0;
    if guest_ctx == 0 {
        return Ok(u32::MAX);
    }
    let handle = ctx.mem.read_u32(guest_ctx).map_err(HandlerFault)?;
    if handle == 0 {
        return Ok(0);
    }
    let (sfn, lfn) = ctx.shared.vfs.find_close(handle).unwrap_or((0, 0));
    if sfn != 0 {
        let _ = ctx.mem.heap_free(sfn);
    }
    if lfn != 0 {
        let _ = ctx.mem.heap_free(lfn);
    }
    Ok(0)
}

fn sfn_of(name: &str) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(i) => (&name[..i], &name[i + 1..]),
    };
    let stem: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(8)
        .collect();
    let ext: String = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(3)
        .collect();
    if ext.is_empty() {
        stem.to_uppercase()
    } else {
        format!("{}.{}", stem.to_uppercase(), ext.to_uppercase())
    }
}

fn write_utf16z(ctx: &SvcCtx, va: u32, s: &str) -> Result<(), HandlerFault> {
    let mut buf = Vec::with_capacity(s.len() * 2 + 2);
    for unit in s.encode_utf16() {
        buf.extend_from_slice(&unit.to_le_bytes());
    }
    buf.extend_from_slice(&0u16.to_le_bytes());
    ctx.mem.write_bytes(va, &buf).map_err(HandlerFault)?;
    Ok(())
}

// ── loader file descriptors ────────────────────────────────────────────

/// ref: `_OpenFile(path = r0 ansi, mode = r1 ansi)` → guest fd (0 = fail).
pub fn open_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let path = ctx.guest_str(ctx.args.r0)?.unwrap_or_default();
    let mode = ctx.guest_str(ctx.args.r1)?.unwrap_or_default();
    let cart = ctx.shared.vfs.open(&path, &mode);
    fs_trace(
        ctx,
        "OpenFile",
        json!({ "path": path, "host": host_of(ctx, &path), "mode": mode, "handle": cart.unwrap_or(0) }),
    );
    let Some(cart) = cart else {
        ctx.trace.thread_event(format!("OpenFile: '{path}' failed"));
        return Ok(0);
    };
    let size = ctx.shared.vfs.filesize(cart).unwrap_or(0) as u32;
    let fd = ctx.mem.heap_alloc(LOADER_FD_SIZE).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(fd, &[0u8; LOADER_FD_SIZE])
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(fd + fd::CART, cart)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(fd + fd::SIZE, size)
        .map_err(HandlerFault)?;
    Ok(fd)
}

/// ref: `_OpenSubFile(parent = r0, base = r1, max = r2)` → guest fd.
pub fn open_sub_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let parent = ctx.args.r0;
    let base = ctx.args.r1;
    let max = ctx.args.r2;
    if parent == 0 {
        return Ok(0);
    }
    let parent_size = ctx.mem.read_u32(parent + fd::SIZE).map_err(HandlerFault)?;
    if base.checked_add(max).is_none_or(|end| end > parent_size) {
        return Ok(0);
    }
    let cart = ctx.mem.read_u32(parent + fd::CART).map_err(HandlerFault)?;
    let parent_base = ctx
        .mem
        .read_u32(parent + fd::SUBFILE_BASE)
        .map_err(HandlerFault)?;
    let sub = ctx.mem.heap_alloc(LOADER_FD_SIZE).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(sub, &[0u8; LOADER_FD_SIZE])
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(sub + fd::CART, cart)
        .map_err(HandlerFault)?;
    // The parent descriptor itself (never 0), so closing a sub-file can
    // never close the container's host file.
    ctx.mem
        .write_u32(sub + fd::PARENT, parent)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(sub + fd::SUBFILE_BASE, parent_base.wrapping_add(base))
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(sub + fd::SIZE, max)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u16(sub + fd::SUB_FLAG, 1)
        .map_err(HandlerFault)?;
    Ok(sub)
}

/// ref: `_CloseFile(fd = r0)`; closes the VFS handle at the root parent.
pub fn close_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let fd = ctx.args.r0;
    if fd == 0 {
        return Ok(0);
    }
    let parent_fd = ctx.mem.read_u32(fd + fd::PARENT).map_err(HandlerFault)?;
    if parent_fd == 0 {
        let cart = ctx.mem.read_u32(fd + fd::CART).map_err(HandlerFault)?;
        let _ = ctx.shared.vfs.close(cart);
    }
    let _ = ctx.mem.heap_free(fd);
    Ok(0)
}

/// ref: `_FileSize(fd = r0)` → fd->size.
pub fn file_size(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    if ctx.args.r0 == 0 {
        return Ok(u32::MAX);
    }
    ctx.mem
        .read_u32(ctx.args.r0 + fd::SIZE)
        .map_err(HandlerFault)
}

/// ref: `_FseekFile(fd = r0, offset = r1, whence = r2)` bounded to the fd.
pub fn fseek_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let fd = ctx.args.r0;
    let offset = ctx.args.r1;
    let whence = ctx.args.r2;
    if fd == 0 {
        return Ok(u32::MAX);
    }
    let size = ctx.mem.read_u32(fd + fd::SIZE).map_err(HandlerFault)?;
    let cur = ctx.mem.read_u32(fd + fd::OFFSET).map_err(HandlerFault)?;
    let new = match whence {
        0 => offset,
        1 => cur.wrapping_add(offset),
        2 => size.wrapping_add(offset),
        _ => return Ok(u32::MAX),
    };
    if new > size {
        return Ok(u32::MAX);
    }
    ctx.mem
        .write_u32(fd + fd::OFFSET, new)
        .map_err(HandlerFault)?;
    Ok(0)
}

/// ref: `_ReadFile(fd = r0, dest = r1, size = r2)` → bytes read.
pub fn read_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let fd = ctx.args.r0;
    let dest = ctx.args.r1;
    let size = ctx.args.r2 as usize;
    if fd == 0 || size == 0 {
        return Ok(0);
    }
    let cart = ctx.mem.read_u32(fd + fd::CART).map_err(HandlerFault)?;
    let base = ctx
        .mem
        .read_u32(fd + fd::SUBFILE_BASE)
        .map_err(HandlerFault)?;
    let off = ctx.mem.read_u32(fd + fd::OFFSET).map_err(HandlerFault)?;
    let fsize = ctx.mem.read_u32(fd + fd::SIZE).map_err(HandlerFault)?;
    let remaining = fsize.saturating_sub(off) as usize;
    if remaining == 0 {
        return Ok(0);
    }
    let to_read = size.min(remaining);
    let mut buf = vec![0u8; to_read];
    let n = ctx
        .shared
        .vfs
        .read_at(cart, base.wrapping_add(off) as u64, &mut buf)
        .unwrap_or(0);
    if n > 0 {
        ctx.mem.write_bytes(dest, &buf[..n]).map_err(HandlerFault)?;
        ctx.mem
            .write_u32(fd + fd::OFFSET, off.wrapping_add(n as u32))
            .map_err(HandlerFault)?;
    }
    Ok(n as u32)
}

// ── INI (Win32 Get/WritePrivateProfileString semantics, ref-verified) ──

/// ref: `_GetPrivateProfileString(app = r0, key = r1, default = r2,
/// dest = r3, size = [sp+8], filename = *([sp+0xC]))` → copied length.
pub fn get_private_profile_string(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let app = ctx.guest_str(ctx.args.r0)?.unwrap_or_default();
    let key = ctx.guest_str(ctx.args.r1)?.unwrap_or_default();
    let default = ctx.guest_str(ctx.args.r2)?.unwrap_or_default();
    let dest = ctx.args.r3;
    let size = ctx.stack_arg(0)? as usize;
    let filename = ctx.guest_str(ctx.stack_arg(1)?)?.unwrap_or_default();
    if dest == 0 || size == 0 {
        return Ok(0);
    }
    let value = ini_read(&ctx.shared.vfs, &filename, &app, &key).unwrap_or_else(|| default.clone());
    let copied = value.len().min(size.saturating_sub(1));
    ctx.mem
        .write_bytes(dest, &value.as_bytes()[..copied])
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u8(dest + copied as u32, 0)
        .map_err(HandlerFault)?;
    Ok(copied as u32)
}

/// ref: `_WritePrivateProfileString(app = r0, key = r1, value = r2,
/// filename = *([sp+8]))` → 1 on success.
pub fn write_private_profile_string(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let app = ctx.guest_str(ctx.args.r0)?.unwrap_or_default();
    let key = ctx.guest_str(ctx.args.r1)?.unwrap_or_default();
    let value = ctx.guest_str(ctx.args.r2)?.unwrap_or_default();
    // Win32 / reference ABI: four arguments, the file name in r3 (only
    // GetPrivateProfileString passes its file name on the stack).
    let filename = ctx.guest_str(ctx.args.r3)?.unwrap_or_default();
    fs_trace(
        ctx,
        "WritePrivateProfileString",
        json!({ "path": filename, "host": host_of(ctx, &filename), "section": app, "key": key, "value": value }),
    );
    if filename.is_empty() || app.is_empty() {
        return Ok(0); // reference: nothing written without a file or section
    }
    ini_write(&ctx.shared.vfs, &filename, &app, &key, &value);
    Ok(1)
}

fn ini_read(vfs: &super::fs::Vfs, filename: &str, section: &str, key: &str) -> Option<String> {
    let host = vfs.to_host(filename)?;
    let content = std::fs::read_to_string(host).ok()?;
    let mut in_section = false;
    for line in content.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            in_section = l[1..l.len() - 1].eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_owned());
                }
            }
        }
    }
    None
}

fn ini_write(vfs: &super::fs::Vfs, filename: &str, section: &str, key: &str, value: &str) {
    let Some(host) = vfs.to_host(filename) else {
        return;
    };
    if let Some(p) = host.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let mut content = std::fs::read_to_string(&host).unwrap_or_default();
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut written = false;
    for line in content.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            if in_section && !written {
                out_lines.push(format!("{key}={value}"));
                written = true;
            }
            in_section = l[1..l.len() - 1].eq_ignore_ascii_case(section);
            out_lines.push(line.to_owned());
            continue;
        }
        if in_section && !written {
            if let Some((k, _)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    out_lines.push(format!("{key}={value}"));
                    written = true;
                    continue;
                }
            }
        }
        out_lines.push(line.to_owned());
    }
    if !in_section {
        out_lines.push(format!("[{section}]"));
    }
    if !written {
        out_lines.push(format!("{key}={value}"));
    }
    content = out_lines.join("\r\n") + "\r\n";
    let _ = std::fs::write(&host, content);
}

// ── path split/merge (ref-verified layouts) ────────────────────────────

/// ref: `_afnsplit(path = r0, drive = r1, dir = r2, name = r3, ext = [sp+8])`.
pub fn a_fnsplit(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let Some(path) = ctx.guest_str(ctx.args.r0)? else {
        return Ok(u32::MAX);
    };
    let s = path.replace('/', "\\");
    let mut pos = 0;
    let drive = if s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic()
    {
        pos = 2;
        s[..2].to_owned()
    } else {
        String::new()
    };
    let (dir, filename) = match s[pos..].rfind('\\') {
        Some(i) => (s[pos..pos + i + 1].to_owned(), s[pos + i + 1..].to_owned()),
        None => (String::new(), s[pos..].to_owned()),
    };
    let (name, ext) = match filename.rfind('.') {
        Some(0) | None => (filename.clone(), String::new()),
        Some(i) => (filename[..i].to_owned(), filename[i..].to_owned()),
    };
    if ctx.args.r1 != 0 {
        ctx.mem
            .write_cstr(ctx.args.r1, &drive)
            .map_err(HandlerFault)?;
    }
    if ctx.args.r2 != 0 {
        ctx.mem
            .write_cstr(ctx.args.r2, &dir)
            .map_err(HandlerFault)?;
    }
    if ctx.args.r3 != 0 {
        ctx.mem
            .write_cstr(ctx.args.r3, &name)
            .map_err(HandlerFault)?;
    }
    let ext_ptr = ctx.stack_arg(0)?;
    if ext_ptr != 0 {
        ctx.mem.write_cstr(ext_ptr, &ext).map_err(HandlerFault)?;
    }
    Ok(0)
}

/// ref: `_afnmerge(out = r0, drive = r1, dir = r2, name = r3, ext = *([sp+8]))`.
pub fn a_fnmerge(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let out = ctx.args.r0;
    if out == 0 {
        return Ok(u32::MAX);
    }
    let drive = ctx.guest_str(ctx.args.r1)?.unwrap_or_default();
    let dir = ctx.guest_str(ctx.args.r2)?.unwrap_or_default();
    let name = ctx.guest_str(ctx.args.r3)?.unwrap_or_default();
    let ext_ptr = ctx.stack_arg(0)?;
    let ext = ctx.guest_str(ext_ptr)?.unwrap_or_default();
    let mut out_s = String::new();
    if !drive.is_empty() {
        out_s.push_str(&drive);
        if drive.len() == 1 {
            out_s.push(':');
        }
    }
    if !dir.is_empty() {
        let d = dir.replace('/', "\\");
        out_s.push_str(&d);
    }
    if !dir.is_empty() && !name.is_empty() && !dir.ends_with('\\') {
        out_s.push('\\');
    }
    out_s.push_str(&name);
    if !ext.is_empty() {
        if !ext.starts_with('.') {
            out_s.push('.');
        }
        out_s.push_str(&ext);
    }
    ctx.mem.write_cstr(out, &out_s).map_err(HandlerFault)?;
    Ok(0)
}
