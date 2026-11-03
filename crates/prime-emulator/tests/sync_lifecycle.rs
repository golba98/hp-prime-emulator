//! Real host waits through SVC handlers; no firmware or physical device needed.
use prime_emulator::{
    memory::{GuestMemory, snapshot::Snapshot},
    services::{GuestArgs, HandlerFn, SharedState, SvcCtx, sync, threading},
    trace::{TraceConfig, TraceHub},
};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use unicorn_engine::{
    Unicorn,
    unicorn_const::{Arch, Mode},
};

struct Env {
    mem: Arc<GuestMemory>,
    state: Arc<SharedState>,
    trace: Arc<TraceHub>,
}
impl Env {
    fn new() -> Self {
        let mut mem = GuestMemory::new();
        mem.map("heap", 0x2000_0000, 65536).unwrap();
        mem.set_heap(0x2000_0000, 65536);
        Self {
            mem: Arc::new(mem),
            state: Arc::new(SharedState::new(
                std::path::PathBuf::new(),
                "A:/test".into(),
            )),
            trace: Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
        }
    }
    fn call(&self, handler: HandlerFn, args: [u32; 2]) -> u32 {
        let mut uc = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
        let mut ctx = SvcCtx {
            uc: &mut uc,
            mem: &self.mem,
            shared: &self.state,
            trace: &self.trace,
            thread_id: 1,
            redirected: false,
            args: GuestArgs {
                r0: args[0],
                r1: args[1],
                r2: 0,
                r3: 0,
                sp: 0,
                caller_pc: 0,
                stack_slots: [0; 2],
            },
        };
        handler(&mut ctx).unwrap()
    }
    fn capture(&self) -> Snapshot<'_> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(g) = self.mem.snapshot_gate.try_snapshot() {
                return g;
            }
            assert!(Instant::now() < deadline, "blocked SVC prevented snapshot");
            std::thread::yield_now();
        }
    }
    fn waiter(self: &Arc<Self>, handler: HandlerFn, desc: u32) -> std::thread::JoinHandle<u32> {
        let this = self.clone();
        let (ready, rx) = mpsc::channel();
        let h = std::thread::spawn(move || {
            let _active = this.mem.snapshot_gate.enter();
            ready.send(()).unwrap();
            this.call(handler, [desc, u32::MAX])
        });
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        h
    }
}
#[test]
fn event_close_defers_descriptor_reuse_until_waiter_exits() {
    let e = Arc::new(Env::new());
    let desc = e.call(threading::os_create_event, [0, 0]);
    let waiter = e.waiter(threading::os_wait_for_event, desc);
    let capture = e.capture();
    assert_eq!(e.call(threading::os_close_object, [desc, 0]), 1);
    assert!(
        e.mem.heap_size_of(desc).is_some(),
        "closed descriptor recycled while waiter retained a lease"
    );
    assert_ne!(e.mem.heap_alloc(32).unwrap(), desc);
    drop(capture);
    assert_eq!(waiter.join().unwrap(), sync::WAIT_RESULT_ERROR);
    assert!(e.mem.heap_size_of(desc).is_none());
    assert_eq!(e.mem.heap_alloc(32).unwrap(), desc);
}
#[test]
fn semaphore_wait_parks_for_snapshot_and_resumes_on_release() {
    let e = Arc::new(Env::new());
    let desc = e.call(threading::os_create_semaphore, [0, 0]);
    let waiter = e.waiter(threading::os_wait_for_semaphore, desc);
    drop(e.capture());
    assert_eq!(e.call(threading::os_release_semaphore, [desc, 0]), 1);
    assert_eq!(waiter.join().unwrap(), sync::WAIT_RESULT_RESOLVED);
    assert_eq!(e.call(threading::os_close_object, [desc, 0]), 1);
    assert!(e.mem.heap_size_of(desc).is_none());
}
#[test]
fn critical_section_wait_parks_and_caller_keeps_descriptor_ownership() {
    let e = Arc::new(Env::new());
    let desc = e.mem.heap_alloc(32).unwrap();
    e.call(threading::os_init_critical_section, [desc, 0]);
    e.call(threading::os_enter_critical_section, [desc, 0]);
    let other = e.clone();
    let (ready, rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let _active = other.mem.snapshot_gate.enter();
        ready.send(()).unwrap();
        other.call(threading::os_enter_critical_section, [desc, 0]);
        other.call(threading::os_leave_critical_section, [desc, 0]);
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(e.capture());
    e.call(threading::os_leave_critical_section, [desc, 0]);
    waiter.join().unwrap();
    e.call(threading::os_delete_critical_section, [desc, 0]);
    assert_eq!(
        e.mem.heap_size_of(desc),
        Some(32),
        "caller-owned descriptor was freed"
    );
    assert_eq!(e.mem.read_u32(desc).unwrap(), 0);
}
#[test]
fn closing_a_semaphore_wakes_an_infinite_wait() {
    let e = Arc::new(Env::new());
    let desc = e.call(threading::os_create_semaphore, [0, 0]);
    let waiter = e.waiter(threading::os_wait_for_semaphore, desc);
    drop(e.capture());
    e.call(threading::os_close_object, [desc, 0]);
    assert_eq!(waiter.join().unwrap(), sync::WAIT_RESULT_ERROR);
    assert!(e.mem.heap_size_of(desc).is_none());
}
#[test]
fn svc_json_identifies_both_supported_and_unsupported_threads() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("svc.jsonl");
    let trace = TraceHub::new(TraceConfig {
        svc_trace: true,
        jsonl_path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    trace.svc(0x10037, Some("lmalloc"), 0x1234, 1, true, 7);
    trace.svc(0x102d1, None, 0x5678, 2, false, 8);
    drop(trace);
    let rows: Vec<serde_json::Value> = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(rows[0]["thread_id"], 7);
    assert_eq!(rows[1]["thread_id"], 8);
}

#[test]
fn exhausted_sync_creation_returns_null_without_registering_objects() {
    let e = Env::new();
    e.mem.heap_alloc(65536).unwrap();
    assert_eq!(e.call(threading::os_create_event, [0, 0]), 0);
    assert_eq!(e.call(threading::os_create_semaphore, [0, 0]), 0);
    assert!(e.state.sync_objects.lock().unwrap().is_empty());
}
