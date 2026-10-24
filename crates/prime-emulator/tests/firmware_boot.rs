//! Optional real-firmware integration test.
//!
//! Requires the proprietary HP firmware, which is never bundled: set
//! `PRIME_TEST_FIRMWARE` to an armfir.elf path (and optionally
//! `PRIME_TEST_OS_ROM` to PRIME_OS.ROM). Skips otherwise.

use prime_emulator::runner::{self, Outcome, RunConfig};
use prime_emulator::services::UnsupportedPolicy;
use prime_emulator::trace::TraceConfig;

fn firmware_path() -> Option<std::path::PathBuf> {
    match std::env::var("PRIME_TEST_FIRMWARE") {
        Ok(p) if !p.is_empty() => {
            let path = std::path::PathBuf::from(p);
            path.is_file().then_some(path)
        }
        _ => None,
    }
}

#[test]
fn real_firmware_executes_and_reports_progress() {
    let Some(firmware) = firmware_path() else {
        eprintln!("skipping: PRIME_TEST_FIRMWARE not set or missing");
        return;
    };
    let os_rom = std::env::var("PRIME_TEST_OS_ROM")
        .ok()
        .map(std::path::PathBuf::from);
    let temporary = tempfile::tempdir().expect("isolated boot storage");
    let data_dir = std::env::var("PRIME_TEST_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| temporary.path().join("prime_data"));
    let _ = std::fs::create_dir_all(&data_dir);

    let cfg = RunConfig {
        firmware_path: firmware,
        os_rom_path: os_rom,
        data_dir,
        firmware_guest_path: "A:\\programs\\misc\\armfir.elf".to_owned(),
        slice_instructions: 200_000,
        max_instructions: 100_000_000,
        max_seconds: 240.0,
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        heartbeat_min_events: 3,
        // M3: headless input source — one ON-key press every 250 ms with
        // the verified device keycode semantics (0x83 down+up pair).
        inject_key: Some((0x83, 250)),
        irq_tick_ms: 10,
        watches: Vec::new(),
        trace: TraceConfig {
            verbosity: 1,
            ..TraceConfig::default()
        },
        // M4 fields (scripted input, framebuffer capture) stay off in the
        // M2/M3 boot test; defaults cover them.
        ..RunConfig::default()
    };
    let summary = runner::run(&cfg).expect("run must produce a report");
    let r = &summary.report;

    // Hard evidence gates: the firmware must genuinely execute.
    assert!(
        r.firmware_sha256.is_some(),
        "firmware identity (sha256) must be recorded"
    );
    assert_eq!(r.entry, 0x3060_4AE8, "verified entry point");
    assert_eq!(r.segments.len(), 1, "single PT_LOAD expected");
    assert_eq!(r.segments[0].vaddr, 0x3060_0000);
    assert!(
        r.instructions >= 5_000_000,
        "firmware must execute millions of instructions, got {}",
        r.instructions
    );
    assert!(
        r.svc_calls_total > 1000,
        "many OS services must be dispatched, got {}",
        r.svc_calls_total
    );
    // The verified boot path always passes through these services.
    for required in [0x10119u32, 0x10037] {
        assert!(
            r.used_services.iter().any(|u| u.id == required),
            "expected service {required:#X} in the used set"
        );
    }
    // Virtual IRQ must have been delivered (timer ISR registered + ticked).
    assert!(
        !r.irq_delivered.is_empty(),
        "registered guest ISRs must receive ticks"
    );
    println!(
        "outcome={} instructions={} svc={} irqs={:?} heartbeat={:?}",
        r.outcome, r.instructions, r.svc_calls_total, r.irq_delivered, r.heartbeat
    );
    // M3: the GetEvent pump round-trip is now asserted. Evidence: the
    // pump call site (0x306070B4), the servicing thread (guest thread 1),
    // delivered events, and re-entry count must all be present.
    assert_eq!(
        summary.outcome,
        Outcome::Heartbeat,
        "M3 requires the verified GetEvent round-trip"
    );
    let hb = r.heartbeat.as_ref().expect("heartbeat evidence");
    assert_eq!(hb.caller_pc, 0x3060_70B4, "the pump's GetEvent call site");
    assert_eq!(hb.thread_id, 1, "guest thread 1 services the pump");
    assert!(hb.events_delivered >= 3);
    assert!(hb.pump_reentries >= hb.events_delivered as u64);
}
