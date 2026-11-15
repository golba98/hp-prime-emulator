//! OS auto-power-off state, derived from the installed ROM's 0x30004A50
//! mode switch and one-second counter at 0x3000B248. The calculator normally
//! disables this OS timer and manages its own OFF workflow.
use std::{sync::Arc, time::Duration};

#[derive(Debug)]
pub struct AutoPowerOff {
    pub(super) system_mode: u16,
    pub(super) enabled: bool,
    pub(super) interval_seconds: u16,
    /// Original PowerData +0x2c/+0x30, exposed by system variables 86/87.
    pub(super) idle_inhibit: u32,
    pub(super) shutdown_inhibit: u32,
    pub(super) brightness_inhibit: u32,
    elapsed_seconds: u16,
    last_tick: Duration,
    clock: Arc<crate::clock::Clock>,
    requested: bool,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    system_mode: u16,
    enabled: bool,
    interval_seconds: u16,
    idle_inhibit: u32,
    shutdown_inhibit: u32,
    brightness_inhibit: u32,
    elapsed_seconds: u16,
    last_tick: Duration,
    requested: bool,
}
impl Image {
    pub fn restore(&self, clock: Arc<crate::clock::Clock>) -> Result<AutoPowerOff, String> {
        if self.last_tick > clock.now() {
            return Err("Auto-power tick is later than the reconstructed guest clock".into());
        }
        Ok(AutoPowerOff {
            system_mode: self.system_mode,
            enabled: self.enabled,
            interval_seconds: self.interval_seconds,
            idle_inhibit: self.idle_inhibit,
            shutdown_inhibit: self.shutdown_inhibit,
            brightness_inhibit: self.brightness_inhibit,
            elapsed_seconds: self.elapsed_seconds,
            last_tick: self.last_tick,
            requested: self.requested,
            clock,
        })
    }
}
impl Default for AutoPowerOff {
    fn default() -> Self {
        Self {
            system_mode: 0,
            enabled: false,
            interval_seconds: 0,
            idle_inhibit: 0,
            shutdown_inhibit: 0,
            brightness_inhibit: 0,
            elapsed_seconds: 0,
            last_tick: Duration::ZERO,
            clock: Arc::new(crate::clock::Clock::default()),
            requested: false,
        }
    }
}
impl AutoPowerOff {
    pub fn image(&self) -> Image {
        Image {
            system_mode: self.system_mode,
            enabled: self.enabled,
            interval_seconds: self.interval_seconds,
            idle_inhibit: self.idle_inhibit,
            shutdown_inhibit: self.shutdown_inhibit,
            brightness_inhibit: self.brightness_inhibit,
            elapsed_seconds: self.elapsed_seconds,
            last_tick: self.last_tick,
            requested: self.requested,
        }
    }
    pub fn with_clock(clock: Arc<crate::clock::Clock>) -> Self {
        Self {
            last_tick: clock.now(),
            clock,
            ..Self::default()
        }
    }
    pub fn configure(&mut self, mode: u32, minutes: u32) -> u32 {
        let previous = u32::from(self.enabled);
        match mode {
            0 => {
                self.enabled = false;
                self.reset_unless_at_limit();
            }
            1 => {
                if self.system_mode != 0xda {
                    self.enabled = true;
                }
                self.reset_unless_at_limit();
            }
            2 => {
                if self.system_mode != 0xda {
                    self.enabled = true;
                }
                self.interval_seconds = minutes.wrapping_mul(60) as u16;
                self.reset_activity();
            }
            3 if self.system_mode != 0xda => self.requested = true,
            _ => {}
        }
        previous
    }
    /// BatteryLowCheck posts the power-thread shutdown message and returns.
    /// The original admission check excludes exactly the value 1, independently
    /// of the normal auto-power enable and system-mode settings.
    pub(super) fn request_critical_shutdown(&mut self) {
        if self.shutdown_inhibit != 1 {
            self.requested = true;
        }
    }
    fn reset_unless_at_limit(&mut self) {
        if self.elapsed_seconds != self.interval_seconds {
            self.reset_activity();
        }
    }
    pub(super) fn reset_activity(&mut self) {
        self.elapsed_seconds = 0;
        self.last_tick = self.clock.now();
    }
    pub(super) fn woke(&mut self) {
        self.interval_seconds = self.interval_seconds.clamp(60, 1800);
        self.reset_activity();
    }
    pub(super) fn poll(&mut self) -> bool {
        self.advance(self.clock.now().saturating_sub(self.last_tick))
    }
    fn advance(&mut self, elapsed: Duration) -> bool {
        let seconds = elapsed.as_secs();
        if seconds != 0 {
            self.last_tick += Duration::from_secs(seconds);
            if self.enabled && self.idle_inhibit == 0 {
                // The ROM counter is u16. Stop at the first reaching tick;
                // skipping host time must not wrap past an expiry unnoticed.
                let until =
                    u64::from(self.interval_seconds.saturating_sub(self.elapsed_seconds)).max(1);
                if seconds >= until {
                    self.requested = true;
                } else {
                    self.elapsed_seconds = self.elapsed_seconds.wrapping_add(seconds as u16);
                }
            }
        }
        std::mem::take(&mut self.requested)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_return_values_wrapping_interval_activity_and_wake_bounds() {
        let mut power = AutoPowerOff::default();
        assert_eq!(power.configure(2, 7), 0);
        assert_eq!(power.interval_seconds, 420);
        assert_eq!(power.configure(0, 0), 1);
        assert!(!power.advance(Duration::from_secs(1000)));
        assert_eq!(power.configure(2, 1100), 0);
        assert_eq!(power.interval_seconds, 464);
        assert!(!power.advance(Duration::from_secs(463)));
        assert!(power.advance(Duration::from_secs(1)));
        power.woke();
        assert!(!power.advance(Duration::from_secs(463)));
        power.reset_activity();
        assert!(!power.advance(Duration::from_secs(463)));
        assert!(power.advance(Duration::from_secs(1)));
        power.configure(2, 0);
        power.woke();
        assert_eq!(power.interval_seconds, 60);
        power.configure(2, 60);
        power.woke();
        assert_eq!(power.interval_seconds, 1800);
    }
    #[test]
    fn restricted_system_mode_and_one_shot_request_follow_rom_branches() {
        let mut power = AutoPowerOff {
            system_mode: 0xda,
            ..Default::default()
        };
        assert_eq!(power.configure(1, 0), 0);
        assert!(!power.enabled);
        power.configure(2, 3);
        assert_eq!(power.interval_seconds, 180);
        assert!(!power.enabled);
        power.configure(3, 0);
        assert!(!power.poll());
        power.system_mode = 0;
        power.configure(3, 0);
        assert!(power.poll());
        assert!(!power.poll());
        power.configure(99, 0);
        assert!(!power.enabled);
    }
}
