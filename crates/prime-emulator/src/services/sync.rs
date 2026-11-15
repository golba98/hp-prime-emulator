//! Host-side synchronization objects for the OS sync services.
//!
//! Conventions verified against the reference (PrimU2 `svc_thread.cpp`):
//! descriptors are 0x14-byte guest structs with magic at +0, data fields,
//! and the wait-result namespace 0x82 (timeout) / 0x83 (resolved) /
//! 0x84 (error). The reference maps descriptors to Win32 objects; we map
//! them to these host implementations.

pub mod image;

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;

// Restored images may contain any valid generation. They must not advance a
// process-wide monotonic counter to exhaustion, or collide with new objects.
// Reference counts also allow independent restored worlds in one process.
#[derive(Default)]
struct ResourceIdentities {
    next: u64,
    users: std::collections::BTreeMap<u64, usize>,
}
static RESOURCE_IDENTITIES: std::sync::LazyLock<Mutex<ResourceIdentities>> =
    std::sync::LazyLock::new(|| {
        Mutex::new(ResourceIdentities {
            next: 1,
            ..Default::default()
        })
    });
fn new_resource_identity() -> u64 {
    let mut ids = RESOURCE_IDENTITIES.lock().unwrap();
    loop {
        let id = ids.next;
        ids.next = if id == u64::MAX - 1 { 1 } else { id + 1 };
        if let std::collections::btree_map::Entry::Vacant(entry) = ids.users.entry(id) {
            entry.insert(1);
            return id;
        }
    }
}
fn retain_resource_identity(id: u64) {
    *RESOURCE_IDENTITIES
        .lock()
        .unwrap()
        .users
        .entry(id)
        .or_default() += 1;
}
fn release_resource_identity(id: u64) {
    let mut ids = RESOURCE_IDENTITIES.lock().unwrap();
    let users = ids.users.get_mut(&id).expect("live resource identity");
    *users -= 1;
    if *users == 0 {
        ids.users.remove(&id);
    }
}

pub const WAIT_RESULT_TIMEOUT: u32 = 0x82;
pub const WAIT_RESULT_RESOLVED: u32 = 0x83;
pub const WAIT_RESULT_ERROR: u32 = 0x84;

pub const EVENT_MAGIC: u32 = 0x201;
pub const SEMAPHORE_MAGIC: u32 = 0x200;
pub const CRITSEC_MAGIC: u32 = 0x202;

/// Manual/auto-reset event. `flag` in the guest descriptor mirrors
/// `signaled`; the reference clears it on a resolved auto-reset wait.
pub struct HostEvent {
    state: Mutex<EventState>,
    manual: bool,
    cv: Condvar,
}
struct EventState {
    signaled: bool,
    blocked: std::collections::HashSet<ThreadId>,
    guest_blocked: std::collections::BTreeSet<u32>,
}

impl HostEvent {
    pub fn new(manual: bool, initial: bool) -> HostEvent {
        HostEvent {
            state: Mutex::new(EventState {
                signaled: initial,
                blocked: Default::default(),
                guest_blocked: Default::default(),
            }),
            manual,
            cv: Condvar::new(),
        }
    }

    pub fn set(&self) {
        self.state.lock().unwrap().signaled = true;
        self.cv.notify_all();
    }

    pub fn reset(&self) {
        self.state.lock().unwrap().signaled = false;
    }

    /// A callback entry (or a consumed signal awaiting engine reentry) is not
    /// an idle UI. Query the actual owner's unsignaled condition-variable wait.
    /// Callers observing guest state also hold the guest-memory snapshot gate.
    pub fn waiter_blocked(&self, owner: ThreadId) -> bool {
        self.state
            .try_lock()
            .is_ok_and(|s| !s.signaled && s.blocked.contains(&owner))
    }

    /// Nonblocking owner poll. Logical blocked membership survives between
    /// slices, so UI readiness means the same unsignaled wait as before.
    pub fn guest_waiter_blocked(&self, owner: u32) -> bool {
        self.state
            .try_lock()
            .is_ok_and(|s| !s.signaled && s.guest_blocked.contains(&owner))
    }
    pub fn poll(&self, owner: u32) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.signaled {
            if !self.manual {
                state.signaled = false;
            }
            state.guest_blocked.remove(&owner);
            true
        } else {
            state.guest_blocked.insert(owner);
            false
        }
    }
    pub fn cancel_wait(&self, owner: u32) {
        self.state.lock().unwrap().guest_blocked.remove(&owner);
    }

    /// Wait up to `timeout_ms` (u32::MAX = infinite, Win32 INFINITE).
    /// Returns true if signaled. Also wakes early when `stopping` fires.
    pub fn wait(&self, timeout_ms: u32, stopping: &dyn Fn() -> bool) -> bool {
        let infinite = timeout_ms == u32::MAX;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(u64::from(timeout_ms));
        let mut g = self.state.lock().unwrap();
        loop {
            if g.signaled {
                if !self.manual {
                    g.signaled = false;
                }
                return true;
            }
            if stopping() {
                return false;
            }
            // Sleep the smaller of 10 ms and the remaining wait budget so
            // short timeouts resolve promptly.
            let slice = if infinite {
                std::time::Duration::from_millis(10)
            } else {
                (deadline.saturating_duration_since(std::time::Instant::now()))
                    .min(std::time::Duration::from_millis(10))
            };
            if slice.is_zero() {
                return false;
            }
            let owner = std::thread::current().id();
            g.blocked.insert(owner);
            let (ng, _) = self.cv.wait_timeout(g, slice).unwrap();
            g = ng;
            g.blocked.remove(&owner);
        }
    }
}

/// Counting semaphore (max 0x7FFF, reference default).
pub struct HostSem {
    count: Mutex<i32>,
    cv: Condvar,
}

impl HostSem {
    pub fn new(initial: i32) -> HostSem {
        HostSem {
            count: Mutex::new(initial),
            cv: Condvar::new(),
        }
    }

    pub fn release(&self) {
        let mut c = self.count.lock().unwrap();
        if *c < 0x7FFF {
            *c += 1;
        }
        drop(c);
        self.cv.notify_all();
    }

    pub fn try_wait(&self) -> bool {
        let mut count = self.count.lock().unwrap();
        if *count > 0 {
            *count -= 1;
            true
        } else {
            false
        }
    }

    pub fn wait(&self, timeout_ms: u32, stopping: &dyn Fn() -> bool) -> bool {
        let infinite = timeout_ms == u32::MAX;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(u64::from(timeout_ms));
        let mut c = self.count.lock().unwrap();
        loop {
            if *c > 0 {
                *c -= 1;
                return true;
            }
            if stopping() {
                return false;
            }
            if !infinite && std::time::Instant::now() >= deadline {
                return false;
            }
            let slice = if infinite {
                std::time::Duration::from_millis(25)
            } else {
                deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .min(std::time::Duration::from_millis(25))
            };
            let (nc, _) = self.cv.wait_timeout(c, slice).unwrap();
            c = nc;
        }
    }
}

/// Production ownership uses stable guest thread IDs; host IDs remain only
/// for callers of the standalone blocking compatibility API.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LockOwner {
    Guest(u32),
    Host(ThreadId),
}
pub struct HostCs {
    state: Mutex<(Option<LockOwner>, u32)>,
    cv: Condvar,
}

impl Default for HostCs {
    fn default() -> Self {
        Self::new()
    }
}

impl HostCs {
    pub fn new() -> HostCs {
        HostCs {
            state: Mutex::new((None, 0)),
            cv: Condvar::new(),
        }
    }

    pub fn try_enter_guest(&self, thread: u32) -> bool {
        let me = LockOwner::Guest(thread);
        let mut state = self.state.lock().unwrap();
        match state.0 {
            None => {
                *state = (Some(me), 1);
                true
            }
            Some(owner) if owner == me && state.1 < u32::MAX => {
                state.1 += 1;
                true
            }
            _ => false,
        }
    }
    pub fn leave_guest(&self, thread: u32) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.0 != Some(LockOwner::Guest(thread)) {
            return false;
        }
        state.1 -= 1;
        if state.1 == 0 {
            state.0 = None;
            self.cv.notify_one();
        }
        true
    }

    pub fn enter(&self) {
        self.enter_until(&|| false);
    }

    pub fn enter_until(&self, stopping: &dyn Fn() -> bool) -> bool {
        let me = LockOwner::Host(std::thread::current().id());
        let mut g = self.state.lock().unwrap();
        loop {
            match g.0 {
                Some(owner) if owner == me => {
                    g.1 += 1;
                    return true;
                }
                None => {
                    g.0 = Some(me);
                    g.1 = 1;
                    return true;
                }
                _ => {}
            }
            if stopping() {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, std::time::Duration::from_millis(25))
                .unwrap()
                .0;
        }
    }

    pub fn leave(&self) {
        let me = LockOwner::Host(std::thread::current().id());
        let mut g = self.state.lock().unwrap();
        if g.0 == Some(me) {
            g.1 -= 1;
            if g.1 == 0 {
                g.0 = None;
                drop(g);
                self.cv.notify_one();
            }
        }
    }
}

pub enum SyncObj {
    Event(HostEvent),
    Semaphore(HostSem),
    CritSec(HostCs),
}

/// An operation's Arc is also a lease on its guest descriptor. Removing the
/// entry from the handle table closes it immediately, but cannot recycle that
/// address while an outstanding wait or signal can still access it.
pub struct SyncEntry {
    identity: u64,
    pub object: SyncObj,
    closed: std::sync::atomic::AtomicBool,
    allocation: Option<(std::sync::Arc<crate::memory::GuestMemory>, u32)>,
}
impl SyncEntry {
    pub fn new(
        object: SyncObj,
        allocation: Option<(std::sync::Arc<crate::memory::GuestMemory>, u32)>,
    ) -> Self {
        Self {
            identity: new_resource_identity(),
            object,
            closed: std::sync::atomic::AtomicBool::new(false),
            allocation,
        }
    }
    pub fn identity(&self) -> u64 {
        self.identity
    }
    pub fn closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn close(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        match &self.object {
            SyncObj::Event(e) => e.cv.notify_all(),
            SyncObj::Semaphore(s) => s.cv.notify_all(),
            SyncObj::CritSec(c) => c.cv.notify_all(),
        }
    }
}
impl Drop for SyncEntry {
    fn drop(&mut self) {
        release_resource_identity(self.identity);
        if let Some((mem, address)) = &self.allocation {
            mem.heap_free(*address);
        }
    }
}
/// Live objects keyed by guest descriptor address; waiters release the table
/// lock before blocking, retaining a descriptor lease instead.
#[derive(Default)]
pub struct SyncMap {
    handles: HashMap<u32, Arc<SyncEntry>>,
    // Closed/replaced resources remain discoverable while a pending operation
    // owns a lease. Weak references never delay descriptor reclamation.
    resources: HashMap<u64, (u32, Weak<SyncEntry>)>,
}
impl SyncMap {
    pub fn get(&self, descriptor: &u32) -> Option<&Arc<SyncEntry>> {
        self.handles.get(descriptor)
    }
    pub fn insert(&mut self, descriptor: u32, entry: Arc<SyncEntry>) -> Option<Arc<SyncEntry>> {
        self.resources
            .retain(|_, (_, resource)| resource.strong_count() != 0);
        self.resources
            .insert(entry.identity(), (descriptor, Arc::downgrade(&entry)));
        self.handles.insert(descriptor, entry)
    }
    pub fn remove(&mut self, descriptor: &u32) -> Option<Arc<SyncEntry>> {
        self.handles.remove(descriptor)
    }
    pub fn len(&self) -> usize {
        self.handles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }
}

#[cfg(test)]
mod blocked_wait_tests {
    use super::*;
    #[test]
    fn pending_signal_and_resolved_callback_cannot_acknowledge_ui_idle() {
        let event = std::sync::Arc::new(HostEvent::new(false, true));
        let owner = std::thread::current().id();
        assert!(!event.waiter_blocked(owner));
        assert!(event.wait(u32::MAX, &|| false));
        assert!(
            !event.waiter_blocked(owner),
            "auto-reset consumed the flag, but this owner never blocked"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let (done, release) = std::sync::mpsc::channel();
        let worker = event.clone();
        let thread = std::thread::spawn(move || {
            tx.send(std::thread::current().id()).unwrap();
            assert!(worker.wait(u32::MAX, &|| false));
            // Model a resolved native callback unable to reenter Unicorn while
            // the observation gate is held. It is not an idle UI waiter.
            tx.send(std::thread::current().id()).unwrap();
            release.recv().unwrap();
        });
        let worker_id = rx.recv().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !event.waiter_blocked(worker_id) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            !event.waiter_blocked(owner),
            "another blocked owner does not acknowledge this one"
        );
        event.set();
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(!event.waiter_blocked(worker_id));
        done.send(()).unwrap();
        thread.join().unwrap();
    }
}
