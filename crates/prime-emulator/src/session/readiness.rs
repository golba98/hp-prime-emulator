//! Boot input readiness is not the sustained quiescence required to save state.
//! A renderer can remain responsive while spending more than 20% of wall time
//! painting history. Require actual completed pump cycles instead of treating
//! the interrupt/checkpoint idle-share threshold as a boot-success condition.

#[derive(Clone, Copy)]
pub struct Sample {
    pub valid_frame: bool,
    pub idle: bool,
    pub main_waits: u64,
    pub pump_entries: u64,
    pub timer_deliveries: u64,
}

#[derive(Default)]
pub struct BootReadiness {
    baseline: Option<Sample>,
}
impl BootReadiness {
    /// Two normal timer deliveries must be observed after the first valid LCD,
    /// with pump reentry and a subsequent main-thread wait. No extra keys,
    /// sleeps, synthetic timer bursts or persistence-policy changes are used.
    pub fn observe(&mut self, sample: Sample) -> bool {
        if !sample.valid_frame {
            self.baseline = None;
            return false;
        }
        let Some(base) = self.baseline else {
            self.baseline = Some(sample);
            return false;
        };
        sample.idle
            && sample.main_waits > base.main_waits
            && sample.pump_entries.saturating_sub(base.pump_entries) >= 2
            && sample
                .timer_deliveries
                .saturating_sub(base.timer_deliveries)
                >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Sample {
        Sample {
            valid_frame: true,
            idle: true,
            main_waits: 10,
            pump_entries: 10,
            timer_deliveries: 10,
        }
    }
    #[test]
    fn responsive_busy_renderer_does_not_need_a_machine_speed_dependent_idle_share() {
        let mut gate = BootReadiness::default();
        let mut s = sample();
        assert!(!gate.observe(s));
        for _ in 0..100 {
            s.idle = false;
            assert!(!gate.observe(s));
        }
        s.main_waits += 1;
        s.pump_entries += 2;
        s.timer_deliveries += 2;
        s.idle = true;
        assert!(gate.observe(s));
    }
    #[test]
    fn a_frame_or_timer_delivery_alone_cannot_claim_ready() {
        let mut gate = BootReadiness::default();
        let mut s = sample();
        assert!(!gate.observe(s));
        s.timer_deliveries += 2;
        assert!(!gate.observe(s));
        s.pump_entries += 2;
        assert!(!gate.observe(s));
        s.main_waits += 1;
        s.idle = false;
        assert!(!gate.observe(s));
        s.idle = true;
        assert!(gate.observe(s));
    }
    #[test]
    fn invalid_display_or_restarted_counters_requires_fresh_evidence() {
        let mut gate = BootReadiness::default();
        let mut s = sample();
        assert!(!gate.observe(s));
        s.valid_frame = false;
        s.timer_deliveries += 9;
        assert!(!gate.observe(s));
        s.valid_frame = true;
        assert!(!gate.observe(s));
        s.main_waits = 0;
        s.pump_entries = 0;
        s.timer_deliveries = 0;
        assert!(!gate.observe(s));
    }
}
