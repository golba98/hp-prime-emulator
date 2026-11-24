//! Fresh-process restoration of production CPU/resource continuations. The ARM
//! program is self-authored; these checks do not claim full calculator restore.
use prime_emulator::{
    engine::{CpuThread, ExecutionImage},
    memory::{GuestMemory, image::MemoryImage},
    services::{
        Registry, SharedState, UnsupportedPolicy, handlers, names, pending::ServiceImage,
        sync::image::SyncImage,
    },
    trace::{TraceConfig, TraceHub},
};
use serde::{Deserialize, Serialize};
use std::{cell::Cell, path::Path, sync::Arc};
use unicorn_engine::RegisterARM;

const ID: &str = "production-sync-test-v1";
const BASE: u32 = 0x10000000;
const RESULT: u32 = BASE + 0x4000;
fn memory() -> GuestMemory {
    let mut memory = GuestMemory::new();
    memory.map("test-code-stack", BASE, 65536).unwrap();
    memory.map("test-heap", 0x20000000, 65536).unwrap();
    memory.set_heap(0x20000000, 65536);
    memory
}
struct World {
    mem: Arc<GuestMemory>,
    shared: Arc<SharedState>,
    registry: Arc<Registry>,
    trace: Arc<TraceHub>,
    next_code: Cell<u32>,
}
impl World {
    fn new(mem: GuestMemory) -> Self {
        Self::with_clock(mem, Arc::new(prime_emulator::clock::Clock::default()))
    }
    fn with_clock(mem: GuestMemory, clock: Arc<prime_emulator::clock::Clock>) -> Self {
        let mut registry = Registry::new();
        handlers::install(&mut registry);
        Self {
            mem: Arc::new(mem),
            shared: Arc::new(SharedState::with_clock(
                Default::default(),
                "A:/test".into(),
                clock,
            )),
            registry: Arc::new(registry),
            trace: Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
            next_code: Cell::new(BASE + 0x8000),
        }
    }
    fn cpu(&self, owner: u32) -> CpuThread {
        CpuThread::new(
            self.mem.clone(),
            self.registry.clone(),
            self.shared.clone(),
            self.trace.clone(),
            UnsupportedPolicy::Stop,
            owner,
            vec![],
        )
        .unwrap()
    }
    fn invoke(&self, name: &str, args: [u32; 2], owner: u32, computation: bool) -> CpuThread {
        let entry = self.next_code.get();
        self.next_code.set(entry + 128);
        let mut words = vec![
            0xe92d0001u32,
            0xe92d4000,
            0xef000000 | names::service_id_by_name(name).unwrap(),
        ];
        if computation {
            words.extend([0xe2800007, 0xe5840000]);
        } // ADD r0,#7; STR r0,[r4].
        words.push(0xeafffffe);
        for (i, word) in words.into_iter().enumerate() {
            self.mem.write_u32(entry + 4 * i as u32, word).unwrap();
        }
        let mut cpu = self.cpu(owner);
        cpu.set_entry(entry, BASE + 0xf000 - owner * 0x100, args[0])
            .unwrap();
        cpu.set_lr(entry + 12).unwrap();
        cpu.set_reg(RegisterARM::R1, args[1]).unwrap();
        cpu.set_reg(RegisterARM::R4, RESULT).unwrap();
        cpu.run_slice(32).unwrap();
        cpu
    }
    fn call(&self, name: &str, args: [u32; 2], owner: u32) -> u32 {
        self.invoke(name, args, owner, false).reg(RegisterARM::R0)
    }
}
#[derive(Serialize, Deserialize)]
struct Fixture {
    case: String,
    clock: prime_emulator::clock::ClockImage,
    execution: ExecutionImage,
    pending: ServiceImage,
    sync: SyncImage,
    descriptor: u32,
}
fn make(case: &str, path: &Path) {
    let world = World::new(memory());
    let descriptor = match case {
        "auto" | "manual" | "closed" => {
            world.call("OSCreateEvent", [u32::from(case == "manual"), 0], 42)
        }
        "semaphore" => world.call("OSCreateSemaphore", [0, 0], 42),
        "sleep" => 500,
        "critical" => {
            let descriptor = world.mem.heap_alloc(0x14).unwrap();
            world.call("OSInitCriticalSection", [descriptor, 0], 42);
            world.call("OSEnterCriticalSection", [descriptor, 0], 9);
            world.call("OSEnterCriticalSection", [descriptor, 0], 9);
            descriptor
        }
        _ => unreachable!(),
    };
    assert_ne!(descriptor, 0);
    let service = match case {
        "sleep" => "OSSleep",
        "semaphore" => "OSWaitForSemaphore",
        "critical" => "OSEnterCriticalSection",
        _ => "OSWaitForEvent",
    };
    let mut waiting = world.invoke(service, [descriptor, u32::MAX], 0, true);
    waiting.run_slice(1).unwrap();
    let pending = waiting.pending_service_image().unwrap();
    if case == "closed" {
        assert_eq!(world.call("OSCloseEvent", [descriptor, 0], 42), 1);
    }
    let _frozen = world.shared.clock.freeze(None);
    let fixture = Fixture {
        case: case.into(),
        clock: world.shared.clock.image().unwrap(),
        descriptor,
        execution: waiting.execution_image().unwrap(),
        pending,
        sync: world.shared.sync_objects.lock().unwrap().image().unwrap(),
    };
    MemoryImage::capture(&world.mem, ID)
        .unwrap()
        .save(&path.join("memory.bin"))
        .unwrap();
    std::fs::write(
        path.join("fixture.json"),
        serde_json::to_vec(&fixture).unwrap(),
    )
    .unwrap();
}
#[test]
fn production_waits_resume_correct_arm_results_in_fresh_processes() {
    for case in ["auto", "manual", "semaphore", "critical", "closed", "sleep"] {
        let root = tempfile::tempdir().unwrap();
        make(case, root.path());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "restored_sync_child", "--nocapture"])
            .env("PRIME_SYNC_RESTORE_CHILD", root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("production-sync-resumed"));
    }
}
#[test]
fn restored_sync_child() {
    let Some(path) = std::env::var_os("PRIME_SYNC_RESTORE_CHILD") else {
        return;
    };
    let path = Path::new(&path);
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(path.join("fixture.json")).unwrap()).unwrap();
    if fixture.case == "sleep" {
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
    let mem = MemoryImage::load(&path.join("memory.bin"), ID)
        .unwrap()
        .restore(ID)
        .unwrap();
    let clock = Arc::new(prime_emulator::clock::Clock::restore(fixture.clock).unwrap());
    let world = World::with_clock(mem, clock);
    world.next_code.set(BASE + 0xc000);
    let mut resources = fixture.sync.restore(world.mem.clone()).unwrap();
    resources.install_handles(&world.shared).unwrap();
    let mut cpu = world.cpu(0);
    // A CPU whose arguments contradict the saved service must be rolled back.
    let before = cpu.execution_image().unwrap();
    let mut invalid = fixture.execution.clone();
    invalid.cpu.gprs[1] = 13;
    assert!(cpu.restore_execution_image(invalid, &resources).is_err());
    assert_eq!(cpu.execution_image().unwrap().cpu, before.cpu);
    assert!(cpu.pending_service_image().is_none());
    cpu.restore_execution_image(fixture.execution, &resources)
        .unwrap();
    assert_eq!(
        world.shared.main_wait_entries(),
        0,
        "restoration cannot invent a UI round trip"
    );
    let old_identity = fixture
        .pending
        .resource
        .as_ref()
        .map(|resource| resource.identity);
    drop(resources); // Pending owners and live handles now own all object leases.
    match fixture.case.as_str() {
        "sleep" => {
            cpu.run_slice(1).unwrap();
            assert!(
                cpu.pending_service_image().is_some(),
                "host downtime must not expire the restored guest deadline"
            );
            let remaining = cpu
                .pending_service_image()
                .unwrap()
                .remaining_timeout_ns
                .unwrap();
            std::thread::sleep(
                std::time::Duration::from_nanos(remaining) + std::time::Duration::from_millis(10),
            );
        }
        "auto" | "manual" => {
            assert_eq!(world.call("OSSetEvent", [fixture.descriptor, 0], 42), 1);
        }
        "semaphore" => {
            assert_eq!(
                world.call("OSReleaseSemaphore", [fixture.descriptor, 0], 42),
                1
            );
        }
        "critical" => {
            world.call("OSLeaveCriticalSection", [fixture.descriptor, 0], 9);
            cpu.run_slice(1).unwrap();
            assert!(
                cpu.pending_service_image().is_some(),
                "recursive lock must remain owned after one release"
            );
            world.call("OSLeaveCriticalSection", [fixture.descriptor, 0], 9);
        }
        "closed" => {
            assert!(
                world.mem.heap_size_of(fixture.descriptor).is_some(),
                "closed descriptor recycled before its restored waiter returned"
            );
        }
        _ => unreachable!(),
    }
    cpu.run_slice(16).unwrap();
    let expected = match fixture.case.as_str() {
        "critical" | "sleep" => 7,
        "closed" => 0x84 + 7,
        _ => 0x83 + 7,
    };
    assert_eq!(world.mem.read_u32(RESULT).unwrap(), expected);
    assert_eq!(cpu.reg(RegisterARM::SP), BASE + 0xf000);
    assert!(cpu.pending_service_image().is_none());
    assert!(!world.shared.main_ui_waiting());
    if fixture.case == "closed" {
        assert!(world.mem.heap_size_of(fixture.descriptor).is_none());
        let replacement = world.call("OSCreateEvent", [0, 0], 42);
        assert_eq!(
            replacement, fixture.descriptor,
            "the finished waiter must release its descriptor"
        );
        let table = world.shared.sync_objects.lock().unwrap();
        assert_ne!(
            Some(table.get(&replacement).unwrap().identity()),
            old_identity
        );
        assert!(
            table
                .image()
                .unwrap()
                .resources
                .iter()
                .all(|r| Some(r.identity) != old_identity)
        );
    } else if fixture.case == "auto" || fixture.case == "manual" {
        let table = world.shared.sync_objects.lock().unwrap();
        let resource = table.get(&fixture.descriptor).unwrap();
        if let prime_emulator::services::sync::SyncObj::Event(event) = &resource.object {
            assert!(!event.guest_waiter_blocked(0));
            assert_eq!(
                event.poll(17),
                fixture.case == "manual",
                "manual/auto reset must survive reconstruction"
            );
        } else {
            unreachable!()
        }
    }
    println!("production-sync-resumed");
}
#[test]
fn malformed_resource_images_leave_the_target_allocator_untouched() {
    let root = tempfile::tempdir().unwrap();
    make("auto", root.path());
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(root.path().join("fixture.json")).unwrap()).unwrap();
    let mem = MemoryImage::load(&root.path().join("memory.bin"), ID)
        .unwrap()
        .restore(ID)
        .unwrap();
    let mem = Arc::new(mem);
    let bytes = mem.read_bytes_alloc(0x20000000, 65536).unwrap();
    let allocated = mem.heap_allocated();
    let mut bad = fixture.sync.clone();
    bad.version += 1;
    assert!(bad.restore(mem.clone()).is_err());
    let mut bad = fixture.sync.clone();
    bad.resources.push(bad.resources[0].clone());
    assert!(bad.restore(mem.clone()).is_err());
    let mut bad = fixture.sync.clone();
    bad.resources[0].descriptor = 0xfffffff0;
    assert!(bad.restore(mem.clone()).is_err());
    let mut bad = fixture.sync.clone();
    bad.resources[0].closed = true;
    assert!(bad.restore(mem.clone()).is_err());
    assert_eq!(mem.heap_allocated(), allocated);
    assert_eq!(mem.read_bytes_alloc(0x20000000, 65536).unwrap(), bytes);
}

#[test]
fn restored_high_generations_cannot_exhaust_new_resource_identities() {
    let root = tempfile::tempdir().unwrap();
    make("auto", root.path());
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(root.path().join("fixture.json")).unwrap()).unwrap();
    let memory = MemoryImage::load(&root.path().join("memory.bin"), ID)
        .unwrap()
        .restore(ID)
        .unwrap();
    let mut image = fixture.sync;
    image.resources[0].identity = u64::MAX - 1;
    for identity in image.handles.values_mut() {
        *identity = u64::MAX - 1;
    }
    let restored = image.restore(Arc::new(memory)).unwrap();
    let resource = prime_emulator::services::sync::SyncEntry::new(
        prime_emulator::services::sync::SyncObj::Event(
            prime_emulator::services::sync::HostEvent::new(false, false),
        ),
        None,
    );
    assert_ne!(resource.identity(), 0);
    assert_ne!(resource.identity(), u64::MAX - 1);
    assert_ne!(resource.identity(), u64::MAX);
    assert!(restored.bind(fixture.descriptor, u64::MAX - 1).is_ok());
}
