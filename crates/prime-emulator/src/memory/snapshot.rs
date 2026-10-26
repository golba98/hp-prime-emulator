//! Excludes LCD snapshots from every guest engine, including suspended SVC calls.
//! Captures never wait for an engine: a busy capture is retried at the next safe point.
use std::{
    collections::HashSet,
    sync::{Condvar, Mutex},
    thread::{self, ThreadId},
};
#[derive(Default)]
struct State {
    active: HashSet<ThreadId>,
    capturing: bool,
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
    pub fn enter(&self) -> Active<'_> {
        let mut state = self.state.lock().unwrap();
        while state.capturing {
            state = self.wake.wait(state).unwrap();
        }
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
        if state.capturing || (!state.active.is_empty() && !owner_only) {
            return None;
        }
        state.capturing = true;
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
            let mut state = self.gate.state.lock().unwrap();
            while state.capturing {
                state = self.gate.wake.wait(state).unwrap();
            }
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
