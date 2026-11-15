//! Execute original module parsers. Only file I/O, allocator and the empty
//! process-registry boundary are supplied; module/resource/stat code executes.
use std::sync::{Arc, Mutex};
use unicorn_engine::{
    ArmCpuModel, RegisterARM as R, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};
fn word(cpu: &Unicorn<'_, ()>, at: u32) -> u32 {
    u32::from_le_bytes(
        cpu.mem_read_as_vec(u64::from(at), 4)
            .unwrap()
            .try_into()
            .unwrap(),
    )
}
fn put(cpu: &mut Unicorn<'_, ()>, at: u32, value: u32) {
    cpu.mem_write(u64::from(at), &value.to_le_bytes()).unwrap();
}
fn cpu(rom: &[u8]) -> Unicorn<'static, ()> {
    let mut cpu = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
    cpu.ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)
        .unwrap();
    cpu.mem_map(0x30000000, 0x100000, Prot::ALL).unwrap();
    cpu.mem_write(0x30000000, rom).unwrap();
    cpu.mem_write(0x30090000, &vec![0; 0x70000]).unwrap();
    cpu.mem_map(0x20000000, 0x200000, Prot::ALL).unwrap();
    cpu
}
fn run(cpu: &mut Unicorn<'_, ()>, slot: u32) -> u32 {
    let target = word(cpu, 0x3007f45c + slot * 4);
    for (reg, value) in [
        (R::CPSR, 0x13),
        (R::SP, 0x2000f000),
        (R::LR, 0x2000fff0),
        (R::R0, 0x20001000),
        (R::R1, 0x20002000),
    ] {
        cpu.reg_write(reg, value).unwrap();
    }
    cpu.emu_start(u64::from(target), 0x2000fff0, 1_000_000, 300000)
        .unwrap();
    assert_eq!(cpu.reg_read(R::PC).unwrap(), 0x2000fff0);
    cpu.reg_read(R::R0).unwrap() as u32
}
pub fn invoke(rom: &[u8], payload: &[u8], found: bool, wide: bool) -> (u32, Vec<u8>) {
    let mut cpu = cpu(rom);
    cpu.mem_write(0x20001000, b"C:\\test.bin\0").unwrap();
    cpu.mem_write(0x20002000, &[0xa5; 136]).unwrap();
    let next = Arc::new(Mutex::new(0x20100000u32));
    let payload = Arc::new(payload.to_vec());
    for address in [
        0x3007a864, 0x3007a884, 0x3003b5e4, 0x3003a9cc, 0x3003a8b0, 0x3003a824, 0x30003a1c,
        0x30003a20, 0x30003a28, 0x300754c4, 0x3001bd74, 0x3000616c,
    ] {
        let next = next.clone();
        let payload = payload.clone();
        cpu.add_code_hook(address, address, move |cpu, pc, _| {
            let r = [R::R0, R::R1, R::R2, R::R3].map(|reg| cpu.reg_read(reg).unwrap() as u32);
            let alloc = |cpu: &mut Unicorn<'_, ()>, size: u32| {
                let mut next = next.lock().unwrap();
                let at = *next;
                *next = next.checked_add(size.checked_add(7).unwrap() & !7).unwrap();
                assert!(*next < 0x20200000);
                cpu.mem_write(u64::from(at), &vec![0; size as usize])
                    .unwrap();
                at
            };
            let result = match pc {
                0x3007a864 | 0x3007a884 => {
                    if found {
                        let fd = alloc(cpu, 32);
                        put(cpu, fd, 0x10001);
                        put(cpu, fd + 12, payload.len() as u32);
                        fd
                    } else {
                        0
                    }
                }
                0x3003b5e4 => alloc(cpu, 32),
                0x3003a9cc => {
                    let pos = word(cpu, r[0] + 16);
                    let limit = word(cpu, r[0] + 12);
                    let base = match r[2] {
                        0 => 0,
                        1 => pos,
                        2 => limit,
                        _ => panic!("seek origin"),
                    };
                    let result = i64::from(base) + i64::from(r[1] as i32);
                    let result = u32::try_from(result).unwrap();
                    put(cpu, r[0] + 16, result);
                    result
                }
                0x3003a8b0 => {
                    let pos = word(cpu, r[0] + 16);
                    let base = word(cpu, r[0] + 8);
                    let limit = word(cpu, r[0] + 12);
                    let start = (base + pos) as usize;
                    let count = (r[2].min(limit.saturating_sub(pos)) as usize)
                        .min(payload.len().saturating_sub(start));
                    if count > 0 {
                        cpu.mem_write(u64::from(r[1]), &payload[start..start + count])
                            .unwrap();
                    }
                    put(cpu, r[0] + 16, pos + count as u32);
                    count as u32
                }
                0x30003a1c => alloc(cpu, r[0]),
                0x30003a20 => alloc(cpu, r[0].checked_mul(r[1]).unwrap()),
                _ => 0,
            };
            cpu.reg_write(R::R0, u64::from(result)).unwrap();
            let lr = cpu.reg_read(R::LR).unwrap();
            cpu.reg_write(R::PC, lr).unwrap();
        })
        .unwrap();
    }
    let ret = run(&mut cpu, if wide { 0x2cc } else { 0x2cb });
    (ret, cpu.mem_read_as_vec(0x20002000, 136).unwrap())
}
pub fn legacy_resources(rom: &[u8]) {
    let mut cpu = cpu(rom);
    for slot in [0x28d, 0x28e, 0x293, 0x294, 0x2a4] {
        cpu.mem_write(0x20002000, &[0xa5; 136]).unwrap();
        assert_eq!(run(&mut cpu, slot), 0);
        assert_eq!(cpu.mem_read_as_vec(0x20002000, 136).unwrap(), [0xa5; 136]);
    }
}
