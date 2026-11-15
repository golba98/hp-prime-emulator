//! Architectural state tests for the production ARM926 owner, without firmware.
use prime_emulator::cpu_state::ArmCpuImage;
use unicorn_engine::{
    ArmCpuModel, RegisterARM as R, RegisterARMCP, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

fn engine() -> Unicorn<'static, ()> {
    let mut cpu = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
    cpu.ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)
        .unwrap();
    cpu.mem_map(0x10000000, 4096, Prot::ALL).unwrap();
    cpu
}
#[test]
fn all_banks_float_bits_and_coprocessors_round_trip_without_changing_source() {
    let mut source = engine();
    for mode in [0x10, 0x11, 0x12, 0x13, 0x17, 0x1b] {
        source.reg_write(R::CPSR, mode).unwrap();
        for index in 8..=12 {
            source
                .reg_write(
                    R::R0 as i32 + index,
                    if mode == 0x11 {
                        0xf100 + index as u64
                    } else {
                        0x100 + index as u64
                    },
                )
                .unwrap();
        }
        source.reg_write(R::SP, 0x10000800 + mode * 16).unwrap();
        source.reg_write(R::LR, 0x10000000 + mode * 4).unwrap();
        if mode != 0x10 {
            source.reg_write(R::SPSR, 0x80000010 | mode << 16).unwrap();
        }
    }
    for index in 0..32 {
        source
            .reg_write(R::D0 as i32 + index, 0x1122334400000000 | index as u64)
            .unwrap();
    }
    source.reg_write(R::FPSCR, 0x01000000).unwrap();
    let cp = RegisterARMCP::new().cp(15).crn(3).val(0x55555555);
    source.reg_write_arm_coproc(&cp).unwrap();
    source.reg_write(R::CPSR, 0x60000013).unwrap();
    source.reg_write(R::PC, 0x10000000).unwrap();
    source.reg_write(R::R0, 12).unwrap();
    let before = (
        source.reg_read(R::CPSR).unwrap(),
        source.reg_read(R::SP).unwrap(),
        source.reg_read(R::SPSR).unwrap(),
    );
    let image = ArmCpuImage::capture(&mut source).unwrap();
    assert_eq!(
        before,
        (
            source.reg_read(R::CPSR).unwrap(),
            source.reg_read(R::SP).unwrap(),
            source.reg_read(R::SPSR).unwrap()
        )
    );
    let decoded: ArmCpuImage =
        serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
    let mut restored = engine();
    decoded.restore(&mut restored).unwrap();
    assert_eq!(ArmCpuImage::capture(&mut restored).unwrap(), image);
    assert_eq!(restored.reg_read(R::R0).unwrap(), 12);
    assert_eq!(restored.reg_read(R::D31).unwrap(), 0x112233440000001f);
    assert!(
        image
            .cp15
            .iter()
            .any(|r| r.crn == 3 && r.value == 0x55555555)
    );
    // Execute real code from the restored PC with the preserved flags/registers.
    restored
        .mem_write(0x10000000, &0xe2800007u32.to_le_bytes())
        .unwrap();
    restored.emu_start(0x10000000, 0, 0, 1).unwrap();
    assert_eq!(restored.reg_read(R::R0).unwrap(), 19);
}
#[test]
fn incompatible_images_are_refused_without_partial_cpu_changes() {
    let mut cpu = engine();
    let image = ArmCpuImage::capture(&mut cpu).unwrap();
    for changed in [0, 1, 2, 3] {
        let mut bad = image.clone();
        match changed {
            0 => bad.runtime_version ^= 1,
            1 => bad.banks[1].mode = 0x13,
            2 => bad.gprs[13] = 100,
            _ => bad.cp15.pop().map(|_| ()).unwrap(),
        }
        assert!(bad.restore(&mut cpu).is_err());
        assert_eq!(ArmCpuImage::capture(&mut cpu).unwrap(), image);
    }
}
#[test]
fn production_model_rejects_a_cortex_only_instruction() {
    let mut cpu = engine();
    // MOVW r0,#0x1234 is ARMv6T2+, absent on the physical G1's ARMv5 core.
    cpu.mem_write(0x10000000, &0xe3010234u32.to_le_bytes())
        .unwrap();
    assert!(cpu.emu_start(0x10000000, 0, 0, 1).is_err());
}
