//! Excludes LCD snapshots from every guest engine, including suspended SVC calls.
//! Captures never wait for an engine: a busy capture is retried at the next safe point.
//! Captures busy for [`STARVED_AFTER`] request one: for [`REQUEST_HOLD`],
//! engines finishing a slice do not start another, so a guest that never
//! parks (a polling `WAIT(-1)` or `GETKEY` loop) cannot starve captures. A
//! guest that parks normally is never held. The hold expires on its own, so
//! an abandoned request delays the guest by at most that long.
use std::{
    collections::HashSet,
    sync::{Condvar, Mutex, MutexGuard},
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
pub const REQUEST_HOLD: Duration = Duration::from_millis(20);
pub const STARVED_AFTER: Duration = Duration::from_millis(100);
#[derive(Default)]
struct State {
    active: HashSet<ThreadId>,
    capturing: bool,
    requested: Option<Instant>,
    /// First busy capture attempt since the last successful capture.
    busy_since: Option<Instant>,
}
impl State {
    fn held(&self) -> Option<Duration> {
        if self.capturing {
            return Some(REQUEST_HOLD);
        }
        self.requested
            .and_then(|t| REQUEST_HOLD.checked_sub(t.elapsed()))
            .filter(|d| !d.is_zero())
    }
}
#[derive(Default)]
pub struct Gate {
    state: Mutex<State>,
    wake: Condvar,
}
pub struct Active<'a> {
    gate: &'a Gate,
    owned: bool,
}
pub struct Parked<'a> {
    gate: &'a Gate,
    owned: bool,
}
pub struct Snapshot<'a> {
    gate: &'a Gate,
}
impl Gate {
    /// Waits while a capture runs or a fresh request is pending. A thread
    /// that is already active (nested entry) never waits.
    fn wait_entry<'a>(&'a self, mut state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        if state.active.contains(&thread::current().id()) {
            return state;
        }
        while let Some(wait) = state.held() {
            state = self.wake.wait_timeout(state, wait).unwrap().0;
        }
        state
    }
    pub fn enter(&self) -> Active<'_> {
        let mut state = self.wait_entry(self.state.lock().unwrap());
        let owned = state.active.insert(thread::current().id());
        Active { gate: self, owned }
    }
    pub fn park(&self) -> Parked<'_> {
        let owned = self
            .state
            .lock()
            .unwrap()
            .active
            .remove(&thread::current().id());
        Parked { gate: self, owned }
    }
    pub fn try_snapshot(&self) -> Option<Snapshot<'_>> {
        self.try_snapshot_inner(false)
    }
    pub fn try_snapshot_from_hook(&self) -> Option<Snapshot<'_>> {
        self.try_snapshot_inner(true)
    }
    fn try_snapshot_inner(&self, from_hook: bool) -> Option<Snapshot<'_>> {
        let mut state = self.state.try_lock().ok()?;
        let owner_only =
            from_hook && state.active.len() == 1 && state.active.contains(&thread::current().id());
        if state.capturing {
            return None;
        }
        if !state.active.is_empty() && !owner_only {
            if !from_hook {
                let busy_since = *state.busy_since.get_or_insert_with(Instant::now);
                if busy_since.elapsed() >= STARVED_AFTER
                    && state.requested.is_none_or(|t| t.elapsed() >= REQUEST_HOLD)
                {
                    state.requested = Some(Instant::now());
                }
            }
            return None;
        }
        state.capturing = true;
        state.requested = None;
        state.busy_since = None;
        Some(Snapshot { gate: self })
    }
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if self.owned {
            self.gate
                .state
                .lock()
                .unwrap()
                .active
                .remove(&thread::current().id());
        }
    }
}
impl Drop for Parked<'_> {
    fn drop(&mut self) {
        if self.owned {
            let mut state = self.gate.wait_entry(self.gate.state.lock().unwrap());
            state.active.insert(thread::current().id());
        }
    }
}
impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        self.gate.state.lock().unwrap().capturing = false;
        self.gate.wake.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_exclude_active_guest_but_allow_parked_waits() {
        let gate = Gate::default();
        let active = gate.enter();
        assert!(gate.try_snapshot().is_none());
        {
            let parked = gate.park();
            let snapshot = gate.try_snapshot().unwrap();
            assert!(gate.try_snapshot().is_none());
            drop(snapshot);
            drop(parked);
        }
        assert!(gate.try_snapshot().is_none());
        drop(active);
        assert!(gate.try_snapshot().is_some());
    }
    #[test]
    fn a_busy_capture_holds_new_slices_briefly_then_succeeds() {
        let gate = std::sync::Arc::new(Gate::default());
        let g = gate.clone();
        let (go, slices) = std::sync::mpsc::channel();
        let (stop_tx, stop) = std::sync::mpsc::channel::<()>();
        // A guest that is always in a slice except for an instant between.
        let worker = thread::spawn(move || {
            while stop.try_recv().is_err() {
                let _active = g.enter();
                let _ = go.send(());
                thread::sleep(Duration::from_micros(200));
            }
        });
        slices.recv().unwrap();
        let started = Instant::now();
        let mut snapshot = None;
        while snapshot.is_none() && started.elapsed() < Duration::from_secs(2) {
            snapshot = gate.try_snapshot();
            thread::sleep(Duration::from_millis(2));
        }
        assert!(snapshot.is_some(), "capture starved by a busy guest");
        drop(snapshot);
        // The request is cleared: the guest runs freely again.
        while slices.try_recv().is_ok() {}
        slices.recv_timeout(Duration::from_secs(1)).unwrap();
        stop_tx.send(()).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn an_abandoned_request_expires() {
        let gate = Gate::default();
        let active = gate.enter();
        assert!(gate.try_snapshot().is_none());
        thread::sleep(STARVED_AFTER);
        assert!(gate.try_snapshot().is_none());
        drop(active);
        let started = Instant::now();
        drop(gate.enter());
        assert!(started.elapsed() <= REQUEST_HOLD + Duration::from_millis(50));
    }
    #[test]
    fn a_briefly_busy_capture_does_not_hold_the_guest() {
        let gate = Gate::default();
        let active = gate.enter();
        assert!(gate.try_snapshot().is_none());
        drop(active);
        let started = Instant::now();
        drop(gate.enter());
        assert!(started.elapsed() < Duration::from_millis(5));
    }
    #[test]
    fn other_engines_wait_until_capture_finishes() {
        let gate = std::sync::Arc::new(Gate::default());
        let snapshot = gate.try_snapshot().unwrap();
        let g = gate.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let _active = g.enter();
            tx.send(()).unwrap();
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(20))
                .is_err()
        );
        drop(snapshot);
        rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }
}
