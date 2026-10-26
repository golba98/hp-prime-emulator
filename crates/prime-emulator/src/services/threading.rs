//! Thread services. `OSCreateThread` creates a real guest thread: a new
//! native thread with its own Unicorn engine over the shared memory blocks
//! (the reference emulator's proven model). The engine construction happens
//! inside the spawned thread (Unicorn handles are not `Send`).

use super::{HandlerFault, SvcCtx};

/// ref: thread descriptor is 0x54 bytes; magic 0x100 at +0, tid at +0x14,
/// thread_func at +0x18, exit_code at +0xC, priority slot at +0x22.
pub const THREAD_DESC_SIZE: usize = 0x54;
pub const THREAD_MAGIC: u32 = 0x100;

/// ref: `OSCreateThread(func = r0, user_data = r1, stack_size = r2,
/// defer_start = r3)` returns a guest thread descriptor pointer.
pub fn os_create_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let func = ctx.args.r0;
    let user_data = ctx.args.r1;
    let stack_size = if ctx.args.r2 == 0 {
        0x2000
    } else {
        ctx.args.r2 as usize
    };
    let defer_start = ctx.args.r3;

    let desc = ctx.mem.heap_alloc(THREAD_DESC_SIZE).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(desc, &[0u8; THREAD_DESC_SIZE])
        .map_err(HandlerFault)?;
    ctx.mem
        .write_u32(desc, THREAD_MAGIC)
        .map_err(HandlerFault)?;
    ctx.mem.write_u32(desc + 0x18, func).map_err(HandlerFault)?;

    if defer_start == 0 {
        let tid = ctx.shared.alloc_thread_id();
        ctx.mem.write_u32(desc + 0x14, tid).map_err(HandlerFault)?;
        ctx.shared.spawn_thread(tid, func, user_data, stack_size);
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
/// returns 0 (reference stub; thread keep running there).
pub fn os_terminate_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    if desc != 0 {
        ctx.mem
            .write_u32(desc + 0x0C, ctx.args.r1)
            .map_err(HandlerFault)?;
    }
    Ok(0)
}

/// Reference stub returns 0 and lets the thread continue; we mark the
/// calling host thread as exiting so its run loop stops cleanly after the
/// slice (documented divergence — see M2_SVC_IMPLEMENTATIONS.md).
pub fn os_exit_thread(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let code = ctx.args.r0;
    ctx.shared.mark_thread_exiting();
    ctx.trace
        .thread_event(format!("guest thread exit requested ({code})"));
    let _ = ctx.uc.emu_stop();
    Ok(0)
}

/// ref: OSSuspendThread / OSResumeThread / OSWakeUpThread are stubs that
/// return 1 (threads keep running in the reference too).
pub fn os_thread_stub_one(_ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    Ok(1)
}

/// ref: `OSSetThreadPriority(desc, slot)` stores the slot at +0x22.
pub fn os_set_thread_priority(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    if desc == 0 {
        return Ok(0);
    }
    ctx.mem
        .write_u16(desc + 0x22, ctx.args.r1 as u16)
        .map_err(HandlerFault)?;
    Ok(1)
}

/// ref: `OSGetThreadPriority(desc)` returns the stored slot; default normal.
pub fn os_get_thread_priority(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    if desc == 0 {
        return Ok(0);
    }
    Ok(ctx.mem.read_u16(desc + 0x22).map_err(HandlerFault)? as u32)
}

// Sync services over host primitives (conventions from the reference):

pub fn os_create_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let latch_on = ctx.args.r0 != 0;
    let flag = ctx.args.r1 != 0;
    let desc = alloc_desc(ctx, super::sync::EVENT_MAGIC)?;
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
        std::sync::Arc::new(super::sync::SyncObj::Event(super::sync::HostEvent::new(
            latch_on, flag,
        ))),
    );
    Ok(desc)
}

pub fn os_wait_for_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let timeout = ctx.args.r1;
    ctx.trace
        .sync_event("event-wait", desc, ctx.args.caller_pc, ctx.thread_id);
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    let Some(obj) = obj else {
        return Ok(super::sync::WAIT_RESULT_ERROR);
    };
    let ev = match obj.as_ref() {
        super::sync::SyncObj::Event(ev) => ev,
        _ => return Ok(super::sync::WAIT_RESULT_ERROR),
    };
    // The main thread blocked in a wait is the UI at rest (key scripts
    // only send the next input then).
    let main = ctx.thread_id == 0;
    if main {
        ctx.shared.set_main_waiting(true);
    }
    let signaled = {
        let _parked = ctx.mem.snapshot_gate.park();
        ev.wait(timeout, &|| ctx.shared.stopping())
    };
    if main {
        ctx.shared.set_main_waiting(false);
    }
    if signaled {
        ctx.trace.sync_event(
            "event-wait-resolved",
            desc,
            ctx.args.caller_pc,
            ctx.thread_id,
        );
        // Auto-reset: clear the guest flag unless latched (ref).
        let latch = ctx.mem.read_u16(desc + 0x08).unwrap_or(0);
        if latch == 0 {
            let _ = ctx.mem.write_u32(desc + 0x04, 0);
        }
        return Ok(super::sync::WAIT_RESULT_RESOLVED);
    }
    Ok(super::sync::WAIT_RESULT_TIMEOUT)
}

pub fn os_set_event(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    match obj.as_deref() {
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
    match obj.as_deref() {
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
    let desc = ctx.args.r0;
    if ctx
        .shared
        .sync_objects
        .lock()
        .unwrap()
        .remove(&desc)
        .is_some()
    {
        let _ = ctx.mem.heap_free(desc);
        Ok(1)
    } else {
        Ok(0)
    }
}

pub fn os_create_semaphore(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let init = ctx.args.r0 as i32;
    let desc = alloc_desc(ctx, super::sync::SEMAPHORE_MAGIC)?;
    ctx.mem
        .write_u16(desc + 0x08, init as u16)
        .map_err(HandlerFault)?;
    ctx.shared.sync_objects.lock().unwrap().insert(
        desc,
        std::sync::Arc::new(super::sync::SyncObj::Semaphore(super::sync::HostSem::new(
            init,
        ))),
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
    let sem = match obj.as_ref() {
        super::sync::SyncObj::Semaphore(sem) => sem,
        _ => return Ok(super::sync::WAIT_RESULT_ERROR),
    };
    let ok = sem.wait(timeout, &|| ctx.shared.stopping());
    Ok(if ok {
        super::sync::WAIT_RESULT_RESOLVED
    } else {
        super::sync::WAIT_RESULT_TIMEOUT
    })
}

pub fn os_release_semaphore(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let desc = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&desc).cloned();
    if let Some(super::sync::SyncObj::Semaphore(sem)) = obj.as_deref() {
        sem.release();
        let c = ctx.mem.read_u16(desc + 0x08).unwrap_or(0);
        let _ = ctx.mem.write_u16(desc + 0x08, (c + 1).min(0x7FFF));
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
        std::sync::Arc::new(super::sync::SyncObj::CritSec(super::sync::HostCs::new())),
    );
    Ok(0)
}

pub fn os_enter_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&ptr).cloned();
    if let Some(super::sync::SyncObj::CritSec(cs)) = obj.as_deref() {
        if !cs.enter_until(&|| ctx.shared.stopping()) {
            return Ok(0);
        }
        let rc = ctx.mem.read_u16(ptr + 0x08).unwrap_or(0);
        let _ = ctx.mem.write_u16(ptr + 0x08, rc + 1);
    }
    Ok(0)
}

pub fn os_leave_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    let obj = ctx.shared.sync_objects.lock().unwrap().get(&ptr).cloned();
    if let Some(super::sync::SyncObj::CritSec(cs)) = obj.as_deref() {
        let rc = ctx.mem.read_u16(ptr + 0x08).unwrap_or(0);
        if rc > 0 {
            let _ = ctx.mem.write_u16(ptr + 0x08, rc - 1);
        }
        cs.leave();
    }
    Ok(0)
}

pub fn os_delete_critical_section(ctx: &mut SvcCtx) -> Result<u32, HandlerFault> {
    let ptr = ctx.args.r0;
    ctx.shared.sync_objects.lock().unwrap().remove(&ptr);
    if ptr != 0 {
        ctx.mem
            .write_bytes(ptr, &[0u8; 0x14])
            .map_err(HandlerFault)?;
    }
    Ok(0)
}

/// Allocate a zeroed 0x14-byte guest descriptor with the given magic.
fn alloc_desc(ctx: &SvcCtx, magic: u32) -> Result<u32, HandlerFault> {
    let va = ctx.mem.heap_alloc(0x14).map_err(HandlerFault)?;
    ctx.mem
        .write_bytes(va, &[0u8; 0x14])
        .map_err(HandlerFault)?;
    ctx.mem.write_u32(va, magic).map_err(HandlerFault)?;
    Ok(va)
}
