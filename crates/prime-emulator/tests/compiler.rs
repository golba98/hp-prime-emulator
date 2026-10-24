//! Synthetic guest/report tests. These establish observer safety, never
//! claim any HP compiler behavior; firmware_compiler.rs tests that separately.
use std::sync::Arc;

use prime_emulator::checkprog::{Diagnostic, Status, outcome_status, validate};
use prime_emulator::compiler::*;
use prime_emulator::engine::CpuThread;
use prime_emulator::memory::GuestMemory;
use prime_emulator::runner::Outcome;
use prime_emulator::services::{Registry, SharedState, UnsupportedPolicy};
use prime_emulator::sha256;
use prime_emulator::trace::{TraceConfig, TraceHub};
use unicorn_engine::RegisterARM;

fn observation(source: &str) -> CompilerTrace {
    CompilerTrace {
        checks: vec![CheckObservation {
            step: Some(3),
            compiler_entered: true,
            program_va: Some(0x2000),
            result_va: Some(0x3000),
            sources: vec![ObservedSource {
                source_index: 0,
                address: 0x4000,
                sha256: sha256::hex(source.as_bytes()),
                utf16_units: source.encode_utf16().count(),
            }],
            parser_returns: vec![0],
            return_value: Some(1),
            message: Some("synthetic success".into()),
            dialog_resource: Some(0x410),
            started_ms: 10.0,
            returned_ms: Some(11.0),
            message_ms: Some(12.0),
            ..Default::default()
        }],
        dropped: 0,
    }
}

#[test]
fn fresh_guest_result_is_required_for_exact_source_and_step() {
    let trace = observation("source");
    assert!(validate(&trace, "source", 3).is_ok());
    assert!(validate(&trace, "source2", 3).is_err());
    assert!(validate(&trace, "source", 2).is_err());
    assert!(validate(&CompilerTrace::default(), "source", 3).is_err());
    let mut trace = trace;
    trace.checks.push(trace.checks[0].clone());
    assert!(validate(&trace, "source", 3).is_err());
}

#[test]
fn incomplete_corrupt_or_dropped_results_cannot_be_valid() {
    for change in 0..13 {
        let mut trace = observation("source");
        let c = &mut trace.checks[0];
        match change {
            0 => c.return_value = None,
            1 => c.return_value = Some(42),
            2 => c.message = None,
            3 => c.compiler_entered = false,
            4 => c.errors.push("bad pointer".into()),
            5 => c.cpsr = 0x20,
            6 => c.returned_ms = None,
            7 => c.message_ms = Some(5.0),
            8 => c.parser_returns.clear(),
            9 => c.dialog_resource = Some(0x3416),
            10 => c.program_va = None,
            11 => c.sources[0].utf16_units += 1,
            _ => trace.dropped = 1,
        }
        assert!(validate(&trace, "source", 3).is_err(), "change {change}");
    }
}

#[test]
fn invalid_source_is_distinct_from_infrastructure_and_timeout() {
    assert_eq!(outcome_status(Outcome::ScriptComplete), None);
    assert_eq!(
        outcome_status(Outcome::Fault),
        Some(Status::InfrastructureError)
    );
    assert_eq!(
        outcome_status(Outcome::UnsupportedStop),
        Some(Status::InfrastructureError)
    );
    assert_eq!(outcome_status(Outcome::ScriptFailed), Some(Status::Timeout));
    assert_eq!(outcome_status(Outcome::LimitReached), Some(Status::Timeout));
    assert_eq!(
        outcome_status(Outcome::Heartbeat),
        Some(Status::Unsupported)
    );
    for (s, code) in [
        (Status::Valid, 0),
        (Status::Invalid, 7),
        (Status::InfrastructureError, 4),
        (Status::Timeout, 3),
        (Status::Unsupported, 8),
    ] {
        assert_eq!(s.exit_code(), code);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Status>(&json).unwrap(), s);
    }
    let mut trace = observation("bad");
    let c = &mut trace.checks[0];
    c.return_value = Some(0);
    c.parser_returns = vec![22];
    c.dialog_resource = Some(0x3416);
    c.raw_position = Some(RawPosition {
        source_index: 0,
        line: 1,
        offset_utf16: 2,
    });
    assert!(validate(&trace, "bad", 3).is_ok());
}

#[test]
fn utf16_positions_preserve_raw_units_and_map_scalar_columns() {
    let src = "\r\n\n// é😀\nA := ;";
    let raw = RawPosition {
        source_index: 0,
        line: 4,
        offset_utf16: src[..src.find(';').unwrap()].encode_utf16().count() as i32,
    };
    assert_eq!(position(src, &raw), Some((4, 6)));
    let interior = RawPosition {
        source_index: 0,
        line: 3,
        offset_utf16: 8,
    };
    assert_eq!(position(src, &interior), None, "inside the surrogate pair");
    for raw in [
        RawPosition {
            source_index: 1,
            ..raw.clone()
        },
        RawPosition {
            line: 2,
            ..raw.clone()
        },
        RawPosition {
            offset_utf16: -1,
            ..raw.clone()
        },
        RawPosition {
            offset_utf16: 999,
            ..raw
        },
    ] {
        assert_eq!(position(src, &raw), None);
    }
}

#[test]
fn diagnostics_round_trip_with_missing_and_multiple_fields() {
    let d = Diagnostic {
        severity: "error".into(),
        code: None,
        message: None,
        line: None,
        column: None,
        raw_position: None,
        source: "guest-firmware".into(),
    };
    for list in [vec![], vec![d.clone()], vec![d.clone(), d]] {
        let json = serde_json::to_value(&list).unwrap();
        let decoded: Vec<Diagnostic> = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), json);
    }
}

#[test]
fn strict_guest_strings_handle_surrogates_and_reject_invalid_buffers() {
    let mut mem = GuestMemory::new();
    mem.map("test", 0x1000, 0x1000).unwrap();
    let s = "é😀";
    let bytes: Vec<_> = s
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    mem.write_bytes(0x1000, &bytes).unwrap();
    assert_eq!(read_utf16(&mem, 0x1000, 4).unwrap(), s);
    assert!(read_utf16(&mem, 0x1000, 3).is_err());
    assert!(read_utf16(&mem, 0, 4).is_err());
    assert!(read_utf16(&mem, 0x1001, 4).is_err());
    assert!(read_utf16(&mem, 0x3000, 4).is_err());
    mem.write_u16(0x1ffe, 0xd800).unwrap();
    assert!(read_utf16(&mem, 0x1ffe, 2).is_err());
    mem.write_u16(0x1000, 0xd800).unwrap();
    mem.write_u16(0x1002, 0).unwrap();
    assert!(read_utf16(&mem, 0x1000, 4).is_err());
}

#[test]
fn actual_guest_hooks_capture_one_invocation_and_do_not_reuse_it() {
    let mut mem = GuestMemory::new();
    mem.map("synthetic-code", 0x3060_0000, 0x30_0000).unwrap();
    mem.map("heap", 0x2000_0000, 0x10000).unwrap();
    mem.set_heap(0x2000_0000, 0x10000);
    for pc in [
        CHECK_ENTRY,
        COMPILE_ENTRY,
        SOURCE_CALL,
        PARSE_RETURN,
        CHECK_RETURN,
        DIALOG_ENTRY,
        DIALOG_TEXT,
    ] {
        mem.write_u32(pc, 0xe1a00000).unwrap(); // ARM nop; the real firmware is not present.
    }
    let source = "synthetic é😀";
    let text: Vec<_> = source
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    mem.write_bytes(0x20001000, &text).unwrap();
    let msg: Vec<_> = "synthetic failure"
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    mem.write_bytes(0x20002000, &msg).unwrap();
    for (i, x) in [0, 1, 3].iter().enumerate() {
        mem.write_u32(0x20007000 + i as u32 * 4, *x).unwrap();
    }
    let mem = Arc::new(mem);
    let trace = Arc::new(
        TraceHub::new(TraceConfig {
            compiler: true,
            verbosity: 0,
            ..Default::default()
        })
        .unwrap(),
    );
    trace.set_script_step(Some(3));
    let mut cpu = CpuThread::new(
        mem,
        Arc::new(Registry::new()),
        Arc::new(SharedState::new(
            "/tmp/unused-synthetic".into(),
            "A:\\test".into(),
        )),
        trace.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    let execute = |cpu: &mut CpuThread, pc, arg| {
        cpu.set_entry(pc, 0x20007000, arg).unwrap();
        cpu.run_slice(1).unwrap();
    };
    execute(&mut cpu, CHECK_ENTRY, 0);
    cpu.set_lr(CHECK_RETURN).unwrap();
    cpu.set_reg(RegisterARM::R1, 1).unwrap();
    cpu.set_reg(RegisterARM::R2, 0).unwrap();
    cpu.set_reg(RegisterARM::R3, 0x20007000).unwrap();
    execute(&mut cpu, COMPILE_ENTRY, 0x20003000);
    cpu.set_reg(RegisterARM::R3, 0).unwrap();
    execute(&mut cpu, SOURCE_CALL, 0x20001000);
    execute(&mut cpu, PARSE_RETURN, 22);
    execute(&mut cpu, CHECK_RETURN, 0);
    cpu.set_lr(0x308f2cf4).unwrap();
    execute(&mut cpu, DIALOG_ENTRY, 0x3416);
    cpu.set_reg(RegisterARM::R4, 0x3416).unwrap();
    execute(&mut cpu, DIALOG_TEXT, 0x20002000);
    let first = trace.compiler.as_ref().unwrap().snapshot();
    assert_eq!(
        validate(&first, source, 3).unwrap().message.as_deref(),
        Some("synthetic failure")
    );
    assert_eq!(first.checks[0].parser_returns, vec![22]);
    execute(&mut cpu, PARSE_RETURN, 99); // an unrelated later operation cannot alter the result
    assert_eq!(
        trace.compiler.as_ref().unwrap().snapshot().checks[0].parser_returns,
        vec![22]
    );
    trace.set_script_step(Some(8));
    execute(&mut cpu, CHECK_ENTRY, 0);
    let second = trace.compiler.as_ref().unwrap().snapshot();
    assert!(
        validate(&second, source, 8).is_err(),
        "a fresh entry has no inherited result"
    );
    assert!(validate(&second, source, 3).is_ok());
    assert_eq!(second.checks[1].sequence, 1);
}
