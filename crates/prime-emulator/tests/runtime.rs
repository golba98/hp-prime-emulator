//! Representation and lifecycle tests without proprietary firmware.
use prime_emulator::{
    memory::GuestMemory,
    runtime::{ExecutionRequest, ExecutionStatus, Observer},
    runtime_value::{self, FirmwareValue},
};

#[test]
fn lossless_numeric_json_and_validation() {
    let args =
        runtime_value::parse_arguments(r#"[7,-4,0.1,1e400,{"type":"real","value":"-0.0000200"}]"#)
            .unwrap();
    assert_eq!(
        args.iter().map(|a| a.value.as_str()).collect::<Vec<_>>(),
        ["7", "-4", "0.1", "1e400", "-0.00002"]
    );
    for text in [
        "null",
        "{}",
        "[true]",
        "[\"7\"]",
        "[1e500]",
        r#"[{"type":"string","value":"x"}]"#,
        r#"[{"type":"real","value":"1);KILL","extra":1}]"#,
    ] {
        assert!(runtime_value::parse_arguments(text).is_err(), "{text}");
    }
    for n in ["NaN", "inf", "1+2", "1e", "--4", "1."] {
        assert!(runtime_value::canonical_decimal(n).is_err());
    }
    assert_eq!(runtime_value::canonical_decimal("-0e999").unwrap(), "0");
}
fn memory() -> GuestMemory {
    let mut m = GuestMemory::new();
    m.map("values", 0x1000, 0x1000).unwrap();
    m
}
fn real(text: &str) -> Option<FirmwareValue> {
    Some(FirmwareValue::Real { value: text.into() })
}
#[test]
fn decode_exact_guest_rationals_and_fail_closed() {
    let m = memory();
    m.write_u32(0x1000, 0x010f0001).unwrap();
    m.write_u32(0x1004, 0).unwrap();
    m.write_bytes(0x1008, &49u64.to_le_bytes()).unwrap();
    m.write_bytes(0x1010, &1u64.to_le_bytes()).unwrap();
    assert_eq!(runtime_value::decode(&m, 0x1000).value, real("49"));
    m.write_bytes(0x1008, &1u64.to_le_bytes()).unwrap();
    m.write_bytes(0x1010, &10u64.to_le_bytes()).unwrap();
    assert_eq!(runtime_value::decode(&m, 0x1000).value, real("0.1"));
    m.write_bytes(0x1010, &3u64.to_le_bytes()).unwrap();
    assert!(matches!(
        runtime_value::decode(&m, 0x1000).value,
        Some(FirmwareValue::Rational { .. })
    ));
    m.write_bytes(0x1010, &0u64.to_le_bytes()).unwrap();
    assert!(runtime_value::decode(&m, 0x1000).unavailable.is_some());
    for p in [0, 0x1001, 0xfffffffcu32] {
        assert!(runtime_value::decode(&m, p).value.is_none());
    }
    m.write_u32(0x1000, 0x00090001).unwrap();
    assert_eq!(runtime_value::decode(&m, 0x1000).tag, Some(9));
    assert_eq!(runtime_value::decode(&m, 0).tag, None);
    assert!(runtime_value::decode(&m, 0x1000).value.is_none());
}
#[test]
fn decode_bcd_and_utf16_without_lossy_replacement() {
    let m = memory();
    m.write_u32(0x1000, 0x01000001).unwrap();
    m.write_u32(0x1004, 1).unwrap();
    m.write_bytes(0x1008, &0x0490000000000000u64.to_le_bytes())
        .unwrap();
    assert_eq!(runtime_value::decode(&m, 0x1000).value, real("49"));
    m.write_bytes(0x1008, &0x0a90000000000000u64.to_le_bytes())
        .unwrap();
    assert!(runtime_value::decode(&m, 0x1000).value.is_none());
    m.write_u32(0x1000, 0x00020001).unwrap();
    m.write_u32(0x1004, 2).unwrap();
    m.write_bytes(0x1008, &[0x3d, 0xd8, 0x00, 0xde, 0, 0])
        .unwrap();
    assert_eq!(
        runtime_value::decode(&m, 0x1000).value,
        Some(FirmwareValue::String {
            value: "😀".into()
        })
    );
    m.write_bytes(0x1008, &[0x3d, 0xd8, 0, 0, 0, 0]).unwrap();
    assert!(runtime_value::decode(&m, 0x1000).value.is_none());
}
#[test]
fn invocation_selection_and_independent_requests() {
    assert!(runtime_value::invocation_keys("HELLO;KILL", &[]).is_err());
    assert!(runtime_value::invocation_keys("", &[]).is_err());
    let o = Observer::default();
    let a = ExecutionRequest {
        entry: "HELLO".into(),
        arguments: vec![],
    };
    o.arm(0x1234, "hash-one", &a, 1.0).unwrap();
    o.finish_invocation(ExecutionStatus::RuntimeError);
    o.arm(
        0x1234,
        "hash-two",
        &ExecutionRequest {
            entry: "SQUARE".into(),
            arguments: runtime_value::parse_arguments("[7]").unwrap(),
        },
        2.0,
    )
    .unwrap();
    let t = o.snapshot();
    assert_eq!(t.invocations[1].id, 1);
    assert_eq!(t.invocations[1].source_sha256, "hash-two");
    assert!(t.invocations[1].result.is_none());
    assert!(t.invocations[1].output.is_empty());
    assert_eq!(o.progress(), (false, false));
    let json = serde_json::to_string(&t).unwrap();
    let decoded: prime_emulator::runtime::RuntimeTrace = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.invocations.len(), 2);
    assert!(decoded.invocations[1].diagnostic.is_none());
}
#[test]
fn outcomes_do_not_conflate_failures() {
    let statuses = [
        ExecutionStatus::Completed,
        ExecutionStatus::RuntimeError,
        ExecutionStatus::CompileError,
        ExecutionStatus::InvalidEntry,
        ExecutionStatus::InvalidArguments,
        ExecutionStatus::TimedOut,
        ExecutionStatus::GuestExecutionFault,
        ExecutionStatus::InfrastructureError,
        ExecutionStatus::Unsupported,
        ExecutionStatus::Cancelled,
    ];
    let mut codes = statuses.map(|s| s.exit_code()).to_vec();
    codes.sort();
    codes.dedup();
    assert_eq!(codes.len(), statuses.len());
    let c = prime_emulator::runprog::ExecutionControl::default();
    assert!(!c.cancelled());
    c.cancel();
    assert!(c.cancelled());
}

#[test]
fn actual_hooks_require_root_frame_teardown_and_correlate_output() {
    use prime_emulator::{
        engine::CpuThread,
        runtime::*,
        services::{Registry, SharedState, UnsupportedPolicy},
        trace::{TraceConfig, TraceHub},
    };
    use std::sync::Arc;
    use unicorn_engine::RegisterARM;
    let mut m = GuestMemory::new();
    m.map("anchors", 0x30600000, 0x400000).unwrap();
    m.map("data", 0x20000000, 0x10000).unwrap();
    m.set_heap(0x20000000, 0x10000);
    for pc in [
        FUNCTION_ENTRY,
        BODY_ENTRY,
        EVALUATOR_RETURN,
        PRINT_TEXT,
        PRINT_CLEAR,
        ERROR_TEXT,
        ERROR_CONTEXT,
    ] {
        m.write_u32(pc, 0xe1a00000).unwrap();
    }
    let def = 0x20001000;
    let node = 0x20002000;
    let argv = 0x20003000;
    let ctx = 0x20004000;
    let val = 0x20005000;
    let text = 0x20006000;
    m.write_u32(def + 0x10, 0x1234).unwrap();
    m.write_u32(def + 0x20, node).unwrap();
    m.write_u32(argv, ctx).unwrap();
    let utf = |s: &str| {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    m.write_bytes(node + 8, &utf("SYNTH")).unwrap();
    m.write_bytes(text, &utf("guest é😀")).unwrap();
    m.write_u32(ctx + 8, 0x20007000).unwrap();
    m.write_u32(val, 0x010f0001).unwrap();
    m.write_bytes(val + 8, &49u64.to_le_bytes()).unwrap();
    m.write_bytes(val + 16, &1u64.to_le_bytes()).unwrap();
    let m = Arc::new(m);
    let trace = Arc::new(
        TraceHub::new(TraceConfig {
            runtime: true,
            verbosity: 0,
            ..Default::default()
        })
        .unwrap(),
    );
    let obs = trace.runtime.as_ref().unwrap();
    obs.arm(
        0x1234,
        "source",
        &ExecutionRequest {
            entry: "SYNTH".into(),
            arguments: vec![],
        },
        0.,
    )
    .unwrap();
    let mut cpu = CpuThread::new(
        m.clone(),
        Arc::new(Registry::new()),
        Arc::new(SharedState::new(
            "/tmp/unused-runtime".into(),
            "A:\\test".into(),
        )),
        trace.clone(),
        UnsupportedPolicy::Stop,
        0,
        vec![],
    )
    .unwrap();
    let execute = |cpu: &mut CpuThread, pc, arg| {
        cpu.set_entry(pc, 0x2000f000, arg).unwrap();
        cpu.run_slice(1).unwrap();
    };
    cpu.set_reg(RegisterARM::R1, argv).unwrap();
    cpu.set_reg(RegisterARM::R2, 0).unwrap();
    execute(&mut cpu, FUNCTION_ENTRY, def);
    assert_eq!(obs.progress(), (true, false));
    cpu.set_reg(RegisterARM::R6, def).unwrap();
    execute(&mut cpu, BODY_ENTRY, 0);
    cpu.set_reg(RegisterARM::R1, text).unwrap();
    execute(&mut cpu, PRINT_TEXT, 0);
    cpu.set_reg(RegisterARM::R5, ctx).unwrap();
    cpu.set_reg(RegisterARM::R4, val).unwrap();
    execute(&mut cpu, EVALUATOR_RETURN, 0);
    assert_eq!(
        obs.progress(),
        (true, false),
        "nested frame is not completion"
    );
    m.write_u32(ctx + 8, 0).unwrap();
    execute(&mut cpu, EVALUATOR_RETURN, 0);
    assert_eq!(obs.progress(), (true, true));
    let first = obs.snapshot();
    assert_eq!(
        first.invocations[0].result.as_ref().unwrap().value,
        real("49")
    );
    assert_eq!(
        first.invocations[0].output[0].text.as_deref(),
        Some("guest é😀")
    );
    obs.finish_invocation(ExecutionStatus::Completed);
    execute(&mut cpu, PRINT_TEXT, 0);
    assert_eq!(obs.snapshot().invocations[0].output.len(), 1);
    obs.arm(
        0x1234,
        "new-source",
        &ExecutionRequest {
            entry: "SYNTH".into(),
            arguments: vec![],
        },
        10.,
    )
    .unwrap();
    assert_eq!(obs.progress(), (false, false));
    assert!(obs.snapshot().invocations[1].output.is_empty());
    execute(&mut cpu, FUNCTION_ENTRY, u32::MAX);
    assert_eq!(obs.progress(), (false, false));
    assert!(
        obs.snapshot().invocations[1]
            .unavailable
            .iter()
            .any(|e| e.contains("overflow"))
    );
}
#[test]
fn cancellation_releases_a_waiter_without_granting_the_owned_lock() {
    use prime_emulator::services::sync::HostCs;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let cs = Arc::new(HostCs::new());
    cs.enter();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let other = cs.clone();
    let stop = cancelled.clone();
    let worker = std::thread::spawn(move || {
        tx.send(()).unwrap();
        other.enter_until(&|| stop.load(Ordering::Acquire))
    });
    rx.recv().unwrap();
    cancelled.store(true, Ordering::Release);
    assert!(!worker.join().unwrap());
    cs.leave();
    assert!(cs.enter_until(&|| false));
    cs.leave();
}

#[test]
fn report_missing_source_invalid_requests_and_diagnostic_lists() {
    use prime_emulator::{
        checkprog::CheckConfig,
        runprog::{self, ExecutionControl, RunProgramConfig},
        runtime::RuntimeDiagnostic,
    };
    use std::time::Duration;
    let temp = tempfile::tempdir().unwrap();
    let cfg = RunProgramConfig {
        check: CheckConfig {
            program: temp.path().join("MISSING.ppl"),
            name: None,
            as_source: false,
            firmware: temp.path().join("absent.elf"),
            os_rom: None,
            factory_a: temp.path().join("factory"),
            out_dir: temp.path().join("runs"),
            init_data: None,
            settle_ms: 300,
            boot_timeout: Duration::from_secs(1),
            key_timeout: Duration::from_secs(1),
            max_seconds: 1.,
            max_instructions: 100,
        },
        requests: vec![ExecutionRequest {
            entry: "MISSING".into(),
            arguments: vec![],
        }],
        runtime_keys: vec![],
        execution_timeout: Duration::from_secs(1),
        control: ExecutionControl::default(),
        trace_runtime: false,
        screenshot: None,
    };
    let r = runprog::run(&cfg);
    assert_eq!(r.execution_status, ExecutionStatus::InfrastructureError);
    assert!(r.runtime.invocations.is_empty());
    assert!(r.return_value.is_none());
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["engine"], "firmware");
    assert!(json["return_value"].is_null());
    assert_eq!(json["output"], serde_json::json!([]));
    let mut empty = cfg.clone();
    empty.requests.clear();
    assert_eq!(
        runprog::run(&empty).execution_status,
        ExecutionStatus::InvalidArguments
    );
    let d = RuntimeDiagnostic {
        source: "guest-firmware".into(),
        raw_code: Some(24),
        message: None,
        raw_context: Some("synthetic context".into()),
        line: None,
        column: None,
    };
    for list in [vec![], vec![d.clone()], vec![d.clone(), d]] {
        let j = serde_json::to_string(&list).unwrap();
        let decoded: Vec<RuntimeDiagnostic> = serde_json::from_str(&j).unwrap();
        assert_eq!(decoded.len(), list.len());
        assert!(
            decoded
                .iter()
                .all(|d| d.line.is_none() && d.column.is_none())
        );
    }
}
#[test]
fn faults_and_timeouts_cannot_become_completed_or_reuse_a_value() {
    let o = Observer::default();
    o.arm(
        1,
        "hash",
        &ExecutionRequest {
            entry: "ANY".into(),
            arguments: vec![],
        },
        0.,
    )
    .unwrap();
    o.finish_invocation(ExecutionStatus::TimedOut);
    o.terminal(ExecutionStatus::TimedOut, "deadline");
    assert_eq!(o.terminal_status(), Some(ExecutionStatus::TimedOut));
    assert!(o.snapshot().invocations[0].result.is_none());
    let fault = prime_emulator::engine::FaultReport {
        kind: "synthetic unmapped read".into(),
        pc: 0x1234,
        addr: Some(0),
        regs: [0; 16],
        cpsr: 0x10,
        thread_id: 3,
        last_pcs: vec![],
    };
    o.fault(&fault);
    let t = o.snapshot();
    assert_eq!(t.terminal, Some(ExecutionStatus::GuestExecutionFault));
    assert_eq!(t.fault.unwrap().pc, 0x1234);
    assert!(t.invocations[0].diagnostic.is_none());
}
