//! One monotonic guest clock. Power and bounded inspection pauses stop it;
//! image restoration retains guest elapsed time and excludes host downtime.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Debug)]
struct State {
    elapsed: Duration,
    last: Instant,
    pauses: BTreeMap<u64, Option<Instant>>,
    next: u64,
}
#[derive(Debug)]
pub struct Clock {
    state: Mutex<State>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockImage {
    pub version: u32,
    pub elapsed_ns: u64,
}
#[derive(Debug)]
pub struct Freeze {
    clock: Arc<Clock>,
    id: u64,
}
impl Default for Clock {
    fn default() -> Self {
        Self::with_elapsed(Duration::ZERO)
    }
}
impl Clock {
    fn with_elapsed(elapsed: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                elapsed,
                last: Instant::now(),
                pauses: BTreeMap::new(),
                next: 1,
            }),
        }
    }
    pub fn now(&self) -> Duration {
        let mut state = self.state.lock().unwrap();
        Self::advance(&mut state, Instant::now());
        state.elapsed
    }
    fn advance(state: &mut State, now: Instant) {
        // All current pauses began at or before `last`. Their union extends
        // to the latest deadline, or indefinitely for a power pause.
        let active_from = if state.pauses.values().any(Option::is_none) {
            now
        } else {
            state
                .pauses
                .values()
                .flatten()
                .copied()
                .max()
                .unwrap_or(state.last)
                .clamp(state.last, now)
        };
        state.elapsed += now.saturating_duration_since(active_from);
        state.last = now;
        state
            .pauses
            .retain(|_, until| until.is_none_or(|until| until > now));
    }
    pub fn freeze(self: &Arc<Self>, until: Option<Instant>) -> Freeze {
        let mut state = self.state.lock().unwrap();
        Self::advance(&mut state, Instant::now());
        let id = state.next;
        state.next = state
            .next
            .checked_add(1)
            .expect("guest clock pause identities exhausted");
        state.pauses.insert(id, until);
        Freeze {
            clock: self.clone(),
            id,
        }
    }
    pub fn image(&self) -> Result<ClockImage, String> {
        let elapsed_ns = u64::try_from(self.now().as_nanos())
            .map_err(|_| "Guest elapsed clock exceeds image range")?;
        Ok(ClockImage {
            version: 1,
            elapsed_ns,
        })
    }
    /// The whole-machine restorer separately reinstates power and owner state.
    /// Inspection leases belong to the old process and are never resurrected.
    pub fn restore(image: ClockImage) -> Result<Self, String> {
        if image.version != 1 {
            return Err("Unsupported guest clock image".into());
        }
        Ok(Self::with_elapsed(Duration::from_nanos(image.elapsed_ns)))
    }
}
impl Drop for Freeze {
    fn drop(&mut self) {
        let mut state = self.clock.state.lock().unwrap();
        Clock::advance(&mut state, Instant::now());
        state.pauses.remove(&self.id);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_power_and_inspection_pauses_preserve_guest_time_and_restore_offset() {
        let clock = Arc::new(Clock::default());
        let first = clock.freeze(None);
        let at = clock.now();
        let inspection = clock.freeze(Some(Instant::now() + Duration::from_secs(1)));
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(clock.now(), at);
        drop(first);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(clock.now(), at);
        let image = clock.image().unwrap();
        let decoded = serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
        let startup = Instant::now();
        let restored = Arc::new(Clock::restore(decoded).unwrap());
        let freeze = restored.freeze(None);
        assert!(restored.now() >= at);
        assert!(restored.now() <= at + startup.elapsed());
        drop(freeze);
        drop(inspection);
        std::thread::sleep(Duration::from_millis(10));
        assert!(clock.now() >= at + Duration::from_millis(10));
        assert!(
            Clock::restore(ClockImage {
                version: 2,
                elapsed_ns: 0
            })
            .is_err()
        );
    }
    #[test]
    fn expiring_inspection_does_not_freeze_time_after_its_deadline() {
        let clock = Arc::new(Clock::default());
        let lease = clock.freeze(Some(Instant::now() + Duration::from_millis(15)));
        let paused = clock.now();
        std::thread::sleep(Duration::from_millis(35));
        assert!(clock.now() >= paused + Duration::from_millis(15));
        let before = clock.now();
        drop(lease);
        assert!(clock.now() >= before);
    }
}

/// Calculator RTC continues during power sleep, while inspection pauses it.
/// It is initialized from host UTC once; no host clock or physical RTC is changed.
#[derive(Debug)]
pub struct Rtc {
    pub clock: Arc<Clock>,
    unix_millis: u64,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RtcImage {
    pub version: u32,
    pub unix_millis: u64,
    pub elapsed: ClockImage,
}
impl Default for Rtc {
    fn default() -> Self {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("RTC initialization requires a host UTC date after 1970")
            .as_millis();
        Self {
            clock: Arc::new(Clock::default()),
            unix_millis: u64::try_from(millis).expect("host UTC date exceeds RTC image range"),
        }
    }
}
impl Rtc {
    /// The original OS conversion at 0x3000A79C uses 1980-01-01 as zero.
    pub const UNIX_OFFSET: u64 = 315_532_800;
    pub fn unix_time(&self) -> Duration {
        Duration::from_millis(self.unix_millis) + self.clock.now()
    }
    pub fn seconds(&self) -> u32 {
        self.unix_time().as_secs().saturating_sub(Self::UNIX_OFFSET) as u32
    }
    pub fn image(&self) -> Result<RtcImage, String> {
        Ok(RtcImage {
            version: 1,
            unix_millis: self.unix_millis,
            elapsed: self.clock.image()?,
        })
    }
    pub fn restore(image: RtcImage) -> Result<Self, String> {
        if image.version != 1
            || image.unix_millis < Self::UNIX_OFFSET * 1000
            || image.unix_millis > (Self::UNIX_OFFSET + u64::from(u32::MAX)) * 1000
        {
            return Err("RTC image version or calendar origin is invalid".into());
        }
        Ok(Self {
            clock: Arc::new(Clock::restore(image.elapsed)?),
            unix_millis: image.unix_millis,
        })
    }
}

#[cfg(test)]
mod rtc_tests {
    use super::*;
    #[test]
    fn rtc_advances_in_power_sleep_but_all_calculator_clocks_freeze_for_inspection() {
        let shared = crate::services::SharedState::new(std::path::PathBuf::new(), String::new());
        shared.enter_sleep();
        let elapsed = shared.clock.now();
        let rtc = shared.rtc.clock.now();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(shared.clock.now(), elapsed);
        assert!(shared.rtc.clock.now() > rtc);
        shared.leave_sleep();
        let lease = shared.safe_points.pause(Duration::from_secs(1)).unwrap();
        let elapsed = shared.clock.now();
        let rtc = shared.rtc.clock.now();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(shared.clock.now(), elapsed);
        assert_eq!(shared.rtc.clock.now(), rtc);
        let image = shared.rtc.image().unwrap();
        drop(lease);
        std::thread::sleep(Duration::from_millis(15));
        let restored = Rtc::restore(image).unwrap();
        assert!(shared.rtc.unix_time() > restored.unix_time());
    }
    #[test]
    fn transient_sleep_cannot_be_mistaken_for_a_completed_firmware_save() {
        let shared = crate::services::SharedState::new(std::path::PathBuf::new(), String::new());
        shared.power.lock().unwrap().auto.configure(3, 0);
        assert!(!shared.power_execution_allowed(false));
        assert!(shared.sleeping());
        assert!(!shared.firmware_sleeping());
        shared.push_key_press(crate::services::ON_KEY_CODE);
        assert!(shared.power_execution_allowed(false));
        shared.enter_indicator_sleep();
        assert!(shared.sleeping());
        assert!(!shared.firmware_sleeping());
        shared.finish_indicator_sleep();
        assert!(!shared.sleeping());
        shared.enter_sleep();
        assert!(shared.firmware_sleeping());
    }
}
