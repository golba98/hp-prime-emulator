//! Call-component contracts. Controlled callee returns below are unit-test
//! inputs, not evidence that the firmware compiler produced those results.
//! The separately run firmware_session tests exercise the actual compiler.
use super::*;
use crate::{
    direct::Plan,
    firmware::ElfImage,
    memory::{GuestMemory, image::MemoryImage, page_round},
};
use std::path::Path;
use unicorn_engine::{
    ArmCpuModel, RegisterARM as R,
    unicorn_const::{Arch, Mode, Prot},
};

const ID: &str = "managed-call-component-v1";
const STACK: u32 = 0x10000000;
const SOURCE: &str = "EXPORT T() BEGIN RETURN 17; END; // λ😀\n";
const REQUEST: u64 = 812;
type Output = Arc<Mutex<Option<Outcome>>>;

fn memory() -> GuestMemory {
    let mut mem = GuestMemory::new();
    mem.map("stack-code", STACK, 65536).unwrap();
    mem.map("heap", 0x20000000, 65536).unwrap();
    mem.set_heap(0x20000000, 65536);
    mem
}
fn cpu(mem: &GuestMemory) -> Unicorn<'_, ()> {
    let mut cpu = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
    cpu.ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)
        .unwrap();
    for block in mem.blocks() {
        // SAFETY: CPU cannot outlive the borrowed memory, and tests run one
        // engine at a time, exclusively, between component captures.
        unsafe {
            cpu.mem_map_ptr(
                block.vaddr as u64,
                block.size as u64,
                Prot::ALL,
                block.host_ptr().cast(),
            )
            .unwrap();
        }
    }
    cpu.reg_write(R::CPSR, 0x11).unwrap();
    cpu.reg_write(R::R8, 0xf123).unwrap();
    cpu.reg_write(R::CPSR, 0x12).unwrap();
    cpu.reg_write(R::SP, u64::from(STACK + 0xd000)).unwrap();
    cpu.reg_write(R::CPSR, 0x13).unwrap();
    cpu.reg_write(R::SP, u64::from(STACK + 0xf000)).unwrap();
    cpu.reg_write(R::PC, u64::from(STACK + 0x104)).unwrap();
    cpu.reg_write(R::D31, 0x123456789abcdef0).unwrap();
    cpu
}
fn text(mem: &GuestMemory, source: &str) -> u32 {
    let bytes: Vec<_> = source
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let address = mem.heap_alloc(bytes.len()).unwrap();
    mem.write_bytes(address, &bytes).unwrap();
    address
}
fn firmware_memory() -> Option<GuestMemory> {
    let Some(path) = std::env::var_os("PRIME_TEST_FIRMWARE") else {
        eprintln!("managed-call component with traced code: PRIME_TEST_FIRMWARE not configured");
        return None;
    };
    let elf = ElfImage::load(Path::new(&path)).unwrap();
    let mut mem = memory();
    for segment in &elf.segments {
        let base = segment.vaddr & !4095;
        mem.map(
            "traced-code",
            base,
            page_round(segment.mem_size + (segment.vaddr - base) as usize),
        )
        .unwrap();
        mem.write_bytes(segment.vaddr, elf.segment_bytes(segment))
            .unwrap();
    }
    crate::direct::code_matches(&mem).unwrap();
    Some(mem)
}
fn queue(mem: &GuestMemory) -> (GuestCalls, Output, u32, u32) {
    let calls = GuestCalls::default();
    calls.prepare(mem).unwrap();
    let output = Arc::new(Mutex::new(None));
    let ctx = mem.heap_alloc(0x400).unwrap();
    calls
        .submit(Plan::new(ctx, text(mem, SOURCE), REQUEST, SOURCE, output.clone()).unwrap())
        .unwrap();
    (
        calls,
        output,
        mem.heap_alloc(0x48).unwrap(),
        text(mem, SOURCE),
    )
}
fn controlled_return(calls: &GuestCalls, mem: &GuestMemory, cpu: &mut Unicorn<'_, ()>, r0: u32) {
    cpu.reg_write(R::R0, u64::from(r0)).unwrap();
    cpu.reg_write(R::D31, 0xffffffffffffffff).unwrap();
    assert!(calls.complete(cpu, mem).unwrap());
    assert!(!calls.complete(cpu, mem).unwrap(), "return is applied once");
    assert_eq!(cpu.reg_read(R::D31).unwrap(), 0x123456789abcdef0);
    assert_eq!(cpu.reg_read(R::PC).unwrap(), u64::from(STACK + 0x100));
    // An actual main-wait SVC advances PC before calling inject again.
    cpu.reg_write(R::PC, u64::from(STACK + 0x104)).unwrap();
}

#[test]
fn idle_trap_round_trip_and_corruption_admission_do_not_mutate_cpu_or_allocator() {
    let mem = memory();
    let mut cpu = cpu(&mem);
    let calls = GuestCalls::default();
    let trap = calls.prepare(&mem).unwrap();
    let image = calls.image(&mem, ID, &cpu).unwrap();
    let decoded = GuestCallsImage::from_bytes(&image.to_bytes().unwrap()).unwrap();
    let before = ArmCpuImage::capture(&mut cpu).unwrap();
    let heap_before = mem.heap_stats();
    let restored = decoded.restore(&mem, ID, &cpu, None).unwrap();
    assert_eq!(restored.prepare(&mem).unwrap(), trap);
    assert!(!restored.busy());
    assert!(!restored.complete(&mut cpu, &mem).unwrap());
    for changed in [0, 1, 2, 3] {
        let mut bad = decoded.clone();
        match changed {
            0 => bad.version += 1,
            1 => bad.return_va += 2,
            2 => {
                bad.last = Some(Returned {
                    r0: 1,
                    frame: vec![],
                    frame_va: STACK,
                    elapsed_ms: 0.0,
                })
            }
            _ => bad.resource_identity = "other".into(),
        }
        // Recomputed checksums must not bypass semantic admission.
        bad.sha256 = bad.digest().unwrap();
        assert!(bad.restore(&mem, ID, &cpu, None).is_err());
    }
    assert_eq!(ArmCpuImage::capture(&mut cpu).unwrap(), before);
    assert_eq!(mem.heap_stats(), heap_before);
    mem.write_u32(trap, 0).unwrap();
    assert!(decoded.restore(&mem, ID, &cpu, None).is_err());
    assert!(GuestCallsImage::from_bytes(&vec![b' '; 65537]).is_err());
    assert!(GuestCallsImage::from_bytes(b"{}").is_err());
}

#[test]
fn diagnostic_timing_round_trips_without_invalidating_the_image_digest() {
    let mem = memory();
    let cpu = cpu(&mem);
    let calls = GuestCalls::default();
    let mut image = calls.image(&mem, ID, &cpu).unwrap();
    // These adversarial bit patterns exercise decimal parsing at rounding
    // boundaries. serde_json's float_roundtrip feature is required for a
    // checksummed manifest containing native diagnostic f64 values.
    for bits in [
        0x3fda9170410697a5u64,
        0x3f6d68c692f6e829,
        0x412e848000000001,
    ] {
        let ms = f64::from_bits(bits);
        image.last = Some(Returned {
            r0: 0,
            frame: vec![],
            frame_va: STACK,
            elapsed_ms: ms,
        });
        image.sha256 = image.digest().unwrap();
        let decoded = GuestCallsImage::from_bytes(&image.to_bytes().unwrap()).unwrap();
        assert_eq!(decoded.last.as_ref().unwrap().elapsed_ms.to_bits(), bits);
        assert_eq!(decoded.sha256, decoded.digest().unwrap());
    }
}

#[test]
fn each_pending_phase_rebinds_only_the_exact_request_and_source() {
    let Some(mem) = firmware_memory() else {
        return;
    };
    let mut cpu = cpu(&mem);
    let (mut calls, _, item, copy) = queue(&mem);
    // Unit inputs exercise all eight call shapes, then the terminal output.
    // They do not execute the compiler or assert a genuine Check verdict.
    let mut output = Arc::new(Mutex::new(None));
    for phase in 0..8 {
        let queued = calls.image(&mem, ID, &cpu).unwrap();
        output = Arc::new(Mutex::new(None));
        calls = queued
            .restore(
                &mem,
                ID,
                &cpu,
                Some(OutputBinding {
                    request: REQUEST,
                    source: SOURCE,
                    output: output.clone(),
                }),
            )
            .unwrap();
        assert!(calls.inject(&mut cpu, &mem).unwrap());
        let image = calls.image(&mem, ID, &cpu).unwrap();
        if phase == 6 {
            let before = ArmCpuImage::capture(&mut cpu).unwrap();
            for changed in 0..7 {
                let mut value = serde_json::to_value(&image).unwrap();
                match changed {
                    0 => value["plan"]["stage"] = "finished".into(),
                    1 => value["active"]["frame_words"] = 0.into(),
                    2 => value["active"]["saved"]["banks"][1]["mode"] = 0x13.into(),
                    3 => value["active"]["frame_va"] = 0.into(),
                    4 => value["plan"]["binding"]["utf16_units"] = (1024 * 1024 + 1).into(),
                    5 => value["plan"]["outcome"]["return_value"] = 1.into(),
                    _ => value["plan"]["outcome"]["calls"][0][0] = "unrecognized phase".into(),
                }
                let mut bad: GuestCallsImage = serde_json::from_value(value).unwrap();
                bad.sha256 = bad.digest().unwrap();
                assert!(
                    bad.restore(
                        &mem,
                        ID,
                        &cpu,
                        Some(OutputBinding {
                            request: REQUEST,
                            source: SOURCE,
                            output: output.clone()
                        })
                    )
                    .is_err()
                );
                assert!(output.lock().unwrap().is_none());
                assert_eq!(ArmCpuImage::capture(&mut cpu).unwrap(), before);
            }
        }
        for (request, source) in [(REQUEST + 1, SOURCE), (REQUEST, "different source")] {
            assert!(
                image
                    .restore(
                        &mem,
                        ID,
                        &cpu,
                        Some(OutputBinding {
                            request,
                            source,
                            output: Arc::new(Mutex::new(None))
                        })
                    )
                    .is_err()
            );
        }
        let decoded = GuestCallsImage::from_bytes(&image.to_bytes().unwrap()).unwrap();
        calls = decoded
            .restore(
                &mem,
                ID,
                &cpu,
                Some(OutputBinding {
                    request: REQUEST,
                    source: SOURCE,
                    output: output.clone(),
                }),
            )
            .unwrap();
        controlled_return(
            &calls,
            &mem,
            &mut cpu,
            match phase {
                2 => item,
                3 => copy,
                6 => 1,
                7 => text(&mem, "unit return text"),
                _ => 0,
            },
        );
        let returned = calls.image(&mem, ID, &cpu).unwrap();
        calls = returned
            .restore(
                &mem,
                ID,
                &cpu,
                Some(OutputBinding {
                    request: REQUEST,
                    source: SOURCE,
                    output: output.clone(),
                }),
            )
            .unwrap();
    }
    assert!(!calls.inject(&mut cpu, &mem).unwrap());
    assert!(!calls.busy());
    let result = output.lock().unwrap().take().unwrap();
    assert_eq!(result.message.as_deref(), Some("unit return text"));
    assert_eq!(result.calls.len(), 9);
    assert!(result.error.is_none());
    assert!(calls.image(&mem, ID, &cpu).unwrap().binding().is_none());
}

#[derive(Serialize, Deserialize)]
struct Fixture {
    current: ArmCpuImage,
    expected: ArmCpuImage,
}
#[test]
fn active_return_frame_and_full_ui_context_resume_in_a_fresh_process() {
    let Some(mem) = firmware_memory() else {
        return;
    };
    let mut cpu = cpu(&mem);
    let (calls, _, item, copy) = queue(&mem);
    for phase in 0..6 {
        assert!(calls.inject(&mut cpu, &mem).unwrap());
        controlled_return(
            &calls,
            &mem,
            &mut cpu,
            match phase {
                2 => item,
                3 => copy,
                _ => 0,
            },
        );
    }
    let mut expected = ArmCpuImage::capture(&mut cpu).unwrap();
    expected.gprs[15] -= 4;
    assert!(calls.inject(&mut cpu, &mem).unwrap());
    // Self-authored callee tail: increment r7 then return via the production
    // trap. This verifies return continuation, not the HP compiler's body.
    mem.write_u32(STACK + 0x200, 0xe2877001).unwrap();
    mem.write_u32(STACK + 0x204, 0xe12fff1e).unwrap();
    cpu.reg_write(R::PC, u64::from(STACK + 0x200)).unwrap();
    cpu.reg_write(R::R0, 0).unwrap(); // No diagnostic source: plan finishes on return.
    cpu.reg_write(R::D31, 0xffffffffffffffff).unwrap();
    let fixture = Fixture {
        current: ArmCpuImage::capture(&mut cpu).unwrap(),
        expected,
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("calls.json"),
        calls.image(&mem, ID, &cpu).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("cpu.json"),
        serde_json::to_vec(&fixture).unwrap(),
    )
    .unwrap();
    MemoryImage::capture(&mem, ID)
        .unwrap()
        .save(&root.path().join("memory.bin"))
        .unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "guest_call::image::tests::restored_call_child",
            "--nocapture",
        ])
        .env("PRIME_MANAGED_CALL_CHILD", root.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("managed-return-context-resumed"));
}
#[test]
fn restored_call_child() {
    let Some(root) = std::env::var_os("PRIME_MANAGED_CALL_CHILD") else {
        return;
    };
    let root = Path::new(&root);
    let mem = MemoryImage::load(&root.join("memory.bin"), ID)
        .unwrap()
        .restore(ID)
        .unwrap();
    let mut cpu = cpu(&mem);
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(root.join("cpu.json")).unwrap()).unwrap();
    fixture.current.restore(&mut cpu).unwrap();
    let image =
        GuestCallsImage::from_bytes(&std::fs::read(root.join("calls.json")).unwrap()).unwrap();
    let output = Arc::new(Mutex::new(None));
    let calls = image
        .restore(
            &mem,
            ID,
            &cpu,
            Some(OutputBinding {
                request: REQUEST,
                source: SOURCE,
                output: output.clone(),
            }),
        )
        .unwrap();
    let r7 = cpu.reg_read(R::R7).unwrap();
    cpu.emu_start(u64::from(STACK + 0x200), 0, 0, 2).unwrap();
    assert_eq!(cpu.reg_read(R::R7).unwrap(), r7 + 1);
    assert!(calls.complete(&mut cpu, &mem).unwrap());
    assert!(!calls.complete(&mut cpu, &mem).unwrap());
    assert_eq!(ArmCpuImage::capture(&mut cpu).unwrap(), fixture.expected);
    cpu.reg_write(R::PC, u64::from(STACK + 0x104)).unwrap();
    assert!(!calls.inject(&mut cpu, &mem).unwrap());
    let outcome = output.lock().unwrap().take().unwrap();
    assert_eq!(outcome.return_value, Some(0));
    assert_eq!(outcome.record, Some([u32::MAX, 0, 0]));
    assert!(!calls.busy());
    println!("managed-return-context-resumed");
}
