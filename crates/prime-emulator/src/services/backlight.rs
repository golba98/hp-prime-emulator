//! Virtual backlight controller: original driver 0x3000CE84, level table
//! 0x30082494 and asynchronous unit PWM ramp 0x3000CCDC.
use std::time::Duration;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backlight {
    pub mode: u32,
    pub level: u32,
    pub duty: u32,
    pub timeout: u32,
    pub option: u32,
    pub target: u32,
    pub pwm: u32,
    pub brightness: u16,
    pub last_step: Duration,
    pub ramping: bool,
}
impl Default for Backlight {
    fn default() -> Self {
        Self {
            mode: 4,
            level: 2,
            duty: duty(2),
            timeout: 30,
            option: 0,
            target: duty(2),
            pwm: 0,
            brightness: 2,
            last_step: Duration::ZERO,
            ramping: false,
        }
    }
}
pub fn duty(level: u32) -> u32 {
    [2, 10, 30, 50][level.min(3) as usize] * 7
}
impl Backlight {
    pub fn open(&mut self) {
        self.mode = 0;
        self.pwm = self.duty;
    }
    pub fn set_brightness(&mut self, value: Option<u16>, inhibited: bool) -> u32 {
        if let Some(value) = value.filter(|&value| value < 4) {
            self.brightness = value;
            self.level = u32::from(value);
            self.duty = duty(self.level);
            if self.mode != 4 && !inhibited {
                self.pwm = self.duty;
            }
        }
        u32::from(self.brightness)
    }
    pub fn poll(&mut self, now: Duration, inhibited: bool) {
        if !self.ramping {
            return;
        }
        let step = Duration::from_millis(if self.target >= self.duty { 2 } else { 10 });
        let elapsed = now.saturating_sub(self.last_step);
        let count = (elapsed.as_nanos() / step.as_nanos()).min(u128::from(u32::MAX)) as u32;
        let change = count.min(self.duty.abs_diff(self.target));
        if self.target >= self.duty {
            self.duty += change;
        } else {
            self.duty -= change;
        }
        self.last_step += step * change;
        if self.mode != 4 && !inhibited {
            self.pwm = self.duty;
        }
        self.ramping = self.duty != self.target;
    }
}
