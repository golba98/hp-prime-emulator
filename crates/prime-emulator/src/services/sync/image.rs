//! Portable synchronization resources used by production service continuations.
//! The caller captures these after all owners acknowledge a pause and restores
//! them over the restored guest allocator/memory before binding saved waits.
use super::*;
use crate::{memory::GuestMemory, services::SharedState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const VERSION: u32 = 1;
const MAX_RESOURCES: usize = 8192;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectImage {
    Event {
        manual: bool,
        signaled: bool,
        waiters: BTreeSet<u32>,
    },
    Semaphore {
        count: i32,
    },
    CriticalSection {
        owner: Option<u32>,
        depth: u32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceImage {
    pub identity: u64,
    pub descriptor: u32,
    pub owns_descriptor: bool,
    pub closed: bool,
    pub object: ObjectImage,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncImage {
    pub version: u32,
    pub resources: Vec<ResourceImage>,
    pub handles: BTreeMap<u32, u64>,
}
/// Holds retired resource leases until every pending owner has been rebound.
/// After binding, dropping this set releases its temporary strong references.
pub struct RestoredSync {
    handles: SyncMap,
    resources: BTreeMap<u64, (u32, Arc<SyncEntry>)>,
}
impl RestoredSync {
    pub fn bind(&self, descriptor: u32, identity: u64) -> Result<Arc<SyncEntry>, String> {
        self.resources
            .get(&identity)
            .filter(|(address, _)| *address == descriptor)
            .map(|(_, entry)| entry.clone())
            .ok_or_else(|| {
                "Saved wait refers to a missing resource or another descriptor generation".into()
            })
    }
    pub fn install_handles(&mut self, shared: &SharedState) -> Result<(), String> {
        let mut target = shared.sync_objects.lock().unwrap();
        if !target.is_empty()
            || target
                .resources
                .values()
                .any(|(_, entry)| entry.strong_count() != 0)
        {
            return Err("Synchronization restore requires an unused target table".into());
        }
        *target = std::mem::take(&mut self.handles);
        Ok(())
    }
}
impl SyncMap {
    pub fn image(&self) -> Result<SyncImage, String> {
        let mut resources = BTreeMap::new();
        for (&identity, (descriptor, weak)) in &self.resources {
            let Some(entry) = weak.upgrade() else {
                continue;
            };
            let object = match &entry.object {
                SyncObj::Event(event) => {
                    let state = event.state.lock().unwrap();
                    if !state.blocked.is_empty() {
                        return Err(
                            "Native blocking event callers are not portable guest waiters".into(),
                        );
                    }
                    ObjectImage::Event {
                        manual: event.manual,
                        signaled: state.signaled,
                        waiters: state.guest_blocked.clone(),
                    }
                }
                SyncObj::Semaphore(sem) => ObjectImage::Semaphore {
                    count: *sem.count.lock().unwrap(),
                },
                SyncObj::CritSec(cs) => {
                    let state = cs.state.lock().unwrap();
                    let owner = match state.0 {
                        None => None,
                        Some(LockOwner::Guest(id)) => Some(id),
                        Some(LockOwner::Host(_)) => {
                            return Err(
                                "Native critical-section ownership is not a guest identity".into(),
                            );
                        }
                    };
                    ObjectImage::CriticalSection {
                        owner,
                        depth: state.1,
                    }
                }
            };
            resources.insert(
                identity,
                ResourceImage {
                    identity,
                    descriptor: *descriptor,
                    owns_descriptor: entry.allocation.is_some(),
                    closed: entry.closed(),
                    object,
                },
            );
        }
        let image = SyncImage {
            version: VERSION,
            resources: resources.into_values().collect(),
            handles: self
                .handles
                .iter()
                .map(|(&address, resource)| (address, resource.identity()))
                .collect(),
        };
        image.validate()?;
        Ok(image)
    }
}
impl SyncImage {
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION
            || self.resources.len() > MAX_RESOURCES
            || self.handles.len() > MAX_RESOURCES
        {
            return Err("Unsupported synchronization image version or resource bound".into());
        }
        let mut ids = BTreeMap::new();
        let mut owned = BTreeSet::new();
        for resource in &self.resources {
            if resource.identity == 0
                || resource.identity == u64::MAX
                || resource.descriptor == 0
                || resource.descriptor % 4 != 0
                || ids.insert(resource.identity, resource).is_some()
                || (resource.owns_descriptor && !owned.insert(resource.descriptor))
            {
                return Err("Invalid or duplicate synchronization identity/descriptor".into());
            }
            match &resource.object {
                ObjectImage::Event { waiters, .. } if waiters.len() > 128 => {
                    return Err("Too many guest event waiters".into());
                }
                ObjectImage::Semaphore { count } if !(0..=0x7fff).contains(count) => {
                    return Err("Invalid semaphore count".into());
                }
                ObjectImage::CriticalSection { owner, depth }
                    if owner.is_some() != (*depth != 0) =>
                {
                    return Err("Critical-section owner/depth mismatch".into());
                }
                _ => {}
            }
        }
        for (descriptor, identity) in &self.handles {
            let resource = ids
                .get(identity)
                .ok_or("Handle references an absent resource")?;
            if resource.closed || resource.descriptor != *descriptor {
                return Err(
                    "Live handle references a closed resource or another descriptor".into(),
                );
            }
        }
        Ok(())
    }
    /// All validation precedes construction of allocator-owning leases, so a
    /// refused image cannot free or alter any target guest allocation.
    pub fn restore(&self, memory: Arc<GuestMemory>) -> Result<RestoredSync, String> {
        self.validate()?;
        for resource in &self.resources {
            memory
                .validate_range(resource.descriptor, 0x14)
                .map_err(|e| e.to_string())?;
            if resource.owns_descriptor
                && memory
                    .heap_size_of(resource.descriptor)
                    .is_none_or(|size| size < 0x14)
            {
                return Err(
                    "Owned synchronization descriptor is absent from the restored allocator".into(),
                );
            }
            if self.handles.get(&resource.descriptor) == Some(&resource.identity) {
                let magic = match resource.object {
                    ObjectImage::Event { .. } => EVENT_MAGIC,
                    ObjectImage::Semaphore { .. } => SEMAPHORE_MAGIC,
                    ObjectImage::CriticalSection { .. } => CRITSEC_MAGIC,
                };
                if memory
                    .read_u32(resource.descriptor)
                    .map_err(|e| e.to_string())?
                    != magic
                {
                    return Err("Restored descriptor type disagrees with its live resource".into());
                }
            }
        }
        let mut result = RestoredSync {
            handles: SyncMap::default(),
            resources: BTreeMap::new(),
        };
        for resource in &self.resources {
            let object = match &resource.object {
                ObjectImage::Event {
                    manual,
                    signaled,
                    waiters,
                } => {
                    let event = HostEvent::new(*manual, *signaled);
                    event.state.lock().unwrap().guest_blocked = waiters.clone();
                    SyncObj::Event(event)
                }
                ObjectImage::Semaphore { count } => SyncObj::Semaphore(HostSem::new(*count)),
                ObjectImage::CriticalSection { owner, depth } => {
                    let cs = HostCs::new();
                    *cs.state.lock().unwrap() = (owner.map(LockOwner::Guest), *depth);
                    SyncObj::CritSec(cs)
                }
            };
            retain_resource_identity(resource.identity);
            let entry = Arc::new(SyncEntry {
                identity: resource.identity,
                object,
                closed: std::sync::atomic::AtomicBool::new(resource.closed),
                allocation: resource
                    .owns_descriptor
                    .then(|| (memory.clone(), resource.descriptor)),
            });
            result.handles.resources.insert(
                resource.identity,
                (resource.descriptor, Arc::downgrade(&entry)),
            );
            result
                .resources
                .insert(resource.identity, (resource.descriptor, entry));
        }
        for (&descriptor, &identity) in &self.handles {
            result
                .handles
                .handles
                .insert(descriptor, result.resources[&identity].1.clone());
        }
        Ok(result)
    }
}
