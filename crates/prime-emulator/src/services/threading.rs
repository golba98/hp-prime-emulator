//! Thread services. `OSCreateThread` creates a real guest thread: a new
//! native thread with its own Unicorn engine over the shared memory blocks
//! (the reference emulator's proven model). The engine construction happens
//! inside the spawned thread (Unicorn handles are not `Send`).

use super::{HandlerFault, SvcCtx};

/// ref: thread descriptor is 0x74 bytes (installed OS ROM); magic 0x100 at +0, tid at +0x14,
/// thread_func at +0x18, exit_code at +0xC, priority slot at +0x22.
pub const THREAD_DESC_SIZE: usize = 0x74;
pub const THREAD_MAGIC: u32 = 0x100;

/// ref: `OSCreateThread(func = r0, user_data = r1, stack_size = r2,
/// defer_start = r3)` returns a guest thread descriptor pointer.
pub fn os_create_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let func = ctx.args.r0;
    let user_data = ctx.args.r1;
    let stack_size = (ctx.args.r2 as usize).max(0x1000);
    let defer_start = ctx.args.r3 & 1;

    let desc = ctx.mem.heap_alloc(THREAD_DESC_SIZE).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(desc, &[0u8; THREAD_DESC_SIZE])
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(desc, THREAD_MAGIC)
        .map_err(HandlerFault)?;
    ctx.mem.write_u32(desc + 0x18, func).map_err(HandlerFault)?;

    let tid = ctx.shared.alloc_thread_id();
    ctx.mem.write_u32(desc + 0x14, tid).map_err(HandlerFault)?;
    ctx.mem
        .write_u16(desc + 0x20, if defer_start != 0 { 8 } else { 0 })
        .map_err(HandlerFault)?;
    let priority = ctx.shared.create_guest_thread(
        tid,
        super::ThreadRecord {
            entry: func,
            arg: user_data,
            stack_size,
        },
        defer_start != 0,
    );
    let Some(priority) = priority else {
        ctx.mem.heap_free(desc);
        return Ok(0);
    };
    ctx.mem
        .write_u16(desc + 0x22, priority)
        .map_err(HandlerFault)?;
    if defer_start == 0 {
        ctx.trace.thread_event(format!(
            "guest thread {tid} spawned: func {func:#010X} arg {user_data:#010X} stack {stack_size:#X}"
        ));
    } else {
        ctx.trace.thread_event(format!(
            "guest thread deferred: func {func:#010X} arg {user_data:#010X}"
        ));
    }
    Ok(desc)
}

/// ref: `OSTerminateThread(desc, exit_code)` writes exit_code at +0xC and
/// returns 0. The execution owner and blocked service waits observe termination.
pub fn os_terminate_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    if desc != 0 {
        if ctx.mem.read_u32(desc).map_err(HandlerFault)? != THREAD_MAGIC {
            return Ok(0);
        }
        let tid = ctx.mem.read_u32(desc + 0x14).map_err(HandlerFault)?;
        ctx.shared.terminate_guest_thread(tid);
        ctx.mem
            .write_u32(desc + 0x0C, ctx.args.r1)
            .map_err(HandlerFault)?;
        if tid == ctx.thread_id {
            let _ = ctx.uc.emu_stop();
        }
    }
    Ok(0)
}

/// Reference stub returns 0 and lets the thread continue; we mark the
/// calling host thread as exiting so its run loop stops cleanly after the
/// slice (documented divergence — see M2_SVC_IMPLEMENTATIONS.md).
pub fn os_exit_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let code = ctx.args.r0;
    ctx.shared.scheduler.retire(ctx.thread_id);
    ctx.shared.mark_thread_exiting();
    ctx.trace
        .thread_event(format!("guest thread exit requested ({code})"));
    let _ = ctx.uc.emu_stop();
    Ok(0)
}

fn descriptor_thread(ctx: &SvcCtx) -> Result<Option<u32>, HandlerFault> {
    if ctx.args.r0 == 0 {
        return Ok(None);
    }
    if ctx.mem.read_u32(ctx.args.r0).map_err(HandlerFault)? != THREAD_MAGIC {
        return Ok(None);
    }
    Ok(Some(
        ctx.mem.read_u32(ctx.args.r0 + 0x14).map_err(HandlerFault)?,
    ))
}
pub fn os_suspend_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let Some(tid) = descriptor_thread(ctx)? else {
        return Ok(0);
    };
    let suspended = ctx.shared.suspend_guest_thread(tid);
    if suspended {
        let flags = ctx.mem.read_u16(ctx.args.r0 + 0x20).map_err(HandlerFault)?;
        ctx.mem
            .write_u16(ctx.args.r0 + 0x20, flags | 8)
            .map_err(HandlerFault)?;
    }
    if suspended && tid == ctx.thread_id {
        let _ = ctx.uc.emu_stop();
    }
    Ok(u32::from(suspended))
}
pub fn os_resume_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let Some(tid) = descriptor_thread(ctx)? else {
        return Ok(0);
    };
    let resumed = ctx.shared.resume_guest_thread(tid);
    if resumed {
        let flags = ctx.mem.read_u16(ctx.args.r0 + 0x20).map_err(HandlerFault)?;
        ctx.mem
            .write_u16(ctx.args.r0 + 0x20, flags & !8)
            .map_err(HandlerFault)?;
    }
    Ok(u32::from(resumed))
}

/// Installed ROM 0x30001DAC: clear timed sleep, keep suspension unchanged.
pub fn os_wake_up_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let Some(tid) = descriptor_thread(ctx)? else {
        return Ok(0);
    };
    let woke = ctx.shared.wake_guest_thread(tid);
    if woke {
        ctx.mem
            .write_u16(ctx.args.r0 + 0x1e, 0)
            .map_err(HandlerFault)?;
    }
    Ok(u32::from(woke))
}

/// Installed ROM requires a free priority slot in 0..63.
pub fn os_set_thread_priority(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let Some(tid) = descriptor_thread(ctx)? else {
        return Ok(0);
    };
    ctx.mem
        .validate_range(ctx.args.r0, 0x24)
        .map_err(HandlerFault)?;
    if ctx.args.r1 > 63 || !ctx.shared.scheduler.set_priority(tid, ctx.args.r1 as u16) {
        return Ok(0);
    }
    ctx.mem
        .write_u16(ctx.args.r0 + 0x22, ctx.args.r1 as u16)
        .map_err(HandlerFault)?;
    Ok(1)
}

pub fn os_get_thread_priority(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let Some(tid) = descriptor_thread(ctx)? else {
        return Ok(0x78);
    };
    Ok(ctx.shared.scheduler.priority(tid).map_or(0x78, u32::from))
}

// Sync services over host primitives (conventions from the reference):

pub fn os_create_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let latch_on = ctx.args.r0 != 0;
    let flag = ctx.args.r1 != 0;
    let Some(desc) = alloc_desc(ctx, super::sync::EVENT_MAGIC)? else {
        return Ok(0);
    };
    ctx.trace
        .sync_event("event-create", desc, ctx.args.caller_pc, ctx.thread_id);
    ctx.mem
        .write_u32(desc + 0x04, flag as u32)
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u16(desc + 0x08, latch_on as u16)
        .map_err(HandlerFault)?;
    ctx.shared.sync_objects.lock().unwrap().insert(
        desc,
        std::sync::Arc::new(super::sync::SyncEntry::new(
            super::sync::SyncObj::Event(super::sync::HostEvent::new(latch_on, flag)),
            Some((ctx.mem.clone(), desc)),
        )),
    );
    Ok(desc)
}

pub fn os_wait_for_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    ctx.trace
        .sync_event("event-wait", desc, ctx.args.caller_pc, ctx.thread_id);
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    let Some(obj) = obj else {
        return Ok(super::sync::WAIT_RESULT_ERROR);
    };
    match &obj.object {
        super::sync::SyncObj::Event(_) => {}
        _ => return Ok(super::sync::WAIT_RESULT_ERROR),
    }
    // The main thread blocked in a wait is the UI at rest (key scripts
    // only send the next input then).
    let main = ctx.thread_id == 0;
    // Host guest calls run here, at the main loop's wait for the next event
    // (see `guest_call`); the wait is then re-entered from scratch.
    let call_point = main && ctx.args.caller_pc == crate::guest_call::MAIN_WAIT_CALLER;
    if call_point && inject_call(ctx)? {
        return Ok(0);
    }
    if main {
        *ctx.shared.main_ui_wait_owner.lock().unwrap() = Some((desc, ctx.thread_id));
    }
    if main {
        ctx.shared.set_main_waiting(true);
    }
    ctx.deferred = Some(super::pending::PendingService::event(
        ctx.args,
        obj,
        main,
        call_point,
        ctx.thread_id,
    ));
    Ok(0)
}

/// Starts the pending guest call, if any; true when the registers now enter
/// it (the SVC's normal return must then be skipped).
fn inject_call(ctx: &mut SvcCtx) -> Result<bool, HandlerFault> {
    match ctx.shared.guest_calls.inject(ctx.uc, ctx.mem) {
        Ok(true) => {
            ctx.redirected = true;
            Ok(true)
        }
        Ok(false) => Ok(false),
        Err(e) => {
            ctx.trace
                .fault(format!("guest call: {e}"), ctx.args.caller_pc);
            Ok(false)
        }
    }
}

pub fn os_set_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    match obj.as_ref().map(|entry| &entry.object) {
        Some(super::sync::SyncObj::Event(ev)) => {
            ctx.trace
                .sync_event("event-set", desc, ctx.args.caller_pc, ctx.thread_id);
            ev.set();
            ctx.mem.write_u32(desc + 0x04, 1).map_err(HandlerFault)?;
            Ok(1)
        }
        _ => Ok(0),
    }
}

pub fn os_reset_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    match obj.as_ref().map(|entry| &entry.object) {
        Some(super::sync::SyncObj::Event(ev)) => {
            ctx.trace
                .sync_event("event-reset", desc, ctx.args.caller_pc, ctx.thread_id);
            ev.reset();
            ctx.mem.write_u32(desc + 0x04, 0).map_err(HandlerFault)?;
            Ok(1)
        }
        _ => Ok(0),
    }
}

pub fn os_close_object(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let entry = ctx.shared.sync_objects.lock().unwrap().remove(&ctx.args.r0);
    if let Some(entry) = entry {
        entry.close(); // reclamation waits for all active operation leases
        Ok(1)
    } else {
        Ok(0)
    }
}

pub fn os_create_semaphore(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let init = ctx.args.r0 as i32;
    let Some(desc) = alloc_desc(ctx, super::sync::SEMAPHORE_MAGIC)? else {
        return Ok(0);
    };
    ctx.mem
        .write_u16(desc + 0x08, init as u16)
        .map_err(HandlerFault)?;
    ctx.shared.sync_objects.lock().unwrap().insert(
        desc,
        std::sync::Arc::new(super::sync::SyncEntry::new(
            super::sync::SyncObj::Semaphore(super::sync::HostSem::new(init)),
            Some((ctx.mem.clone(), desc)),
        )),
    );
    Ok(desc)
}

pub fn os_wait_for_semaphore(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let timeout = ctx.args.r1;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    let Some(obj) = obj else {
        return Ok(super::sync::WAIT_RESULT_ERROR);
    };
    let sem = match &obj.object {
        super::sync::SyncObj::Semaphore(sem) => sem,
        _ => return Ok(super::sync::WAIT_RESULT_ERROR),
    };
    if sem.try_wait() {
        return Ok(super::sync::WAIT_RESULT_RESOLVED);
    }
    ctx.deferred = Some(super::pending::PendingService::resource(
        ctx.args,
        super::pending::Wait::Semaphore {
            descriptor: desc,
            timeout_ms: timeout,
        },
        obj,
        ctx.thread_id,
    ));
    Ok(0)
}

pub fn os_release_semaphore(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    if let Some(super::sync::SyncObj::Semaphore(sem)) = obj.as_ref().map(|entry| &entry.object) {
        sem.release();
        let c = ctx.mem.read_u16(desc + 0x08).unwrap_or(0);
        let _ = ctx
            .mem
            .write_u16(desc + 0x08, c.saturating_add(1).min(0x7FFF));
        Ok(1)
    } else {
        Ok(0)
    }
}

pub fn os_init_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    if ptr == 0 {
        return Ok(0);
    }
    ctx.mem
        .write_u32(ptr, super::sync::CRITSEC_MAGIC)
        .map_err(HandlerFault)?;
    ctx.mem.write_u32(ptr + 0x04, 0).map_err(HandlerFault)?;
    ctx.mem.write_u16(ptr + 0x08, 0).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(ptr + 0x0A, &[0u8; 9])
        .map_err(HandlerFault)?;
    ctx.shared.sync_objects.lock().unwrap().insert(
        ptr,
        std::sync::Arc::new(super::sync::SyncEntry::new(
            super::sync::SyncObj::CritSec(super::sync::HostCs::new()),
            None,
        )),
    );
    Ok(0)
}

pub fn os_enter_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&ptr).cloned();
    let Some(obj) = obj else {
        return Ok(0);
    };
    if let super::sync::SyncObj::CritSec(cs) = &obj.object {
        if cs.try_enter_guest(ctx.thread_id) {
            let rc = ctx.mem.read_u16(ptr + 0x08).map_err(HandlerFault)?;
            ctx.mem
                .write_u16(ptr + 0x08, rc.saturating_add(1))
                .map_err(HandlerFault)?;
        } else {
            ctx.deferred = Some(super::pending::PendingService::resource(
                ctx.args,
                super::pending::Wait::CriticalSection { descriptor: ptr },
                obj,
                ctx.thread_id,
            ));
        }
    }
    Ok(0)
}

pub fn os_leave_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&ptr).cloned();
    if let Some(super::sync::SyncObj::CritSec(cs)) = obj.as_ref().map(|entry| &entry.object) {
        if cs.leave_guest(ctx.thread_id) {
            let rc = ctx.mem.read_u16(ptr + 0x08).map_err(HandlerFault)?;
            if rc > 0 {
                ctx.mem
                    .write_u16(ptr + 0x08, rc - 1)
                    .map_err(HandlerFault)?;
            }
        }
    }
    Ok(0)
}

pub fn os_delete_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    let entry = ctx.shared.sync_objects.lock().unwrap().remove(&ptr);
    if let Some(entry) = entry {
        entry.close();
    }
    if ptr != 0 {
        ctx.mem
            .write_bytes(ptr, &[0u8; 0x14])
            .map_err(HandlerFault)?;
    }
    Ok(0)
}

/// Allocate a zeroed 0x14-byte guest descriptor with the given magic.
fn alloc_desc(ctx: &SvcCtx, magic: u32) -> Result<Option<u32>, HandlerFault> {
    let Some(va) = ctx.mem.try_heap_alloc(0x14) else {
        return Ok(None);
    };
    ctx.mem
        .write_bytes(va, &[0u8; 0x14])
        .map_err(HandlerFault)?;
    ctx.mem.write_u32(va, magic).map_err(HandlerFault)?;
    Ok(Some(va))
}
