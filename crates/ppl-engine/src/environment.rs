//! Explicit virtual calculator inputs, independent of physical USB/hardware.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryPower {
    #[default]
    Battery,
    Charging,
    Charged,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatteryConfig {
    pub adc: u16,
    pub power: BatteryPower,
}
impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            adc: 1000,
            power: BatteryPower::Battery,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentConfig {
    pub battery: BatteryConfig,
    pub storage_capacity_bytes: u64,
    /// Virtual board-identification GPIO pins (two bits, not host hardware).
    pub board_revision_pins: u8,
}
impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            battery: BatteryConfig::default(),
            storage_capacity_bytes: 256 * 1024 * 1024,
            board_revision_pins: 0,
        }
    }
}
impl EnvironmentConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.board_revision_pins > 3 {
            return Err("Board revision pins must be a two-bit value".into());
        }
        if self.battery.adc > 1023 {
            return Err("Battery ADC must be a 10-bit sample (0–1023)".into());
        }
        if self.storage_capacity_bytes == 0 {
            return Err("Internal storage capacity must be positive".into());
        }
        Ok(())
    }
}

pub const BATTERY_VOLTS_PER_SAMPLE: f32 = f32::from_bits(0x3b954a80);
impl BatteryConfig {
    pub fn voltage(self) -> f32 {
        f32::from(if self.adc <= 64 { 930 } else { self.adc }) * BATTERY_VOLTS_PER_SAMPLE
    }
}

/// Outputs driven by the original guest's virtual device calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeripheralState {
    pub indicator: u32,
    pub backlight_duty: u32,
    pub backlight_on: bool,
}
