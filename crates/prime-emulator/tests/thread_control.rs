//! Real synthetic ARM service calls and execution-owner suspension controls.
//! These tests do not load firmware or access a physical device.
use prime_emulator::{
    engine::{CpuThread, RunSliceOutcome, StopReason},
    memory::GuestMemory,
    services::{Registry, SharedState, ThreadRecord, UnsupportedPolicy, handlers, names},
    trace::{TraceConfig, TraceHub},
};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use unicorn_engine::RegisterARM;

type Env = (
    Arc<GuestMemory>,
    Arc<Registry>,
    Arc<SharedState>,
    Arc<TraceHub>,
);
fn env() -> Env {
    let mut memory = GuestMemory::new();
    memory.map("code", 0x1000_0000, 0x10000).unwrap();
    memory.map("heap", 0x2000_0000, 0x10000).unwrap();
    memory.set_heap(0x2000_0000, 0x10000);
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    (
        Arc::new(memory),
        Arc::new(registry),
        Arc::new(SharedState::new(
            std::path::PathBuf::from("/tmp/unused-thread-control"),
            "A:\\test".into(),
        )),
        Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
    )
}
fn call(env: &Env, name: &str, args: [u32; 4]) -> u32 {
    let svc = 0xef00_0000 | names::service_id_by_name(name).unwrap();
    let mut code = Vec::new();
    for instruction in [0xe92d0001u32, 0xe92d4000, svc, 0xeafffffe] {
        code.extend(instruction.to_le_bytes());
    }
    env.0.write_bytes(0x1000_0000, &code).unwrap();
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0000, 0x1000_f000, args[0]).unwrap();
    cpu.set_lr(0x1000_000c).unwrap();
    for (reg, value) in [
        (RegisterARM::R1, args[1]),
        (RegisterARM::R2, args[2]),
        (RegisterARM::R3, args[3]),
    ] {
        cpu.set_reg(reg, value).unwrap();
    }
    cpu.run_slice(16).unwrap();
    cpu.reg(RegisterARM::R0)
}
#[test]
fn deferred_creation_preserves_launch_arguments_and_resumes_once() {
    let env = env();
    let launches = Arc::new(Mutex::new(Vec::new()));
    let observed = launches.clone();
    env.2
        .set_thread_spawner(Arc::new(move |tid, entry, arg, stack| {
            observed.lock().unwrap().push((tid, entry, arg, stack))
        }));
    let desc = call(&env, "OSCreateThread", [0x1000_0200, 0x1234, 0x1000, 1]);
    let tid = env.0.read_u32(desc + 0x14).unwrap();
    assert_ne!(tid, 0);
    assert!(launches.lock().unwrap().is_empty());
    assert_eq!(call(&env, "OSResumeThread", [desc, 0, 0, 0]), 1);
    assert_eq!(
        *launches.lock().unwrap(),
        vec![(tid, 0x1000_0200, 0x1234, 0x1000)]
    );
    assert_eq!(call(&env, "OSResumeThread", [desc, 0, 0, 0]), 0);
    assert_eq!(launches.lock().unwrap().len(), 1);
}
#[test]
fn suspend_resume_and_termination_are_not_unconditional_success_stubs() {
    let env = env();
    let desc = call(&env, "OSCreateThread", [0x1000_0200, 0, 0x1000, 0]);
    let tid = env.0.read_u32(desc + 0x14).unwrap();
    assert_eq!(call(&env, "OSSuspendThread", [desc, 0, 0, 0]), 1);
    let state = env.2.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || tx.send(state.guest_thread_checkpoint(tid)).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(40)).is_err());
    assert_eq!(call(&env, "OSWakeUpThread", [desc, 0, 0, 0]), 0);
    assert!(rx.recv_timeout(Duration::from_millis(40)).is_err());
    // Suspension is a flag: repeated Suspend requires only one Resume.
    assert_eq!(call(&env, "OSSuspendThread", [desc, 0, 0, 0]), 1);
    assert_eq!(call(&env, "OSResumeThread", [desc, 0, 0, 0]), 1);
    assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap());
    worker.join().unwrap();
    call(&env, "OSTerminateThread", [desc, 99, 0, 0]);
    assert_eq!(env.0.read_u32(desc + 0x0c).unwrap(), 99);
    assert!(env.2.guest_thread_terminated(tid));
    assert!(!env.2.guest_thread_checkpoint(tid));
    assert_eq!(call(&env, "OSResumeThread", [desc, 0, 0, 0]), 0);
    assert_eq!(call(&env, "OSSuspendThread", [0, 0, 0, 0]), 0);
}
#[test]
fn guest_cpu_continues_after_resume_and_stops_after_termination() {
    let env = env();
    env.0
        .write_bytes(
            0x1000_0200,
            &[0x01, 0x00, 0x80, 0xe2, 0xfd, 0xff, 0xff, 0xea],
        )
        .unwrap(); // add r0,#1; b start
    env.2.create_guest_thread(
        7,
        ThreadRecord {
            entry: 0x1000_0200,
            arg: 0,
            stack_size: 0x1000,
        },
        false,
    );
    assert!(env.2.suspend_guest_thread(7));
    let (tx, rx) = mpsc::channel();
    let memory = env.0.clone();
    let registry = env.1.clone();
    let state = env.2.clone();
    let trace = env.3.clone();
    let worker = std::thread::spawn(move || {
        let mut cpu = CpuThread::new(
            memory,
            registry,
            state.clone(),
            trace,
            UnsupportedPolicy::Stop,
            7,
            Vec::new(),
        )
        .unwrap();
        cpu.set_entry(0x1000_0200, 0x1000_f000, 0).unwrap();
        assert!(matches!(
            cpu.run_slice(20).unwrap(),
            RunSliceOutcome::Progress
        ));
        tx.send(cpu.reg(RegisterARM::R0)).unwrap();
        while !state.guest_thread_terminated(7) {
            std::thread::yield_now();
        }
        assert!(matches!(
            cpu.run_slice(20).unwrap(),
            RunSliceOutcome::Stopped(StopReason::Requested)
        ));
        tx.send(cpu.reg(RegisterARM::R0)).unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_millis(40)).is_err());
    assert!(env.2.resume_guest_thread(7));
    let before = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(before, 10);
    assert!(env.2.terminate_guest_thread(7));
    assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), before);
    worker.join().unwrap();
}
#[test]
fn suspended_owner_can_be_terminated_or_shut_down_without_resume() {
    for terminate in [true, false] {
        let env = env();
        env.2.create_guest_thread(
            7,
            ThreadRecord {
                entry: 0,
                arg: 0,
                stack_size: 0x1000,
            },
            true,
        );
        let state = env.2.clone();
        let worker = std::thread::spawn(move || state.guest_thread_checkpoint(7));
        if terminate {
            env.2.terminate_guest_thread(7);
        } else {
            env.2.set_stopping();
        }
        assert!(!worker.join().unwrap());
    }
}

#[test]
fn interrupt_services_mask_and_restore_registered_handlers_in_deterministic_order() {
    let env = env();
    assert_eq!(
        call(&env, "InterruptInitialize", [17, 0, 0x1000_0200, 0]),
        1
    );
    assert_eq!(call(&env, "InterruptInitialize", [5, 0, 0x1000_0300, 0]), 1);
    assert_eq!(
        env.2.take_due_isrs(),
        vec![(5, 0x1000_0300), (17, 0x1000_0200)]
    );
    assert_eq!(call(&env, "InterruptDisable", [17, 0, 0, 0]), 1);
    assert_eq!(env.2.take_due_isrs(), vec![(5, 0x1000_0300)]);
    assert_eq!(call(&env, "InterruptEnable", [17, 0, 0, 0]), 1);
    assert_eq!(
        env.2.take_due_isrs(),
        vec![(5, 0x1000_0300), (17, 0x1000_0200)]
    );
    assert_eq!(call(&env, "InterruptDone", [17, 0, 0, 0]), 1);
    for name in ["InterruptEnable", "InterruptDisable", "InterruptDone"] {
        assert_eq!(call(&env, name, [99, 0, 0, 0]), 0);
    }
}

#[test]
fn actual_arm_isr_executes_and_restores_cpu_context_but_faults_are_not_success() {
    let env = env();
    let mut code = Vec::new();
    for instruction in [0xe59f1004u32, 0xe5810000, 0xe12fff1e, 0x10000300] {
        code.extend(instruction.to_le_bytes());
    }
    env.0.write_bytes(0x1000_0200, &code).unwrap();
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(0x1000_0400, 0x1000_f000, 123).unwrap();
    cpu.set_reg(RegisterARM::R1, 456).unwrap();
    cpu.run_isr(0x1000_0200, 17).unwrap();
    assert_eq!(env.0.read_u32(0x1000_0300).unwrap(), 17);
    assert_eq!(cpu.reg(RegisterARM::R0), 123);
    assert_eq!(cpu.reg(RegisterARM::R1), 456);
    assert_eq!(cpu.pc(), 0x1000_0400);
    assert!(cpu.run_isr(0x7000_0000, 17).is_err());
    assert_eq!(cpu.pc(), 0x1000_0400);
    assert_eq!(cpu.reg(RegisterARM::R0), 123);
}

#[test]
fn suspended_owner_acknowledges_snapshot_pause_without_being_resumed() {
    let env = env();
    env.2.create_guest_thread(
        7,
        ThreadRecord {
            entry: 0,
            arg: 0,
            stack_size: 0x1000,
        },
        true,
    );
    let shared = env.2.clone();
    let (started, ready) = mpsc::channel();
    let (completed, result) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _owner = shared.safe_points.register(7);
        started.send(()).unwrap();
        completed.send(shared.guest_thread_checkpoint(7)).unwrap();
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    let pause = env.2.safe_points.pause(Duration::from_millis(500)).unwrap();
    assert!(pause.is_current());
    assert!(
        result.try_recv().is_err(),
        "a snapshot acknowledgement must not wake the suspended guest"
    );
    drop(pause);
    assert!(env.2.resume_guest_thread(7));
    assert!(result.recv_timeout(Duration::from_secs(1)).unwrap());
    worker.join().unwrap();
}

#[test]
fn wake_cancels_only_timed_sleep_and_keeps_suspend_and_saved_wake_state() {
    let env = env();
    let desc = call(&env, "OSCreateThread", [0x10000200, 0, 0x1000, 0]);
    let tid = env.0.read_u32(desc + 0x14).unwrap();
    let id = names::service_id_by_name("OSSleep").unwrap();
    for (index, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        env.0
            .write_u32(0x10000200 + index as u32 * 4, word)
            .unwrap();
    }
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        tid,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000200, 0x10008000, u16::MAX as u32)
        .unwrap();
    cpu.set_lr(0x1000020c).unwrap();
    cpu.run_slice(16).unwrap();
    let before = cpu.pending_service_image().unwrap();
    assert!(
        before.remaining_timeout_ns.is_none(),
        "65535 is indefinite, not a 65-second timer"
    );
    assert_eq!(call(&env, "OSSuspendThread", [desc, 0, 0, 0]), 1);
    assert_eq!(call(&env, "OSWakeUpThread", [desc, 0, 0, 0]), 1);
    assert_eq!(
        env.0.read_u16(desc + 0x20).unwrap() & 8,
        8,
        "Wake must keep the suspend flag"
    );
    assert_eq!(call(&env, "OSWakeUpThread", [desc, 0, 0, 0]), 0);
    assert_eq!(call(&env, "OSResumeThread", [desc, 0, 0, 0]), 1);
    // Serialization preserves a wake accepted after capture of the waiting CPU.
    let woken = cpu.pending_service_image().unwrap();
    assert!(woken.sleep_woken);
    let encoded = serde_json::to_vec(&woken).unwrap();
    let pending = prime_emulator::services::pending::PendingService::from_image(
        serde_json::from_slice(&encoded).unwrap(),
    )
    .unwrap();
    assert_eq!(pending.poll(&env.0, &env.2, tid).unwrap(), Some(0));
    let mut invalid = before;
    invalid.wait = prime_emulator::services::pending::Wait::PowerOff;
    invalid.sleep_woken = true;
    assert!(prime_emulator::services::pending::PendingService::from_image(invalid).is_err());
    cpu.run_slice(1).unwrap();
    assert!(cpu.pending_service_image().is_none());
    assert_eq!(cpu.pc(), 0x1000020c);
    assert_eq!(cpu.sp(), 0x10008000);
    assert_eq!(
        call(&env, "OSWakeUpThread", [desc, 0, 0, 0]),
        0,
        "completed sleep cannot retain a wake for the next one"
    );
    cpu.run_slice(1).unwrap();
    assert_eq!(cpu.sp(), 0x10008000, "return occurs once");
}

#[test]
fn automatic_power_request_pauses_real_arm_and_on_resumes_its_current_instruction() {
    let env = env();
    assert_eq!(call(&env, "SetAutoPowerOff", [2, 7, 0, 0]), 0);
    assert_eq!(call(&env, "SetAutoPowerOff", [0, 0, 0, 0]), 1);
    // The ROM system-variable callback controls the same mode consulted by APO.
    assert_eq!(call(&env, "SetSystemVariable", [3, 0, 0xda, 0]), 0);
    assert_eq!(call(&env, "SetSystemVariable", [3, 0xffff, 999, 0]), 0xda);
    assert_eq!(call(&env, "SetAutoPowerOff", [1, 0, 0, 0]), 0);
    assert_eq!(call(&env, "SetAutoPowerOff", [0, 0, 0, 0]), 0);
    assert_eq!(call(&env, "SetSystemVariable", [3, 0, 0, 0]), 0xda);
    env.0
        .write_bytes(
            0x10000200,
            &[0x01, 0x00, 0x80, 0xe2, 0xfd, 0xff, 0xff, 0xea],
        )
        .unwrap();
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        7,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000200, 0x10008000, 0).unwrap();
    cpu.run_slice(20).unwrap();
    assert_eq!(cpu.reg(RegisterARM::R0), 10);
    assert_eq!(call(&env, "SetAutoPowerOff", [3, 0, 0, 0]), 0);
    let pc = cpu.pc();
    cpu.run_slice(20).unwrap();
    assert!(env.2.sleeping());
    assert_eq!(cpu.reg(RegisterARM::R0), 10);
    assert_eq!(cpu.pc(), pc);
    env.2.push_key_press(0x31);
    cpu.run_slice(20).unwrap();
    assert_eq!(cpu.reg(RegisterARM::R0), 10);
    env.2.push_key_press(prime_emulator::services::ON_KEY_CODE);
    cpu.run_slice(20).unwrap();
    assert!(!env.2.sleeping());
    assert_eq!(cpu.reg(RegisterARM::R0), 20);
    assert_eq!(env.2.sleeps(), 1);
    assert_eq!(
        env.2.input_events_pending(),
        0,
        "wake key must not enter the program's key queue"
    );
}

#[test]
fn priority_services_change_the_execution_order_of_queued_arm_owners() {
    let env = env();
    let first = call(&env, "OSCreateThread", [0x10000200, 0, 0x1000, 0]);
    let second = call(&env, "OSCreateThread", [0x10000200, 0, 0x1000, 0]);
    let high = env.0.read_u32(first + 0x14).unwrap();
    let low = env.0.read_u32(second + 0x14).unwrap();
    assert_eq!(call(&env, "OSGetThreadPriority", [first, 0, 0, 0]), 17);
    assert_eq!(call(&env, "OSSetThreadPriority", [first, 1, 0, 0]), 1);
    assert_eq!(call(&env, "OSSetThreadPriority", [second, 32, 0, 0]), 1);
    assert_eq!(call(&env, "OSSetThreadPriority", [second, 1, 0, 0]), 0);
    assert_eq!(call(&env, "OSSetThreadPriority", [second, 64, 0, 0]), 0);
    assert_eq!(call(&env, "OSGetThreadPriority", [second, 0, 0, 0]), 32);
    assert_eq!(env.0.read_u16(second + 0x22).unwrap(), 32);
    env.0
        .write_bytes(
            0x10000200,
            &[0x01, 0x00, 0x80, 0xe2, 0xfd, 0xff, 0xff, 0xea],
        )
        .unwrap();
    let controller = env.2.scheduler.try_enter(0, true).unwrap();
    let spawn = |tid| {
        let memory = env.0.clone();
        let registry = env.1.clone();
        let shared = env.2.clone();
        let trace = env.3.clone();
        let (go, steps) = mpsc::channel();
        let (report, results) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut cpu = CpuThread::new(
                memory,
                registry,
                shared,
                trace,
                UnsupportedPolicy::Stop,
                tid,
                vec![],
            )
            .unwrap();
            cpu.set_entry(0x10000200, 0x10008000, 0).unwrap();
            while steps.recv().unwrap_or(false) {
                cpu.run_slice(20).unwrap();
                report.send(cpu.reg(RegisterARM::R0)).unwrap();
            }
        });
        (go, results, worker)
    };
    let (high_go, high_result, high_worker) = spawn(high);
    let (low_go, low_result, low_worker) = spawn(low);
    high_go.send(true).unwrap();
    low_go.send(true).unwrap();
    assert_eq!(high_result.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
    assert_eq!(low_result.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
    drop(controller);
    low_go.send(true).unwrap();
    assert_eq!(
        low_result.recv_timeout(Duration::from_secs(1)).unwrap(),
        0,
        "a queued higher-priority owner must run first"
    );
    high_go.send(true).unwrap();
    assert_eq!(
        high_result.recv_timeout(Duration::from_secs(1)).unwrap(),
        10
    );
    high_go.send(false).unwrap();
    high_worker.join().unwrap();
    low_go.send(true).unwrap();
    assert_eq!(low_result.recv_timeout(Duration::from_secs(1)).unwrap(), 10);
    low_go.send(false).unwrap();
    low_worker.join().unwrap();
}

#[test]
fn real_arm_sleep_deadline_stops_during_power_and_inspection_pauses() {
    let env = env();
    let id = names::service_id_by_name("OSSleep").unwrap();
    for (index, word) in [0xe92d0001u32, 0xe92d4000, 0xef000000 | id, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        env.0
            .write_u32(0x10000200 + index as u32 * 4, word)
            .unwrap();
    }
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        7,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10000200, 0x10008000, 50).unwrap();
    cpu.set_lr(0x1000020c).unwrap();
    let starting = env.2.clock.freeze(None);
    cpu.run_slice(16).unwrap();
    env.2.enter_sleep();
    drop(starting);
    let remaining = cpu.pending_service_image().unwrap().remaining_timeout_ns;
    std::thread::sleep(Duration::from_millis(80));
    cpu.run_slice(16).unwrap();
    assert_eq!(
        cpu.pending_service_image().unwrap().remaining_timeout_ns,
        remaining
    );
    let waking = env.2.clock.freeze(None);
    env.2.push_key_press(prime_emulator::services::ON_KEY_CODE);
    assert!(env.2.take_wake());
    cpu.run_slice(1).unwrap();
    assert!(
        cpu.pending_service_image().is_some(),
        "host time asleep cannot expire guest Sleep"
    );
    // No CPU boundary is needed to prove the shared inspection clock lease.
    // The sleeping CPU remains registered, so coordinate its owner explicitly.
    let points = env.2.safe_points.clone();
    let clock = env.2.clock.clone();
    let (ready, rx) = mpsc::channel();
    let pause_thread = std::thread::spawn(move || {
        let lease = points.pause(Duration::from_secs(1)).unwrap();
        drop(waking);
        let paused_at = clock.now();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(clock.now(), paused_at);
        // Hold time at the end of inspection until the CPU owner has
        // observed the still-pending service. Host scheduling can delay
        // that observation beyond the entire guest sleep interval.
        let inspected = clock.freeze(None);
        drop(lease);
        ready.send(inspected).unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let inspected = loop {
        match rx.try_recv() {
            Ok(inspected) => break inspected,
            Err(mpsc::TryRecvError::Empty) => {
                assert!(std::time::Instant::now() < deadline);
                cpu.run_slice(1).unwrap();
            }
            Err(mpsc::TryRecvError::Disconnected) => panic!("inspection owner failed"),
        }
    };
    pause_thread.join().unwrap();
    cpu.run_slice(1).unwrap();
    assert!(cpu.pending_service_image().is_some());
    drop(inspected);
    std::thread::sleep(Duration::from_millis(60));
    cpu.run_slice(1).unwrap();
    assert!(cpu.pending_service_image().is_none());
    assert_eq!(cpu.sp(), 0x10008000);
}

#[test]
fn disk_room_reports_complete_live_totals_after_actual_arm_file_operations() {
    use std::sync::atomic::Ordering;
    let mut env = env();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("C")).unwrap();
    env.2 = Arc::new(SharedState::new(
        root.path().to_path_buf(),
        "A:/test".into(),
    ));
    let capacity = 1u64 << 40;
    env.2.storage_capacity.store(capacity, Ordering::Release);
    let path = 0x10000400;
    let mode = 0x10000600;
    let data = 0x10001000;
    let out = 0x10003000;
    for (address, value) in [(path, r"C:\usage"), (mode, "wb")] {
        let bytes: Vec<u8> = value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        env.0.write_bytes(address, &bytes).unwrap();
    }
    env.0.write_bytes(data, &vec![0x5a; 3000]).unwrap();
    let handle = call(&env, "__wfopen", [path, mode, 0, 0]);
    assert_ne!(handle, 0);
    assert_eq!(call(&env, "_fwrite", [data, 1, 3000, handle]), 3000);
    assert_eq!(call(&env, "_fclose", [handle, 0, 0, 0]), 1);
    assert_eq!(call(&env, "FSGetDiskRoomState", [0, out, 0, 0]), 0);
    let bytes = env.0.read_bytes_alloc(out, 60).unwrap();
    for offset in [0x14, 0x1c, 0x24] {
        assert_eq!(
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()),
            capacity
        );
    }
    assert_eq!(
        u64::from_le_bytes(bytes[0x2c..0x34].try_into().unwrap()),
        3000
    );
    assert_eq!(
        u64::from_le_bytes(bytes[0x34..0x3c].try_into().unwrap()),
        capacity - 3000
    );
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
    assert_eq!(call(&env, "__wremove", [path, 0, 0, 0]), 1);
    assert_eq!(call(&env, "FSGetDiskRoomState", [0, out, 0, 0]), 0);
    let bytes = env.0.read_bytes_alloc(out, 60).unwrap();
    assert_eq!(
        u64::from_le_bytes(bytes[0x34..0x3c].try_into().unwrap()),
        capacity
    );
    env.0.write_bytes(out, &[0xaa; 60]).unwrap();
    assert_eq!(call(&env, "FSGetDiskRoomState", [1, out, 0, 0]), u32::MAX);
    assert_eq!(
        env.0.read_bytes_alloc(out, 60).unwrap(),
        [0xaa; 60],
        "an unavailable volume must leave output untouched"
    );
}

#[test]
fn virtual_battery_updates_real_arm_driver_queries_and_delivers_os_payloads() {
    use ppl_engine::environment::{BatteryPower, EnvironmentConfig};
    let env = env();
    let name = 0x10000400;
    let out = 0x10000800;
    let retlen = 0x10000700;
    env.0.write_cstr(name, r"\\?\BAT").unwrap();
    let handle = call(&env, "CreateFile", [name, 0, 0, 0]);
    assert_ne!(handle, 0);
    let config = EnvironmentConfig {
        battery: ppl_engine::environment::BatteryConfig {
            adc: 800,
            power: BatteryPower::Charged,
        },
        ..Default::default()
    };
    env.2.configure_environment(config.clone()).unwrap();
    for (index, value) in [out, 16, retlen, 0].into_iter().enumerate() {
        env.0
            .write_u32(0x1000f000 + index as u32 * 4, value)
            .unwrap();
    }
    env.0.write_bytes(out, &[0xa5; 16]).unwrap();
    assert_eq!(
        call(&env, "DeviceIoControl", [handle, 256, 0xdeadbeef, 0xffff]),
        800,
        "BAT ignores unused input rather than reading it"
    );
    assert_eq!(env.0.read_u32(retlen).unwrap(), 800);
    assert_eq!(env.0.read_u16(out + 6).unwrap(), 800);
    assert_eq!(env.0.read_u16(out + 10).unwrap(), 1);
    assert_eq!(
        env.0.read_bytes_alloc(out + 12, 4).unwrap(),
        [1, 4, 2, 0xa5]
    );
    assert_eq!(env.0.read_u16(out + 8).unwrap(), 0xa5a5);
    assert_eq!(call(&env, "BatteryLowCheck", [0, 0, 0, 0]), 0);
    for (subtype, value) in [(2, 1), (0, 1)] {
        assert_eq!(call(&env, "GetEvent", [out, 0, 0, 0]), 0);
        assert_eq!(env.0.read_u32(out + 4).unwrap(), 0x4000);
        assert_eq!(env.0.read_u32(out + 8).unwrap(), subtype);
        assert_eq!(env.0.read_u32(out + 12).unwrap(), value);
    }
    let mut invalid = config.clone();
    invalid.battery.adc = 1024;
    assert!(env.2.configure_environment(invalid).is_err());
    assert_eq!(*env.2.environment.lock().unwrap(), config);
    for event in 0x2000..0x2008 {
        env.2.push_system_event(event);
    }
    let mut changed = config.clone();
    changed.battery.power = BatteryPower::Battery;
    assert!(env.2.configure_environment(changed).is_err());
    assert_eq!(
        *env.2.environment.lock().unwrap(),
        config,
        "queue refusal must not partially change the battery"
    );
}

#[test]
fn critical_battery_action_respects_shutdown_inhibit_and_on_resumes_execution() {
    use ppl_engine::environment::{BatteryConfig, BatteryPower, EnvironmentConfig};
    for inhibit in [0, 1, 2, u32::MAX] {
        let env = env();
        env.2
            .configure_environment(EnvironmentConfig {
                battery: BatteryConfig {
                    adc: 740,
                    power: BatteryPower::Battery,
                },
                ..Default::default()
            })
            .unwrap();
        assert_eq!(call(&env, "SetSystemVariable", [87, 0, inhibit, 0]), 0);
        assert_eq!(
            call(&env, "SetSystemVariable", [87, 0xffff, 17, 0]),
            inhibit
        );
        assert_eq!(call(&env, "BatteryLowCheck", [0, 0, 0, 0]), (-2i32) as u32);
        assert!(!env.2.sleeping(), "query is not a shutdown request");
        assert_eq!(call(&env, "BatteryLowCheck", [0, 1, 0, 0]), (-2i32) as u32);
        // The posted request is consumed at the next owner boundary.
        assert_eq!(env.2.power_execution_allowed(false), inhibit == 1);
        assert_eq!(env.2.sleeping(), inhibit != 1);
        if inhibit != 1 {
            env.2.push_key_press(prime_emulator::services::ON_KEY_CODE);
            assert!(env.2.power_execution_allowed(false));
            assert!(!env.2.sleeping());
            assert_eq!(env.2.sleeps(), 1);
        }
    }
}

#[test]
fn absent_devices_fail_without_success_handles_or_fabricated_output() {
    let env = env();
    let name = 0x10000400;
    let output = 0x10000800;
    let retlen = 0x10000700;
    // The initialized original G1 registry has no FACTORY driver. Its actual
    // OPEN_EXISTING call returns INVALID_HANDLE_VALUE, without factory data.
    env.0.write_u32(0x1000f000, 3).unwrap();
    for absent in [r"\\?\ABSENT", r"\\?\FACTORY"] {
        env.0.write_cstr(name, absent).unwrap();
        assert_eq!(call(&env, "CreateFile", [name, 0, 0, 0]), u32::MAX);
    }
    assert!(env.2.devices.lock().unwrap().is_empty());
    for (index, value) in [output, 16, retlen, 0].into_iter().enumerate() {
        env.0
            .write_u32(0x1000f000 + index as u32 * 4, value)
            .unwrap();
    }
    env.0.write_bytes(output, &[0x5a; 16]).unwrap();
    env.0.write_u32(retlen, 123).unwrap();
    assert_eq!(
        call(&env, "DeviceIoControl", [0xdead, 256, 0xdeadbeef, u32::MAX]),
        0
    );
    assert_eq!(env.0.read_u32(retlen).unwrap(), 0);
    assert_eq!(env.0.read_bytes_alloc(output, 16).unwrap(), [0x5a; 16]);
}

#[test]
fn arm_backlight_controls_share_brightness_with_lcd_and_follow_timed_pwm_ramps() {
    let mut env = env();
    let memory = Arc::get_mut(&mut env.0).unwrap();
    memory.map("LCD heap", 0x21000000, 0x100000).unwrap();
    memory.set_heap(0x21000000, 0x100000);
    let name = 0x10000400;
    let input = 0x10000600;
    let output = 0x10000800;
    let retlen = 0x10000700;
    env.0.write_cstr(name, r"\\?\BACK").unwrap();
    let handle = call(&env, "CreateFile", [name, 0, 0, 0]);
    assert_ne!(handle, 0);
    for (i, value) in [output, 4, retlen, 0].into_iter().enumerate() {
        env.0.write_u32(0x1000f000 + i as u32 * 4, value).unwrap();
    }
    assert_eq!(call(&env, "SetSystemVariable", [6, 0xffff, 0, 0]), 2);
    assert_eq!(call(&env, "SetSystemVariable", [6, 2, 1, 0]), 1);
    let lcd = call(&env, "GetActiveLCD", [0, 0, 0, 0]);
    assert_eq!(
        env.0
            .read_u16(lcd + prime_emulator::services::system2::LCD_MAGIC_OFF + 10)
            .unwrap(),
        1
    );
    assert_eq!(call(&env, "DeviceIoControl", [handle, 257, 0, 0]), 1);
    assert_eq!(env.0.read_u32(output).unwrap(), 1);
    assert_eq!(
        call(&env, "SetSystemVariable", [6, 2, 99, 0]),
        1,
        "invalid brightness does not change the selection"
    );
    env.0.write_u32(input, 3).unwrap();
    assert_eq!(
        call(&env, "DeviceIoControl", [handle, 256, input, 4]),
        u32::MAX,
        "original setter return is not Boolean success"
    );
    assert_eq!(call(&env, "DeviceIoControl", [handle, 259, 0, 0]), 350);
    env.0.write_u32(input, 1).unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 258, input, 4]), 0);
    env.0.write_u32(input, 4).unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 262, input, 4]), 4);
    std::thread::sleep(Duration::from_millis(12));
    assert_eq!(call(&env, "DeviceIoControl", [handle, 259, 0, 0]), 4);
    assert_eq!(env.2.backlight_output().pwm, 4);
    // The same guest clock freezes a pending ramp during inspection/power.
    env.0.write_u32(input, 10).unwrap();
    call(&env, "DeviceIoControl", [handle, 262, input, 4]);
    let frozen = env.2.clock.freeze(None);
    let before = env.2.backlight_output().duty;
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(env.2.backlight_output().duty, before);
    drop(frozen);
    std::thread::sleep(Duration::from_millis(22));
    assert_eq!(env.2.backlight_output().duty, 10);
}

#[test]
fn arm_usb_host_uses_explicit_peer_descriptors_and_bidirectional_endpoint_queues() {
    use ppl_engine::usb::{Link, Operation, Peer};
    let env = env();
    assert!(
        env.2
            .usb_operation(Operation::Connect {
                link: Link::Calculator
            })
            .is_err()
    );
    assert_eq!(env.2.usb.status().link, Link::Disconnected);
    let peer = Peer {
        vendor: 0x03f0,
        product: 0x1541,
        class: 3,
        receive_bytes: 64,
        transmit_bytes: 64,
        extension: true,
    };
    env.2
        .usb_operation(Operation::ConfigurePeer { peer: Some(peer) })
        .unwrap();
    env.2
        .usb_operation(Operation::Connect {
            link: Link::Calculator,
        })
        .unwrap();
    let name = 0x10000400;
    let buffer = 0x10000800;
    let output = 0x10000700;
    env.0.write_cstr(name, r"\\?\USBHOST").unwrap();
    let handle = call(&env, "CreateFile", [name, 0, 0, 0]);
    assert_ne!(handle, 0);
    for (i, value) in [output, 4, 0, 0].into_iter().enumerate() {
        env.0.write_u32(0x1000f000 + i as u32 * 4, value).unwrap();
    }
    assert_eq!(
        call(&env, "DeviceIoControl", [handle, 256, 0, 0]),
        1,
        "cable detection is independent of driver enable"
    );
    assert_eq!(call(&env, "DeviceIoControl", [handle, 4, 0, 0]), 1);
    assert_eq!(call(&env, "DeviceIoControl", [handle, 261, 0, 0]), 1);
    assert_eq!(
        call(&env, "DeviceIoControl", [handle, 257, buffer, 0x12345678]),
        1
    );
    assert_eq!(
        env.0.read_bytes_alloc(buffer, 16).unwrap(),
        [
            0xf0, 3, 0x41, 0x15, 3, 0, 0, 0, 64, 0, 64, 0, 0x78, 0x56, 0x34, 0x12
        ]
    );
    assert_eq!(call(&env, "DeviceIoControl", [handle, 260, 0, 0]), 2);
    assert_eq!(call(&env, "DeviceIoControl", [handle, 263, 0, 0]), 1);
    env.0.write_bytes(buffer, b"actual wire bytes").unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 258, buffer, 17]), 1);
    assert_eq!(env.2.usb.read(true).unwrap(), b"actual wire bytes");
    assert!(
        env.2
            .usb_operation(Operation::ConfigurePeer { peer: None })
            .is_err()
    );
    assert_eq!(call(&env, "DeviceIoControl", [handle, 5, 0, 0]), 1);
    assert_eq!(call(&env, "DeviceIoControl", [handle, 258, buffer, 17]), 0);
    env.2
        .usb_operation(Operation::Connect {
            link: Link::Disconnected,
        })
        .unwrap();
    env.0.write_bytes(buffer, &[0x5a; 16]).unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 257, buffer, 16]), 0);
    assert_eq!(env.0.read_bytes_alloc(buffer, 16).unwrap(), [0x5a; 16]);
}

#[test]
fn arm_indicator_pattern_sleeps_through_rtc_steps_restores_color_and_returns_once() {
    let env = env();
    let name = 0x10000400;
    let record = 0x10000600;
    let colors = 0x10000700;
    let output = 0x10000800;
    env.0.write_cstr(name, r"\\?\INDICATOR").unwrap();
    let handle = call(&env, "CreateFile", [name, 0, 0, 0]);
    assert_ne!(handle, 0);
    for (i, value) in [output, 4, 0, 0].into_iter().enumerate() {
        env.0.write_u32(0x1000f000 + i as u32 * 4, value).unwrap();
    }
    env.0.write_u32(colors, 5).unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 257, colors, 4]), 0);
    assert_eq!(call(&env, "DeviceIoControl", [handle, 256, 0, 0]), 0);
    assert_eq!(env.0.read_u32(output).unwrap(), 5);
    assert_eq!(call(&env, "DeviceIoControl", [handle, 258, 0, 0]), u32::MAX);
    env.0.write_bytes(colors, &[1, 2]).unwrap();
    let start = env.2.rtc.seconds() + 2;
    for (i, value) in [colors, 2, 1, 2, start, 0, 0, 0, 0, 0]
        .into_iter()
        .enumerate()
    {
        env.0.write_u32(record + i as u32 * 4, value).unwrap();
    }
    let svc = 0xef000000 | names::service_id_by_name("DeviceIoControl").unwrap();
    for (i, word) in [0xe92d0001u32, 0xe92d4000, svc, 0xe2855001, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        env.0.write_u32(0x10002000 + i as u32 * 4, word).unwrap();
    }
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10002000, 0x1000f000, handle).unwrap();
    cpu.set_lr(0x1000200c).unwrap();
    for (reg, value) in [
        (RegisterARM::R1, 260),
        (RegisterARM::R2, record),
        (RegisterARM::R3, 40),
    ] {
        cpu.set_reg(reg, value).unwrap();
    }
    cpu.run_slice(16).unwrap();
    assert!(cpu.pending_service_image().is_some());
    assert!(env.2.sleeping());
    assert!(!env.2.firmware_sleeping());
    assert_eq!(cpu.reg(RegisterARM::R5), 0);
    let mut seen = std::collections::BTreeSet::new();
    let end = std::time::Instant::now() + Duration::from_secs(12);
    while cpu.pending_service_image().is_some() {
        cpu.run_slice(16).unwrap();
        seen.insert(env.2.peripheral_output().indicator);
        assert!(
            std::time::Instant::now() < end,
            "indicator wait did not finish"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        seen.contains(&1) && seen.contains(&2),
        "real RTC must advance both pattern colors: {seen:?}"
    );
    assert_eq!(env.0.read_u32(record + 20).unwrap(), 2);
    assert_eq!(env.2.peripheral_output().indicator, 5);
    assert_eq!(cpu.reg(RegisterARM::R0), 0);
    assert_eq!(cpu.reg(RegisterARM::R5), 1);
    assert!(!env.2.sleeping());
    cpu.run_slice(16).unwrap();
    assert_eq!(cpu.reg(RegisterARM::R5), 1);
}

#[test]
fn arm_indicator_on_cancels_infinite_wait_and_inactive_descriptors_ignore_unused_data() {
    let env = env();
    let name = 0x10000400;
    let record = 0x10000600;
    let colors = 0x10000700;
    env.0.write_cstr(name, r"\\?\indicator").unwrap();
    let handle = call(&env, "CreateFile", [name, 0, 0, 0]);
    assert_ne!(
        handle, 0,
        "original registry comparison is case insensitive"
    );
    for i in 0..4 {
        env.0.write_u32(0x1000f000 + i * 4, 0).unwrap();
    }
    for (i, value) in [0xdeadbeef, 1, 1, 0, env.2.rtc.seconds() + 10, 0, 0, 0, 0, 0]
        .into_iter()
        .enumerate()
    {
        env.0.write_u32(record + i as u32 * 4, value).unwrap();
    }
    assert_eq!(
        call(&env, "DeviceIoControl", [handle, 260, record, 40]),
        0,
        "inactive patterns do not read their color pointer"
    );
    assert!(!env.2.sleeping());
    assert!(env.2.indicator.lock().unwrap().active.is_none());
    env.0.write_u32(record + 8, 60).unwrap();
    assert_eq!(
        call(&env, "DeviceIoControl", [handle, 260, record, 40]),
        u32::MAX
    );
    env.0.write_bytes(colors, &[7]).unwrap();
    for (i, value) in [
        colors,
        1,
        0,
        u32::MAX,
        env.2.rtc.seconds() + 2,
        0,
        0,
        0,
        0,
        0,
    ]
    .into_iter()
    .enumerate()
    {
        env.0.write_u32(record + i as u32 * 4, value).unwrap();
    }
    let svc = 0xef000000 | names::service_id_by_name("DeviceIoControl").unwrap();
    for (i, word) in [0xe92d0001u32, 0xe92d4000, svc, 0xe2855001, 0xeafffffe]
        .into_iter()
        .enumerate()
    {
        env.0.write_u32(0x10002000 + i as u32 * 4, word).unwrap();
    }
    let mut cpu = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.set_entry(0x10002000, 0x1000f000, handle).unwrap();
    cpu.set_lr(0x1000200c).unwrap();
    for (reg, value) in [
        (RegisterARM::R1, 260),
        (RegisterARM::R2, record),
        (RegisterARM::R3, 40),
    ] {
        cpu.set_reg(reg, value).unwrap();
    }
    cpu.run_slice(16).unwrap();
    assert!(cpu.pending_service_image().is_some());
    assert!(env.2.sleeping());
    env.2.push_key_press(prime_emulator::services::ON_KEY_CODE);
    cpu.run_slice(16).unwrap();
    assert!(cpu.pending_service_image().is_none());
    assert!(!env.2.sleeping());
    assert_eq!(cpu.reg(RegisterARM::R0), 0);
    assert_eq!(cpu.reg(RegisterARM::R5), 1);
    assert_eq!(
        env.2.input_events_pending(),
        0,
        "wake key cannot leak into firmware input"
    );
    assert!(env.2.indicator.lock().unwrap().active.is_none());
    assert_eq!(env.2.peripheral_output().indicator, 0);
}

#[test]
fn arm_arch_reports_configured_board_pins_with_exact_output_and_failure_contracts() {
    use ppl_engine::environment::EnvironmentConfig;
    let env = env();
    let name = 0x10000400;
    let output = 0x10000800;
    let retlen = 0x10000700;
    env.0.write_cstr(name, r"\\?\ARCH").unwrap();
    let handle = call(&env, "CreateFile", [name, 0, 0, 0]);
    for (i, value) in [output, 4, retlen, 0].into_iter().enumerate() {
        env.0.write_u32(0x1000f000 + i as u32 * 4, value).unwrap();
    }
    env.2
        .configure_environment(EnvironmentConfig {
            board_revision_pins: 3,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 260, 0, 0]), 0);
    assert_eq!(env.0.read_u32(output).unwrap(), 1);
    assert_eq!(call(&env, "DeviceIoControl", [handle, 261, 0, 0]), 0);
    assert_eq!(env.0.read_u32(output).unwrap(), 3);
    env.0.write_bytes(output, &[0xa5; 16]).unwrap();
    env.0.write_u32(0x1000f004, 16).unwrap();
    assert_eq!(call(&env, "DeviceIoControl", [handle, 260, 0, 0]), u32::MAX);
    assert_eq!(env.0.read_bytes_alloc(output, 16).unwrap(), [0xa5; 16]);
    assert_eq!(env.0.read_u32(retlen).unwrap(), u32::MAX);
    assert!(
        env.2
            .configure_environment(EnvironmentConfig {
                board_revision_pins: 4,
                ..Default::default()
            })
            .is_err()
    );
    assert_eq!(env.2.environment.lock().unwrap().board_revision_pins, 3);
}

#[test]
fn sleeping_worker_must_withdraw_its_scheduler_request_so_on_can_resume_the_power_owner() {
    let env = env();
    let id = names::service_id_by_name("SysPowerOff").unwrap();
    for (i, instruction) in [
        0xe92d0001u32,
        0xe92d4000,
        0xef000000 | id,
        0xe3a0404d,
        0xeafffffe,
    ]
    .into_iter()
    .enumerate()
    {
        env.0
            .write_u32(0x10000200 + 4 * i as u32, instruction)
            .unwrap();
    }
    env.0.write_u32(0x10000300, 0xeafffffe).unwrap();
    let mut power = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    power.set_entry(0x10000200, 0x10008000, 0).unwrap();
    power.set_lr(0x1000020c).unwrap();
    let mut background = CpuThread::new(
        env.0.clone(),
        env.1.clone(),
        env.2.clone(),
        env.3.clone(),
        UnsupportedPolicy::Stop,
        1,
        vec![],
    )
    .unwrap();
    background.set_entry(0x10000300, 0x10009000, 0).unwrap();
    power.run_slice(16).unwrap();
    assert!(env.2.firmware_sleeping());
    assert!(matches!(
        power.pending_service_image().unwrap().wait,
        prime_emulator::services::pending::Wait::PowerOff
    ));
    // A runnable background owner can request its next turn before OFF blocks
    // it. Reproduce the observed kernel image: exhausted main quantum and a
    // queued, unused background turn, with an ON wake already accepted.
    let occupied = env.2.scheduler.try_enter(0, false).unwrap();
    assert!(env.2.scheduler.try_enter(1, false).is_none());
    std::thread::sleep(Duration::from_millis(35));
    drop(occupied);
    background.run_slice(16).unwrap();
    env.2.push_key_press(prime_emulator::services::ON_KEY_CODE);
    power.run_slice(16).unwrap();
    assert!(
        !env.2.sleeping(),
        "a sleeping background request must not starve SysPowerOff completion"
    );
    assert!(power.pending_service_image().is_none());
    assert_eq!(
        power.reg(RegisterARM::R4),
        77,
        "the original ARM frame must resume after OFF"
    );
}
