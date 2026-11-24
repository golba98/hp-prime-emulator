//! Execute the installed firmware's original grouped-string decoder. Only its
//! allocation boundary is supplied by this test; decoding, CR handling, byte
//! skipping, output-array construction and UTF16 traversal run as original ARM.
//! No vendor instructions or resource text are bundled and no hardware is used.
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use unicorn_engine::{
    ArmCpuModel, RegisterARM as R, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

const SCRATCH: u64 = 0x20000000;
const HEAP: u64 = 0x20200000;
const RETURN: u64 = SCRATCH + 0xffc;

#[derive(Default)]
struct Heap {
    next: u64,
    sizes: BTreeMap<u64, usize>,
}

fn word(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
}
fn read_word(cpu: &Unicorn<'_, ()>, at: u64) -> u64 {
    u64::from(u32::from_le_bytes(
        cpu.mem_read_as_vec(at, 4).unwrap().try_into().unwrap(),
    ))
}

#[test]
fn original_arm_decoder_matches_all_installed_interface_string_groups() {
    let (Some(firmware), Some(resource)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_FONT_RESOURCE"),
    ) else {
        eprintln!(
            "Original ARM string validation requires PRIME_TEST_FIRMWARE and PRIME_TEST_FONT_RESOURCE"
        );
        return;
    };
    let firmware = std::fs::read(firmware).unwrap();
    assert_eq!(
        prime_emulator::sha256::hex(&firmware),
        "ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39"
    );
    let resource = std::fs::read(resource).unwrap();
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-help-resource-controls.json"
    ))
    .unwrap();
    assert_eq!(
        prime_emulator::sha256::hex(&resource),
        controls["archive_sha256"]
    );
    let elf = goblin::elf::Elf::parse(&firmware).unwrap();
    let mut cpu = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
    cpu.ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)
        .unwrap();
    for segment in elf
        .program_headers
        .iter()
        .filter(|p| p.p_type == goblin::elf::program_header::PT_LOAD)
    {
        let start = segment.p_vaddr & !4095;
        let end = (segment.p_vaddr + segment.p_memsz + 4095) & !4095;
        cpu.mem_map(start, end - start, Prot::ALL).unwrap();
        cpu.mem_write(
            segment.p_vaddr,
            &firmware[segment.p_offset as usize..(segment.p_offset + segment.p_filesz) as usize],
        )
        .unwrap();
    }
    assert_eq!(
        prime_emulator::sha256::hex(&cpu.mem_read_as_vec(0x30611218, 0x248).unwrap()),
        "160258056396815ddb563ecd75c938e68620554628189a4ed6e2c442b6e53736"
    );
    cpu.mem_map(SCRATCH, 0x1000000, Prot::ALL).unwrap();
    let heap = Rc::new(RefCell::new(Heap::default()));
    for (address, resize) in [(0x3061a19c, false), (0x3061a2ec, true)] {
        let heap = heap.clone();
        cpu.add_code_hook(address, address, move |cpu, _, _| {
            let first = cpu.reg_read(R::R0).unwrap();
            let old = if resize { first } else { 0 };
            let size = if resize {
                cpu.reg_read(R::R1).unwrap()
            } else {
                first
            } as usize;
            assert!(size <= 1024 * 1024);
            let mut heap = heap.borrow_mut();
            let copy = if old == 0 {
                Vec::new()
            } else {
                cpu.mem_read_as_vec(old, heap.sizes[&old].min(size))
                    .unwrap()
            };
            let at = heap.next;
            heap.next += (size as u64 + 15) & !15;
            assert!(heap.next < SCRATCH + 0x1000000);
            heap.sizes.insert(at, size);
            // Poison fresh allocations so missing initialization cannot pass
            // because an allocator happened to supply NUL-filled memory.
            cpu.mem_write(at, &vec![0xa5; size]).unwrap();
            cpu.mem_write(at, &copy).unwrap();
            cpu.reg_write(R::R0, at).unwrap();
            let target = cpu.reg_read(R::LR).unwrap();
            cpu.reg_write(R::PC, target).unwrap();
        })
        .unwrap();
    }
    let mut actual_strings = 0;
    let mut actual_groups = 0;
    for language in ppl_engine::help::Language::ALL {
        let book = ppl_engine::help::Book::from_resource(&resource, language).unwrap();
        let record = controls["books"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["language"] == language as u32)
            .unwrap()["resource"]
            .as_u64()
            .unwrap() as usize;
        let start = 28 + word(&resource, 28 + record * 8);
        let data = &resource[start..start + word(&resource, 28 + record * 8 + 4)];
        let dictionary = &data[word(data, 40)..word(data, 40) + word(data, 44)];
        cpu.mem_write(SCRATCH + 0x1000, dictionary).unwrap();
        let index = word(data, 60);
        let encoded = word(data, 64);
        for (position, group) in book.groups.iter().enumerate() {
            let entry = index + position * 12;
            let descriptor = word(data, entry);
            let offset = word(data, entry + 4);
            let len = word(data, entry + 8) & 0x7fffffff;
            assert!(len < 0x80000);
            cpu.mem_write(
                SCRATCH + 0x4000,
                &data[encoded + offset..encoded + offset + len],
            )
            .unwrap();
            *heap.borrow_mut() = Heap {
                next: HEAP,
                ..Default::default()
            };
            cpu.mem_write(SCRATCH, &[0; 8]).unwrap();
            cpu.mem_write(SCRATCH + 0x20, &((SCRATCH + 0x4000) as u32).to_le_bytes())
                .unwrap();
            cpu.mem_write(SCRATCH + 0x24, &(len as u32).to_le_bytes())
                .unwrap();
            cpu.reg_write(R::CPSR, 0x13).unwrap();
            cpu.reg_write(R::SP, SCRATCH + 0xff000).unwrap();
            cpu.mem_write(
                SCRATCH + 0xff000,
                &((descriptor >> 23) as u32).to_le_bytes(),
            )
            .unwrap();
            for (register, value) in [
                (R::R0, SCRATCH),
                (R::R1, SCRATCH + 0x20),
                (R::R2, SCRATCH + 0x24),
                (R::R3, SCRATCH + 0x1000),
                (R::LR, RETURN),
            ] {
                cpu.reg_write(register, value).unwrap();
            }
            cpu.emu_start(0x30611218, RETURN, 5_000_000, 20_000_000)
                .unwrap_or_else(|e| panic!("{language:?} group{}: {e:?}", group.id));
            assert_eq!(
                cpu.reg_read(R::PC).unwrap(),
                RETURN,
                "{language:?} group{} did not return",
                group.id
            );
            assert_eq!(read_word(&cpu, SCRATCH + 4) as usize, group.strings.len());
            let pointers = read_word(&cpu, SCRATCH);
            for (index, expected) in group.strings.iter().enumerate() {
                let pointer = read_word(&cpu, pointers + index as u64 * 4);
                let actual = cpu
                    .mem_read_as_vec(pointer, (expected.len() + 1) * 2)
                    .unwrap();
                let mut expected_bytes = expected
                    .iter()
                    .flat_map(|u| u.to_le_bytes())
                    .collect::<Vec<_>>();
                expected_bytes.extend([0, 0]);
                assert_eq!(
                    actual, expected_bytes,
                    "{language:?} group{} string{index}",
                    group.id
                );
                actual_strings += 1;
            }
            actual_groups += 1;
        }
    }
    assert_eq!((actual_groups, actual_strings), (281, 29181));
}
