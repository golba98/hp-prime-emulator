//! Cooperative single-core scheduling at bounded Unicorn slice boundaries.
//! OS slots are unique (0..63); automatic allocation uses 16..54, as in the
//! installed ROM. Lower slots run first. An executing owner gets the ROM's
//! (1 + (64-slot)/16) five-millisecond quantum before other ready slots run.
//! Host scheduling may delay a boundary; this is not cycle-accurate timing.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    slots: BTreeMap<u32, u16>,
    requests: BTreeSet<u32>,
    used: BTreeMap<u32, Duration>,
    active: Option<u32>,
}
/// Ready owners and consumed quanta at a CPU boundary. An executing permit
/// belongs to the old process and is never serialized.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    version: u32,
    slots: BTreeMap<u32, u16>,
    requests: BTreeSet<u32>,
    used: BTreeMap<u32, Duration>,
}
impl Image {
    pub fn restore(&self) -> Result<Scheduler, String> {
        let unique: BTreeSet<_> = self.slots.values().copied().collect();
        if self.version != 1
            || !self.slots.contains_key(&0)
            || self.slots.len() > 64
            || unique.len() != self.slots.len()
            || unique.iter().any(|slot| *slot > 63)
            || self.requests.iter().any(|id| !self.slots.contains_key(id))
            || self.used.keys().any(|id| !self.slots.contains_key(id))
        {
            return Err("Invalid scheduler owner, slot or ready-queue image".into());
        }
        Ok(Scheduler {
            state: Mutex::new(State {
                slots: self.slots.clone(),
                requests: self.requests.clone(),
                used: self.used.clone(),
                active: None,
            }),
            released: Condvar::new(),
        })
    }
    pub fn owners(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.keys().copied()
    }
}
pub struct Scheduler {
    state: Mutex<State>,
    released: Condvar,
}
impl Default for Scheduler {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                slots: BTreeMap::from([(0, 16)]),
                ..Default::default()
            }),
            released: Condvar::new(),
        }
    }
}
impl Scheduler {
    pub fn image(&self) -> Result<Image, String> {
        let state = self.state.lock().unwrap();
        if state.active.is_some() {
            return Err("Scheduler capture requires every executing permit to return".into());
        }
        Ok(Image {
            version: 1,
            slots: state.slots.clone(),
            requests: state.requests.clone(),
            used: state.used.clone(),
        })
    }
    pub fn register(&self, thread: u32) -> Option<u16> {
        let mut state = self.state.lock().unwrap();
        if let Some(slot) = state.slots.get(&thread) {
            return Some(*slot);
        }
        let slot = (16..55).find(|slot| !state.slots.values().any(|used| used == slot))?;
        state.slots.insert(thread, slot);
        Some(slot)
    }
    pub fn priority(&self, thread: u32) -> Option<u16> {
        self.state.lock().unwrap().slots.get(&thread).copied()
    }
    pub fn set_priority(&self, thread: u32, slot: u16) -> bool {
        let mut state = self.state.lock().unwrap();
        if slot > 63
            || !state.slots.contains_key(&thread)
            || state.slots.values().any(|used| *used == slot)
        {
            return false;
        }
        state.slots.insert(thread, slot);
        if state.active.is_none() {
            self.released.notify_all();
        }
        true
    }
    pub fn retire(&self, thread: u32) {
        let mut state = self.state.lock().unwrap();
        state.slots.remove(&thread);
        state.requests.remove(&thread);
        state.used.remove(&thread);
        if state.active.is_none() {
            self.released.notify_all();
        }
    }
    pub fn cancel_request(&self, thread: u32) {
        let mut state = self.state.lock().unwrap();
        state.requests.remove(&thread);
        state.used.remove(&thread);
        if state.active.is_none() {
            self.released.notify_all();
        }
    }
    pub fn try_enter(self: &Arc<Self>, thread: u32, interrupt: bool) -> Option<Permit> {
        self.try_enter_locked(&mut self.state.lock().unwrap(), thread, interrupt)
    }

    /// Wait briefly on contention, then let the CPU recheck its boundary.
    ///
    /// A scheduler change can wake a guest before the polling interval. The
    /// guest always checks safe points and the power gate before a fresh try.
    pub fn enter_slice(self: &Arc<Self>, thread: u32) -> Option<Permit> {
        let mut state = self.state.lock().unwrap();
        if let Some(permit) = self.try_enter_locked(&mut state, thread, false) {
            return Some(permit);
        }
        drop(
            self.released
                .wait_timeout(state, Duration::from_millis(1))
                .unwrap(),
        );
        None
    }

    fn try_enter_locked(
        self: &Arc<Self>,
        state: &mut State,
        thread: u32,
        interrupt: bool,
    ) -> Option<Permit> {
        if !state.slots.contains_key(&thread) {
            return None;
        }
        state.requests.insert(thread);
        if state.active.is_some() {
            return None;
        }
        if !interrupt {
            let eligible = |state: &State| {
                state
                    .requests
                    .iter()
                    .filter(|owner| {
                        let slot = state.slots[owner];
                        state.used.get(owner).copied().unwrap_or_default() < quantum(slot)
                    })
                    .min_by_key(|owner| state.slots[owner])
                    .copied()
            };
            let next = if let Some(owner) = eligible(state) {
                owner
            } else {
                let ready: Vec<_> = state.requests.iter().copied().collect();
                for owner in ready {
                    state.used.insert(owner, Duration::ZERO);
                }
                eligible(state)?
            };
            if next != thread {
                return None;
            }
        }
        state.active = Some(thread);
        Some(Permit {
            scheduler: self.clone(),
            thread,
            start: Instant::now(),
            interrupt,
        })
    }
}
fn quantum(slot: u16) -> Duration {
    Duration::from_millis(u64::from(1 + (64 - slot) / 16) * 5)
}
pub struct Permit {
    scheduler: Arc<Scheduler>,
    thread: u32,
    start: Instant,
    interrupt: bool,
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.scheduler.state.lock().unwrap();
        debug_assert_eq!(state.active, Some(self.thread));
        state.active = None;
        state.requests.remove(&self.thread);
        if !self.interrupt {
            *state.used.entry(self.thread).or_default() += self.start.elapsed();
        }
        self.scheduler.released.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_are_unique_and_reusable_with_rom_allocation_limits() {
        let scheduler = Scheduler::default();
        assert_eq!(scheduler.register(1), Some(17));
        assert!(!scheduler.set_priority(1, 16));
        assert!(!scheduler.set_priority(1, 64));
        assert!(scheduler.set_priority(1, 2));
        assert_eq!(scheduler.priority(1), Some(2));
        assert!(!scheduler.set_priority(1, 2));
        scheduler.retire(1);
        assert_eq!(scheduler.register(2), Some(17));
        for thread in 3..40 {
            assert!(scheduler.register(thread).is_some());
        }
        assert!(scheduler.register(40).is_none());
    }
    #[test]
    fn queued_priority_changes_and_blocked_owners_affect_actual_grants() {
        let scheduler = Arc::new(Scheduler::default());
        scheduler.register(1);
        scheduler.register(2);
        let main = scheduler.try_enter(0, false).unwrap();
        assert!(scheduler.try_enter(2, false).is_none());
        assert!(scheduler.try_enter(1, false).is_none());
        scheduler.cancel_request(0);
        drop(main);
        assert!(scheduler.try_enter(2, false).is_none());
        let first = scheduler.try_enter(1, false).unwrap();
        assert!(
            scheduler.try_enter(2, true).is_none(),
            "ISR cannot concurrently write shared guest RAM"
        );
        scheduler.cancel_request(1);
        drop(first);
        let second = scheduler.try_enter(2, false).unwrap();
        drop(second);
        scheduler.cancel_request(2);
        assert!(scheduler.try_enter(0, false).is_some());
    }
}
