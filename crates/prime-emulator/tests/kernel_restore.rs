//! Fresh-process CPU + RAM + kernel + complete storage reconstruction. These
//! self-authored ARM controls exercise production services, not an HP boot.
use prime_emulator::{
    engine::CpuThread,
    memory::{GuestMemory, image::MemoryImage},
    services::{Registry, SharedState, UnsupportedPolicy, handlers, image::KernelImage, names},
    storage::image::StorageImage,
    trace::{TraceConfig, TraceHub},
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use unicorn_engine::RegisterARM;
const ID: &str = "kernel-arm-roundtrip-v1";
const BASE: u32 = 0x10000000;
const COUNT: u32 = BASE + 0x4000;
const FILE: u32 = 0x10001;
fn registry() -> Arc<Registry> {
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    Arc::new(registry)
}
fn trace() -> Arc<TraceHub> {
    Arc::new(TraceHub::new(TraceConfig::default()).unwrap())
}
fn make(path: &Path, power: bool) {
    let data = path.join("data");
    std::fs::create_dir_all(data.join("C")).unwrap();
    let mut mem = GuestMemory::new();
    mem.map("code-stack", BASE, 65536).unwrap();
    mem.map("heap", 0x20000000, 1024 * 1024).unwrap();
    mem.set_heap(0x20000000, 1024 * 1024);
    let mem = Arc::new(mem);
    let shared = Arc::new(SharedState::new(data.clone(), "A:/test-arm".into()));
    let file = shared.vfs.open("C:/working", "w+").unwrap();
    assert_eq!(file, FILE);
    assert_eq!(shared.vfs.write(file, b"first-second"), Some(12));
    shared.vfs.seek_set(file, 6).unwrap();
    let device = shared.register_device(r"\\?\USB");
    assert_eq!(device, 1);
    shared.usb.enable(false, true);
    shared.usb.connect(ppl_engine::usb::Link::Computer);
    shared.usb.callback(false, BASE + 0x100);
    shared.usb.receive(false, vec![0x41, 0, 0xff]).unwrap();
    assert!(shared.usb.transmit(false, vec![0x22, 0x33]));
    shared.indicator.lock().unwrap().raw = 5;
    {
        let mut back = shared.backlight.lock().unwrap();
        back.open();
        back.duty = 210;
        back.pwm = 210;
        back.target = 200;
        back.ramping = true;
        back.last_step = shared.clock.now();
    }
    shared.register_isr(7, BASE + 0x100);
    assert!(shared.disable_isr(7));
    shared.note_irq_delivered(7);
    // The callback/ISR increments a distinct real ARM counter and returns.
    for (i, word) in [0xe5941004, 0xe2811001, 0xe5841004, 0xe12fff1e]
        .into_iter()
        .enumerate()
    {
        mem.write_u32(BASE + 0x100 + i as u32 * 4, word).unwrap();
    }
    let service = if power { "SysPowerOff" } else { "OSSleep" };
    let words = [
        0xe92d0001,
        0xe92d4000,
        0xef000000 | names::service_id_by_name(service).unwrap(),
        0xe5941000,
        0xe2811001,
        0xe5841000,
        0xeafffffe,
    ];
    for (i, word) in words.into_iter().enumerate() {
        mem.write_u32(BASE + i as u32 * 4, word).unwrap();
    }
    let reg = registry();
    let log = trace();
    let state = shared.clone();
    let memory = mem.clone();
    let worker = std::thread::spawn(move || {
        let mut cpu = CpuThread::new(
            memory,
            reg,
            state.clone(),
            log,
            UnsupportedPolicy::Stop,
            0,
            vec![],
        )
        .unwrap();
        cpu.set_entry(BASE, BASE + 0xf000, 800).unwrap();
        cpu.set_reg(RegisterARM::LR, BASE + 12).unwrap();
        cpu.set_reg(RegisterARM::R4, COUNT).unwrap();
        while !state.stopping() {
            cpu.run_slice(16).unwrap();
        }
    });
    let end = Instant::now() + Duration::from_secs(5);
    while shared.pending_services.lock().unwrap().is_empty() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    if !power {
        assert!(shared.scheduler.set_priority(0, 3));
    }
    let pause = shared.safe_points.pause(Duration::from_secs(5)).unwrap();
    let gate = mem.snapshot_gate.try_snapshot().unwrap();
    let image = KernelImage::capture_while_guarded(&shared, &mem, &pause, &gate, ID).unwrap();
    std::fs::write(path.join("kernel.bin"), image.to_bytes().unwrap()).unwrap();
    MemoryImage::capture_while_guarded(&mem, &gate, ID)
        .unwrap()
        .save(&path.join("memory.bin"))
        .unwrap();
    StorageImage::capture(&data, &path.join("storage.bin"), ID).unwrap();
    assert_eq!(mem.read_u32(COUNT).unwrap(), 0);
    assert!(pause.is_current());
    drop(gate);
    drop(pause);
    shared.set_stopping();
    worker.join().unwrap();
    // Downtime exceeds the saved guest deadline. It must not complete the wait.
    std::thread::sleep(Duration::from_millis(850));
}
#[test]
fn kernel_and_real_arm_waits_resume_after_fresh_process_reconstruction() {
    for power in [false, true] {
        let root = tempfile::tempdir().unwrap();
        make(root.path(), power);
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "restored_kernel_child", "--nocapture"])
            .env("PRIME_KERNEL_RESTORE_CHILD", root.path())
            .env("PRIME_KERNEL_POWER", if power { "1" } else { "0" })
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "power={power}: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("kernel-arm-resumed-exactly-once")
        );
    }
}
#[test]
fn restored_kernel_child() {
    let Some(path) = std::env::var_os("PRIME_KERNEL_RESTORE_CHILD") else {
        return;
    };
    let path = Path::new(&path);
    let power = std::env::var("PRIME_KERNEL_POWER").unwrap() == "1";
    let mem = Arc::new(
        MemoryImage::load(&path.join("memory.bin"), ID)
            .unwrap()
            .restore(ID)
            .unwrap(),
    );
    let storage = StorageImage::open(&path.join("storage.bin"), ID)
        .unwrap()
        .restore(path)
        .unwrap();
    let bytes = std::fs::read(path.join("kernel.bin")).unwrap();
    let image = KernelImage::from_bytes(&bytes).unwrap();
    assert!(
        image
            .restore(mem.clone(), storage.path().to_owned(), "wrong-assets")
            .is_err()
    );
    let mut corrupted = bytes.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    assert!(KernelImage::from_bytes(&corrupted).is_err());
    let restored = image
        .restore(mem.clone(), storage.path().to_owned(), ID)
        .unwrap();
    let shared = Arc::new(restored.shared);
    assert_eq!(
        shared.scheduler.priority(0),
        Some(if power { 16 } else { 3 })
    );
    assert_eq!(shared.device_name(1).as_deref(), Some(r"\\?\USB"));
    assert_eq!(shared.register_device(r"\\?\BAT"), 2);
    assert_eq!(shared.indicator.lock().unwrap().raw, 5);
    assert_eq!(shared.usb.read(false), Some(vec![0x22, 0x33]));
    assert_eq!(shared.usb.read(false), None);
    let delivery = shared.usb.take_delivery().unwrap();
    assert_eq!(delivery.bytes, vec![0x41, 0, 0xff]);
    assert_eq!(delivery.callback, BASE + 0x100);
    assert!(shared.usb.take_delivery().is_none());
    assert!(shared.take_due_isrs().is_empty());
    assert_eq!(shared.irq_delivery_counts(), vec![(7, 1)]);
    assert!(shared.enable_isr(7));
    assert_eq!(shared.take_due_isrs(), vec![(7, BASE + 0x100)]);
    assert_eq!(shared.vfs.tell(FILE), Some(6));
    let mut text = [0; 6];
    assert_eq!(shared.vfs.read(FILE, &mut text), Some(6));
    assert_eq!(&text, b"second");
    assert_eq!(shared.vfs.write(FILE, b"!"), Some(1));
    assert_eq!(
        std::fs::read(storage.path().join("C/working")).unwrap(),
        b"first-second!"
    );
    let mut cpu = CpuThread::new(
        mem.clone(),
        registry(),
        shared.clone(),
        trace(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.restore_execution_image(restored.owners[&0].clone(), &restored.synchronization)
        .unwrap();
    drop(restored.startup);
    cpu.run_slice(16).unwrap();
    assert_eq!(mem.read_u32(COUNT).unwrap(), 0);
    if power {
        assert!(shared.firmware_sleeping());
        let before = shared.clock.now();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(shared.clock.now(), before);
        shared.push_key_press(prime_emulator::services::ON_KEY_CODE);
    } else {
        let remaining = cpu
            .pending_service_image()
            .unwrap()
            .remaining_timeout_ns
            .unwrap();
        std::thread::sleep(Duration::from_nanos(remaining) + Duration::from_millis(5));
    }
    cpu.run_slice(32).unwrap();
    assert_eq!(mem.read_u32(COUNT).unwrap(), 1);
    assert!(cpu.pending_service_image().is_none());
    cpu.run_isr(BASE + 0x100, 7).unwrap();
    assert_eq!(mem.read_u32(COUNT + 4).unwrap(), 1);
    shared.note_irq_delivered(7);
    assert_eq!(shared.irq_delivery_counts(), vec![(7, 2)]);
    cpu.run_slice(32).unwrap();
    assert_eq!(mem.read_u32(COUNT).unwrap(), 1);
    assert_eq!(mem.read_u32(COUNT + 4).unwrap(), 1);
    assert_eq!(shared.backlight.lock().unwrap().target, 200);
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(shared.backlight_output().duty, 200);
    println!("kernel-arm-resumed-exactly-once power={power}");
}

#[test]
fn an_irq_paused_at_entry_survives_kernel_restore_and_executes_once() {
    use std::sync::mpsc;
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(data.path().join("C")).unwrap();
    let mut mem = GuestMemory::new();
    mem.map("irq-control", BASE, 65536).unwrap();
    mem.map("heap", 0x20000000, 65536).unwrap();
    mem.set_heap(0x20000000, 65536);
    let mem = Arc::new(mem);
    for (i, word) in [0xe5941000u32, 0xe2811001, 0xe5841000, 0xe12fff1e]
        .into_iter()
        .enumerate()
    {
        mem.write_u32(BASE + 0x100 + i as u32 * 4, word).unwrap();
    }
    let shared = Arc::new(SharedState::new(
        data.path().into(),
        "A:/irq-control".into(),
    ));
    shared.register_isr(7, BASE + 0x100);
    shared.queue_due_isrs();
    shared.queue_due_isrs();
    assert_eq!(shared.pending_isrs(), [(7, BASE + 0x100)]);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel();
    let state = shared.clone();
    let ram = mem.clone();
    let worker = std::thread::spawn(move || {
        let mut cpu = CpuThread::new(
            ram,
            registry(),
            state,
            trace(),
            UnsupportedPolicy::Stop,
            0,
            vec![],
        )
        .unwrap();
        cpu.set_entry(BASE, BASE + 0xf000, 0).unwrap();
        cpu.set_reg(RegisterARM::R4, COUNT).unwrap();
        ready_tx.send(()).unwrap();
        go_rx.recv().unwrap();
        cpu.dispatch_pending_isrs();
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let points = shared.safe_points.clone();
    let pause_worker = std::thread::spawn(move || points.pause(Duration::from_secs(5)));
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.safe_points.requested_epoch().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    go_tx.send(()).unwrap();
    let pause = pause_worker.join().unwrap().unwrap();
    let gate = mem.snapshot_gate.try_snapshot().unwrap();
    let kernel = KernelImage::capture_while_guarded(&shared, &mem, &pause, &gate, ID).unwrap();
    let kernel = KernelImage::from_bytes(&kernel.to_bytes().unwrap()).unwrap();
    let ram = MemoryImage::capture_while_guarded(&mem, &gate, ID).unwrap();
    assert_eq!(mem.read_u32(COUNT).unwrap(), 0);
    assert_eq!(shared.pending_isrs(), [(7, BASE + 0x100)]);
    drop(gate);
    drop(pause);
    worker.join().unwrap();
    assert_eq!(mem.read_u32(COUNT).unwrap(), 1);
    assert!(shared.pending_isrs().is_empty());
    assert_eq!(shared.irq_delivery_counts(), [(7, 1)]);

    let ram = Arc::new(ram.restore(ID).unwrap());
    let restored = kernel.restore(ram.clone(), data.path().into(), ID).unwrap();
    let shared = Arc::new(restored.shared);
    let mut cpu = CpuThread::new(
        ram.clone(),
        registry(),
        shared.clone(),
        trace(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    cpu.restore_execution_image(
        restored.owners.get(&0).unwrap().clone(),
        &restored.synchronization,
    )
    .unwrap();
    drop(restored.startup);
    cpu.dispatch_pending_isrs();
    cpu.dispatch_pending_isrs();
    assert_eq!(ram.read_u32(COUNT).unwrap(), 1);
    assert!(shared.pending_isrs().is_empty());
    assert_eq!(shared.irq_delivery_counts(), [(7, 1)]);
}
