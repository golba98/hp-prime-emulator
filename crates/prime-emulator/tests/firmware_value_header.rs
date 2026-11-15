//! Execute the original type initializer, with independently varied allocator
//! contents. This establishes which header bytes it actually initializes;
//! production decode still validates allocation envelopes and every pointer.
use prime_emulator::{memory::GuestMemory, runtime_value};
use unicorn_engine::{
    ArmCpuModel, RegisterARM as R, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

#[test]
fn original_managed_list_initializer_preserves_the_unused_header_byte() {
    let Some(path) = std::env::var_os("PRIME_TEST_FIRMWARE") else {
        eprintln!("Original initializer requires PRIME_TEST_FIRMWARE");
        return;
    };
    let data = std::fs::read(path).unwrap();
    assert_eq!(
        prime_emulator::sha256::hex(&data),
        "ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39"
    );
    let elf = goblin::elf::Elf::parse(&data).unwrap();
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
            &data[segment.p_offset as usize..(segment.p_offset + segment.p_filesz) as usize],
        )
        .unwrap();
    }
    let constructor = 0x306bb234;
    assert_eq!(
        prime_emulator::sha256::hex(&cpu.mem_read_as_vec(constructor, 0x28).unwrap()),
        "3d2b3224539b641c85261ca871692d2b9a22982b9f7698b1ac5951b05e11503d"
    );
    cpu.mem_map(0x20000000, 4096, Prot::ALL).unwrap();
    let mut mem = GuestMemory::new();
    mem.map("controlled list", 0x20000000, 4096).unwrap();
    // One independently defined rational child, separate from the list object.
    mem.write_u32(0x20000100, 0x011f0000).unwrap();
    mem.write_u32(0x20000104, 0).unwrap();
    mem.write_bytes(0x20000108, &7u64.to_le_bytes()).unwrap();
    mem.write_bytes(0x20000110, &1u64.to_le_bytes()).unwrap();
    for unused in 0..=255u8 {
        let mut allocation = [0xa5u8; 12];
        allocation[3] = unused;
        allocation[4..8].copy_from_slice(&1u32.to_le_bytes());
        allocation[8..12].copy_from_slice(&0x20000100u32.to_le_bytes());
        cpu.mem_write(0x20000000, &allocation).unwrap();
        cpu.reg_write(R::R0, 0x20000000).unwrap();
        cpu.reg_write(R::R1, 6).unwrap();
        cpu.reg_write(R::LR, 0x20000ffc).unwrap();
        cpu.emu_start(constructor, 0x20000ffc, 1_000_000, 64)
            .unwrap();
        assert_eq!(cpu.reg_read(R::PC).unwrap(), 0x20000ffc);
        let actual = cpu.mem_read_as_vec(0x20000000, 12).unwrap();
        assert_eq!(&actual[..4], &[0, 0, 0x16, unused]);
        assert_eq!(&actual[4..], &allocation[4..]);
        mem.write_bytes(0x20000000, &actual).unwrap();
        let evidence = runtime_value::decode_return(&mem, 0x20000000);
        assert_eq!(evidence.value.as_ref().unwrap().display(), "{7}");
        let graph = evidence.representation.unwrap();
        assert!(graph.complete && graph.coherent);
        assert_eq!(
            runtime_value::decode_representation(graph).value,
            evidence.value
        );
    }
}
