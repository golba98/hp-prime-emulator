//! Independently restorable heap/allocator component, **not** a live machine snapshot.
//!
//! CPU contexts, other mappings, blocked native stacks and host resources are absent.
//! Restoration requires exclusive access to a fresh, empty `GuestMemory`. Capture
//! holds the memory gate and allocator lock; it does not rely on an expiring owner
//! pause. Files use a bounded binary payload and a checksummed JSON manifest.
use super::{GuestMemory, Heap};
use crate::sha256;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"HPHEAP01";
const VERSION: u32 = 1;
const KIND: &str = "prime.heap-component";
pub const MAX_HEAP_BYTES: usize = 256 * 1024 * 1024;
const MAX_INTERVALS: usize = 100_000;
const MAX_MANIFEST: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interval {
    pub offset: u32,
    pub length: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeapManifest {
    pub kind: String,
    pub version: u32,
    pub live_resume: bool,
    pub resource_identity: String,
    pub base: u32,
    pub size: u32,
    pub allocated_bytes: u32,
    pub free: Vec<Interval>,
    pub allocations: Vec<Interval>,
    pub payload_sha256: String,
    pub manifest_sha256: String,
}
pub struct HeapComponent {
    pub manifest: HeapManifest,
    pub bytes: Vec<u8>,
}
type Result<T> = std::result::Result<T, String>;
impl HeapManifest {
    fn digest(&self) -> Result<String> {
        serde_json::to_vec(&(
            &self.kind,
            self.version,
            self.live_resume,
            &self.resource_identity,
            self.base,
            self.size,
            self.allocated_bytes,
            &self.free,
            &self.allocations,
            &self.payload_sha256,
        ))
        .map(|bytes| sha256::hex(&bytes))
        .map_err(|e| e.to_string())
    }
    fn validate(&self, identity: &str) -> Result<()> {
        if self.kind != KIND || self.version != VERSION || self.live_resume {
            return Err("unsupported heap component kind/version or live-resume claim".into());
        }
        if identity.is_empty() || identity.len() > 256 || self.resource_identity != identity {
            return Err("heap component resource identity mismatch".into());
        }
        if self.size == 0
            || self.size as usize > MAX_HEAP_BYTES
            || self.base.checked_add(self.size).is_none()
            || self.free.len().saturating_add(self.allocations.len()) > MAX_INTERVALS
        {
            return Err("heap component exceeds supported memory/layout bounds".into());
        }
        if self.manifest_sha256 != self.digest()? {
            return Err("heap component manifest checksum mismatch".into());
        }
        // Each list has a canonical order. Together they must partition the heap.
        for list in [&self.free, &self.allocations] {
            if list.windows(2).any(|w| w[0].offset >= w[1].offset) {
                return Err("heap component intervals are not strictly sorted".into());
            }
        }
        let mut all = self
            .free
            .iter()
            .chain(&self.allocations)
            .collect::<Vec<_>>();
        all.sort_unstable_by_key(|i| i.offset);
        let mut end = 0u32;
        for i in all {
            if i.length == 0 || i.offset != end {
                return Err(
                    "heap component layout overlaps, has a gap or an empty interval".into(),
                );
            }
            end = end
                .checked_add(i.length)
                .ok_or("heap component layout overflows")?;
            if end > self.size {
                return Err("heap component layout exceeds heap".into());
            }
        }
        if end != self.size
            || self
                .allocations
                .iter()
                .map(|i| u64::from(i.length))
                .sum::<u64>()
                != u64::from(self.allocated_bytes)
        {
            return Err("heap component allocation accounting mismatch".into());
        }
        Ok(())
    }
}
impl HeapComponent {
    pub fn capture(memory: &GuestMemory, identity: &str) -> Result<Self> {
        if memory.heap_size == 0 || memory.heap_size > MAX_HEAP_BYTES {
            return Err("heap component memory budget unsupported".into());
        }
        let _gate = memory
            .snapshot_gate
            .try_snapshot()
            .ok_or("heap capture has active memory writers")?;
        let heap = memory
            .heap
            .lock()
            .map_err(|_| "heap allocator lock poisoned")?;
        let convert = |(offset, length): (u32, usize)| -> Result<Interval> {
            Ok(Interval {
                offset,
                length: u32::try_from(length).map_err(|_| "heap interval too large")?,
            })
        };
        if heap.free.len().saturating_add(heap.allocs.len()) > MAX_INTERVALS {
            return Err("heap component interval budget exceeded".into());
        }
        let mut allocations = heap
            .allocs
            .iter()
            .map(|(&a, &b)| convert((a, b)))
            .collect::<Result<Vec<_>>>()?;
        allocations.sort_unstable_by_key(|i| i.offset);
        let bytes = memory
            .read_bytes_alloc(memory.heap_base, memory.heap_size)
            .map_err(|e| e.to_string())?;
        let mut manifest = HeapManifest {
            kind: KIND.into(),
            version: VERSION,
            live_resume: false,
            resource_identity: identity.into(),
            base: memory.heap_base,
            size: memory.heap_size as u32,
            allocated_bytes: heap.total_allocated as u32,
            free: heap
                .free
                .iter()
                .copied()
                .map(convert)
                .collect::<Result<_>>()?,
            allocations,
            payload_sha256: sha256::hex(&bytes),
            manifest_sha256: String::new(),
        };
        manifest.manifest_sha256 = manifest.digest()?;
        manifest.validate(identity)?;
        Ok(Self { manifest, bytes })
    }
    fn validate(&self, identity: &str) -> Result<()> {
        self.manifest.validate(identity)?;
        if self.bytes.len() != self.manifest.size as usize
            || sha256::hex(&self.bytes) != self.manifest.payload_sha256
        {
            return Err("heap component payload length/checksum mismatch".into());
        }
        Ok(())
    }
    /// Requires a newly constructed target, not a booted or running machine.
    /// All validation completes before either RAM or allocator metadata changes.
    pub fn restore(&self, target: &mut GuestMemory, identity: &str) -> Result<()> {
        self.validate(identity)?;
        if target.heap_base != self.manifest.base || target.heap_size != self.manifest.size as usize
        {
            return Err("heap component target memory map mismatch".into());
        }
        let _gate = target
            .snapshot_gate
            .try_snapshot()
            .ok_or("heap restore target is active")?;
        if target.snapshot_gate.counts() != (0, 0) {
            return Err("heap restore target has parked native continuations".into());
        }
        let mut heap = target
            .heap
            .lock()
            .map_err(|_| "heap allocator lock poisoned")?;
        if !heap.allocs.is_empty() || heap.total_allocated != 0 {
            return Err("heap restore requires an empty allocator".into());
        }
        target
            .validate_range(target.heap_base, self.bytes.len())
            .map_err(|e| e.to_string())?;
        let restored = Heap {
            free: self
                .manifest
                .free
                .iter()
                .map(|i| (i.offset, i.length as usize))
                .collect(),
            allocs: self
                .manifest
                .allocations
                .iter()
                .map(|i| (i.offset, i.length as usize))
                .collect::<HashMap<_, _>>(),
            total_allocated: self.manifest.allocated_bytes as usize,
        };
        target
            .write_bytes(target.heap_base, &self.bytes)
            .map_err(|e| e.to_string())?;
        *heap = restored;
        Ok(())
    }
    /// Atomic replacement leaves an existing component untouched on write failure.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate(&self.manifest.resource_identity)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        let manifest = serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?;
        if manifest.len() > MAX_MANIFEST {
            return Err("heap manifest exceeds size bound".into());
        }
        file.write_all(MAGIC)
            .and_then(|_| file.write_all(&(manifest.len() as u32).to_le_bytes()))
            .and_then(|_| file.write_all(&manifest))
            .and_then(|_| file.write_all(&self.bytes))
            .and_then(|_| file.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        std::fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
    pub fn load(path: &Path, identity: &str) -> Result<Self> {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let file_len = file.metadata().map_err(|e| e.to_string())?.len();
        if file_len > (MAX_MANIFEST + MAX_HEAP_BYTES + 12) as u64 {
            return Err("heap component file exceeds size bound".into());
        }
        let mut magic = [0; 8];
        let mut length = [0; 4];
        file.read_exact(&mut magic)
            .and_then(|_| file.read_exact(&mut length))
            .map_err(|e| e.to_string())?;
        let length = u32::from_le_bytes(length) as usize;
        if &magic != MAGIC || length == 0 || length > MAX_MANIFEST {
            return Err("invalid heap component header".into());
        }
        let mut header = vec![0; length];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let manifest: HeapManifest = serde_json::from_slice(&header).map_err(|e| e.to_string())?;
        manifest.validate(identity)?;
        if file_len != 12 + length as u64 + u64::from(manifest.size) {
            return Err("truncated or trailing heap component data".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(manifest.size as usize)
            .map_err(|e| e.to_string())?;
        bytes.resize(manifest.size as usize, 0);
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        let component = Self { manifest, bytes };
        component.validate(identity)?;
        Ok(component)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "synthetic-resource-identity-v1";
    const BASE: u32 = 0x2000_0000;
    fn fresh() -> GuestMemory {
        let mut m = GuestMemory::new();
        m.map("synthetic-heap", BASE, 4096).unwrap();
        m.set_heap(BASE, 4096);
        m
    }
    fn image() -> HeapComponent {
        let m = fresh();
        let a = m.heap_alloc(32).unwrap();
        let b = m.heap_alloc(48).unwrap();
        m.heap_alloc(16).unwrap();
        m.write_bytes(a, b"heap-marker").unwrap();
        assert!(m.heap_free(b));
        HeapComponent::capture(&m, ID).unwrap()
    }
    #[test]
    fn heap_component_roundtrip_in_a_fresh_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heap.bin");
        image().save(&path).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "memory::component::tests::heap_component_child",
                "--nocapture",
            ])
            .env("PRIME_HEAP_COMPONENT_CHILD", &path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("heap-child-restored"));
    }
    #[test]
    fn heap_component_child() {
        let Some(path) = std::env::var_os("PRIME_HEAP_COMPONENT_CHILD") else {
            return;
        };
        let image = HeapComponent::load(Path::new(&path), ID).unwrap();
        let mut memory = fresh();
        image.restore(&mut memory, ID).unwrap();
        assert_eq!(memory.read_bytes_alloc(BASE, 11).unwrap(), b"heap-marker");
        assert_eq!(memory.heap_allocated(), 48);
        assert_eq!(memory.heap_alloc(40).unwrap(), BASE + 32);
        assert_eq!(memory.heap_size_of(BASE + 80), Some(16));
        println!("heap-child-restored");
    }
    #[test]
    fn corrupt_payload_layout_identity_and_occupied_target_are_rejected_before_writes() {
        let mut im = image();
        let mut target = fresh();
        target.write_u32(BASE, 123).unwrap();
        im.bytes[0] ^= 1;
        assert!(im.restore(&mut target, ID).is_err());
        assert_eq!(target.read_u32(BASE).unwrap(), 123);
        im.bytes[0] ^= 1;
        assert!(im.restore(&mut target, "wrong-resource").is_err());
        im.manifest.allocations[0].length += 1;
        im.manifest.manifest_sha256 = im.manifest.digest().unwrap();
        assert!(im.restore(&mut target, ID).is_err());
        assert_eq!(target.heap_allocated(), 0);
        let im = image();
        target.heap_alloc(16).unwrap();
        assert!(im.restore(&mut target, ID).is_err());
        assert_eq!(target.read_u32(BASE).unwrap(), 123);
    }
    #[test]
    fn bounded_binary_atomic_replacement_and_interrupted_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heap.bin");
        let mut im = image();
        im.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        im.manifest.live_resume = true;
        assert!(im.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // An abandoned partial staging file never replaces the validated component.
        let mut interrupted = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        interrupted.write_all(&original[..20]).unwrap();
        assert!(HeapComponent::load(interrupted.path(), ID).is_err());
        assert!(HeapComponent::load(&path, ID).is_ok());
        std::fs::write(
            &path,
            [b"HPHEAP01".as_slice(), &u32::MAX.to_le_bytes()].concat(),
        )
        .unwrap();
        assert!(HeapComponent::load(&path, ID).is_err());
    }
    #[test]
    fn active_writer_defers_component_capture() {
        let m = fresh();
        let active = m.snapshot_gate.enter();
        assert!(HeapComponent::capture(&m, ID).is_err());
        drop(active);
        assert!(HeapComponent::capture(&m, ID).is_ok());
    }
}
