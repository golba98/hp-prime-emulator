//! LCD object, virtual devices, system variables, and the guest debug-print
//! service. Conventions from the reference (`LCD.cpp/h`, `svc_system.cpp`).

use super::{HandlerFault, SvcCtx};

/// LCD object layout, reference `LCD` struct ("BLIGLCD", PrimU2 LCD.h),
/// offsets in bytes:
///   0   LCD_MAGIC* LCDMagicPtr
///   4   secondBufferstart
///   8   bufferSize
///   12  itself
///   16  unk[0x70]
///   464 LCDSubObject*         (0x1D0)
///   468 xRes u16               (0x1D4)
///   470 yRes u16               (0x1D6)
///   472 unk0..unk4 (5 x u32)
///   492 enterCriticalSecFunc   (0x1EC)
///   496 leaveCriticalSecFunc  (0x1F0)
///   500 unk5[92]              (0x1F4)
///   592 LcdMagic (LCD_MAGIC)   (0x250)
///   616 buffer (ARGB32)        (0x268), 320*240*4 bytes used of the
///                               reference's uint16[320*240*3] tail
pub const LCD_STRUCT_SIZE: usize = 616 + 320 * 240 * 4;
pub const LCD_MAGIC_OFF: u32 = 592;
pub const LCD_BUFFER_OFF: u32 = 616;

/// ref: `GetActiveLCD()` returns the guest VA of the LCD object singleton.
pub fn get_active_lcd(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let mut slot = ctx.shared.lcd_object.lock().unwrap();
    if let Some(va) = *slot {
        return Ok(va);
    }
    let va = ctx.mem.heap_alloc(LCD_STRUCT_SIZE).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(va, &vec![0u8; LCD_STRUCT_SIZE])
        .map_err(HandlerFault)?;
    // LCD_MAGIC fields (ref LCD.cpp ctor).
    let m = va + LCD_MAGIC_OFF;
    ctx.mem.write_u16(m + 2, 320).map_err(HandlerFault)?; // x_res
    ctx.mem.write_u16(m + 4, 240).map_err(HandlerFault)?; // y_res
    ctx.mem.write_u16(m + 6, 32).map_err(HandlerFault)?; // pixel_bits (host 32bpp)
    ctx.mem.write_u16(m + 8, 640).map_err(HandlerFault)?; // unk2_640
    ctx.mem.write_u16(m + 10, 2).map_err(HandlerFault)?; // brightness_level
    ctx.mem.write_u32(m + 12, 8).map_err(HandlerFault)?; // unk1_0
    let buffer_va = va + LCD_BUFFER_OFF;
    ctx.mem.write_u32(m + 16, buffer_va).map_err(HandlerFault)?; // window1_bufferstart
    // Object header fields.
    ctx.mem.write_u32(va, m).map_err(HandlerFault)?; // LCDMagicPtr
    ctx.mem.write_u32(va + 4, buffer_va).map_err(HandlerFault)?; // secondBufferstart (ref: buffer VA)
    ctx.mem
        .write_u32(va + 8, 320 * 240 * 4)
        .map_err(HandlerFault)?; // bufferSize
    ctx.mem.write_u32(va + 12, va).map_err(HandlerFault)?; // itself
    ctx.mem.write_u16(va + 468, 320).map_err(HandlerFault)?; // xRes
    ctx.mem.write_u16(va + 470, 240).map_err(HandlerFault)?; // yRes
    // Black ARGB framebuffer.
    let black = vec![0u8; 320 * 240 * 4];
    for (i, chunk) in black.chunks(0x10000).enumerate() {
        ctx.mem
            .write_bytes(buffer_va + (i as u32) * 0x10000, chunk)
            .map_err(HandlerFault)?;
    }
    *slot = Some(va);
    ctx.shared
        .lcd_watch
        .set_range(buffer_va, (320 * 240 * 4) as u32);
    ctx.trace.thread_event(format!(
        "LCD object created at {va:#010X} (320x240 ARGB32, host 32bpp convention)"
    ));
    Ok(va)
}

/// ref: `LCDOn()` returns 0.
pub fn lcd_on(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(0)
}

/// ref: `SetSystemVariable(var = r0, type = r1, value = r2)`: var 89 → -1;
/// var 6/type 2 → brightness; everything else → 0.
pub fn set_system_variable(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let var = ctx.args.r0;
    let typ = ctx.args.r1;
    let value = ctx.args.r2;
    if var == 89 {
        return Ok(u32::MAX);
    }
    if var == 6 && typ == 2 {
        let va = *ctx.shared.lcd_object.lock().unwrap();
        if let Some(va) = va {
            ctx.mem
                .write_u16(va + LCD_MAGIC_OFF + 10, value as u16)
                .map_err(HandlerFault)?;
        }
    }
    Ok(0)
}

/// ref: `GetMasterIDInfo(out = r0)` fills a 172-byte identity struct with
/// zeros and its own strings; we return zeros with our project identity
/// instead of the reference's joke strings (documented divergence).
pub fn get_master_id_info(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let out = ctx.args.r0;
    ctx.mem
        .write_bytes(out, &[0u8; 172])
        .map_err(HandlerFault)?;
    let id = b"prime-emulator";
    ctx.mem.write_bytes(out + 40, id).map_err(HandlerFault)?;
    Ok(0)
}

/// ref: `WriteComDebugMsg(fmt = r0, ...)` is the firmware's debug printf.
/// The reference marshals a full va_list; we log the format string only
/// (partial: argument formatting not implemented).
pub fn write_com_debug_msg(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    if let Some(fmt) = ctx.guest_str(ctx.args.r0)? {
        ctx.trace.thread_event(format!("guest: {fmt}"));
    }
    Ok(0)
}

/// ref: `SetAutoPowerOff` is not implemented in the reference; zero-return
/// (disable) was observed harmless in diagnostic runs.
pub fn set_auto_power_off(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(0)
}

// ── PE module loading (ref: PELoader.cpp / svc_system.cpp _LoadLibraryA) ──

/// Load a guest PE module by name. Returns the module base (0 = failure,
/// matching the reference's failure path).
pub fn load_library(ctx: &mut SvcCtx, name: &str) -> Result<u32, HandlerFault> {
    if let Some(base) = ctx.shared.pe_module_by_name(name) {
        return Ok(base); // reference: already-loaded modules return their base
    }
    // Try the given name, then A:\WINDOW\SYSTEM\<name> (ref fallback).
    let mut candidates = vec![name.to_owned()];
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    candidates.push(format!("A:\\WINDOW\\SYSTEM\\{file}"));
    for cand in &candidates {
        if let Some(host) = ctx.shared.vfs.to_host(cand) {
            let host = ctx.shared.vfs.resolve_case(&host);
            if let Ok(bytes) = std::fs::read(&host) {
                let mem = ctx.mem;
                let shared = &ctx.shared;
                let load_ctx = super::pe::LoadCtx {
                    name: file,
                    guest_path: cand,
                    alloc: &|size| mem.heap_alloc_aligned(size, 0x1_0000).unwrap_or(0),
                    alloc_at_preferred: &|preferred, size| {
                        mem.covering_block(preferred, size)
                            .filter(|&b| b == preferred)
                    },
                    resolve: &|dll, imp| shared.pe_resolve(dll, imp),
                    stub_alloc: &|| shared.alloc_import_stub(mem).unwrap_or(0),
                };
                match super::pe::load_module(mem, &bytes, &load_ctx) {
                    Ok(m) => {
                        let base = m.base;
                        ctx.trace.thread_event(format!(
                            "PE module '{}' loaded at {base:#010X} ({} exports, entry +{:#X})",
                            m.name,
                            m.exports_by_name.len(),
                            m.entry_rva
                        ));
                        ctx.shared.add_pe_module(m);
                        return Ok(base);
                    }
                    Err(e) => {
                        ctx.trace
                            .thread_event(format!("PE load of '{cand}' failed: {e}"));
                    }
                }
            }
        }
    }
    ctx.trace
        .thread_event(format!("LoadLibrary '{name}' failed: file not found"));
    Ok(0)
}

/// ref: `_LoadLibraryA(r0 = char* name)` → module base.
pub fn load_library_a(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let name = ctx.guest_str(ctx.args.r0)?.unwrap_or_default();
    load_library(ctx, &name)
}

/// ref: `_LoadLibraryW(r0 = wchar_t* name)` → module base.
pub fn load_library_w(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let name = ctx.guest_wstr(ctx.args.r0)?.unwrap_or_default();
    load_library(ctx, &name)
}

/// ref: `_GetProcAddress(r0 = module base, r1 = char* name)` → symbol VA.
pub fn get_proc_address(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let base = ctx.args.r0;
    let name = ctx.guest_str(ctx.args.r1)?.unwrap_or_default();
    let Some(module) = ctx.shared.pe_module_by_base(base) else {
        return Ok(0);
    };
    Ok(module.exports_by_name.get(&name).copied().unwrap_or(0))
}

/// ref: `_FreeLibrary` is a stub printing a warning and returning 0.
pub fn free_library(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(0)
}

/// ref: `_GetModuleFileNameA(r0 = base, r1 = buffer, r2 = size)` → length.
pub fn get_module_file_name_a(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let buf = ctx.args.r1;
    let size = ctx.args.r2;
    let Some(module) = ctx.shared.pe_module_by_base(ctx.args.r0) else {
        return Ok(0);
    };
    let path = module.guest_path;
    let copied = path.len().min(size.saturating_sub(1) as usize);
    ctx.mem
        .write_bytes(buf, &path.as_bytes()[..copied])
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u8(buf + copied as u32, 0)
        .map_err(HandlerFault)?;
    Ok(copied as u32)
}

// ── virtual devices (ref: CreateFile registers every name; the IOCTL
//    dispatch keys on the name suffix) ─────────────────────────────────

/// ref: `CreateFile(name = r0)` returns a device handle.
pub fn create_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let name = ctx.guest_str(ctx.args.r0)?.unwrap_or_default();
    let handle = ctx.shared.register_device(&name);
    ctx.trace
        .thread_event(format!("device opened: '{name}' as handle {handle:#X}"));
    Ok(handle)
}

/// ref: `CloseHandle(handle = r0)` → 1/0.
pub fn close_handle(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(ctx.shared.unregister_device(ctx.args.r0) as u32)
}

/// ref: `DeviceIoControl(handle = r0, request = r1, in = r2, size = r3,
/// out = *([sp+8]), outlen = *([sp+12]), retlen = *([sp+16]),
/// overlapped = *([sp+20]))` → 1.
pub fn device_io_control(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let handle = ctx.args.r0;
    let request = ctx.args.r1;
    let in_ptr = ctx.args.r2;
    let size = ctx.args.r3 as usize;
    let out_ptr = ctx.stack_arg(0)?;
    let outlen = ctx.stack_arg(1)? as usize;
    let retlen_ptr = ctx.stack_arg(2)?;
    let Some(name) = ctx.shared.device_name(handle) else {
        ctx.trace
            .thread_event(format!("DeviceIoControl on unknown handle {handle:#X}"));
        return Ok(0);
    };
    let in_buf = if size > 0 && in_ptr != 0 {
        ctx.mem
            .read_bytes_alloc(in_ptr, size)
            .map_err(HandlerFault)?
    } else {
        Vec::new()
    };
    let ret = match name.as_str() {
        n if n.ends_with("BAT") => {
            if outlen >= 16 && out_ptr != 0 {
                let mut bat = vec![0u8; outlen];
                if bat.len() >= 16 {
                    bat[10..12].copy_from_slice(&4u16.to_le_bytes()); // level 4/4
                    bat[14..16].copy_from_slice(&0u16.to_le_bytes()); // not charging
                    bat[6..8].copy_from_slice(&1000u16.to_le_bytes()); // ADC ~3.42V
                }
                ctx.mem.write_bytes(out_ptr, &bat).map_err(HandlerFault)?;
            }
            1
        }
        n if n.ends_with("ARCH") => {
            // ref: request 260 = hardware type (VerA/VerC); writes 0.
            if out_ptr != 0 && outlen >= 4 {
                ctx.mem
                    .write_bytes(out_ptr, &vec![0u8; outlen])
                    .map_err(HandlerFault)?;
            }
            1
        }
        n if n.ends_with("FACTORY") => {
            // ref: FRP mount/read/write/erase with an in-memory blob.
            match request {
                258 => {
                    let blob = ctx.shared.frp_data.lock().unwrap();
                    let n = blob.len().min(outlen);
                    if out_ptr != 0 {
                        let mut data = vec![0u8; outlen];
                        if !blob.is_empty() {
                            data[..n].copy_from_slice(&blob[..n]);
                        }
                        ctx.mem.write_bytes(out_ptr, &data).map_err(HandlerFault)?;
                    }
                    if retlen_ptr != 0 {
                        let n = if blob.is_empty() { outlen } else { n };
                        ctx.mem
                            .write_u32(retlen_ptr, n as u32)
                            .map_err(HandlerFault)?;
                    }
                }
                259 => {
                    let mut blob = ctx.shared.frp_data.lock().unwrap();
                    blob.clear();
                    blob.extend_from_slice(&in_buf);
                    if retlen_ptr != 0 {
                        ctx.mem
                            .write_u32(retlen_ptr, in_buf.len() as u32)
                            .map_err(HandlerFault)?;
                    }
                }
                260 => {
                    ctx.shared.frp_data.lock().unwrap().clear();
                    if retlen_ptr != 0 {
                        ctx.mem.write_u32(retlen_ptr, 0).map_err(HandlerFault)?;
                    }
                }
                256 | 257 if retlen_ptr != 0 => {
                    ctx.mem.write_u32(retlen_ptr, 0).map_err(HandlerFault)?;
                }
                _ => {}
            }
            1
        }
        n if n.ends_with("USB") || n.ends_with("USBHOST") || n.ends_with("INDICATOR") => {
            // ref: enable/disable/status/callback registration all succeed.
            1
        }
        _ => {
            if out_ptr != 0 && outlen > 0 {
                ctx.mem
                    .write_bytes(out_ptr, &vec![0xFFu8; outlen])
                    .map_err(HandlerFault)?;
            }
            1
        }
    };
    Ok(ret)
}
