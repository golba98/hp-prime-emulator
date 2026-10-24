//! Framebuffer capture and input→guest→screen synchronisation without the
//! proprietary firmware: a synthetic guest gets the LCD object from
//! `GetActiveLCD`, waits in `GetEvent` and draws one pixel per key batch,
//! exactly the path the real firmware's UI takes (M4).

use std::sync::Arc;
use std::time::{Duration, Instant};

use prime_emulator::display::{DisplayError, LcdView, PixelFormat};
use prime_emulator::engine::CpuThread;
use prime_emulator::memory::GuestMemory;
use prime_emulator::services::{
    Registry, SharedState, TouchSample, UI_EVENT_TYPE_DEVICE_TIMER, UI_EVENT_TYPE_SYS_TIMER,
    UI_EVENT_TYPE_TOUCH_BEGIN, UnsupportedPolicy, handlers,
};
use prime_emulator::trace::{TraceConfig, TraceHub};

const CODE: u32 = 0x1000_0000;
const HEAP: u32 = 0x2000_0000;

/// Assembled with clang (armv5te) from:
///
/// ```text
/// _start: bl   get_active_lcd
///         add  r4, r0, #616          @ framebuffer (window1_bufferstart)
///         ldr  r5, =0x10000400       @ event struct
///         mov  r6, #0
/// loop:   mov  r0, r5
///         bl   get_event
///         ldr  r1, [r5, #4]          @ event type
///         ldr  r2, =0x100010         @ KEY_BATCH
///         cmp  r1, r2
///         bne  loop                  @ timers/touch: nothing to draw
///         ldrh r1, [r5, #34]         @ first entry's key code (28 + 6)
///         orr  r1, r1, #0x00FF0000
///         str  r1, [r4, r6, lsl #2]  @ fb[r6] = 0x00FF00kk
///         add  r6, r6, #1
///         b    loop
/// get_active_lcd: push {r0}; push {lr}; svc #0x1008D; pop {pc}
/// get_event:      push {r0}; push {lr}; svc #0x1003F; pop {pc}
/// ```
///
/// The two `bl` offsets are resolved by hand (no linker); the stubs use the
/// firmware's verified `stmdb`/`ldmia` encodings.
const GUEST: [u32; 25] = [
    0xEB00_000D,
    0xE280_4F9A,
    0xE59F_504C,
    0xE3A0_6000,
    0xE1A0_0005,
    0xEB00_000C,
    0xE595_1004,
    0xE59F_203C,
    0xE151_0002,
    0x1AFF_FFF9,
    0xE1D5_12B2,
    0xE381_18FF,
    0xE784_1106,
    0xE286_6001,
    0xEAFF_FFF4,
    0xE92D_0001,
    0xE92D_4000,
    0xEF01_008D,
    0xE8BD_8000,
    0xE92D_0001,
    0xE92D_4000,
    0xEF01_003F,
    0xE8BD_8000,
    0x1000_0400,
    0x0010_0010,
];

fn setup() -> (Arc<GuestMemory>, Arc<SharedState>, CpuThread) {
    let mut mem = GuestMemory::new();
    mem.map("code", CODE, 0x1_0000).unwrap();
    mem.map("heap", HEAP, 0x10_0000).unwrap();
    mem.set_heap(HEAP, 0x10_0000);
    let bytes: Vec<u8> = GUEST.iter().flat_map(|w| w.to_le_bytes()).collect();
    mem.write_bytes(CODE, &bytes).unwrap();
    let mem = Arc::new(mem);
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    let shared = Arc::new(SharedState::new(
        "/tmp/prime-emulator-test".into(),
        "A:\\t".into(),
    ));
    let mut cpu = CpuThread::new(
        mem.clone(),
        Arc::new(registry),
        shared.clone(),
        Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
        UnsupportedPolicy::Stop,
        1,
        Vec::new(),
    )
    .unwrap();
    cpu.set_entry(CODE, CODE + 0x8000, 0).unwrap();
    (mem, shared, cpu)
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn keys_reach_the_guest_and_its_pixels_are_captured() {
    let (mem, shared, mut cpu) = setup();
    let (m2, s2) = (mem.clone(), shared.clone());
    // The "device": sends each input once the guest is parked in GetEvent
    // again (guest-observable synchronisation), then captures.
    let device = std::thread::spawn(move || {
        let idle =
            || s2.getevent_parked() && s2.input_events_pending() == 0 && s2.touch_pending() == 0;
        wait_for("boot to GetEvent", idle);
        let lcd = *s2.lcd_object.lock().unwrap();
        let view = LcdView::locate(&m2, lcd).expect("LCD object");
        let initial = view.capture(&m2).unwrap();
        for code in [0x5A, 0xB9, 0x5A] {
            s2.push_key_press(code);
            wait_for("key delivery", || s2.input_events_pending() == 0);
            // Non-key events in between: the guest must ignore them.
            s2.push_system_event(UI_EVENT_TYPE_SYS_TIMER);
            s2.push_system_event(UI_EVENT_TYPE_DEVICE_TIMER);
            s2.push_touch(TouchSample {
                status: UI_EVENT_TYPE_TOUCH_BEGIN,
                x: 10,
                y: 10,
            });
            wait_for("guest idle", idle);
        }
        let after = view.capture(&m2).unwrap();
        s2.set_stopping();
        (view, initial, after)
    });
    // The guest runs on this thread until the device stops the session.
    while !shared.stopping() {
        let _ = cpu.run_slice(100_000).unwrap();
    }
    let (view, initial, after) = device.join().unwrap();

    assert_eq!(view.fb_va, view.lcd_va + 616);
    assert_eq!((view.width, view.height, view.stride), (320, 240, 1280));
    assert_eq!(view.format, PixelFormat::Xrgb8888);
    assert_eq!(initial.unique_colors(), 1, "cleared to black by the OS");
    // One pixel per key batch, in order, with the key code.
    assert_eq!(after.pixel(0, 0), 0x00FF_005A);
    assert_eq!(after.pixel(1, 0), 0x00FF_00B9);
    assert_eq!(after.pixel(2, 0), 0x00FF_005A);
    assert_eq!(after.pixel(3, 0), 0, "timers and touch drew nothing");
    let d = after.diff(&initial, None);
    assert_eq!((d.changed, d.bbox), (3, Some([0, 0, 2, 0])));
    assert_eq!(shared.input_delivered(), 3);
    assert_eq!(shared.input_thread(), 1);
    assert!(shared.system_events_delivered() >= 6);
    assert_eq!(shared.touch_delivered(), 3);
    // RGBA conversion and PNG output of the captured guest pixels.
    assert_eq!(&after.to_rgba()[..4], &[0xFF, 0x00, 0x5A, 0xFF]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screen.png");
    after.save_png(&path).unwrap();
    assert_eq!(prime_emulator::verify::load_png(&path).unwrap(), after);
}

#[test]
fn lcd_layout_is_validated_before_capture() {
    let (mem, shared, mut cpu) = setup();
    // No GetActiveLCD yet: nothing to capture.
    assert_eq!(
        LcdView::locate(&mem, None),
        Err(DisplayError::NotInitialized)
    );
    // Let the guest create the object and park in GetEvent.
    let s2 = shared.clone();
    let stopper = std::thread::spawn(move || {
        wait_for("GetEvent", || s2.getevent_parked());
        s2.set_stopping();
    });
    while !shared.stopping() {
        let _ = cpu.run_slice(100_000).unwrap();
    }
    stopper.join().unwrap();
    let lcd = shared.lcd_object.lock().unwrap().expect("object created");
    let view = LcdView::locate(&mem, Some(lcd)).unwrap();
    let frame = view.capture(&mem).unwrap();
    assert_eq!(frame.pixels.len(), 320 * 240);
    // Capture is read-only: the guest buffer is unchanged by it.
    assert_eq!(view.capture(&mem).unwrap(), frame);

    // A pointer that is not an LCD object.
    assert!(matches!(
        LcdView::locate(&mem, Some(HEAP + 0x8_0000)),
        Err(DisplayError::InvalidLayout(_))
    ));
    // Corrupted fields are rejected, each with its own reason.
    let corrupt = |off: u32, value: u32| {
        let old = mem.read_u32(lcd + off).unwrap();
        mem.write_u32(lcd + off, value).unwrap();
        let r = LcdView::locate(&mem, Some(lcd));
        mem.write_u32(lcd + off, old).unwrap();
        r
    };
    for (off, value, why) in [
        (0, 0x1234, "LCDMagicPtr"),
        (12, 0, "itself"),
        (592 + 16, HEAP, "window1_bufferstart"),
        (8, 16, "bufferSize"),
    ] {
        match corrupt(off, value) {
            Err(DisplayError::InvalidLayout(m)) => assert!(m.contains(why), "{m}"),
            other => panic!("{why}: {other:?}"),
        }
    }
    // Wrong pixel depth (LCD_MAGIC.pixel_bits, a u16 at +592+6).
    mem.write_u16(lcd + 592 + 6, 16).unwrap();
    assert!(matches!(
        LcdView::locate(&mem, Some(lcd)),
        Err(DisplayError::InvalidLayout(m)) if m.contains("pixel_bits")
    ));
}

/// Measurements for the M4 report: `scripts/emulator/run.sh test --release
/// -p prime-emulator --test display -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_capture_and_encoding() {
    let (mem, shared, mut cpu) = setup();
    let s2 = shared.clone();
    let stopper = std::thread::spawn(move || {
        wait_for("GetEvent", || s2.getevent_parked());
        s2.set_stopping();
    });
    while !shared.stopping() {
        let _ = cpu.run_slice(100_000).unwrap();
    }
    stopper.join().unwrap();
    let view = LcdView::locate(&mem, *shared.lcd_object.lock().unwrap()).unwrap();
    // Firmware-like content (a few hundred colours) in the guest buffer.
    let pixels: Vec<u8> = (0..320 * 240u32)
        .flat_map(|i| (0x00F2_F2F2 ^ ((i / 7 % 640) * 0x0001_0203)).to_le_bytes())
        .collect();
    mem.write_bytes(view.fb_va, &pixels).unwrap();
    let time = |what: &str, n: u32, f: &mut dyn FnMut()| {
        let t = Instant::now();
        for _ in 0..n {
            f();
        }
        println!(
            "{what}: {:.3} ms",
            t.elapsed().as_secs_f64() * 1e3 / f64::from(n)
        );
    };
    let frame = view.capture(&mem).unwrap();
    println!("unique colours: {}", frame.unique_colors());
    time("capture (guest read + decode)", 200, &mut || {
        let _ = view.capture(&mem).unwrap();
    });
    time("fingerprint (settle tracking)", 200, &mut || {
        let _ = frame.fingerprint();
    });
    time("sha256", 50, &mut || {
        let _ = frame.sha256();
    });
    time("PNG encode", 50, &mut || {
        let _ = frame.encode_png().unwrap();
    });
    println!("PNG size: {} bytes", frame.encode_png().unwrap().len());
}
