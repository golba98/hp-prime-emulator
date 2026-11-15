//! Optional executable ABI controls from a legitimately installed OS ROM.
//! This is isolated kernel-service execution, not boot or hardware verification.
//! Only the scheduler context-switch boundary is intercepted: it returns to the
//! service so descriptor changes/return values can be observed without a live
//! kernel run queue. No ROM instructions or assets are bundled with the tests.
use unicorn_engine::{
    ArmCpuModel, RegisterARM as R, Unicorn,
    unicorn_const::{Arch, Mode, Prot},
};

struct Kernel {
    cpu: Unicorn<'static, ()>,
}
impl Kernel {
    fn installed() -> Option<Self> {
        let path = std::env::var_os("PRIME_TEST_OS_ROM")?;
        let rom = std::fs::read(path).expect("read configured OS ROM");
        assert_eq!(
            prime_emulator::sha256::hex(&rom),
            "f6aeba161189a5aa1acb57be428dc5730cfa5ff7cd534494e59225debd5a8999",
            "control offsets require the inspected OS ROM"
        );
        let mut cpu = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
        cpu.ctl_set_cpu_model(ArmCpuModel::Model_926 as i32)
            .unwrap();
        cpu.mem_map(0x30000000, 0x100000, Prot::ALL).unwrap();
        cpu.mem_write(0x30000000, &rom).unwrap();
        // Fresh controlled kernel BSS, descriptor, stack and return sentinel.
        cpu.mem_write(0x30090000, &vec![0; 0x70000]).unwrap();
        cpu.mem_map(0x20000000, 0x10000, Prot::ALL).unwrap();
        cpu.add_code_hook(0x30001078, 0x30001078, |uc, _, _| {
            let target = uc.reg_read(R::LR).unwrap();
            uc.reg_write(R::PC, target).unwrap();
        })
        .unwrap();
        Some(Self { cpu })
    }
    fn word(&mut self, address: u64, value: u32) {
        self.cpu.mem_write(address, &value.to_le_bytes()).unwrap();
    }
    fn short(&mut self, address: u64, value: u16) {
        self.cpu.mem_write(address, &value.to_le_bytes()).unwrap();
    }
    fn read_short(&self, address: u64) -> u16 {
        u16::from_le_bytes(
            self.cpu
                .mem_read_as_vec(address, 2)
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }
    fn invoke(&mut self, slot: u32, a: u32, b: u32) -> u32 {
        let address = u32::from_le_bytes(
            self.cpu
                .mem_read_as_vec(0x3007f45c + 4 * u64::from(slot), 4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        self.invoke_address(address, a, b)
    }
    fn invoke_address(&mut self, address: u32, a: u32, b: u32) -> u32 {
        // USB link detection deliberately samples the GPIO 500 times.
        self.invoke_address_bounded(address, a, b, 50_000)
    }
    fn invoke_address_bounded(&mut self, address: u32, a: u32, b: u32, instructions: usize) -> u32 {
        self.cpu.reg_write(R::CPSR, 0x13).unwrap();
        self.cpu.reg_write(R::SP, 0x2000f000).unwrap();
        self.cpu.reg_write(R::LR, 0x2000fff0).unwrap();
        self.cpu.reg_write(R::R0, u64::from(a)).unwrap();
        self.cpu.reg_write(R::R1, u64::from(b)).unwrap();
        self.cpu
            .emu_start(u64::from(address), 0x2000fff0, 1_000_000, instructions)
            .unwrap();
        assert_eq!(
            self.cpu.reg_read(R::PC).unwrap(),
            0x2000fff0,
            "ROM service must return normally"
        );
        self.cpu.reg_read(R::R0).unwrap() as u32
    }
}
#[test]
fn genuine_os_wake_clears_sleep_without_resuming_and_suspend_is_idempotent() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    let desc = 0x20000100;
    kernel.word(desc, 0x100);
    kernel.short(desc + 0x1e, 65535);
    assert_eq!(kernel.invoke(4, desc as u32, 0), 1); // Suspend
    assert_eq!(kernel.invoke(4, desc as u32, 0), 1);
    assert_eq!(kernel.read_short(desc + 0x20) & 8, 8);
    assert_eq!(kernel.invoke(6, desc as u32, 0), 1); // Wake
    assert_eq!(kernel.read_short(desc + 0x1e), 0);
    assert_eq!(kernel.read_short(desc + 0x20) & 8, 8);
    assert_eq!(kernel.invoke(6, desc as u32, 0), 0);
    assert_eq!(kernel.invoke(5, desc as u32, 0), 1); // Resume
    assert_eq!(kernel.read_short(desc + 0x20) & 8, 0);
    assert_eq!(kernel.invoke(5, desc as u32, 0), 0);
    assert_eq!(kernel.invoke(6, 0, 0), 0);
}
#[test]
fn genuine_auto_power_modes_return_previous_enable_and_store_minutes_as_seconds() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    // The original system-variable callback table is retained, including the
    // mode read used by this service. Only controlled BSS values are supplied.
    let power = 0x300e69fc;
    kernel.short(power + 0x16, 300);
    assert_eq!(kernel.invoke(0x2f, 1, 0), 0);
    assert_eq!(kernel.read_short(power + 0x14), 1);
    assert_eq!(kernel.invoke(0x2f, 2, 7), 1);
    assert_eq!(kernel.read_short(power + 0x16), 420);
    assert_eq!(kernel.invoke(0x2f, 0, 0), 1);
    assert_eq!(kernel.read_short(power + 0x14), 0);
    assert_eq!(kernel.invoke(0x2f, 2, 1100), 0);
    assert_eq!(kernel.read_short(power + 0x16), 464); // u16 storage, not host saturation
    assert_eq!(kernel.invoke(0x2f, 99, 0), 1);
    assert_eq!(kernel.read_short(power + 0x14), 1);
    kernel.invoke(0x2f, 0, 0);
    kernel.short(0x300c4dc0, 0xda);
    assert_eq!(kernel.invoke(0x2f, 1, 0), 0);
    assert_eq!(kernel.read_short(power + 0x14), 0);
    kernel.invoke(0x2f, 2, 3);
    assert_eq!(kernel.read_short(power + 0x16), 180);
    assert_eq!(kernel.read_short(power + 0x14), 0);
}

#[test]
fn genuine_priority_slots_are_unique_bounded_and_written_to_the_descriptor() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    let desc = 0x20000100;
    kernel.word(desc, 0x100);
    kernel.short(desc + 0x22, 17);
    kernel.word(0x300c59e8 + 17 * 4, desc as u32);
    assert_eq!(kernel.invoke(3, desc as u32, 0), 17);
    assert_eq!(kernel.invoke(2, desc as u32, 32), 1);
    assert_eq!(kernel.read_short(desc + 0x22), 32);
    assert_eq!(kernel.invoke(2, desc as u32, 32), 0);
    assert_eq!(kernel.invoke(2, desc as u32, 64), 0);
    assert_eq!(kernel.invoke(3, desc as u32, 0), 32);
    assert_eq!(kernel.invoke(3, 0, 0), 0x78);
}

#[test]
fn genuine_disk_room_layout_matches_all_byte_and_kibibyte_fields() {
    for (sectors, sector_bytes, total_clusters, free_clusters) in [
        (4u32, 512u32, 100000u32, 40000u32),
        (128, 512, 0xf0000000, 0x70000001),
    ] {
        let Some(mut kernel) = Kernel::installed() else {
            return;
        };
        kernel.word(0x300c4dfc, 1); // initialized filesystem admission
        kernel
            .cpu
            .add_code_hook(0x30053ba4, 0x30053ba4, move |uc, _, _| {
                // Supply only the disk driver's measured geometry. The original
                // service and its multiplication/subtraction/packing execute.
                for (register, value) in [
                    (R::R1, sectors),
                    (R::R2, sector_bytes),
                    (R::R3, free_clusters),
                ] {
                    let pointer = uc.reg_read(register).unwrap();
                    uc.mem_write(pointer, &value.to_le_bytes()).unwrap();
                }
                let sp = uc.reg_read(R::SP).unwrap();
                let pointer =
                    u32::from_le_bytes(uc.mem_read_as_vec(sp, 4).unwrap().try_into().unwrap());
                uc.mem_write(u64::from(pointer), &total_clusters.to_le_bytes())
                    .unwrap();
                uc.reg_write(R::R0, 0).unwrap();
                let next = uc.reg_read(R::LR).unwrap();
                uc.reg_write(R::PC, next).unwrap();
            })
            .unwrap();
        let out = 0x20000400;
        assert_eq!(kernel.invoke(0xeb, 0, out), 0);
        let actual = kernel.cpu.mem_read_as_vec(u64::from(out), 60).unwrap();
        let cluster = u64::from(sectors) * u64::from(sector_bytes);
        let expected = prime_emulator::services::disk_room::encode(
            u64::from(total_clusters) * cluster,
            u64::from(free_clusters) * cluster,
        );
        assert_eq!(
            actual, expected,
            "every observed field must match the original OS code"
        );
    }
}

#[test]
fn genuine_battery_driver_matches_variable_samples_power_pins_and_reserved_bytes() {
    use ppl_engine::environment::{BatteryConfig, BatteryPower};
    for adc in [
        0u16, 64, 65, 500, 650, 750, 758, 780, 802, 824, 846, 1000, 1023,
    ] {
        for power in [
            BatteryPower::Battery,
            BatteryPower::Charging,
            BatteryPower::Charged,
        ] {
            let Some(mut kernel) = Kernel::installed() else {
                return;
            };
            kernel.cpu.mem_map(0x56000000, 4096, Prot::ALL).unwrap();
            kernel.word(
                0x56000054,
                if power == BatteryPower::Battery { 0 } else { 8 },
            );
            kernel.word(0x56000104, u32::from(power == BatteryPower::Charged));
            kernel
                .cpu
                .add_code_hook(0x3000d068, 0x3000d068, move |uc, _, _| {
                    // Only the ADC sampling boundary is supplied externally.
                    uc.reg_write(R::R0, u64::from(adc)).unwrap();
                    let next = uc.reg_read(R::LR).unwrap();
                    uc.reg_write(R::PC, next).unwrap();
                })
                .unwrap();
            let out = 0x20000400;
            let mut original = [0xa5; 16];
            original[4..6].copy_from_slice(&7u16.to_le_bytes());
            kernel.cpu.mem_write(u64::from(out), &original).unwrap();
            kernel.word(0x2000f000, out);
            kernel.word(0x2000f004, 16);
            let actual_return = kernel.invoke_address(0x3000d5b0, 0, 256);
            let actual: [u8; 16] = kernel
                .cpu
                .mem_read_as_vec(u64::from(out), 16)
                .unwrap()
                .try_into()
                .unwrap();
            let config = BatteryConfig { adc, power };
            let (expected, expected_return) =
                prime_emulator::services::battery::query(config, original);
            assert_eq!(actual, expected, "ADC {adc}, power {power:?}");
            assert_eq!(actual_return, expected_return);
            let voltage_bits = u32::from_le_bytes(actual[..4].try_into().unwrap());
            kernel.word(0x300c4480 + 12, voltage_bits);
            assert_eq!(
                kernel.invoke(0x36, 0, 0),
                prime_emulator::services::battery::low_check(config) as u32
            );
            assert_eq!(
                kernel.invoke(0x35, 0, 0),
                u32::from(prime_emulator::services::battery::kind(config))
            );
            kernel.word(0x2000f000, out);
            kernel.word(0x2000f004, 4);
            assert_eq!(kernel.invoke_address(0x3000d5b0, 0, 257), 0);
            let actual = u32::from_le_bytes(
                kernel
                    .cpu
                    .mem_read_as_vec(u64::from(out), 4)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(
                actual,
                u32::from(prime_emulator::services::battery::kind(config))
            );
        }
    }
}

#[test]
fn genuine_critical_battery_action_posts_shutdown_only_when_not_inhibited() {
    use std::sync::{Arc, Mutex};
    for inhibit in [0u32, 1, 2, u32::MAX] {
        let Some(mut kernel) = Kernel::installed() else {
            return;
        };
        kernel.cpu.mem_map(0x56000000, 4096, Prot::ALL).unwrap();
        // Battery-only GPIO, cached critical sample. The paired temporary
        // inhibit helpers are skipped by the original initialized-BMS flag.
        kernel.word(0x300c448c, 3.4f32.to_bits());
        kernel.short(0x300c4486, 1);
        kernel.word(0x300e69fc + 0x30, inhibit);
        let messages = Arc::new(Mutex::new(Vec::new()));
        let observed = messages.clone();
        kernel
            .cpu
            .add_code_hook(0x30002d38, 0x30002d38, move |uc, _, _| {
                // The only substituted service is the power thread's mailbox.
                // BatteryLowCheck and shutdown admission execute original ROM.
                let message = uc.reg_read(R::R1).unwrap();
                let value = u32::from_le_bytes(
                    uc.mem_read_as_vec(message + 4, 4)
                        .unwrap()
                        .try_into()
                        .unwrap(),
                );
                observed.lock().unwrap().push(value);
                uc.reg_write(R::R0, 1).unwrap();
                let next = uc.reg_read(R::LR).unwrap();
                uc.reg_write(R::PC, next).unwrap();
            })
            .unwrap();
        assert_eq!(kernel.invoke(0x36, 0, 0), (-2i32) as u32);
        assert!(
            messages.lock().unwrap().is_empty(),
            "query must not request shutdown"
        );
        assert_eq!(kernel.invoke(0x36, 0, 1), (-2i32) as u32);
        assert_eq!(
            *messages.lock().unwrap(),
            if inhibit == 1 { vec![] } else { vec![0x4444] }
        );
        assert_eq!(
            kernel.read_short(0x300c4488),
            0,
            "temporary BMS action flag must clear before return"
        );
    }
}

#[test]
fn genuine_power_inhibit_system_variables_exchange_full_words() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    for (var, offset) in [(86u32, 0x2c), (87, 0x30), (88, 0x34)] {
        kernel.cpu.reg_write(R::R2, 0x89abcdef).unwrap();
        assert_eq!(kernel.invoke(0x4c, var, 0), 0);
        let actual = u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x300e69fc + offset, 4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        assert_eq!(actual, 0x89abcdef);
        kernel.cpu.reg_write(R::R2, 123).unwrap();
        assert_eq!(kernel.invoke(0x4c, var, 0xffff), 0x89abcdef);
    }
}

#[test]
fn genuine_initialized_device_registry_maps_names_to_original_drivers() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    // Execute the ROM's own initialized-data decompression and BSS clear.
    kernel.invoke_address_bounded(0x300001b0, 0, 0, 500_000);
    let table = kernel.cpu.mem_read_as_vec(0x300c486c, 32 * 32).unwrap();
    for entry in table.chunks_exact(32).take_while(|e| e[0] != 0) {
        let name = entry[..24].split(|b| *b == 0).next().unwrap();
        let id = u32::from_le_bytes(entry[24..28].try_into().unwrap());
        println!("ROM device {} -> {id:#x}", String::from_utf8_lossy(name));
    }
    let registry: std::collections::BTreeMap<_, _> = table
        .chunks_exact(32)
        .take_while(|e| e[0] != 0)
        .map(|entry| {
            let name =
                String::from_utf8(entry[..24].split(|b| *b == 0).next().unwrap().to_vec()).unwrap();
            (name, u32::from_le_bytes(entry[24..28].try_into().unwrap()))
        })
        .collect();
    assert!(!registry.contains_key(r"\\?\FACTORY"));
    assert!(!registry.contains_key(r"\\?\FTL"));
    for (name, id, driver) in [
        (r"\\?\BACK", 14, 0x3000ce84u32),
        (r"\\?\INDICATOR", 29, 0x3001a438),
        (r"\\?\BAT", 17, 0x3000d5b0),
        (r"\\?\USB", 5, 0x300148cc),
        (r"\\?\USBHOST", 30, 0x300151d4),
    ] {
        assert_eq!(registry.get(name), Some(&id));
        let mut found = false;
        for offset in (0..128).step_by(8) {
            let pair = kernel.cpu.mem_read_as_vec(0x30082208 + offset, 8).unwrap();
            if u32::from_le_bytes(pair[..4].try_into().unwrap()) == id {
                let address = u32::from_le_bytes(pair[4..].try_into().unwrap());
                let actual = u32::from_le_bytes(
                    kernel
                        .cpu
                        .mem_read_as_vec(u64::from(address) + 0x24, 4)
                        .unwrap()
                        .try_into()
                        .unwrap(),
                );
                assert_eq!(actual, driver);
                found = true;
                break;
            }
        }
        assert!(found, "device {name} needs a real driver-table entry");
    }
}

#[test]
fn genuine_usb_driver_uses_gpio_link_state_callback_pointer_and_send_arguments() {
    use std::sync::{Arc, Mutex};
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    kernel.cpu.mem_map(0x56000000, 4096, Prot::ALL).unwrap();
    for (pins, peripheral, host, mode) in [
        (0x40u32, 0, 0, 0),
        (0x48, 1, 0, 1),
        (0, 0, 1, 2),
        (8, 1, 1, 2),
    ] {
        kernel.word(0x56000054, pins);
        assert_eq!(kernel.invoke_address(0x300148cc, 0, 256), peripheral);
        assert_eq!(kernel.invoke_address(0x300151d4, 0, 256), host);
        assert_eq!(kernel.invoke_address(0x300148cc, 0, 260), mode);
        assert_eq!(kernel.invoke_address(0x300151d4, 0, 260), mode);
    }
    let target = 0x20000200;
    kernel.cpu.reg_write(R::R2, target).unwrap();
    assert_eq!(kernel.invoke_address(0x300148cc, 0, 259), 1);
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x300c45f8, 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        target as u32
    );
    let sent = Arc::new(Mutex::new(Vec::new()));
    let observed = sent.clone();
    kernel
        .cpu
        .add_code_hook(0x30012f2c, 0x30012f2c, move |uc, _, _| {
            observed.lock().unwrap().push([
                uc.reg_read(R::R0).unwrap(),
                uc.reg_read(R::R1).unwrap(),
                uc.reg_read(R::R2).unwrap(),
            ]);
            uc.reg_write(R::R0, 1).unwrap();
            let next = uc.reg_read(R::LR).unwrap();
            uc.reg_write(R::PC, next).unwrap();
        })
        .unwrap();
    kernel.word(0x56000054, 0x48);
    kernel.cpu.reg_write(R::R2, target).unwrap();
    kernel.cpu.reg_write(R::R3, 64).unwrap();
    assert_eq!(kernel.invoke_address(0x300148cc, 0, 258), 1);
    assert_eq!(*sent.lock().unwrap(), vec![[2, target, 64]]);
    kernel.word(0x300c4544, 0); // ordinary PC without peer-specific extension
    assert_eq!(kernel.invoke_address(0x300148cc, 0, 263), 0);
    kernel.word(0x300c4544, 1);
    assert_eq!(kernel.invoke_address(0x300148cc, 0, 263), 1);
}

#[test]
fn genuine_backlight_driver_exposes_levels_duty_timeout_and_exchange_fields() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    let descriptor = 0x3008249cu32;
    let state = 0x300c6550u64;
    assert_eq!(kernel.invoke_address(0x3000cd84, descriptor, 0), 0);
    // Only the physical PWM application is substituted. Driver validation,
    // level conversion, state writes and results execute original ROM.
    kernel
        .cpu
        .add_code_hook(0x3000cb78, 0x3000cb78, |uc, _, _| {
            let next = uc.reg_read(R::LR).unwrap();
            uc.reg_write(R::PC, next).unwrap();
        })
        .unwrap();
    let input = 0x20000200;
    let output = 0x20000300;
    for level in [0u32, 1, 2, 3, 4, 100, u32::MAX] {
        kernel.word(input, level);
        kernel.cpu.reg_write(R::R2, input).unwrap();
        kernel.cpu.reg_write(R::R3, 4).unwrap();
        assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 256), u32::MAX);
        kernel.word(0x2000f000, output as u32);
        assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 257), level);
        let actual = u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(state + 8, 4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        assert_eq!(actual, prime_emulator::services::backlight::duty(level));
        assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 264), actual);
    }
    kernel.word(input, 0x12345678);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 258), 0);
    assert_eq!(
        kernel.invoke_address(0x3000ce84, descriptor, 259),
        0x12345678
    );
    kernel.word(input, 99);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 260), 0);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 261), 99);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 263), 0);
    kernel.word(input, 55);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 263), 99);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 265), 3);
    assert_eq!(kernel.invoke_address(0x3000ce84, descriptor, 8), 4);
}

#[test]
fn genuine_usb_host_descriptor_preserves_the_saved_size_word() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    kernel.cpu.mem_map(0x56000000, 4096, Prot::ALL).unwrap();
    kernel.word(0x56000054, 0); // host cable present
    kernel.word(0x300c95e0, 14); // original enumerator's completed state
    kernel.short(0x300c95e6 + 8, 0x03f0);
    kernel.short(0x300c95e6 + 10, 0x1541);
    kernel.cpu.mem_write(0x300c95e6 + 0x20, &[3]).unwrap();
    kernel.word(0x300c9aa2, 64);
    kernel.word(0x300c9aa6, 64);
    let target = 0x20000400;
    kernel.cpu.reg_write(R::R2, target).unwrap();
    kernel.cpu.reg_write(R::R3, 0x12345678).unwrap();
    assert_eq!(kernel.invoke_address(0x300151d4, 0x1234, 257), 1);
    let descriptor = kernel.cpu.mem_read_as_vec(target, 16).unwrap();
    println!("genuine USBHOST descriptor {descriptor:02x?}");
    assert_eq!(
        descriptor,
        [
            0xf0, 3, 0x41, 0x15, 3, 0, 0, 0, 64, 0, 64, 0, 0x78, 0x56, 0x34, 0x12
        ]
    );
    let usb = prime_emulator::services::usb::Usb::default();
    usb.operation(ppl_engine::usb::Operation::ConfigurePeer {
        peer: Some(ppl_engine::usb::Peer {
            vendor: 0x03f0,
            product: 0x1541,
            class: 3,
            receive_bytes: 64,
            transmit_bytes: 64,
            extension: false,
        }),
    })
    .unwrap();
    usb.operation(ppl_engine::usb::Operation::Connect {
        link: ppl_engine::usb::Link::Calculator,
    })
    .unwrap();
    usb.enable(true, true);
    assert_eq!(
        usb.host_descriptor(0x12345678).unwrap().to_vec(),
        descriptor
    );
}

#[test]
fn genuine_rtc_conversion_uses_the_1980_epoch_and_gregorian_leap_days() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    kernel.invoke_address_bounded(0x300001b0, 0, 0, 500_000);
    for (year, month, day, expected) in [
        (1980, 1, 1, 0u32),
        (1980, 1, 2, 86400),
        (2000, 1, 1, 631152000),
        (2024, 3, 1, 1393718400),
    ] {
        let pointer = 0x20000400;
        let fields = [year, month, 0, day, 0, 0, 0, 0];
        for (index, value) in fields.into_iter().enumerate() {
            kernel.short(pointer + index as u64 * 2, value);
        }
        assert_eq!(
            kernel.invoke_address(0x3000a79c, pointer as u32, 0),
            expected
        );
        let rtc = prime_emulator::clock::Rtc::restore(prime_emulator::clock::RtcImage {
            version: 1,
            unix_millis: (prime_emulator::clock::Rtc::UNIX_OFFSET + u64::from(expected)) * 1000,
            elapsed: prime_emulator::clock::ClockImage {
                version: 1,
                elapsed_ns: 0,
            },
        })
        .unwrap();
        assert_eq!(rtc.seconds(), expected);
    }
}

#[test]
fn genuine_indicator_driver_updates_pins_and_pattern_progress_with_rtc_seconds() {
    use std::sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    };
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    kernel.cpu.mem_map(0x56000000, 4096, Prot::ALL).unwrap();
    let seconds = Arc::new(AtomicU16::new(12));
    let observed = seconds.clone();
    kernel
        .cpu
        .add_code_hook(0x3000aa90, 0x3000aa90, move |uc, _, _| {
            let pointer = uc.reg_read(R::R0).unwrap();
            for (index, value) in [1980u16, 1, 0, 1, 0, 0, observed.load(Ordering::Relaxed), 0]
                .into_iter()
                .enumerate()
            {
                uc.mem_write(pointer + index as u64 * 2, &value.to_le_bytes())
                    .unwrap();
            }
            let next = uc.reg_read(R::LR).unwrap();
            uc.reg_write(R::PC, next).unwrap();
        })
        .unwrap();
    // Alarm programming is the other external hardware boundary. Original
    // time conversion, division, table lookup, pin writes and stop tests run.
    kernel
        .cpu
        .add_code_hook(0x3001a1ec, 0x3001a1ec, |uc, _, _| {
            let next = uc.reg_read(R::LR).unwrap();
            uc.reg_write(R::PC, next).unwrap();
        })
        .unwrap();
    let record = 0x20000400;
    let pattern = 0x20000500;
    kernel.cpu.mem_write(pattern, &[1, 2, 4]).unwrap();
    for period in [0u32, 1, 2] {
        for (index, value) in [pattern as u32, 3, period, 5, 10, 0, 0, 0, 0, 0]
            .into_iter()
            .enumerate()
        {
            kernel.word(record + index as u64 * 4, value);
        }
        kernel.word(0x300c4684, record as u32);
        kernel.invoke_address(0x3001a240, 0, 0);
        let step = u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(record + 20, 4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let color = u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x300c4680, 4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        println!("original indicator period={period}: step={step}, color={color}");
        let model = prime_emulator::services::indicator::Pattern {
            owner: 0,
            record: record as u32,
            colors: vec![1, 2, 4],
            period,
            limit: 5,
            start: 10,
            saved: 0,
        };
        assert_eq!(model.sample(12), Some((step, color, false)));
        if period == 2 {
            assert_eq!(step, 1);
            assert_eq!(color, 2);
        }
    }
    kernel.word(record + 8, 2);
    kernel.word(0x300c4684, record as u32);
    seconds.store(20, Ordering::Relaxed);
    kernel.invoke_address(0x3001a240, 0, 0);
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x300c4684, 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        0
    );
    kernel.word(0x2000f000, pattern as u32);
    kernel.word(0x2000f004, 4);
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 256), 0);
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(pattern, 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        4
    );
    kernel.word(pattern, 0x1234);
    kernel.cpu.reg_write(R::R2, pattern).unwrap();
    kernel.cpu.reg_write(R::R3, 4).unwrap();
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 257), 0);
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x56000024, 4)
                .unwrap()
                .try_into()
                .unwrap()
        ) & 0xe0,
        4 << 5
    );
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 258), u32::MAX);
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 259), u32::MAX);
    // Configuration is request 260, not 264. Inactive records validate the
    // header but do not dereference the unused color pointer.
    for (index, value) in [0xdeadbeef, 1, 1, 0, 100, 0, 0, 0, 0, 0]
        .into_iter()
        .enumerate()
    {
        kernel.word(record + index as u64 * 4, value);
    }
    kernel.cpu.reg_write(R::R2, record).unwrap();
    kernel.cpu.reg_write(R::R3, 40).unwrap();
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 260), 0);
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 264), u32::MAX);
    kernel.cpu.reg_write(R::R3, 39).unwrap();
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 260), u32::MAX);
    let sleeps = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let observed = sleeps.clone();
    kernel
        .cpu
        .add_code_hook(0x3000407c, 0x3000407c, move |uc, _, _| {
            observed.fetch_add(1, Ordering::Relaxed);
            uc.reg_write(R::R0, 0).unwrap();
            let next = uc.reg_read(R::LR).unwrap();
            uc.reg_write(R::PC, next).unwrap();
        })
        .unwrap();
    kernel.cpu.mem_write(pattern, &[1, 2]).unwrap();
    for (index, value) in [pattern as u32, 2, 1, 2, 22, 0, 0, 0, 0, 0]
        .into_iter()
        .enumerate()
    {
        kernel.word(record + index as u64 * 4, value);
    }
    kernel.cpu.reg_write(R::R2, record).unwrap();
    kernel.cpu.reg_write(R::R3, 40).unwrap();
    assert_eq!(kernel.invoke_address(0x3001a438, 0, 260), 0);
    assert_eq!(sleeps.load(Ordering::Relaxed), 1);
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x300c4680, 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        0x1234
    );
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(0x300c4684, 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        0
    );
}

#[test]
fn genuine_arch_revision_queries_preserve_raw_pins_and_legacy_mapping() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    kernel.cpu.mem_map(0x56000000, 4096, Prot::ALL).unwrap();
    let output = 0x20000400;
    kernel.word(0x2000f000, output);
    kernel.word(0x2000f004, 4);
    for pins in 0u32..8 {
        kernel.word(0x56000014, pins << 3);
        for request in [260, 261] {
            assert_eq!(kernel.invoke_address(0x30004d68, 0, request), 0);
            let value = u32::from_le_bytes(
                kernel
                    .cpu
                    .mem_read_as_vec(u64::from(output), 4)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(
                value,
                if request == 260 && pins & 3 == 3 {
                    1
                } else {
                    pins & 3
                }
            );
        }
    }
    kernel.word(u64::from(output), 0xa5a5a5a5);
    kernel.word(0x2000f004, 8);
    assert_eq!(kernel.invoke_address(0x30004d68, 0, 260), u32::MAX);
    assert_eq!(
        u32::from_le_bytes(
            kernel
                .cpu
                .mem_read_as_vec(u64::from(output), 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        0xa5a5a5a5
    );
}

#[test]
fn genuine_open_existing_absent_devices_do_not_supply_factory_handles_or_data() {
    let Some(mut kernel) = Kernel::installed() else {
        return;
    };
    kernel.invoke_address_bounded(0x300001b0, 0, 0, 500000);
    // Supply only the stream-record pool allocation/release boundary. Name
    // lookup, initialized device registry and OPEN_EXISTING failure execute.
    kernel
        .cpu
        .add_code_hook(0x3001ddb0, 0x3001ddb0, |cpu, _, _| {
            cpu.mem_write(0x20000400, &[0; 128]).unwrap();
            cpu.reg_write(R::R0, 0x20000400).unwrap();
            let lr = cpu.reg_read(R::LR).unwrap();
            cpu.reg_write(R::PC, lr).unwrap();
        })
        .unwrap();
    kernel
        .cpu
        .add_code_hook(0x3003a2fc, 0x3003a2fc, |cpu, _, _| {
            cpu.reg_write(R::R0, 0).unwrap();
            let lr = cpu.reg_read(R::LR).unwrap();
            cpu.reg_write(R::PC, lr).unwrap();
        })
        .unwrap();
    for name in [r"\\?\FACTORY", r"\\?\FTL", r"\\?\ABSENT"] {
        kernel
            .cpu
            .mem_write(0x20000100, &[name.as_bytes(), &[0]].concat())
            .unwrap();
        kernel.cpu.reg_write(R::R2, 0).unwrap();
        kernel.cpu.reg_write(R::R3, 0).unwrap();
        kernel.word(0x2000f000, 3);
        kernel.word(0x2000f004, 0);
        assert_eq!(kernel.invoke(0x2a7, 0x20000100, 0), u32::MAX, "{name}");
    }
}
