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
        trace.clone(),
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
    assert_eq!(trace.svc_totals(), vec![(0x102D1, 1)]);
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

    // svc #0x1003F (GetEvent); the owner polls until input arrives.
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while shared.input_delivered() == 0 {
        let _ = cpu.run_slice(64).unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "input continuation failed to complete"
        );
    }

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

#[test]
fn exhausted_malloc_returns_null_to_guest_without_stopping_emulation() {
    let (mem, registry, shared, trace) = test_env();
    let allocation = mem.heap_alloc(0x10_000).unwrap();
    mem.write_u32(allocation, 123).unwrap();
    let code = [
        0x01, 0x00, 0x2d, 0xe9, // push {r0}
        0x00, 0x40, 0x2d, 0xe9, // push {lr}
        0x37, 0x00, 0x01, 0xef, // lmalloc
        0xfe, 0xff, 0xff, 0xea, // loop after return
    ];
    mem.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared,
        trace,
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_0800, 16).unwrap();
    cpu.set_lr(0x1000_000c).unwrap();
    assert!(matches!(
        cpu.run_slice(32).unwrap(),
        prime_emulator::engine::RunSliceOutcome::Progress
    ));
    assert_eq!(cpu.reg(unicorn_engine::RegisterARM::R0), 0);
    assert_eq!(mem.read_u32(allocation).unwrap(), 123);
}

#[test]
fn handler_memory_fault_stops_with_actual_cpu_context_in_both_policies() {
    for policy in [UnsupportedPolicy::Stop, UnsupportedPolicy::ZeroReturn] {
        let (mem, _, shared, trace) = test_env();
        let mut registry = Registry::new();
        registry.register_by_name("SetAutoPowerOff", ServiceStatus::Implemented, |ctx| {
            ctx.guest(0x9000_1234)
        });
        let code: Vec<u8> = [0xe92d0001u32, 0xe92d4000, 0xef01002f, 0xeafffffe]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        mem.write_bytes(0x1000_0000, &code).unwrap();
        let mut cpu = CpuThread::new(
            mem,
            Arc::new(registry),
            shared,
            trace,
            policy,
            42,
            Vec::new(),
        )
        .unwrap();
        cpu.set_entry(0x1000_0000, 0x1000_0800, 123).unwrap();
        cpu.set_lr(0x1000_000c).unwrap();
        assert!(matches!(
            cpu.run_slice(32).unwrap(),
            prime_emulator::engine::RunSliceOutcome::Stopped(
                prime_emulator::engine::StopReason::Fault
            )
        ));
        let fault = cpu.fault_report().unwrap();
        assert_eq!(fault.thread_id, 42);
        assert_eq!(fault.addr, Some(0x9000_1234));
        assert_eq!(fault.regs[0], 123);
        assert_eq!(fault.pc, 0x1000_000c);
    }
}

#[test]
fn invalid_stack_arguments_fail_without_wrapping_or_panicking() {
    let (mem, _, _, _) = test_env();
    let mut args = GuestArgs {
        r0: 0,
        r1: 0,
        r2: 0,
        r3: 0,
        sp: u32::MAX - 4,
        caller_pc: 0,
        stack_slots: [0; 2],
    };
    assert!(args.stack_arg(&mem, 0).is_err());
    args.sp = 0x1000_0000;
    assert!(args.stack_arg(&mem, usize::MAX).is_err());
}

#[test]
fn real_sleep_service_returns_from_unicorn_and_keeps_its_remaining_timer_as_data() {
    use prime_emulator::services::pending::{PendingService, ServiceImage, Wait};
    use std::time::{Duration, Instant};
    let (mem, registry, shared, trace) = test_env();
    let id = names::service_id_by_name("OSSleep").unwrap();
    let code = [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe];
    for (i, word) in code.iter().enumerate() {
        mem.write_u32(0x10000000 + 4 * i as u32, *word).unwrap();
    }
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace,
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000000, 0x10000800, 60_000).unwrap();
    cpu.set_lr(0x1000000c).unwrap();
    let start = Instant::now();
    cpu.run_slice(64).unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "a minute sleep must not retain a native callback"
    );
    assert_eq!(mem.snapshot_gate.counts(), (0, 0));
    let image = cpu.pending_service_image().unwrap();
    assert!(matches!(
        image.wait,
        Wait::Sleep {
            milliseconds: 60_000
        }
    ));
    assert!(image.remaining_timeout_ns.unwrap() > 50_000_000_000);
    let encoded = serde_json::to_vec(&image).unwrap();
    let mut decoded: ServiceImage = serde_json::from_slice(&encoded).unwrap();
    // Completing a reconstructed elapsed timer performs real service polling;
    // this is service-state coverage, not a whole-calculator restore claim.
    decoded.remaining_timeout_ns = Some(0);
    let elapsed = PendingService::from_image(decoded.clone()).unwrap();
    assert_eq!(elapsed.poll(&mem, &shared, 0).unwrap(), Some(0));
    decoded.remaining_timeout_ns = Some(60_000_000_001);
    assert!(PendingService::from_image(decoded).is_err());
    let before = cpu.pc();
    cpu.run_slice(64).unwrap();
    assert_eq!(
        cpu.pc(),
        before,
        "waiting must not execute the instruction after the SVC"
    );
    assert_eq!(mem.snapshot_gate.counts(), (0, 0));
}

#[test]
fn real_input_continuation_can_pause_then_deliver_exact_touch_and_return_once() {
    use prime_emulator::services::TouchSample;
    use std::time::{Duration, Instant};
    let (mem, registry, shared, trace) = test_env();
    let id = names::service_id_by_name("GetEvent").unwrap();
    for (i, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        mem.write_u32(0x10000000 + 4 * i as u32, word).unwrap();
    }
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace,
        UnsupportedPolicy::Stop,
        7,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000000, 0x10000800, 0x10000400).unwrap();
    cpu.set_lr(0x1000000c).unwrap();
    cpu.run_slice(64).unwrap();
    assert!(cpu.pending_service_image().is_some());
    assert!(shared.getevent_parked());
    assert_eq!(mem.snapshot_gate.counts(), (0, 0));
    let points = shared.safe_points.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let observer = std::thread::spawn(move || {
        let lease = points
            .pause(Duration::from_millis(500))
            .expect("production input wait must acknowledge a safe boundary");
        assert!(lease.is_current());
        tx.send(()).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while rx.try_recv().is_err() {
        cpu.run_slice(1).unwrap();
        assert!(Instant::now() < deadline);
    }
    observer.join().unwrap();
    shared.push_touch(TouchSample {
        status: 2,
        x: 123,
        y: 234,
    });
    cpu.run_slice(1).unwrap();
    assert!(cpu.pending_service_image().is_none());
    assert!(!shared.getevent_parked());
    assert_eq!(cpu.pc(), 0x1000000c);
    assert_eq!(cpu.reg(unicorn_engine::RegisterARM::SP), 0x10000800);
    assert_eq!(
        mem.read_u32(0x10000404).unwrap(),
        services::UI_EVENT_TYPE_TOUCH
    );
    assert_eq!(mem.read_u32(0x10000400 + 28).unwrap(), 2);
    assert_eq!(mem.read_u16(0x10000400 + 34).unwrap(), 123);
    assert_eq!(mem.read_u16(0x10000400 + 36).unwrap(), 234);
    cpu.run_slice(1).unwrap();
    assert_eq!(
        cpu.reg(unicorn_engine::RegisterARM::SP),
        0x10000800,
        "the service return frame is consumed once"
    );
}

#[test]
fn real_event_wait_keeps_a_lease_and_does_not_invent_ui_round_trips() {
    use services::sync::{HostEvent, SyncEntry, SyncObj, WAIT_RESULT_RESOLVED};
    use std::time::Duration;
    let (mem, registry, shared, trace) = test_env();
    let desc = 0x20000100;
    mem.write_u16(desc + 8, 0).unwrap();
    let resource = Arc::new(SyncEntry::new(
        SyncObj::Event(HostEvent::new(false, false)),
        None,
    ));
    shared
        .sync_objects
        .lock()
        .unwrap()
        .insert(desc, resource.clone());
    let id = names::service_id_by_name("OSWaitForEvent").unwrap();
    for (i, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        mem.write_u32(0x10000000 + 4 * i as u32, word).unwrap();
    }
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace,
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000000, 0x10000800, desc).unwrap();
    cpu.set_reg(unicorn_engine::RegisterARM::R1, u32::MAX)
        .unwrap();
    cpu.set_lr(0x1000000c).unwrap();
    cpu.run_slice(64).unwrap();
    assert!(cpu.pending_service_image().is_some());
    let generation = shared.main_wait_entries();
    cpu.run_slice(1).unwrap();
    assert!(shared.main_ui_waiting());
    for _ in 0..3 {
        cpu.run_slice(1).unwrap();
    }
    assert_eq!(
        shared.main_wait_entries(),
        generation,
        "polling is not a new firmware event-loop round trip"
    );
    assert_eq!(mem.snapshot_gate.counts(), (0, 0));
    let points = shared.safe_points.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let observer = std::thread::spawn(move || {
        let pause = points.pause(Duration::from_millis(500)).unwrap();
        assert!(pause.is_current());
        tx.send(()).unwrap();
    });
    while rx.try_recv().is_err() {
        cpu.run_slice(1).unwrap();
    }
    observer.join().unwrap();
    if let SyncObj::Event(event) = &resource.object {
        event.set();
    }
    cpu.run_slice(1).unwrap();
    assert!(cpu.pending_service_image().is_none());
    assert!(!shared.main_ui_waiting());
    assert_eq!(
        cpu.reg(unicorn_engine::RegisterARM::R0),
        WAIT_RESULT_RESOLVED
    );
    assert_eq!(cpu.reg(unicorn_engine::RegisterARM::SP), 0x10000800);
    assert_eq!(cpu.pc(), 0x1000000c);
}

#[test]
fn synchronization_continuations_use_guest_ownership_and_resolve_once() {
    use services::sync::{HostCs, HostSem, SyncEntry, SyncObj, WAIT_RESULT_RESOLVED};
    let (mem, registry, shared, trace) = test_env();
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared.clone(),
        trace,
        UnsupportedPolicy::Stop,
        2,
        vec![],
    )
    .unwrap();
    for (service, desc, object) in [
        (
            "OSWaitForSemaphore",
            0x20000100,
            SyncObj::Semaphore(HostSem::new(0)),
        ),
        (
            "OSEnterCriticalSection",
            0x20000200,
            SyncObj::CritSec(HostCs::new()),
        ),
    ] {
        let id = names::service_id_by_name(service).unwrap();
        for (i, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
            .into_iter()
            .enumerate()
        {
            mem.write_u32(0x10000000 + 4 * i as u32, word).unwrap();
        }
        // Each case uses a new code page address so Unicorn never executes a
        // translated block left over from the other service.
        let entry = if service == "OSWaitForSemaphore" {
            0x10001000
        } else {
            0x10002000
        };
        let code = mem.read_bytes_alloc(0x10000000, 16).unwrap();
        mem.write_bytes(entry, &code).unwrap();
        let resource = Arc::new(SyncEntry::new(object, None));
        if let SyncObj::CritSec(cs) = &resource.object {
            assert!(cs.try_enter_guest(1));
            assert!(
                !cs.leave_guest(2),
                "a different guest on the same host thread cannot release this lock"
            );
            mem.write_u16(desc + 8, 1).unwrap();
        }
        shared
            .sync_objects
            .lock()
            .unwrap()
            .insert(desc, resource.clone());
        cpu.set_entry(entry, 0x10000800, desc).unwrap();
        cpu.set_reg(unicorn_engine::RegisterARM::R1, u32::MAX)
            .unwrap();
        cpu.set_lr(entry + 12).unwrap();
        cpu.run_slice(64).unwrap();
        assert!(cpu.pending_service_image().is_some());
        cpu.run_slice(1).unwrap();
        assert_eq!(mem.snapshot_gate.counts(), (0, 0));
        match &resource.object {
            SyncObj::Semaphore(sem) => sem.release(),
            SyncObj::CritSec(cs) => {
                assert!(cs.leave_guest(1));
            }
            _ => unreachable!(),
        }
        cpu.run_slice(1).unwrap();
        assert!(cpu.pending_service_image().is_none());
        assert_eq!(cpu.reg(unicorn_engine::RegisterARM::SP), 0x10000800);
        assert_eq!(
            cpu.reg(unicorn_engine::RegisterARM::R0),
            if service == "OSWaitForSemaphore" {
                WAIT_RESULT_RESOLVED
            } else {
                0
            }
        );
        if let SyncObj::CritSec(cs) = &resource.object {
            assert!(cs.leave_guest(2));
        }
    }
}

#[test]
fn blocking_isr_cannot_overwrite_an_interrupted_service_continuation() {
    let (mem, registry, shared, trace) = test_env();
    let id = names::service_id_by_name("OSSleep").unwrap();
    for entry in [0x10000000, 0x10001000] {
        for (i, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
            .into_iter()
            .enumerate()
        {
            mem.write_u32(entry + 4 * i as u32, word).unwrap();
        }
    }
    let mut cpu = CpuThread::new(
        mem,
        registry,
        shared,
        trace,
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000000, 0x10000800, 60_000).unwrap();
    cpu.set_lr(0x1000000c).unwrap();
    cpu.run_slice(64).unwrap();
    let before = cpu.pc();
    assert!(cpu.run_isr(0x10001000, 7).is_err());
    let fault = cpu.fault_report().unwrap();
    assert_eq!(fault.kind, "blocking service in synchronous ISR");
    assert_eq!(cpu.pc(), before);
    assert_eq!(cpu.pending_service_image().unwrap().args.r0, 60_000);
}

#[test]
fn debug_printf_decodes_arm_variadic_alignment_and_writes_guest_count() {
    use unicorn_engine::RegisterARM;
    let (mem, registry, shared, trace) = test_env();
    let entry = 0x10000000;
    let id = names::service_id_by_name("WriteComDebugMsg").unwrap();
    for (index, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        mem.write_u32(entry + 4 * index as u32, word).unwrap();
    }
    let format = 0x10000400;
    mem.write_cstr(format, "value=%+.2f signed=%lld text=%.*s count=%hhn")
        .unwrap();
    let stack = 0x10000800;
    let integer = (-1_234_567_890_123i64) as u64;
    let words = [
        integer as u32,
        (integer >> 32) as u32,
        3,
        0x1000fffd,
        0x10000600,
    ];
    for (index, word) in words.into_iter().enumerate() {
        mem.write_u32(stack + 4 * index as u32, word).unwrap();
    }
    // Precision permits a non-NUL-terminated buffer ending at the mapping boundary.
    mem.write_bytes(0x1000fffd, b"end").unwrap();
    mem.write_u32(0x10000600, 0xaabbccdd).unwrap();
    let bits = (-2.5f64).to_bits();
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry,
        shared,
        trace.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.set_entry(entry, stack, format).unwrap();
    cpu.set_reg(RegisterARM::R1, 0xdeadbeef).unwrap(); // alignment padding, not an argument
    cpu.set_reg(RegisterARM::R2, bits as u32).unwrap();
    cpu.set_reg(RegisterARM::R3, (bits >> 32) as u32).unwrap();
    cpu.set_lr(entry + 12).unwrap();
    cpu.run_slice(64).unwrap();
    let expected = "value=-2.50 signed=-1234567890123 text=end count=";
    assert!(
        trace
            .ring()
            .iter()
            .any(|entry| entry.message == format!("guest: {expected}")),
        "{:?}",
        trace.ring()
    );
    assert_eq!(
        mem.read_u32(0x10000600).unwrap(),
        0xaabbcc00 | expected.len() as u32
    );
    assert_eq!(cpu.sp(), stack);
    assert_eq!(cpu.reg(RegisterARM::R0), 0);
    cpu.run_slice(1).unwrap();
    assert_eq!(trace.svc_totals(), [(id, 1)]);
}
