//! Host-side synchronization objects for the OS sync services.
//!
//! Conventions verified against the reference (PrimU2 `svc_thread.cpp`):
//! descriptors are 0x14-byte guest structs with magic at +0, data fields,
//! and the wait-result namespace 0x82 (timeout) / 0x83 (resolved) /
//! 0x84 (error). The reference maps descriptors to Win32 objects; we map
//! them to these host implementations.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;

pub const WAIT_RESULT_TIMEOUT: u32 = 0x82;
pub const WAIT_RESULT_RESOLVED: u32 = 0x83;
pub const WAIT_RESULT_ERROR: u32 = 0x84;

pub const EVENT_MAGIC: u32 = 0x201;
pub const SEMAPHORE_MAGIC: u32 = 0x200;
pub const CRITSEC_MAGIC: u32 = 0x202;

/// Manual/auto-reset event. `flag` in the guest descriptor mirrors
/// `signaled`; the reference clears it on a resolved auto-reset wait.
pub struct HostEvent {
    state: Mutex<bool>,
    manual: bool,
    cv: Condvar,
}

impl HostEvent {
    pub fn new(manual: bool, initial: bool) -> HostEvent {
        HostEvent {
            state: Mutex::new(initial),
            manual,
            cv: Condvar::new(),
        }
    }

    pub fn set(&self) {
        *self.state.lock().unwrap() = true;
        self.cv.notify_all();
    }

    pub fn reset(&self) {
        *self.state.lock().unwrap() = false;
    }

    /// Wait up to `timeout_ms` (u32::MAX = infinite, Win32 INFINITE).
    /// Returns true if signaled. Also wakes early when `stopping` fires.
    pub fn wait(&self, timeout_ms: u32, stopping: &dyn Fn() -> bool) -> bool {
        let infinite = timeout_ms == u32::MAX;
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(u64::from(timeout_ms.min(1 << 24)));
        let mut g = self.state.lock().unwrap();
        loop {
            if *g {
                if !self.manual {
                    *g = false;
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
            let (ng, _) = self.cv.wait_timeout(g, slice).unwrap();
            g = ng;
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

    pub fn wait(&self, timeout_ms: u32, stopping: &dyn Fn() -> bool) -> bool {
        let infinite = timeout_ms == u32::MAX;
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(u64::from(timeout_ms.min(1 << 20)));
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
            let (nc, _) = self
                .cv
                .wait_timeout(c, std::time::Duration::from_millis(25))
                .unwrap();
            c = nc;
        }
    }
}

/// Reentrant critical section keyed by host thread id (guest threads are
/// host threads in this model, matching the reference's Win32 semantics).
pub struct HostCs {
    state: Mutex<(Option<ThreadId>, u32)>,
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

    pub fn enter(&self) {
        self.enter_until(&|| false);
    }

    pub fn enter_until(&self, stopping: &dyn Fn() -> bool) -> bool {
        let me = std::thread::current().id();
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
        let me = std::thread::current().id();
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

/// All live sync objects, keyed by guest descriptor address. Objects are
/// behind `Arc` so blocking waits can release the map lock first.
pub type SyncMap = HashMap<u32, std::sync::Arc<SyncObj>>;
