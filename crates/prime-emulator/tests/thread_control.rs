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
    assert_eq!(call(&env, "OSResumeThread", [desc, 0, 0, 0]), 1);
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
    assert_eq!(call(&env, "OSWakeUpThread", [desc, 0, 0, 0]), 1);
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
