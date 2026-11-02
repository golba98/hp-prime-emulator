//! Bounded editing-key repeat, driven by the existing session tick.
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct KeyRepeat {
    active: Option<(u16, Instant)>,
    repress: Option<u16>,
}
impl KeyRepeat {
    pub fn press(&mut self, code: u16, now: Instant) {
        self.repress = None;
        // Device scan codes: arrows and backspace only. Never repeat launch,
        // power, Enter, or modifier actions.
        if matches!(code, 2..=5 | 0x0c) {
            self.active = Some((code, now + Duration::from_millis(450)));
        } else {
            self.clear();
        }
    }
    pub fn release(&mut self, code: u16) {
        if self.active.is_some_and(|(held, _)| held == code) {
            self.clear();
        }
    }
    pub fn clear(&mut self) {
        self.active = None;
        self.repress = None;
    }
    pub fn poll(&mut self, now: Instant) -> Option<(u16, bool)> {
        if let Some(code) = self.repress.take() {
            self.active = Some((code, now + Duration::from_millis(50)));
            return Some((code, true));
        }
        let (code, due) = self.active?;
        if now < due {
            return None;
        }
        self.active = Some((code, now + Duration::from_millis(50)));
        self.repress = Some(code);
        Some((code, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delay_rate_release_and_no_catch_up() {
        let mut r = KeyRepeat::default();
        let t = Instant::now();
        r.press(12, t);
        assert_eq!(r.poll(t + Duration::from_millis(449)), None);
        assert_eq!(r.poll(t + Duration::from_millis(450)), Some((12, false)));
        assert_eq!(r.poll(t + Duration::from_millis(451)), Some((12, true)));
        assert_eq!(r.poll(t + Duration::from_millis(499)), None);
        assert_eq!(r.poll(t + Duration::from_secs(5)), Some((12, false)));
        assert_eq!(r.poll(t + Duration::from_secs(5)), Some((12, true)));
        assert_eq!(r.poll(t + Duration::from_secs(5)), None);
        r.release(12);
        assert_eq!(r.poll(t + Duration::from_secs(6)), None);
    }
    #[test]
    fn release_between_repeat_transitions_cancels_pending_press() {
        let mut r = KeyRepeat::default();
        let t = Instant::now();
        r.press(12, t);
        assert_eq!(r.poll(t + Duration::from_secs(1)), Some((12, false)));
        r.release(12);
        assert_eq!(r.poll(t + Duration::from_secs(2)), None);
    }
    #[test]
    fn launch_modifiers_and_focus_loss_never_repeat() {
        let t = Instant::now();
        for code in [0x83, 0x8b, 0xb6, 0x0d] {
            let mut r = KeyRepeat::default();
            r.press(2, t);
            r.press(code, t);
            assert_eq!(r.poll(t + Duration::from_secs(1)), None);
        }
        let mut r = KeyRepeat::default();
        r.press(3, t);
        r.clear();
        assert_eq!(r.poll(t + Duration::from_secs(1)), None);
    }
}
