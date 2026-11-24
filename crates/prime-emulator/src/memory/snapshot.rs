//! Excludes LCD snapshots from every guest engine, including suspended SVC calls.
//! LCD captures retry when busy. Ephemeral runtime objects instead use a bounded
//! wait from a parked observation hook, before their guest lifetime can end.
//! Captures busy for [`STARVED_AFTER`] request one: for [`REQUEST_HOLD`],
//! engines finishing a slice do not start another, so a guest that never
//! parks (a polling `WAIT(-1)` or `GETKEY` loop) cannot starve captures. A
//! guest that parks normally is never held. The hold expires on its own, so
//! an abandoned request delays the guest by at most that long.
use std::{
    collections::{HashMap, HashSet},
    sync::{Condvar, Mutex, MutexGuard},
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
pub const REQUEST_HOLD: Duration = Duration::from_millis(20);
pub const STARVED_AFTER: Duration = Duration::from_millis(100);
#[derive(Clone, Copy)]
struct Capture {
    owner: ThreadId,
    depth: usize,
    nested_hook_reads: bool,
}
#[derive(Default)]
struct State {
    active: HashSet<ThreadId>,
    entry_waiters: HashSet<ThreadId>,
    parked: HashSet<ThreadId>,
    capturing: Option<Capture>,
    waiting: HashMap<ThreadId, Instant>,
    requested: Option<Instant>,
    /// First busy capture attempt since the last successful capture.
    busy_since: Option<Instant>,
}
impl State {
    fn held(&self) -> Option<Duration> {
        if self.capturing.is_some() {
            return Some(REQUEST_HOLD);
        }
        if let Some(wait) = self
            .waiting
            .values()
            .filter_map(|deadline| deadline.checked_duration_since(Instant::now()))
            .max()
        {
            return Some(wait);
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
    _owner: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Snapshot<'_> {
    /// Enables several machine components to share one exclusion interval.
    /// A guard for another memory owner never grants access to this gate.
    pub fn guards(&self, gate: &Gate) -> bool {
        std::ptr::eq(self.gate, gate)
    }
}
impl Gate {
    /// Waits while a capture runs or a fresh request is pending. A thread
    /// that is already active (nested entry) never waits.
    fn wait_entry<'a>(&'a self, mut state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        if state.active.contains(&thread::current().id()) {
            return state;
        }
        while let Some(wait) = state.held() {
            state.entry_waiters.insert(thread::current().id());
            state = self.wake.wait_timeout(state, wait).unwrap().0;
        }
        state.entry_waiters.remove(&thread::current().id());
        state
    }
    pub fn entries_waiting(&self) -> usize {
        self.state.lock().unwrap().entry_waiters.len()
    }
    pub fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (state.active.len(), state.parked.len())
    }
    pub fn enter(&self) -> Active<'_> {
        let mut state = self.wait_entry(self.state.lock().unwrap());
        let owned = state.active.insert(thread::current().id());
        Active { gate: self, owned }
    }
    pub fn park(&self) -> Parked<'_> {
        let mut state = self.state.lock().unwrap();
        let owned = state.active.remove(&thread::current().id());
        if owned {
            state.parked.insert(thread::current().id());
            self.wake.notify_all();
        }
        Parked { gate: self, owned }
    }
    pub fn try_snapshot(&self) -> Option<Snapshot<'_>> {
        self.try_snapshot_inner(false)
    }
    pub fn try_snapshot_from_hook(&self) -> Option<Snapshot<'_>> {
        self.try_snapshot_inner(true)
    }
    /// The caller parks its engine before waiting and holds no observation
    /// mutex. Other parked hooks can then acquire the gate in turn without
    /// making each other wait for an engine that is itself waiting on a hook.
    pub fn wait_snapshot(&self, budget: Duration) -> Option<Snapshot<'_>> {
        let deadline = Instant::now().checked_add(budget)?;
        let owner = thread::current().id();
        let mut state = self.state.lock().unwrap();
        if state.active.contains(&owner)
            || state
                .capturing
                .is_some_and(|capture| capture.owner == owner)
        {
            return None;
        }
        state.waiting.insert(owner, deadline);
        loop {
            if state.capturing.is_none() && state.active.is_empty() {
                state.waiting.remove(&owner);
                state.capturing = Some(Capture {
                    owner,
                    depth: 1,
                    nested_hook_reads: true,
                });
                state.requested = None;
                state.busy_since = None;
                return Some(Snapshot {
                    gate: self,
                    _owner: Default::default(),
                });
            }
            let now = Instant::now();
            if now >= deadline {
                state.waiting.remove(&owner);
                self.wake.notify_all();
                return None;
            }
            state = self.wake.wait_timeout(state, deadline - now).unwrap().0;
        }
    }
    fn try_snapshot_inner(&self, from_hook: bool) -> Option<Snapshot<'_>> {
        // An owned hook capture must admit its nested decoders even when a
        // parked peer briefly inspects this metadata mutex. Refusing that read
        // would lose an ephemeral return value despite already owning exclusion.
        // The mutex protects only bounded state updates, never guest execution.
        // Ordinary LCD attempts retain their nonblocking behavior.
        let mut state = if from_hook {
            self.state.lock().unwrap()
        } else {
            self.state.try_lock().ok()?
        };
        let owner_only =
            from_hook && state.active.len() == 1 && state.active.contains(&thread::current().id());
        if let Some(capture) = &mut state.capturing {
            if from_hook && capture.nested_hook_reads && capture.owner == thread::current().id() {
                capture.depth += 1;
                return Some(Snapshot {
                    gate: self,
                    _owner: Default::default(),
                });
            }
            return None;
        }
        if state
            .waiting
            .values()
            .any(|deadline| *deadline > Instant::now())
        {
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
        state.capturing = Some(Capture {
            owner: thread::current().id(),
            depth: 1,
            nested_hook_reads: false,
        });
        state.requested = None;
        state.busy_since = None;
        Some(Snapshot {
            gate: self,
            _owner: Default::default(),
        })
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
            self.gate.wake.notify_all();
        }
    }
}
impl Drop for Parked<'_> {
    fn drop(&mut self) {
        if self.owned {
            let mut state = self.gate.wait_entry(self.gate.state.lock().unwrap());
            state.parked.remove(&thread::current().id());
            state.active.insert(thread::current().id());
        }
    }
}
impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().unwrap();
        let capture = state.capturing.as_mut().expect("owned snapshot");
        assert_eq!(capture.owner, thread::current().id());
        capture.depth -= 1;
        if capture.depth == 0 {
            state.capturing = None;
        }
        self.gate.wake.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_hook_decoder_survives_contention_on_the_metadata_mutex() {
        let gate = std::sync::Arc::new(Gate::default());
        let snapshot = gate.wait_snapshot(Duration::from_secs(1)).unwrap();
        let peer = gate.clone();
        let (held, ready) = std::sync::mpsc::channel();
        let (begin, waiting) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let _state = peer.state.lock().unwrap();
            held.send(()).unwrap();
            waiting.recv().unwrap();
            thread::sleep(Duration::from_millis(20));
        });
        ready.recv().unwrap();
        assert!(
            gate.try_snapshot().is_none(),
            "LCD must not block on metadata contention"
        );
        begin.send(()).unwrap();
        let nested = gate
            .try_snapshot_from_hook()
            .expect("owned nested read must not be rejected for mutex contention");
        worker.join().unwrap();
        drop(snapshot);
        assert!(
            gate.try_snapshot().is_none(),
            "nested decoder still owns exclusion"
        );
        drop(nested);
        assert!(gate.try_snapshot().is_some());
    }

    #[test]
    fn simultaneous_parked_hooks_capture_in_turn_and_nested_decoders_keep_exclusion() {
        let gate = std::sync::Arc::new(Gate::default());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let gate = gate.clone();
            let barrier = barrier.clone();
            threads.push(thread::spawn(move || {
                let active = gate.enter();
                barrier.wait();
                let parked = gate.park();
                let snapshot = gate
                    .wait_snapshot(Duration::from_secs(1))
                    .expect("parked hooks must not deadlock");
                assert_eq!(gate.counts().0, 0);
                let nested = gate
                    .try_snapshot_from_hook()
                    .expect("decoder inside owned hook capture");
                drop(snapshot);
                assert!(
                    gate.try_snapshot().is_none(),
                    "nested decoder must retain exclusion"
                );
                drop(nested);
                drop(parked);
                drop(active);
            }));
        }
        for worker in threads {
            worker.join().unwrap();
        }
        assert_eq!(gate.counts(), (0, 0));
        assert!(gate.try_snapshot().is_some());
    }
    #[test]
    fn a_runtime_hook_waits_for_a_peer_slice_but_times_out_without_retaining_a_hold() {
        let gate = std::sync::Arc::new(Gate::default());
        let (ready, started) = std::sync::mpsc::channel();
        let (release, done) = std::sync::mpsc::channel();
        let peer = gate.clone();
        let worker = thread::spawn(move || {
            let active = peer.enter();
            ready.send(()).unwrap();
            done.recv().unwrap();
            drop(active);
        });
        started.recv().unwrap();
        assert!(gate.wait_snapshot(Duration::from_millis(5)).is_none());
        assert!(gate.state.lock().unwrap().waiting.is_empty());
        let hook = gate.enter();
        let parked = gate.park();
        release.send(()).unwrap();
        let snapshot = gate.wait_snapshot(Duration::from_secs(1)).unwrap();
        assert_eq!(gate.counts().0, 0);
        drop(snapshot);
        drop(parked);
        drop(hook);
        worker.join().unwrap();
    }
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
