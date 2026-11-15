//! Data continuations for production firmware services. No Rust callback is
//! retained while a guest waits: the CPU owner polls after emu_start returns.
use super::{GuestArgs, SharedState};
use crate::memory::{GuestMemory, MemResult};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Wait {
    Sleep {
        milliseconds: u32,
    },
    Input {
        output: u32,
    },
    PowerOff,
    Indicator {
        record: u32,
    },
    Usb {
        output: u32,
    },
    Semaphore {
        descriptor: u32,
        timeout_ms: u32,
    },
    CriticalSection {
        descriptor: u32,
    },
    Event {
        descriptor: u32,
        timeout_ms: u32,
        main: bool,
        call_point: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceBinding {
    pub descriptor: u32,
    pub identity: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceImage {
    pub version: u32,
    pub resource: Option<ResourceBinding>,
    pub args: GuestArgs,
    pub owner: u32,
    pub wait: Wait,
    /// Remaining virtual duration at capture; elapsed host downtime is excluded.
    pub remaining_timeout_ns: Option<u64>,
    /// A WakeUp already accepted for this timed sleep, retained across restore.
    #[serde(default)]
    pub sleep_woken: bool,
}

pub struct PendingService {
    pub args: GuestArgs,
    wait: Wait,
    deadline: Option<Duration>,
    clock: Arc<crate::clock::Clock>,
    resource: Option<std::sync::Arc<super::sync::SyncEntry>>,
    owner: u32,
    sleep_wake: Arc<AtomicBool>,
}
impl PendingService {
    pub fn new(args: GuestArgs, wait: Wait, owner: u32) -> Self {
        let clock = Arc::new(crate::clock::Clock::default());
        let deadline = match wait {
            Wait::Sleep { milliseconds } if milliseconds != u16::MAX as u32 => {
                Some(clock.now() + Duration::from_millis(u64::from(milliseconds)))
            }
            _ => None,
        };
        Self {
            args,
            wait,
            deadline,
            clock,
            resource: None,
            owner,
            sleep_wake: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn event(
        args: GuestArgs,
        resource: std::sync::Arc<super::sync::SyncEntry>,
        main: bool,
        call_point: bool,
        owner: u32,
    ) -> Self {
        let mut result = Self::new(
            args,
            Wait::Event {
                descriptor: args.r0,
                timeout_ms: args.r1,
                main,
                call_point,
            },
            owner,
        );
        result.resource = Some(resource);
        result.deadline = (args.r1 != u32::MAX)
            .then(|| result.clock.now() + Duration::from_millis(u64::from(args.r1)));
        result
    }
    pub fn resource(
        args: GuestArgs,
        wait: Wait,
        resource: std::sync::Arc<super::sync::SyncEntry>,
        owner: u32,
    ) -> Self {
        let timeout = match wait {
            Wait::Semaphore { timeout_ms, .. } => Some(timeout_ms),
            _ => None,
        };
        let mut result = Self::new(args, wait, owner);
        result.resource = Some(resource);
        result.deadline = timeout
            .filter(|&t| t != u32::MAX)
            .map(|t| result.clock.now() + Duration::from_millis(u64::from(t)));
        result
    }
    fn finish_event(&self, shared: &SharedState) {
        if let Some(resource) = &self.resource {
            if let super::sync::SyncObj::Event(event) = &resource.object {
                event.cancel_wait(self.owner);
            }
        }
        if matches!(self.wait, Wait::Event { main: true, .. }) {
            *shared.main_ui_wait_owner.lock().unwrap() = None;
            shared.set_main_waiting(false);
        }
    }
    pub fn redirect(
        &self,
        uc: &mut unicorn_engine::Unicorn<'_, ()>,
        mem: &GuestMemory,
        shared: &SharedState,
    ) -> Result<bool, String> {
        if !matches!(
            self.wait,
            Wait::Event {
                call_point: true,
                ..
            }
        ) || !shared.guest_calls.pending()
        {
            return Ok(false);
        }
        if shared.guest_calls.inject(uc, mem)? {
            self.finish_event(shared);
            return Ok(true);
        }
        Ok(false)
    }
    pub fn image(&self) -> ServiceImage {
        ServiceImage {
            version: 1,
            sleep_woken: self.sleep_wake.load(Ordering::Acquire),
            resource: self.resource.as_ref().map(|resource| ResourceBinding {
                descriptor: self.args.r0,
                identity: resource.identity(),
            }),
            args: self.args,
            owner: self.owner,
            wait: self.wait.clone(),
            remaining_timeout_ns: self
                .deadline
                .map(|d| d.saturating_sub(self.clock.now()).as_nanos() as u64),
        }
    }
    /// Reconstruct only the service data. The whole-machine loader must also
    /// restore and validate its CPU, memory, queues and power state.
    pub fn from_image(image: ServiceImage) -> Result<Self, String> {
        Self::restore(image, None)
    }
    pub fn from_bound_image(
        image: ServiceImage,
        resources: &super::sync::image::RestoredSync,
    ) -> Result<Self, String> {
        Self::restore(image, Some(resources))
    }
    fn restore(
        image: ServiceImage,
        resources: Option<&super::sync::image::RestoredSync>,
    ) -> Result<Self, String> {
        if image.version != 1 || image.args.sp % 4 != 0 || image.args.sp.checked_add(8).is_none() {
            return Err("Invalid pending service version or return frame".into());
        }
        if image.sleep_woken && !matches!(image.wait, Wait::Sleep { .. }) {
            return Err("Wake flag belongs only to timed sleep".into());
        }
        let mut descriptor = None;
        let timeout = match image.wait {
            Wait::Sleep { milliseconds } if u32::from(image.args.r0 as u16) == milliseconds => {
                (milliseconds != u16::MAX as u32).then_some(milliseconds)
            }
            Wait::Input { output } | Wait::Usb { output } if image.args.r0 == output => None,
            Wait::PowerOff => None,
            Wait::Indicator { record } if image.args.r1 == 260 && image.args.r2 == record => None,
            Wait::Event {
                descriptor: address,
                timeout_ms,
                main,
                call_point,
            } if image.args.r0 == address
                && image.args.r1 == timeout_ms
                && main == (image.owner == 0)
                && call_point
                    == (main && image.args.caller_pc == crate::guest_call::MAIN_WAIT_CALLER) =>
            {
                descriptor = Some(address);
                (timeout_ms != u32::MAX).then_some(timeout_ms)
            }
            Wait::Semaphore {
                descriptor: address,
                timeout_ms,
            } if image.args.r0 == address && image.args.r1 == timeout_ms => {
                descriptor = Some(address);
                (timeout_ms != u32::MAX).then_some(timeout_ms)
            }
            Wait::CriticalSection {
                descriptor: address,
            } if image.args.r0 == address => {
                descriptor = Some(address);
                None
            }
            _ => return Err("Pending service arguments disagree with its operation".into()),
        };
        let clock = Arc::new(crate::clock::Clock::default());
        let deadline = match (timeout, image.remaining_timeout_ns) {
            (Some(milliseconds), Some(nanos)) if nanos <= u64::from(milliseconds) * 1_000_000 => {
                Some(clock.now() + Duration::from_nanos(nanos))
            }
            (None, None) => None,
            _ => return Err("Pending service timer disagrees with its operation".into()),
        };
        let resource = match (descriptor, image.resource) {
            (None, None) => None,
            (Some(address), Some(binding)) if address == binding.descriptor => {
                let resource = resources
                    .ok_or("Pending synchronization requires a reconstructed resource binding")?
                    .bind(address, binding.identity)?;
                let matches = matches!(
                    (&image.wait, &resource.object),
                    (Wait::Event { .. }, super::sync::SyncObj::Event(_))
                        | (Wait::Semaphore { .. }, super::sync::SyncObj::Semaphore(_))
                        | (
                            Wait::CriticalSection { .. },
                            super::sync::SyncObj::CritSec(_)
                        )
                );
                if !matches {
                    return Err(
                        "Pending wait refers to another synchronization resource type".into(),
                    );
                }
                Some(resource)
            }
            _ => return Err("Pending service descriptor and resource binding disagree".into()),
        };
        Ok(Self {
            args: image.args,
            wait: image.wait,
            deadline,
            clock,
            resource,
            owner: image.owner,
            sleep_wake: Arc::new(AtomicBool::new(image.sleep_woken)),
        })
    }
    pub fn attach_owner_state(&mut self, shared: &SharedState) {
        if !Arc::ptr_eq(&self.clock, &shared.clock) {
            let remaining = self
                .deadline
                .map(|deadline| deadline.saturating_sub(self.clock.now()));
            self.clock = shared.clock.clone();
            self.deadline = remaining.map(|remaining| self.clock.now() + remaining);
        }
        match self.wait {
            Wait::Event {
                main: true,
                descriptor,
                ..
            } => {
                *shared.main_ui_wait_owner.lock().unwrap() = Some((descriptor, self.owner));
                // Reconstruction is not another firmware event-loop round trip.
                shared
                    .main_waiting
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            Wait::Input { .. } => shared.set_getevent_parked(true),
            Wait::Sleep { .. } => shared.bind_sleep(self.owner, &self.sleep_wake),
            _ => {}
        }
    }
    /// Must run under the owner's memory gate, outside every Unicorn callback.
    pub fn poll(
        &self,
        mem: &GuestMemory,
        shared: &SharedState,
        thread: u32,
    ) -> MemResult<Option<u32>> {
        if self.owner != thread {
            return Err(crate::memory::MemoryError {
                va: self.args.sp,
                msg: "Pending service belongs to a different guest owner".into(),
            });
        }
        if matches!(self.wait, Wait::Indicator { .. }) {
            return super::indicator::poll(mem, shared, thread);
        }
        if let Wait::Semaphore { descriptor, .. } | Wait::CriticalSection { descriptor } = self.wait
        {
            let resource = self.resource.as_ref().ok_or(crate::memory::MemoryError {
                va: descriptor,
                msg: "Pending synchronization operation has no descriptor lease".into(),
            })?;
            let critical = matches!(self.wait, Wait::CriticalSection { .. });
            if resource.closed() {
                return Ok(Some(if critical {
                    0
                } else {
                    super::sync::WAIT_RESULT_ERROR
                }));
            }
            let acquired = match &resource.object {
                super::sync::SyncObj::Semaphore(sem) if !critical => sem.try_wait(),
                super::sync::SyncObj::CritSec(cs) if critical => cs.try_enter_guest(thread),
                _ => {
                    return Ok(Some(if critical {
                        0
                    } else {
                        super::sync::WAIT_RESULT_ERROR
                    }));
                }
            };
            if acquired {
                if critical {
                    let count = mem.read_u16(descriptor + 8)?;
                    mem.write_u16(descriptor + 8, count.saturating_add(1))?;
                }
                return Ok(Some(if critical {
                    0
                } else {
                    super::sync::WAIT_RESULT_RESOLVED
                }));
            }
            if shared.stopping()
                || shared.guest_thread_terminated(thread)
                || self.deadline.is_some_and(|d| self.clock.now() >= d)
            {
                return Ok(Some(if critical {
                    0
                } else {
                    super::sync::WAIT_RESULT_TIMEOUT
                }));
            }
            return Ok(None);
        }
        if let Wait::Event { descriptor, .. } = self.wait {
            let resource = self.resource.as_ref().ok_or(crate::memory::MemoryError {
                va: descriptor,
                msg: "Pending event has no reconstructed descriptor lease".into(),
            })?;
            let result = if resource.closed() {
                Some(super::sync::WAIT_RESULT_ERROR)
            } else if let super::sync::SyncObj::Event(event) = &resource.object {
                if event.poll(self.owner) {
                    let latch = mem.read_u16(descriptor + 8)?;
                    if latch == 0 {
                        mem.write_u32(descriptor + 4, 0)?;
                    }
                    Some(super::sync::WAIT_RESULT_RESOLVED)
                } else if shared.stopping()
                    || shared.guest_thread_terminated(thread)
                    || self.deadline.is_some_and(|d| self.clock.now() >= d)
                {
                    Some(super::sync::WAIT_RESULT_TIMEOUT)
                } else {
                    None
                }
            } else {
                Some(super::sync::WAIT_RESULT_ERROR)
            };
            if result.is_some() {
                self.finish_event(shared);
            }
            return Ok(result);
        }
        if shared.stopping() || shared.guest_thread_terminated(thread) {
            if matches!(self.wait, Wait::Input { .. }) {
                if let Wait::Input { output } = self.wait {
                    mem.write_bytes(output, &[0; 124])?;
                }
                shared.set_getevent_parked(false);
            }
            if matches!(self.wait, Wait::PowerOff) {
                shared.leave_sleep();
            }
            return Ok(Some(0));
        }
        match self.wait {
            Wait::Sleep { .. } => Ok((self.sleep_wake.load(Ordering::Acquire)
                || self
                    .deadline
                    .is_some_and(|deadline| self.clock.now() >= deadline))
            .then_some(0)),
            Wait::Input { output } => super::handlers::poll_get_event(mem, shared, thread, output),
            Wait::PowerOff if shared.take_wake() => Ok(Some(0)),
            Wait::PowerOff => Ok(None),
            Wait::Indicator { .. } => unreachable!(),
            Wait::Usb { output } => {
                mem.validate_range(output, super::usb::RECEIVE_BYTES)?;
                if let Some(delivery) = shared.usb.take_delivery() {
                    mem.write_u32(output, delivery.bytes.len() as u32)?;
                    mem.write_bytes(output + 4, &delivery.bytes)?;
                    Ok(Some(delivery.callback))
                } else {
                    Ok(None)
                }
            }
            Wait::Event { .. } | Wait::Semaphore { .. } | Wait::CriticalSection { .. } => {
                unreachable!()
            }
        }
    }
}

impl std::fmt::Debug for PendingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingService")
            .field("args", &self.args)
            .field("wait", &self.wait)
            .finish_non_exhaustive()
    }
}
impl Drop for PendingService {
    fn drop(&mut self) {
        if let Some(resource) = &self.resource {
            if let super::sync::SyncObj::Event(event) = &resource.object {
                event.cancel_wait(self.owner);
            }
        }
    }
}
