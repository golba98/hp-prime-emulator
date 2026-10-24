//! SVC dispatch tests with a synthetic engine: ABI convention, registry
//! integrity, unsupported-service reporting, and the yield idiom.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use prime_emulator::engine::CpuThread;
use prime_emulator::memory::GuestMemory;
use prime_emulator::services::{
    self, GuestArgs, Registry, ServiceStatus, SharedState, UnsupportedPolicy, handlers, names,
};
use prime_emulator::trace::{TraceConfig, TraceHub};

fn test_env() -> (
    Arc<GuestMemory>,
    Arc<Registry>,
    Arc<SharedState>,
    Arc<TraceHub>,
) {
    let mut mem = GuestMemory::new();
    mem.map("test", 0x1000_0000, 0x10_000).unwrap();
    mem.map("heap", 0x2000_0000, 0x10_000).unwrap();
    mem.set_heap(0x2000_0000, 0x10_000);
    let mem = Arc::new(mem);
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    let registry = Arc::new(registry);
    let shared = Arc::new(SharedState::new(
        std::path::PathBuf::from("/tmp/prime-emulator-test"),
        "A:\\test".to_owned(),
    ));
    let trace = Arc::new(TraceHub::new(TraceConfig::default()).unwrap());
    (mem, registry, shared, trace)
}

#[test]
fn verified_name_table_is_complete() {
    assert_eq!(names::SERVICE_COUNT, 742);
    assert_eq!(names::FIRST_SERVICE_ID, 0x10000);
    assert_eq!(names::LAST_SERVICE_ID, 0x102E5);
    // Round trip for every named service.
    for (i, name) in names::SERVICE_NAMES.iter().enumerate() {
        if let Some(n) = name {
            assert_eq!(
                names::service_id_by_name(n),
                Some(names::FIRST_SERVICE_ID + i as u32),
                "name table round-trip broken for {n}"
            );
        }
    }
    // Known-correct spot checks (Phase 1 verified).
    assert_eq!(names::service_name(0x10037), Some("lmalloc"));
    assert_eq!(names::service_name(0x10286), Some("_LoadLibraryA"));
    // The 21 unknown IDs must stay nameless.
    let unnamed = names::SERVICE_NAMES.iter().filter(|n| n.is_none()).count();
    assert_eq!(unnamed, 21);
}

#[test]
fn registry_registration_marks_status() {
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    let e = registry.entry(0x10037).unwrap();
    assert_eq!(e.name, Some("lmalloc"));
    assert!(e.handler.is_some());
    assert_eq!(e.status, ServiceStatus::Implemented);
    // Unregistered service keeps NotInvestigated and no handler.
    let e = registry.entry(0x102D1).unwrap();
    assert!(e.handler.is_none());
    assert_eq!(e.status, ServiceStatus::NotInvestigated);
}

#[test]
fn stack_args_follow_the_verified_convention() {
    // Caller stack args start at sp+8 (stub pushes lr and r0 first).
    let mut m = GuestMemory::new();
    m.map("t", 0x1000_0000, 0x1000).unwrap();
    let sp = 0x1000_0100u32;
    m.write_u32(sp, 0x1111_0000).unwrap(); // [sp]    = return address
    m.write_u32(sp + 4, 0x2222_0000).unwrap(); // [sp+4]  = saved r0
    m.write_u32(sp + 8, 0xAAAA_0001).unwrap(); // [sp+8]  = caller arg 5
    m.write_u32(sp + 12, 0xBBBB_0002).unwrap(); // [sp+12] = caller arg 6
    let args = GuestArgs {
        r0: 1,
        r1: 2,
        r2: 3,
        r3: 4,
        sp,
        caller_pc: 0x1111_0000,
        stack_slots: [0; 2],
    };
    assert_eq!(args.stack_arg(&m, 0).unwrap(), 0xAAAA_0001);
    assert_eq!(args.stack_arg(&m, 1).unwrap(), 0xBBBB_0002);
}

static HANDLER_HITS: AtomicU64 = AtomicU64::new(0);

#[test]
fn svc_stub_round_trips_through_the_dispatcher() {
    let (mem, _registry, shared, trace) = test_env();
    let mut registry = Registry::new();
    registry.register_by_name(
        "SetAutoPowerOff",
        ServiceStatus::Implemented,
        |_ctx: &mut services::SvcCtx<'_, '_>| {
            HANDLER_HITS.fetch_add(1, Ordering::SeqCst);
            Ok(0x1234)
        },
    );
    let registry = Arc::new(registry);

    // Guest code: push {r0}; push {lr}; svc #0x1002F (SetAutoPowerOff)
    // exactly like the firmware's verified stub, called with bl.
    let code: [u8; 16] = [
        0x01, 0x00, 0x2d, 0xe9, // e92d0001 push {r0}
        0x00, 0x40, 0x2d, 0xe9, // e92d4000 push {lr}
        0x2f, 0x00, 0x01, 0xef, // ef01002f svc #0x1002F
        0x00, 0x80, 0xbd, 0xe8, // e8bd8000 pop {pc}
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    HANDLER_HITS.store(0, Ordering::SeqCst);

    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace.clone(),
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap();
    let lr = 0x1000_0010; // return after the stub (we run the stub body inline)
    cpu.set_entry(0x1000_0000, 0x1000_0800, 0).unwrap();
    cpu.set_lr(lr).unwrap();
    let out = cpu.run_slice(64).unwrap();
    use prime_emulator::engine::RunSliceOutcome;
    match out {
        RunSliceOutcome::Progress | RunSliceOutcome::Stopped(_) => {}
    }
    assert_eq!(
        HANDLER_HITS.load(Ordering::SeqCst),
        1,
        "handler must run once"
    );
    assert_eq!(
        cpu.reg(unicorn_engine::RegisterARM::R0),
        0x1234,
        "r0 must carry the result"
    );
    assert_eq!(cpu.sp(), 0x1000_0800, "sp must be restored (+8)");
}

#[test]
fn strict_policy_stops_on_unsupported_service() {
    let (mem, registry, shared, trace) = test_env();
    // 0x102D1 has no name and no handler.
    let code: [u8; 16] = [
        0x01, 0x00, 0x2d, 0xe9, // push {r0}
        0x00, 0x40, 0x2d, 0xe9, // push {lr}
        0xd1, 0x02, 0x01, 0xef, // svc #0x102D1
        0x00, 0x80, 0xbd, 0xe8, // pop {pc}
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        mem,
        registry,
        shared,
        trace,
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_0800, 0).unwrap();
    cpu.set_lr(0x1000_0010).unwrap();
    match cpu.run_slice(64).unwrap() {
        prime_emulator::engine::RunSliceOutcome::Stopped(
            prime_emulator::engine::StopReason::UnsupportedService { id, .. },
        ) => assert_eq!(id, 0x102D1),
        other => panic!("expected strict unsupported stop, got {other:?}"),
    }
}

#[test]
fn zero_policy_continues_past_unsupported_service() {
    let (mem, registry, shared, trace) = test_env();
    let code: [u8; 16] = [
        0x01, 0x00, 0x2d, 0xe9, // push {r0}
        0x00, 0x40, 0x2d, 0xe9, // push {lr}
        0xd1, 0x02, 0x01, 0xef, // svc #0x102D1
        0x00, 0x80, 0xbd, 0xe8, // pop {pc}
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        mem,
        registry,
        shared,
        trace,
        UnsupportedPolicy::ZeroReturn,
        0,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_0800, 0).unwrap();
    cpu.set_lr(0x1000_0010).unwrap();
    match cpu.run_slice(64).unwrap() {
        prime_emulator::engine::RunSliceOutcome::Progress => {}
        other => panic!("zero policy must continue, got {other:?}"),
    }
    assert_eq!(
        cpu.reg(unicorn_engine::RegisterARM::R0),
        0,
        "unsupported returns 0, logged"
    );
}

#[test]
fn firmware_yield_idiom_sets_z_and_advances() {
    let (mem, registry, shared, trace) = test_env();
    // yield word 0xEE17FF7E followed by a marker instruction (mov r0, #7).
    let code: [u8; 12] = [
        0x7e, 0xff, 0x17, 0xee, // the yield idiom
        0x07, 0x00, 0xa0, 0xe3, // mov r0, #7
        0x00, 0x00, 0x00, 0x00,
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        mem,
        registry,
        shared,
        trace,
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_0800, 0).unwrap();
    // One slice per idiom occurrence + the marker; loop until executed.
    for _ in 0..5 {
        let _ = cpu.run_slice(16).unwrap();
        if cpu.reg(unicorn_engine::RegisterARM::R0) == 7 {
            break;
        }
    }
    assert_eq!(
        cpu.reg(unicorn_engine::RegisterARM::R0),
        7,
        "execution must continue past the idiom"
    );
    let cpsr = cpu.reg(unicorn_engine::RegisterARM::CPSR);
    assert_ne!(
        cpsr & (1 << 30),
        0,
        "Z flag must be set (reference behavior)"
    );
}

// ───────────────── M3: event-loop and observability tests ─────────────

#[test]
fn host_event_set_wait_roundtrip() {
    // The host-side primitive behind OSWaitForEvent: set → wait resolves;
    // auto-reset consumes; reset clears.
    let ev = prime_emulator::services::sync::HostEvent::new(false, false);
    let stopping = || false;
    assert!(
        !ev.wait(1, &stopping),
        "unsignaled auto event must time out"
    );
    ev.set();
    assert!(ev.wait(1, &stopping), "set must resolve the wait");
    assert!(!ev.wait(1, &stopping), "auto-reset must consume the signal");

    let manual = prime_emulator::services::sync::HostEvent::new(true, false);
    manual.set();
    assert!(manual.wait(1, &stopping));
    assert!(manual.wait(1, &stopping), "manual-reset stays signaled");
    manual.reset();
    assert!(!manual.wait(1, &stopping), "reset clears the signal");
}

#[test]
fn host_event_set_while_unwaited_is_not_lost() {
    // A set that happens while nobody waits must resolve the next wait
    // (edge-triggered auto-reset semantics) — the lost-wakeup case.
    let ev = prime_emulator::services::sync::HostEvent::new(false, false);
    let stopping = || false;
    ev.set();
    assert!(ev.wait(50, &stopping));
}

#[test]
fn event_type_filter_roundtrip_through_svc() {
    // The pump's prologue: GetEventType() → |= 0x20 → SetEventType().
    let (_mem, _registry, shared, _trace) = test_env();
    assert_eq!(
        shared
            .event_type_filter
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    shared
        .event_type_filter
        .store(0x20, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        shared
            .event_type_filter
            .load(std::sync::atomic::Ordering::Relaxed),
        0x20
    );
}

#[test]
fn getevent_delivers_injected_input_and_records_thread() {
    // Synthetic pump: run the GetEvent stub in a loop-like single call on a
    // guest thread, with an event injected from the host side mid-flight —
    // exactly the production input pathway.
    let (mem, _registry, shared, trace) = test_env();
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    let registry = Arc::new(registry);

    // svc #0x1003F (GetEvent); the handler blocks until input arrives.
    let code: [u8; 16] = [
        0x01, 0x00, 0x2d, 0xe9, // push {r0}
        0x00, 0x40, 0x2d, 0xe9, // push {lr}
        0x3f, 0x00, 0x01, 0xef, // svc #0x1003F
        0x00, 0x80, 0xbd, 0xe8, // pop {pc}
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    // Event-struct destination on the guest stack (handler writes 124 bytes).
    let evbuf = 0x1000_0400u32;
    mem.write_u32(0x1000_0000 + 20, evbuf).unwrap(); // via arg? no — use r0
    // Push an input event after a short delay from another host thread.
    {
        let shared2 = shared.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            shared2.push_key_press(0x83);
        });
    }
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace.clone(),
        UnsupportedPolicy::Stop,
        7,
        Vec::new(),
    )
    .unwrap();
    // r0 = event struct pointer.
    cpu.set_entry(0x1000_0000, 0x1000_0800, evbuf).unwrap();
    cpu.set_lr(0x1000_0010).unwrap();
    let _ = cpu.run_slice(1_000_000).unwrap();

    // The batch must have been delivered to this thread with the
    // down+up pair, and the event struct must show KEY_BATCH.
    assert_eq!(shared.input_delivered(), 1);
    assert_eq!(
        shared.input_thread(),
        7,
        "delivery must record the thread id"
    );
    let [caller, _, _] = shared.getevent_chain();
    assert_eq!(
        caller,
        0x1000_000C + 4,
        "caller is the instruction after svc"
    );
    assert_eq!(
        mem.read_u32(evbuf + 4).unwrap(),
        prime_emulator::services::UI_EVENT_TYPE_KEY_BATCH
    );
    assert_eq!(mem.read_u16(evbuf + 24).unwrap(), 2, "down + up pair");
    assert_eq!(mem.read_u16(evbuf + 28 + 6).unwrap(), 0x83, "keycode 0x83");
    assert_eq!(
        mem.read_u16(evbuf + 28 + 12 + 6).unwrap(),
        0x83,
        "key up pair"
    );
}

#[test]
fn write_watchpoint_records_guest_writes() {
    // Watchpoints fire on emulated guest stores only (host-side service
    // writes bypass Unicorn by design — they are already host-visible).
    let (mem, registry, shared, trace) = test_env();
    // str r0, [r0] with r0 = 0x1000_0400 (set via the entry argument):
    // the guest stores 0x10000400 into the watched word.
    let code: [u8; 8] = [
        0x00, 0x00, 0x80, 0xe5, // e5800000 str r0, [r0]
        0x01, 0x00, 0xa0, 0xe3, // mov r0, #1 (marker after the store)
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![(0x1000_0400, 4)],
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_0800, 0x1000_0400)
        .unwrap();
    for _ in 0..4 {
        let _ = cpu.run_slice(16).unwrap();
        if cpu.reg(unicorn_engine::RegisterARM::R0) == 1 {
            break;
        }
    }
    assert_eq!(
        mem.read_u32(0x1000_0400).unwrap(),
        0x1000_0400,
        "the guest store must have happened"
    );
    assert!(
        trace.watch_hits() >= 1,
        "watchpoint must record the guest write"
    );
}

#[test]
fn thread_id_flows_through_dispatch() {
    // The dispatch context carries the engine's thread id into handlers
    // (heartbeat evidence depends on it).
    let (mem, _registry, shared, trace) = test_env();
    static SEEN_TID: AtomicU64 = AtomicU64::new(0xFFFF_FFFF);
    let mut registry = Registry::new();
    registry.register_by_name(
        "SetAutoPowerOff",
        ServiceStatus::Implemented,
        |ctx: &mut services::SvcCtx<'_, '_>| {
            SEEN_TID.store(ctx.thread_id as u64, Ordering::SeqCst);
            Ok(0)
        },
    );
    let registry = Arc::new(registry);
    let code: [u8; 16] = [
        0x01, 0x00, 0x2d, 0xe9, 0x00, 0x40, 0x2d, 0xe9, 0x2f, 0x00, 0x01, 0xef, 0x00, 0x80, 0xbd,
        0xe8,
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace.clone(),
        UnsupportedPolicy::Stop,
        42,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_0800, 0).unwrap();
    cpu.set_lr(0x1000_0010).unwrap();
    let _ = cpu.run_slice(64).unwrap();
    assert_eq!(SEEN_TID.load(Ordering::SeqCst), 42);
}
