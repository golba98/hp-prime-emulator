//! Implemented OS service handlers.
//!
//! Every handler documents the evidence for its argument and return
//! conventions. References:
//! - "ref" = PrimU2 commit 461d994 (`PrimU/svc_*.cpp`, `Marshal.h`);
//! - "fw"  = verified directly in the 20260909 armfir.elf image or its
//!   companion SDKLIB.DLL during Phase 1/2.
//!
//! Handlers are implemented on demand in execution order; nothing here is
//! registered "just in case". See M2_SVC_IMPLEMENTATIONS.md for the ledger.

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use super::fsops;
use super::system2;
use super::threading;
use super::{HandlerFault, Registry, ServiceStatus, SvcCtx, UI_EVENT_TYPE_KEY_BATCH};

pub fn install(registry: &mut Registry) {
    // ── Memory (fw: most-called stubs; ref: svc_system.cpp lmalloc family) ──
    registry.register_by_name("lmalloc", ServiceStatus::Implemented, lmalloc);
    registry.register_by_name("lcalloc", ServiceStatus::Implemented, lcalloc);
    registry.register_by_name("lrealloc", ServiceStatus::Implemented, lrealloc);
    registry.register_by_name("_lfree", ServiceStatus::Implemented, lfree);

    // ── Startup / identity (fw: entry calls GetCurrentPathA when r0 != 4;
    //    ref: PrimU2 routes GetCurrentPathA to its GetCurrentExecutable impl) ──
    registry.register_by_name(
        "GetCurrentPathA",
        ServiceStatus::PartiallyImplemented,
        get_current_path,
    );
    registry.register_by_name("GetSysTime", ServiceStatus::Implemented, get_sys_time);

    // ── Blocking / power (ref: svc_system.cpp GetEvent block loop,
    //    OSSleep → native sleep, BatteryLowCheck → 0) ──
    registry.register_by_name("OSSleep", ServiceStatus::Implemented, os_sleep);
    registry.register_by_name("GetEvent", ServiceStatus::PartiallyImplemented, get_event);
    registry.register_by_name(
        "BatteryLowCheck",
        ServiceStatus::Implemented,
        battery_low_check,
    );

    // ── Program management (fw: hit at pc 0x30615A94 during M2 run 1;
    //    ref: svc_system.cpp ProgramIsRunningA/W allocate one zeroed 0x250
    //    byte program struct in the guest heap and return its pointer) ──
    registry.register_by_name(
        "ProgramIsRunningA",
        ServiceStatus::PartiallyImplemented,
        program_is_running,
    );
    registry.register_by_name(
        "ProgramIsRunningW",
        ServiceStatus::PartiallyImplemented,
        program_is_running,
    );

    // ── Interrupt registration (ref: svc_system.cpp InterruptInitialize
    //    registers (irq, isr=r2) and returns 1; Mask/Disable/Done return 1.
    //    Note: id 0x1001F's verified SDKLIB export name is InterruptEnable;
    //    PrimU2's enum mislabels it InterruptMask — same id, same handler.)
    //    Partial: ISRs are recorded but not dispatched yet) ──
    registry.register_by_name(
        "InterruptInitialize",
        ServiceStatus::PartiallyImplemented,
        interrupt_initialize,
    );
    registry.register_by_name(
        "InterruptEnable",
        ServiceStatus::PartiallyImplemented,
        interrupt_noop_one,
    );
    registry.register_by_name(
        "InterruptDisable",
        ServiceStatus::PartiallyImplemented,
        interrupt_disable,
    );
    registry.register_by_name(
        "InterruptDone",
        ServiceStatus::PartiallyImplemented,
        interrupt_noop_one,
    );

    // ── Threads (fw: OSCreateThread hit during M2 run 2; ref: native
    //    thread + own engine per guest thread, PrimU2 Thread.cpp) ──
    registry.register_by_name(
        "OSCreateThread",
        ServiceStatus::PartiallyImplemented,
        threading::os_create_thread,
    );
    registry.register_by_name(
        "OSTerminateThread",
        ServiceStatus::PartiallyImplemented,
        threading::os_terminate_thread,
    );
    registry.register_by_name(
        "OSExitThread",
        ServiceStatus::PartiallyImplemented,
        threading::os_exit_thread,
    );
    registry.register_by_name(
        "OSSuspendThread",
        ServiceStatus::PartiallyImplemented,
        threading::os_thread_stub_one,
    );
    registry.register_by_name(
        "OSResumeThread",
        ServiceStatus::PartiallyImplemented,
        threading::os_thread_stub_one,
    );
    registry.register_by_name(
        "OSWakeUpThread",
        ServiceStatus::PartiallyImplemented,
        threading::os_thread_stub_one,
    );
    registry.register_by_name(
        "OSSetThreadPriority",
        ServiceStatus::PartiallyImplemented,
        threading::os_set_thread_priority,
    );
    registry.register_by_name(
        "OSGetThreadPriority",
        ServiceStatus::PartiallyImplemented,
        threading::os_get_thread_priority,
    );

    // ── Sync primitives (ref: svc_thread.cpp descriptor + wait-result ABI) ──
    registry.register_by_name(
        "OSCreateEvent",
        ServiceStatus::Implemented,
        threading::os_create_event,
    );
    registry.register_by_name(
        "OSWaitForEvent",
        ServiceStatus::Implemented,
        threading::os_wait_for_event,
    );
    registry.register_by_name(
        "OSSetEvent",
        ServiceStatus::Implemented,
        threading::os_set_event,
    );
    registry.register_by_name(
        "OSResetEvent",
        ServiceStatus::Implemented,
        threading::os_reset_event,
    );
    registry.register_by_name(
        "OSCloseEvent",
        ServiceStatus::Implemented,
        threading::os_close_object,
    );
    registry.register_by_name(
        "OSCreateSemaphore",
        ServiceStatus::Implemented,
        threading::os_create_semaphore,
    );
    registry.register_by_name(
        "OSWaitForSemaphore",
        ServiceStatus::Implemented,
        threading::os_wait_for_semaphore,
    );
    registry.register_by_name(
        "OSReleaseSemaphore",
        ServiceStatus::Implemented,
        threading::os_release_semaphore,
    );
    registry.register_by_name(
        "OSCloseSemaphore",
        ServiceStatus::Implemented,
        threading::os_close_object,
    );
    registry.register_by_name(
        "OSInitCriticalSection",
        ServiceStatus::Implemented,
        threading::os_init_critical_section,
    );
    registry.register_by_name(
        "OSEnterCriticalSection",
        ServiceStatus::Implemented,
        threading::os_enter_critical_section,
    );
    registry.register_by_name(
        "OSLeaveCriticalSection",
        ServiceStatus::Implemented,
        threading::os_leave_critical_section,
    );
    registry.register_by_name(
        "OSDeleteCriticalSection",
        ServiceStatus::Implemented,
        threading::os_delete_critical_section,
    );

    // ── LCD / identity / debug (fw: hit in M2 run 2; ref: LCD.cpp,
    //    svc_system.cpp) ──
    registry.register_by_name(
        "GetActiveLCD",
        ServiceStatus::PartiallyImplemented,
        system2::get_active_lcd,
    );
    registry.register_by_name("LCDOn", ServiceStatus::Implemented, system2::lcd_on);
    registry.register_by_name(
        "SetSystemVariable",
        ServiceStatus::PartiallyImplemented,
        system2::set_system_variable,
    );
    registry.register_by_name(
        "GetMasterIDInfo",
        ServiceStatus::PartiallyImplemented,
        system2::get_master_id_info,
    );
    registry.register_by_name(
        "WriteComDebugMsg",
        ServiceStatus::PartiallyImplemented,
        system2::write_com_debug_msg,
    );
    registry.register_by_name(
        "SetAutoPowerOff",
        ServiceStatus::PartiallyImplemented,
        system2::set_auto_power_off,
    );

    // ── Devices (ref: svc_system.cpp CreateFile/DeviceIoControl/CloseHandle) ──
    registry.register_by_name(
        "_LoadLibraryA",
        ServiceStatus::Implemented,
        system2::load_library_a,
    );
    registry.register_by_name(
        "_LoadLibraryW",
        ServiceStatus::Implemented,
        system2::load_library_w,
    );
    registry.register_by_name(
        "_GetProcAddress",
        ServiceStatus::Implemented,
        system2::get_proc_address,
    );
    registry.register_by_name(
        "_FreeLibrary",
        ServiceStatus::Implemented,
        system2::free_library,
    );
    registry.register_by_name(
        "_GetModuleFileNameA",
        ServiceStatus::Implemented,
        system2::get_module_file_name_a,
    );
    registry.register_by_name(
        "CreateFile",
        ServiceStatus::PartiallyImplemented,
        system2::create_file,
    );
    registry.register_by_name(
        "DeviceIoControl",
        ServiceStatus::PartiallyImplemented,
        system2::device_io_control,
    );
    registry.register_by_name(
        "CloseHandle",
        ServiceStatus::PartiallyImplemented,
        system2::close_handle,
    );

    // ── Filesystem (fw: hit in M2 run 2; ref: svc_filesystem.cpp) ──
    registry.register_by_name("__wfopen", ServiceStatus::Implemented, fsops::w_fopen);
    registry.register_by_name("_fread", ServiceStatus::Implemented, fsops::fread);
    registry.register_by_name("_fwrite", ServiceStatus::Implemented, fsops::fwrite);
    registry.register_by_name("_fclose", ServiceStatus::Implemented, fsops::fclose);
    registry.register_by_name("__fseek", ServiceStatus::Implemented, fsops::fseek);
    registry.register_by_name("_ftell", ServiceStatus::Implemented, fsops::ftell);
    registry.register_by_name("_filesize", ServiceStatus::Implemented, fsops::filesize);
    registry.register_by_name("_wmkdir", ServiceStatus::Implemented, fsops::w_mkdir);
    registry.register_by_name("_wrmdir", ServiceStatus::Implemented, fsops::w_rmdir);
    registry.register_by_name("_wchdir", ServiceStatus::Implemented, fsops::w_chdir);
    registry.register_by_name(
        "_wfindfirst",
        ServiceStatus::Implemented,
        fsops::w_findfirst,
    );
    registry.register_by_name("_wfindnext", ServiceStatus::Implemented, fsops::w_findnext);
    registry.register_by_name("_findclose", ServiceStatus::Implemented, fsops::findclose);
    registry.register_by_name("_OpenFile", ServiceStatus::Implemented, fsops::open_file);
    registry.register_by_name(
        "_OpenSubFile",
        ServiceStatus::Implemented,
        fsops::open_sub_file,
    );
    registry.register_by_name("_CloseFile", ServiceStatus::Implemented, fsops::close_file);
    registry.register_by_name("_FileSize", ServiceStatus::Implemented, fsops::file_size);
    registry.register_by_name("_FseekFile", ServiceStatus::Implemented, fsops::fseek_file);
    registry.register_by_name("_ReadFile", ServiceStatus::Implemented, fsops::read_file);
    registry.register_by_name(
        "_GetPrivateProfileString",
        ServiceStatus::PartiallyImplemented,
        fsops::get_private_profile_string,
    );
    registry.register_by_name(
        "_WritePrivateProfileString",
        ServiceStatus::PartiallyImplemented,
        fsops::write_private_profile_string,
    );
    registry.register_by_name("_afnsplit", ServiceStatus::Implemented, fsops::a_fnsplit);
    registry.register_by_name("_afnmerge", ServiceStatus::Implemented, fsops::a_fnmerge);
    registry.register_by_name("__wremove", ServiceStatus::Implemented, fsops::w_remove);

    // ── Event-type filter (fw: the UI pump at 0x3060708C calls
    //    GetEventType() | 0x20 → SetEventType() before its GetEvent loop;
    //    both were exercised by the real firmware in the M3 heartbeat runs) ──
    registry.register_by_name(
        "SetEventType",
        ServiceStatus::PartiallyImplemented,
        set_event_type,
    );
    registry.register_by_name(
        "GetEventType",
        ServiceStatus::PartiallyImplemented,
        get_event_type,
    );
}

// ─────────────────────────────── memory ───────────────────────────────

/// ref: `lmalloc(r0 = size)` returns a heap pointer, 0 on failure.
fn lmalloc(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let size = ctx.args.r0 as usize;
    let va = ctx.mem.heap_alloc(size).map_err(HandlerFault)?;
    Ok(va)
}

/// ref: `lcalloc(r0 = elem, r1 = count)`. We zero the block (correct calloc
/// semantics); the reference does not zero on reuse — noted divergence.
fn lcalloc(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let bytes = ctx.args.r0 as usize * ctx.args.r1 as usize;
    let va = ctx.mem.heap_alloc(bytes).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(va, &vec![0u8; bytes])
        .map_err(HandlerFault)?;
    Ok(va)
}

/// ref: `lrealloc(r0 = ptr, r1 = new_size)`: new block, copy, free old.
/// ptr == 0 behaves like malloc (ref handles this case explicitly).
fn lrealloc(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    let new_size = ctx.args.r1 as usize;
    if ptr == 0 {
        return ctx.mem.heap_alloc(new_size).map_err(HandlerFault);
    }
    let old_size = ctx.mem.heap_size_of(ptr).unwrap_or(0);
    let new_va = ctx.mem.heap_alloc(new_size).map_err(HandlerFault)?;
    let copy = old_size.min(new_size);
    if copy > 0 {
        let buf = ctx.mem.read_bytes_alloc(ptr, copy).map_err(HandlerFault)?;
        ctx.mem.write_bytes(new_va, &buf).map_err(HandlerFault)?;
    }
    let _ = ctx.mem.heap_free(ptr);
    Ok(new_va)
}

/// ref: `_lfree(r0 = ptr)` returns its argument.
fn lfree(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    if ptr != 0 && !ctx.mem.heap_free(ptr) {
        ctx.trace.fault(
            format!("_lfree of non-heap pointer {ptr:#010X} ignored"),
            ctx.args.caller_pc,
        );
    }
    Ok(ptr)
}

// ─────────────────────────────── startup ──────────────────────────────

/// ref: PrimU2 registers `GetCurrentPathA` → `GetCurrentExecutable`
/// (Services.cpp:164-165), which returns a guest pointer to the running
/// image's VM path. Argument registers are ignored by the reference; we
/// match that. Partial because the reference itself reuses an adjacent
/// handler — exact per-service ABI unverified.
fn get_current_path(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    ctx.heap_cstr(ctx.shared.firmware_guest_path())
}

/// ref: `GetSysTime(r0 = SYSTEMTIME*)` fills {Year,Month,DayOfWeek,Day,
/// Hour,Minute,Second,Milliseconds} as u16 fields from host local time and
/// returns r0.
fn get_sys_time(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let out = ctx.args.r0;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86_400;
    let (y, mo, d, dow) = civil_from_days(days as i64);
    let tod = secs % 86_400;
    let st = [
        (y as u16).to_le_bytes(),   // wYear
        (mo as u16).to_le_bytes(),  // wMonth
        (dow as u16).to_le_bytes(), // wDayOfWeek
        (d as u16).to_le_bytes(),   // wDay
        ((tod / 3600) as u16).to_le_bytes(),
        (((tod / 60) % 60) as u16).to_le_bytes(),
        ((tod % 60) as u16).to_le_bytes(),
        ((now.subsec_millis()) as u16).to_le_bytes(),
    ];
    for (i, f) in st.iter().enumerate() {
        ctx.mem
            .write_bytes(out + (i as u32) * 2, f)
            .map_err(HandlerFault)?;
    }
    Ok(out)
}

/// Days-since-epoch to civil date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    let weekday = ((z % 7) + 7) % 7; // 0 = Thursday-based epoch; adjust to Windows (0 = Sunday)
    let dow = ((weekday + 4) % 7) as u32;
    (year, m, d, dow)
}

// ─────────────────────────────── blocking ─────────────────────────────

/// ref: `OSSleep(r0 = ms)` — native sleep of the calling guest thread.
fn os_sleep(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ms = ctx.args.r0 as u64;
    let mut left = ms;
    while left > 0 {
        if ctx.shared.stopping() {
            break;
        }
        let step = left.min(25);
        ctx.bounded_sleep(step);
        left -= step;
    }
    Ok(0)
}

/// ref: `GetEvent(r0 = ui_event_prime_s*)` never returns empty: the firmware
/// parks in its event pump until an input event exists. Layout verified in
/// PrimU2 ui.h (24-byte header + 8×12-byte multipress events). Partial:
/// bounded wake on runner shutdown, batch semantics simplified to whatever
/// is queued (up to 8).
fn get_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    const EV_SIZE: u32 = 124; // 28-byte header + 8 * 12
    const MP_OFF: u32 = 28;
    let out = ctx.args.r0;
    ctx.shared.note_getevent_chain([
        ctx.args.caller_pc,
        ctx.args.stack_slots[0],
        ctx.args.stack_slots[1],
    ]);
    ctx.shared.note_getevent_entry();
    loop {
        // Timer/system events first (reference: `special_event` precedes
        // touch and key input). Types >= 0x100 and DEVICE_TIMER are
        // delivered as their own event type; smaller codes are SYSTEM
        // events with the code as subtype.
        if let Some(k) = ctx.shared.pop_system_event() {
            ctx.mem
                .write_bytes(out, &vec![0u8; EV_SIZE as usize])
                .map_err(HandlerFault)?;
            if k < 0x100 && k != super::UI_EVENT_TYPE_DEVICE_TIMER {
                ctx.mem
                    .write_u32(out + 4, super::UI_EVENT_TYPE_SYSTEM)
                    .map_err(HandlerFault)?;
                ctx.mem.write_u32(out + 8, k).map_err(HandlerFault)?;
            } else {
                ctx.mem.write_u32(out + 4, k).map_err(HandlerFault)?;
            }
            ctx.shared.set_getevent_parked(false);
            return Ok(0);
        }
        // Touch before keys (reference order).
        if let Some(t) = ctx.shared.pop_touch() {
            ctx.mem
                .write_bytes(out, &vec![0u8; EV_SIZE as usize])
                .map_err(HandlerFault)?;
            ctx.mem
                .write_u32(out + 4, super::UI_EVENT_TYPE_TOUCH)
                .map_err(HandlerFault)?;
            ctx.mem.write_u16(out + 24, 1).map_err(HandlerFault)?;
            ctx.mem
                .write_u32(out + MP_OFF, t.status)
                .map_err(HandlerFault)?;
            ctx.mem
                .write_u16(out + MP_OFF + 4, 0)
                .map_err(HandlerFault)?; // finger_id
            ctx.mem
                .write_u16(out + MP_OFF + 6, t.x)
                .map_err(HandlerFault)?;
            ctx.mem
                .write_u16(out + MP_OFF + 8, t.y)
                .map_err(HandlerFault)?;
            ctx.shared.set_getevent_parked(false);
            return Ok(0);
        }
        let events = ctx.shared.pop_input_events(8);
        if !events.is_empty() {
            ctx.shared.set_getevent_parked(false);
            // Zero the whole event struct first (ref: `event = {}`).
            ctx.mem
                .write_bytes(out, &vec![0u8; EV_SIZE as usize])
                .map_err(HandlerFault)?;
            ctx.mem
                .write_u32(out + 4, UI_EVENT_TYPE_KEY_BATCH)
                .map_err(HandlerFault)?;
            ctx.mem
                .write_u16(out + 24, events.len() as u16)
                .map_err(HandlerFault)?;
            for (i, ev) in events.iter().enumerate() {
                let p = out + MP_OFF + (i as u32) * 12;
                ctx.mem.write_u32(p, ev.status).map_err(HandlerFault)?;
                ctx.mem.write_u16(p + 4, 0).map_err(HandlerFault)?; // finger_id
                ctx.mem
                    .write_u16(p + 6, ev.key_code0)
                    .map_err(HandlerFault)?;
            }
            ctx.shared.note_input_delivered(ctx.thread_id);
            return Ok(0);
        }
        if ctx.shared.stopping() {
            // Unwind so the run loop can end the session; the event written
            // is all-zero (event_type = UI_EVENT_TYPE_INVALID = 0).
            ctx.mem
                .write_bytes(out, &vec![0u8; EV_SIZE as usize])
                .map_err(HandlerFault)?;
            return Ok(0);
        }
        // Nothing queued: the pump is idle until input arrives.
        ctx.shared.set_getevent_parked(true);
        ctx.bounded_sleep(25);
    }
}

/// ref: `BatteryLowCheck()` returns 0 (battery OK; -1 low, -2 critical).
fn battery_low_check(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(0)
}

/// ref: `ProgramIsRunningA/W(name)` return a pointer to the running-program
/// record; the reference allocates ONE zeroed 0x250-byte struct in the guest
/// heap and returns the same pointer on every call (i.e. "no program
/// running"). Partial: struct semantics not yet verified beyond the
/// reference behavior.
fn program_is_running(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let mut slot = ctx.shared.program_record.lock().unwrap();
    if let Some(va) = *slot {
        return Ok(va);
    }
    let va = ctx.mem.heap_alloc(0x250).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(va, &vec![0u8; 0x250])
        .map_err(HandlerFault)?;
    *slot = Some(va);
    Ok(va)
}

/// fw/M3-verified: the pump's prologue is `GetEventType() → r0 |= 0x20 →
/// SetEventType(r0)` (call sites 0x30607090/0x30607098). A stored filter is
/// the faithful get/set semantics; GetEvent does not filter on it yet
/// (documented divergence, see SharedState::event_type_filter).
fn set_event_type(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    ctx.shared
        .event_type_filter
        .store(ctx.args.r0, Ordering::Relaxed);
    Ok(ctx.args.r0)
}

fn get_event_type(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(_ctx.shared.event_type_filter.load(Ordering::Relaxed))
}

// ─────────────────────────────── interrupts ───────────────────────────

/// ref: `InterruptInitialize(irq = r0, ? = r1, isr = r2)` registers the ISR
/// and returns 1. Partial: recorded, not dispatched.
fn interrupt_initialize(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let irq = ctx.args.r0;
    let isr = ctx.args.r2;
    ctx.shared.register_isr(irq, isr);
    ctx.trace
        .thread_event(format!("guest registered ISR irq={irq} isr={isr:#010X}"));
    Ok(1)
}

/// ref: InterruptMask / InterruptDisable / InterruptDone return 1.
fn interrupt_noop_one(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(1)
}

/// ref: `InterruptDisable(irq = r0)` unregisters the ISR (PrimU2
/// `InterruptController::UnregisterISR`), preventing further delivery until
/// the firmware re-registers it with InterruptInitialize.
fn interrupt_disable(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let irq = ctx.args.r0;
    ctx.shared.unregister_isr(irq);
    ctx.trace
        .thread_event(format!("guest disabled IRQ {irq} (unregistered)"));
    Ok(1)
}
