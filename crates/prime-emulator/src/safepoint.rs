//! Engine-owner phase accounting and expiring boundary coordination.
//! Native SVC continuations are explicit blockers, never resumable checkpoints.
use ppl_engine::inspection::{OwnerObservation, OwnerPhase};
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

const MAX_OWNERS: usize = 128;
#[derive(Default)]
struct State {
    owners: BTreeMap<u32, OwnerObservation>,
    dropped: u64,
    epoch: u64,
    pause: Option<(u64, Instant)>,
}
pub struct SafePoints {
    start: Instant,
    state: Mutex<State>,
    wake: Condvar,
}
impl Default for SafePoints {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        }
    }
}
pub struct OwnerLease {
    points: Arc<SafePoints>,
    tid: u32,
    tracked: bool,
}
pub struct SvcLease {
    points: Arc<SafePoints>,
    tid: u32,
}
pub struct BlockedLease {
    points: Arc<SafePoints>,
    tid: u32,
}
pub struct PauseLease {
    points: Arc<SafePoints>,
    epoch: u64,
}
impl SafePoints {
    pub fn register(self: &Arc<Self>, tid: u32) -> OwnerLease {
        let mut s = self.state.lock().unwrap();
        while let Some((_, deadline)) = s.pause {
            if Instant::now() >= deadline {
                s.pause = None;
                self.wake.notify_all();
                break;
            }
            s = self
                .wake
                .wait_timeout(s, deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .0;
        }
        let duplicate = s
            .owners
            .get(&tid)
            .is_some_and(|o| o.phase != OwnerPhase::Exited);
        if s.owners.len() >= MAX_OWNERS {
            s.owners.retain(|_, o| o.phase != OwnerPhase::Exited);
        }
        let tracked = !duplicate && (s.owners.len() < MAX_OWNERS || s.owners.contains_key(&tid));
        if tracked {
            s.owners.insert(
                tid,
                OwnerObservation {
                    thread_id: tid,
                    phase: OwnerPhase::Boundary,
                    service_id: None,
                    service_name: None,
                    caller_pc: None,
                    blocked_handle: None,
                    entered_ms: None,
                    completed_ms: None,
                    acknowledged_epoch: 0,
                },
            );
        } else {
            s.dropped += 1;
        }
        OwnerLease {
            points: self.clone(),
            tid,
            tracked,
        }
    }
    fn phase(&self, tid: u32, phase: OwnerPhase) {
        let mut s = self.state.lock().unwrap();
        if let Some(o) = s.owners.get_mut(&tid) {
            o.phase = phase;
        }
        if s.pause.is_some() {
            self.wake.notify_all();
        }
    }
    /// Called only by an engine owner, outside emu_start and native callbacks.
    /// Expiry releases every owner even if the inspecting client disappears.
    pub fn boundary(&self, tid: u32) {
        let mut s = self.state.lock().unwrap();
        if let Some(o) = s.owners.get_mut(&tid) {
            o.phase = OwnerPhase::Boundary;
        }
        while let Some((epoch, deadline)) = s.pause {
            if Instant::now() >= deadline {
                s.pause = None;
                self.wake.notify_all();
                break;
            }
            if let Some(o) = s.owners.get_mut(&tid) {
                o.acknowledged_epoch = epoch;
            }
            self.wake.notify_all();
            let remaining = deadline.saturating_duration_since(Instant::now());
            s = self.wake.wait_timeout(s, remaining).unwrap().0;
        }
    }
    pub fn executing(&self, tid: u32) {
        self.phase(tid, OwnerPhase::Executing);
    }
    pub fn enter_svc(self: &Arc<Self>, tid: u32, id: u32, pc: u32) -> SvcLease {
        let mut s = self.state.lock().unwrap();
        if let Some(o) = s.owners.get_mut(&tid) {
            o.phase = OwnerPhase::SvcEnter;
            o.service_id = Some(id);
            // Names are materialized only by observations(), never on this hot path.
            o.service_name = None;
            o.caller_pc = Some(pc);
            o.blocked_handle = None;
            o.entered_ms = Some(self.start.elapsed().as_secs_f64() * 1000.0);
            o.completed_ms = None;
        }
        if s.pause.is_some() {
            self.wake.notify_all();
        }
        SvcLease {
            points: self.clone(),
            tid,
        }
    }
    pub fn blocked(self: &Arc<Self>, tid: u32, handle: Option<u32>) -> BlockedLease {
        let mut s = self.state.lock().unwrap();
        if let Some(o) = s.owners.get_mut(&tid) {
            o.phase = OwnerPhase::SvcBlocked;
            o.blocked_handle = handle;
        }
        if s.pause.is_some() {
            self.wake.notify_all();
        }
        BlockedLease {
            points: self.clone(),
            tid,
        }
    }
    pub fn cancel(&self) {
        self.state.lock().unwrap().pause = None;
        self.wake.notify_all();
    }
    pub fn observations(&self) -> Vec<OwnerObservation> {
        self.state
            .lock()
            .unwrap()
            .owners
            .values()
            .cloned()
            .map(|mut o| {
                o.service_name = o
                    .service_id
                    .and_then(crate::services::names::service_name)
                    .map(str::to_owned);
                o
            })
            .collect()
    }
    pub fn pause(self: &Arc<Self>, budget: Duration) -> Result<PauseLease, String> {
        let deadline = Instant::now()
            .checked_add(budget)
            .ok_or("invalid safe-point deadline")?;
        let mut s = self.state.lock().unwrap();
        if s.pause.is_some() {
            return Err("another inspection pause is active".into());
        }
        if s.dropped != 0 {
            return Err("owner registry exceeded its bound".into());
        }
        s.epoch = s.epoch.checked_add(1).ok_or("safe-point epoch exhausted")?;
        let epoch = s.epoch;
        s.pause = Some((epoch, deadline));
        self.wake.notify_all();
        loop {
            let blocked = s
                .owners
                .values()
                .filter(|o| matches!(o.phase, OwnerPhase::SvcEnter | OwnerPhase::SvcBlocked))
                .map(|o| o.thread_id)
                .collect::<Vec<_>>();
            if !blocked.is_empty() {
                s.pause = None;
                self.wake.notify_all();
                return Err(format!(
                    "native callback continuations in guest threads {blocked:?}"
                ));
            }
            let missing = s
                .owners
                .values()
                .filter(|o| o.phase != OwnerPhase::Exited && o.acknowledged_epoch != epoch)
                .map(|o| o.thread_id)
                .collect::<Vec<_>>();
            if missing.is_empty() {
                return Ok(PauseLease {
                    points: self.clone(),
                    epoch,
                });
            }
            let now = Instant::now();
            if now >= deadline {
                s.pause = None;
                self.wake.notify_all();
                return Err(format!(
                    "safe-point acknowledgement deadline; threads {missing:?}"
                ));
            }
            s = self.wake.wait_timeout(s, deadline - now).unwrap().0;
            if s.pause.is_none() {
                return Err("safe-point lease expired".into());
            }
        }
    }
}
impl Drop for OwnerLease {
    fn drop(&mut self) {
        if self.tracked {
            self.points.phase(self.tid, OwnerPhase::Exited);
        }
    }
}
impl Drop for SvcLease {
    fn drop(&mut self) {
        let mut s = self.points.state.lock().unwrap();
        if let Some(o) = s.owners.get_mut(&self.tid) {
            o.phase = OwnerPhase::SvcExit;
            o.completed_ms = Some(self.points.start.elapsed().as_secs_f64() * 1000.0);
        }
        if s.pause.is_some() {
            self.points.wake.notify_all();
        }
    }
}
impl Drop for BlockedLease {
    fn drop(&mut self) {
        self.points.phase(self.tid, OwnerPhase::SvcEnter);
    }
}
impl PauseLease {
    /// Expiring leases never imply that owners are still paused indefinitely.
    pub fn is_current(&self) -> bool {
        self.points
            .state
            .lock()
            .unwrap()
            .pause
            .is_some_and(|(epoch, deadline)| epoch == self.epoch && Instant::now() < deadline)
    }
}
impl Drop for PauseLease {
    fn drop(&mut self) {
        let mut s = self.points.state.lock().unwrap();
        if s.pause.is_some_and(|p| p.0 == self.epoch) {
            s.pause = None;
            self.points.wake.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_callback_is_an_explicit_blocker_with_service_and_handle() {
        let p = Arc::new(SafePoints::default());
        let _owner = p.register(0);
        let svc = p.enter_svc(0, 65550, 0x1234);
        assert!(p.state.lock().unwrap().owners[&0].service_name.is_none());
        let blocked = p.blocked(0, Some(9));
        assert!(p.pause(Duration::from_secs(1)).is_err());
        let o = &p.observations()[0];
        assert_eq!(o.phase, OwnerPhase::SvcBlocked);
        assert_eq!(o.blocked_handle, Some(9));
        assert_eq!(o.service_name.as_deref(), Some("OSWaitForEvent"));
        drop(blocked);
        drop(svc);
        assert_eq!(p.observations()[0].phase, OwnerPhase::SvcExit);
        assert!(p.observations()[0].completed_ms.is_some());
    }
    #[test]
    fn actual_owner_acknowledgement_is_required_and_drop_resumes_it() {
        let p = Arc::new(SafePoints::default());
        let owner = p.register(0);
        p.executing(0);
        assert!(p.pause(Duration::from_millis(5)).is_err());
        let worker = p.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _owner = owner;
            loop {
                worker.boundary(0);
                if rx.try_recv().is_ok() {
                    break;
                }
                std::thread::yield_now();
            }
        });
        let lease = p.pause(Duration::from_secs(1)).unwrap();
        let o = p.observations();
        assert!(o[0].acknowledged_epoch > 0);
        assert_eq!(o[0].phase, OwnerPhase::Boundary);
        assert!(lease.is_current());
        drop(lease);
        tx.send(()).unwrap();
        handle.join().unwrap();
    }
    #[test]
    fn cancellation_releases_an_acknowledged_owner() {
        let p = Arc::new(SafePoints::default());
        let owner = p.register(0);
        let worker = p.clone();
        let handle = std::thread::spawn(move || {
            let _owner = owner;
            loop {
                worker.boundary(0);
                if worker.observations()[0].acknowledged_epoch > 0 {
                    break;
                }
                std::thread::yield_now();
            }
        });
        let lease = p.pause(Duration::from_secs(1)).unwrap();
        p.cancel();
        handle.join().unwrap();
        drop(lease);
    }
    #[test]
    fn new_owners_do_not_enter_a_held_pause() {
        let p = Arc::new(SafePoints::default());
        let lease = p.pause(Duration::from_secs(1)).unwrap();
        let worker = p.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _owner = worker.register(7);
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(10)).is_err());
        drop(lease);
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }
    #[test]
    fn lease_currency_reports_expiry_without_waiting_for_an_owner() {
        let p = Arc::new(SafePoints::default());
        let lease = p.pause(Duration::from_millis(1)).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        assert!(!lease.is_current());
    }
    #[test]
    fn an_abandoned_pause_expires_instead_of_sticking_an_engine() {
        let p = Arc::new(SafePoints::default());
        let owner = p.register(0);
        let worker = p.clone();
        let handle = std::thread::spawn(move || {
            let _owner = owner;
            loop {
                worker.boundary(0);
                if worker.observations()[0].acknowledged_epoch > 0 {
                    break;
                }
                std::thread::yield_now();
            }
        });
        let lease = p.pause(Duration::from_millis(30)).unwrap();
        handle.join().unwrap();
        drop(lease);
    }
}
