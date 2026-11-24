//! Software battery-driver contract from OS ROM 0x3000D278/0x3000D5B0.
//! Voltage arithmetic/thresholds are the original f32 operations. Configured
//! ADC and power pins describe a virtual device; no host battery/USB is opened.
use ppl_engine::environment::{BatteryConfig, BatteryPower};
pub fn voltage(config: BatteryConfig) -> f32 {
    config.voltage()
}
pub fn level(config: BatteryConfig) -> i16 {
    let volts = voltage(config);
    if volts > f32::from_bits(0x40766666) {
        4
    } else if volts > f32::from_bits(0x40700000) {
        3
    } else if volts >= f32::from_bits(0x4069999a) {
        2
    } else if volts > f32::from_bits(0x40633333) {
        1
    } else if volts >= f32::from_bits(0x405ccccd) {
        0
    } else {
        -1
    }
}
pub fn kind(config: BatteryConfig) -> u8 {
    if config.power == BatteryPower::Battery {
        1
    } else {
        4
    }
}
pub fn low_check(config: BatteryConfig) -> i32 {
    if config.power != BatteryPower::Battery {
        return 0;
    }
    let volts = voltage(config);
    if volts <= f32::from_bits(0x405ccccd) {
        -2
    } else if volts < f32::from_bits(0x40633333) {
        -1
    } else {
        0
    }
}
pub fn query(config: BatteryConfig, mut output: [u8; 16]) -> ([u8; 16], u32) {
    // Selector(+4) and driver-reserved bytes(+8..9,+15) are caller-owned.
    output[..4].copy_from_slice(&voltage(config).to_bits().to_le_bytes());
    output[6..8].copy_from_slice(&config.adc.to_le_bytes());
    output[10..12].copy_from_slice(&level(config).to_le_bytes());
    output[12] = 1;
    output[13] = kind(config);
    output[14] = match config.power {
        BatteryPower::Battery => 0,
        _ if voltage(config) < 3.0 => 255,
        BatteryPower::Charging => 1,
        BatteryPower::Charged => 2,
    };
    (output, u32::from(config.adc))
}
