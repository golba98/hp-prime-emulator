//! Execution must resume in the instruction set it stopped in.
//!
//! Unicorn's ARM `emu_start(begin)` takes the Thumb state from bit 0 of
//! `begin`. Resuming a slice (or returning from a virtual IRQ) at the even
//! PC switched Thumb code to ARM decoding: on the real firmware this
//! surfaced as `UC_ERR_INSN_INVALID` at `0x307FB134` (Thumb `movs r0,#1`
//! executed with CPSR.T clear) once boot reached Thumb-heavy code (M4).

use std::sync::Arc;

use prime_emulator::engine::{CpuThread, RunSliceOutcome};
use prime_emulator::memory::GuestMemory;
use prime_emulator::services::{Registry, SharedState, UnsupportedPolicy, handlers};
use prime_emulator::trace::{TraceConfig, TraceHub};
use unicorn_engine::RegisterARM;

fn cpu_with(code: &[u8]) -> CpuThread {
    let mut mem = GuestMemory::new();
    mem.map("code", 0x1000_0000, 0x1000).unwrap();
    mem.map("heap", 0x2000_0000, 0x10_000).unwrap();
    mem.set_heap(0x2000_0000, 0x10_000);
    mem.write_bytes(0x1000_0000, code).unwrap();
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    CpuThread::new(
        Arc::new(mem),
        Arc::new(registry),
        Arc::new(SharedState::new(
            std::path::PathBuf::from("/tmp/prime-emulator-test"),
            "A:\\test".to_owned(),
        )),
        Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
        UnsupportedPolicy::Stop,
        0,
        Vec::new(),
    )
    .unwrap()
}

/// Thumb: movs r0,#0; adds r0,#1 ×8; b .
fn thumb_counter() -> Vec<u8> {
    let mut code = vec![0x00, 0x20];
    for _ in 0..8 {
        code.extend([0x01, 0x30]);
    }
    code.extend([0xFE, 0xE7]);
    code
}

#[test]
fn thumb_code_survives_single_instruction_slices() {
    let mut cpu = cpu_with(&thumb_counter());
    // Bit 0 of the entry selects Thumb.
    cpu.set_entry(0x1000_0001, 0x2000_8000, 0).unwrap();
    for i in 0..9 {
        match cpu.run_slice(1).unwrap() {
            RunSliceOutcome::Progress => {}
            RunSliceOutcome::Stopped(r) => panic!("slice {i} stopped: {r:?}"),
        }
        let cpsr = cpu.reg(RegisterARM::CPSR);
        assert_ne!(cpsr & (1 << 5), 0, "slice {i} left Thumb state");
    }
    assert_eq!(cpu.reg(RegisterARM::R0), 8, "every adds ran as Thumb");
    assert_eq!(cpu.pc(), 0x1000_0012, "parked on `b .`");
}

#[test]
fn arm_code_still_resumes_as_arm() {
    // ARM: mov r0,#0; add r0,r0,#1 ×4; b .
    let mut code: Vec<u8> = vec![0x00, 0x00, 0xA0, 0xE3];
    for _ in 0..4 {
        code.extend([0x01, 0x00, 0x80, 0xE2]);
    }
    code.extend([0xFE, 0xFF, 0xFF, 0xEA]);
    let mut cpu = cpu_with(&code);
    cpu.set_entry(0x1000_0000, 0x2000_8000, 0).unwrap();
    for _ in 0..5 {
        assert!(matches!(
            cpu.run_slice(1).unwrap(),
            RunSliceOutcome::Progress
        ));
        assert_eq!(cpu.reg(RegisterARM::CPSR) & (1 << 5), 0);
    }
    assert_eq!(cpu.reg(RegisterARM::R0), 4);
}

#[test]
fn virtual_irq_returns_to_interrupted_thumb_code() {
    // Thumb counter at 0x1000_0000; an ARM ISR (`bx lr`) at 0x1000_0100.
    let mut code = thumb_counter();
    code.resize(0x100, 0);
    code.extend([0x1E, 0xFF, 0x2F, 0xE1]); // bx lr
    let mut cpu = cpu_with(&code);
    cpu.set_entry(0x1000_0001, 0x2000_8000, 0).unwrap();
    assert!(matches!(
        cpu.run_slice(3).unwrap(),
        RunSliceOutcome::Progress
    ));
    cpu.run_isr(0x1000_0100, 14).unwrap();
    for _ in 0..6 {
        assert!(matches!(
            cpu.run_slice(1).unwrap(),
            RunSliceOutcome::Progress
        ));
    }
    assert_ne!(cpu.reg(RegisterARM::CPSR) & (1 << 5), 0, "still Thumb");
    assert_eq!(cpu.reg(RegisterARM::R0), 8);
}
