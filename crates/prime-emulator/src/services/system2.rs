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
    let brightness = ctx.shared.backlight.lock().unwrap().brightness;
    ctx.mem
        .write_u16(m + 10, brightness)
        .map_err(HandlerFault)?; // brightness_level
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

/// ref: `SetSystemVariable(var = r0, type = r1, value = r2)`: var 6/type 2
/// → brightness; everything else → 0.
///
/// fw (M13): var 89 (`0x59, 0, 0` from 0x308CE570) is read as a getter by
/// the Memory Manager's Info screen, which shows it as "Memory" in bytes.
/// The reference returned -1, shown as "Memory: 4.0GB". It is the guest
/// heap's free bytes. Partial: whether the G1 reports free or total memory
/// here, and the device figure, are calibrated against the physical G1 in
/// M15 (the emulated heap is 256 MiB; the G1 has 32 MB of RAM).
pub fn set_system_variable(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let var = ctx.args.r0;
    let typ = ctx.args.r1;
    let value = ctx.args.r2;
    if var == 3 {
        // ROM table entry 3 at 0x30037B18: type 65535 reads without a write;
        // all other types replace the u16 value, returning its previous value.
        let mut power = ctx.shared.power.lock().unwrap();
        let previous = power.auto.system_mode;
        if typ != 0xffff {
            power.auto.system_mode = value as u16;
        }
        return Ok(u32::from(previous));
    }
    if (86..=88).contains(&var) {
        // ROM callbacks 0x30037C48/64/80: full u32 storage; type 0xffff
        // reads only, all other types exchange and return the old value.
        let mut power = ctx.shared.power.lock().unwrap();
        let slot = match var {
            86 => &mut power.auto.idle_inhibit,
            87 => &mut power.auto.shutdown_inhibit,
            _ => &mut power.auto.brightness_inhibit,
        };
        let previous = *slot;
        if typ != 0xffff {
            *slot = value;
        }
        return Ok(previous);
    }
    if var == 89 {
        return Ok(ctx.mem.heap_free_bytes().min(u32::MAX as usize) as u32);
    }
    if var == 6 {
        let inhibited = ctx.shared.power.lock().unwrap().auto.brightness_inhibit != 0;
        let result = ctx
            .shared
            .backlight
            .lock()
            .unwrap()
            .set_brightness((typ != 0xffff).then_some(value as u16), inhibited);
        if let Some(va) = *ctx.shared.lcd_object.lock().unwrap() {
            ctx.mem
                .write_u16(va + LCD_MAGIC_OFF + 10, result as u16)
                .map_err(HandlerFault)?;
        }
        return Ok(result);
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

mod debug_format;

/// Firmware debug printf, using the ARM EABI variadic register/stack ABI.
/// Parsed conversions use host-owned values; guest pointers never reach libc.
pub fn write_com_debug_msg(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    if let Some(fmt) = ctx.guest_str(ctx.args.r0)? {
        let formatted = debug_format::format::<HandlerFault>(
            &fmt,
            |index| match index {
                1 => Ok(ctx.args.r1),
                2 => Ok(ctx.args.r2),
                3 => Ok(ctx.args.r3),
                n => ctx.stack_arg(n - 4),
            },
            |address, wide, precision| debug_text(ctx, address, wide, precision),
            |address, bytes, count| {
                ctx.mem
                    .write_bytes(address, &(count as u64).to_le_bytes()[..bytes as usize])
                    .map_err(HandlerFault)
            },
        )?;
        match formatted {
            Some(message) => ctx.trace.thread_event(format!("guest: {message}")),
            None => ctx.trace.thread_event(format!(
                "guest debug format unsupported or exceeds limits: {fmt}"
            )),
        }
    }
    Ok(0)
}

fn debug_text(
    ctx: &SvcCtx,
    address: u32,
    wide: bool,
    precision: Option<usize>,
) -> Result<String, HandlerFault> {
    if address == 0 {
        return Ok("(null)".into());
    }
    let failure = |message: &str| {
        HandlerFault(crate::memory::MemoryError {
            va: address,
            msg: message.into(),
        })
    };
    let read_address = |offset: usize| {
        u32::try_from(offset)
            .ok()
            .and_then(|n| address.checked_add(n))
            .ok_or_else(|| failure("debug string address overflow"))
    };
    if !wide {
        let mut bytes = Vec::new();
        for index in 0..precision.unwrap_or(4096) {
            let byte = ctx
                .mem
                .read_bytes_alloc(read_address(index)?, 1)
                .map_err(HandlerFault)?[0];
            if byte == 0 {
                return String::from_utf8(bytes).map_err(|_| failure("invalid UTF-8 debug string"));
            }
            bytes.push(byte);
        }
        if precision.is_none() {
            return Err(failure("unterminated debug string exceeds 4096 bytes"));
        }
        // C narrow precision counts bytes. A final partial UTF-8 scalar is a
        // byte sequence, represented lossily only when logging it as Rust text.
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let mut output = String::new();
    let mut index = 0usize;
    while index < 4096 {
        if precision.is_some_and(|limit| output.len() >= limit) {
            return Ok(output);
        }
        let first = ctx
            .mem
            .read_u16(read_address(index * 2)?)
            .map_err(HandlerFault)?;
        if first == 0 {
            return Ok(output);
        }
        index += 1;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            if index == 4096 {
                return Err(failure("debug surrogate pair exceeds string limit"));
            }
            let second = ctx
                .mem
                .read_u16(read_address(index * 2)?)
                .map_err(HandlerFault)?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(failure("invalid UTF-16 debug string"));
            }
            index += 1;
            0x10000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00
        } else {
            u32::from(first)
        };
        let character =
            char::from_u32(scalar).ok_or_else(|| failure("invalid UTF-16 debug string"))?;
        if precision.is_some_and(|limit| output.len() + character.len_utf8() > limit) {
            return Ok(output);
        }
        output.push(character);
    }
    Err(failure(
        "unterminated debug string exceeds 4096 UTF-16 units",
    ))
}

/// Emulated user storage capacity (the G1's 256 MiB flash). Provisional:
/// calibrated against the physical G1's Memory Manager Info in M15.
pub const STORAGE_CAPACITY: u64 = 256 * 1024 * 1024;

/// Internal virtual volume0 (C:), using the OS ROM's complete disk-room
/// layout. Capacity is the configured emulator volume budget; no physical
/// partition geometry or FAT allocation-overhead calibration is claimed.
pub fn fs_get_disk_room_state(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    if ctx.args.r0 != 0 {
        return Ok(u32::MAX);
    }
    let out = ctx.args.r1;
    ctx.mem
        .validate_range(out, super::disk_room::SIZE)
        .map_err(HandlerFault)?;
    let used = match super::disk_room::used_bytes(&ctx.shared.data_dir.join("C")) {
        Ok(used) => used,
        Err(error) => {
            ctx.trace
                .thread_event(format!("FSGetDiskRoomState: {error}"));
            return Ok(u32::MAX);
        }
    };
    let capacity = ctx
        .shared
        .storage_capacity
        .load(std::sync::atomic::Ordering::Acquire);
    let bytes = super::disk_room::encode(capacity, capacity.saturating_sub(used));
    ctx.mem.write_bytes(out, &bytes).map_err(HandlerFault)?;
    Ok(0)
}

/// ROM mode ABI: 0 disable, 1 enable, 2 set interval in minutes and enable,
/// 3 request automatic sleep. Returns the previous enable state.
pub fn set_auto_power_off(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(ctx
        .shared
        .power
        .lock()
        .unwrap()
        .auto
        .configure(ctx.args.r0, ctx.args.r1))
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

// ── virtual devices: implemented names only, explicit failure otherwise ──

/// ref: `CreateFile(name = r0)` returns a device handle.
pub fn create_file(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let name = ctx
        .guest_str(ctx.args.r0)?
        .unwrap_or_default()
        .to_ascii_uppercase();
    // Initialized OS device registry is executable in os_rom_contract.rs.
    // Never allocate a successful handle to an absent or unimplemented driver.
    if !["ARCH", "BAT", "USB", "USBHOST", "INDICATOR", "BACK"]
        .iter()
        .any(|driver| name == format!(r"\\?\{driver}"))
    {
        ctx.trace
            .thread_event(format!("CreateFile: unsupported device/path {name:?}"));
        // The original OPEN_EXISTING path returns INVALID_HANDLE_VALUE for
        // an absent registry/stream device. Keep null-name failure separate.
        return Ok(if ctx.args.r0 != 0 && ctx.stack_arg(0)? == 3 {
            u32::MAX
        } else {
            0
        });
    }
    if name.ends_with("BACK") {
        ctx.shared.backlight.lock().unwrap().open();
    }
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
    if retlen_ptr != 0 {
        ctx.mem
            .validate_range(retlen_ptr, 4)
            .map_err(HandlerFault)?;
    }
    let Some(name) = ctx.shared.device_name(handle) else {
        if retlen_ptr != 0 {
            ctx.mem.write_u32(retlen_ptr, 0).map_err(HandlerFault)?;
        }
        ctx.trace
            .thread_event(format!("DeviceIoControl on unknown handle {handle:#X}"));
        return Ok(0);
    };
    if name.ends_with("INDICATOR") {
        let result = super::indicator::ioctl(ctx, request, in_ptr, size, out_ptr, outlen)?;
        if retlen_ptr != 0 {
            ctx.mem
                .write_u32(retlen_ptr, result)
                .map_err(HandlerFault)?;
        }
        return Ok(result);
    }
    if name.ends_with("BACK") {
        let inhibited = ctx.shared.power.lock().unwrap().auto.brightness_inhibit != 0;
        let mut back = ctx.shared.backlight.lock().unwrap();
        back.poll(ctx.shared.clock.now(), inhibited);
        let input = |ctx: &SvcCtx| ctx.mem.read_u32(in_ptr).map_err(HandlerFault);
        let output = |ctx: &SvcCtx, value: u32| -> Result<u32, HandlerFault> {
            if out_ptr != 0 {
                ctx.mem.write_u32(out_ptr, value).map_err(HandlerFault)?;
            }
            Ok(value)
        };
        let result = match request {
            8 => back.mode,
            9 => {
                if input(ctx)? == 4 {
                    back.mode = 4;
                    back.pwm = 0;
                } else {
                    back.open();
                }
                back.mode
            }
            256 => {
                let level = input(ctx)?;
                if back.level != level {
                    back.level = level;
                    back.duty = super::backlight::duty(level);
                    if back.mode != 4 && !inhibited {
                        back.pwm = back.duty;
                    }
                }
                u32::MAX
            }
            257 => output(ctx, back.level)?,
            258 => {
                back.duty = input(ctx)?;
                if back.mode != 4 && !inhibited {
                    back.pwm = back.duty;
                }
                0
            }
            259 => output(ctx, back.duty)?,
            260 => {
                back.timeout = input(ctx)?;
                0
            }
            261 => output(ctx, back.timeout)?,
            262 => {
                back.target = input(ctx)?;
                back.ramping = back.target != back.duty;
                back.last_step = ctx.shared.clock.now();
                back.target
            }
            263 => {
                let previous = back.option;
                back.option = input(ctx)?;
                previous
            }
            264 => output(ctx, super::backlight::duty(input(ctx)?))?,
            265 => output(ctx, 3)?,
            _ => u32::MAX,
        };
        if retlen_ptr != 0 {
            ctx.mem
                .write_u32(retlen_ptr, result)
                .map_err(HandlerFault)?;
        }
        return Ok(result);
    }
    if name.ends_with("BAT") {
        let config = ctx.shared.environment.lock().unwrap().battery;
        if retlen_ptr != 0 {
            ctx.mem
                .validate_range(retlen_ptr, 4)
                .map_err(HandlerFault)?;
        }
        let result = match request {
            256 if out_ptr != 0 && outlen == 16 => {
                let current: [u8; 16] = ctx
                    .mem
                    .read_bytes_alloc(out_ptr, 16)
                    .map_err(HandlerFault)?
                    .try_into()
                    .unwrap();
                let (output, result) = super::battery::query(config, current);
                ctx.mem
                    .write_bytes(out_ptr, &output)
                    .map_err(HandlerFault)?;
                result
            }
            257 if out_ptr != 0 && outlen == 4 => {
                ctx.mem
                    .write_u32(out_ptr, u32::from(super::battery::kind(config)))
                    .map_err(HandlerFault)?;
                0
            }
            _ => u32::MAX,
        };
        if retlen_ptr != 0 {
            ctx.mem
                .write_u32(retlen_ptr, result)
                .map_err(HandlerFault)?;
        }
        return Ok(result);
    }
    if name.ends_with("USB") || name.ends_with("USBHOST") {
        let host = name.ends_with("USBHOST");
        if retlen_ptr != 0 {
            ctx.mem
                .validate_range(retlen_ptr, 4)
                .map_err(HandlerFault)?;
        }
        let result = match request {
            4 | 5 => {
                ctx.shared.usb.enable(host, request == 4);
                1
            }
            256 => u32::from(ctx.shared.usb.connected(host)),
            257 if host => {
                if let Some(bytes) = ctx.shared.usb.host_descriptor(ctx.args.r3) {
                    ctx.mem.write_bytes(in_ptr, &bytes).map_err(HandlerFault)?;
                    1
                } else {
                    0
                }
            }
            261 | 262 if host => u32::from(ctx.shared.usb.initialize_host()),
            258 => {
                // The native driver consumes the input payload, not the output
                // buffer. Disconnected sends return zero without dereferencing it.
                if !ctx.shared.usb.connected(host) || size > 1024 * 1024 {
                    0
                } else {
                    let bytes = ctx
                        .mem
                        .read_bytes_alloc(in_ptr, size)
                        .map_err(HandlerFault)?;
                    u32::from(ctx.shared.usb.transmit(host, bytes))
                }
            }
            259 => {
                if in_ptr != 0 {
                    ctx.mem
                        .validate_range(in_ptr & !1, 4)
                        .map_err(HandlerFault)?;
                }
                ctx.shared.usb.callback(host, in_ptr);
                1
            }
            260 => ctx.shared.usb.mode(),
            // The OS negotiates this peer extension separately from cable
            // presence. A standard virtual PC has no calculator extension.
            263 => ctx.shared.usb.peer_extension(host),
            _ => {
                ctx.trace.thread_event(format!(
                    "unsupported virtual USB{} request {request}",
                    if host { "HOST" } else { "" }
                ));
                u32::MAX
            }
        };
        if retlen_ptr != 0 {
            ctx.mem
                .write_u32(retlen_ptr, result)
                .map_err(HandlerFault)?;
        }
        return Ok(result);
    }
    if name.ends_with("ARCH") {
        // Original 0x30004D68 distinguishes the legacy revision query from
        // the raw two GPIO bits. Neither returns a Boolean success code.
        let pins = u32::from(ctx.shared.environment.lock().unwrap().board_revision_pins);
        let result = match request {
            260 | 261 if out_ptr != 0 && outlen == 4 => {
                let value = if request == 260 && pins == 3 { 1 } else { pins };
                ctx.mem.write_u32(out_ptr, value).map_err(HandlerFault)?;
                0
            }
            _ => {
                ctx.trace
                    .thread_event(format!("unsupported virtual ARCH request {request}"));
                u32::MAX
            }
        };
        if retlen_ptr != 0 {
            ctx.mem
                .write_u32(retlen_ptr, result)
                .map_err(HandlerFault)?;
        }
        return Ok(result);
    }
    ctx.trace.thread_event(format!(
        "DeviceIoControl: unsupported driver {name:?}, request {request}"
    ));
    if retlen_ptr != 0 {
        ctx.mem
            .write_u32(retlen_ptr, u32::MAX)
            .map_err(HandlerFault)?;
    }
    Ok(u32::MAX)
}

#[cfg(test)]
mod debug_format_tests {
    use super::*;
    #[test]
    fn formats_common_arm_debug_arguments_in_order() {
        let mut args = [u32::MAX, 123, 0xabcd, 0x100, 65, 7].into_iter();
        assert_eq!(
            debug_format::format::<HandlerFault>(
                "n=%d u=%u x=%#08x s=%ls c=%c tail=%u %%",
                |_| Ok(args.next().unwrap()),
                |p, wide, _| {
                    assert_eq!(p, 0x100);
                    assert!(wide);
                    Ok("hello".into())
                },
                |_, _, _| unreachable!()
            )
            .unwrap(),
            Some("n=-1 u=123 x=0x00abcd s=hello c=A tail=7 %".into())
        );
        assert!(args.next().is_none());
    }
    #[test]
    fn preserves_sign_and_string_padding_and_reports_unknown_formats() {
        let mut args = [(-12i32) as u32, 3].into_iter();
        assert_eq!(
            debug_format::format::<HandlerFault>(
                "%06d %-5s",
                |_| Ok(args.next().unwrap()),
                |_, _, _| Ok("abc".into()),
                |_, _, _| unreachable!()
            )
            .unwrap(),
            Some("-00012 abc  ".into())
        );
        let mut args = [7, 0, 0].into_iter();
        assert_eq!(
            debug_format::format::<HandlerFault>(
                "% 05d %#x %#X",
                |_| Ok(args.next().unwrap()),
                |_, _, _| unreachable!(),
                |_, _, _| unreachable!()
            )
            .unwrap(),
            Some(" 0007 0 0".into())
        );
        for format in ["%q", "%999999999d", "%"] {
            assert_eq!(
                debug_format::format::<HandlerFault>(
                    format,
                    |_| panic!("unsupported format must not read arguments"),
                    |_, _, _| unreachable!(),
                    |_, _, _| unreachable!()
                )
                .unwrap(),
                None
            );
        }
    }
}
