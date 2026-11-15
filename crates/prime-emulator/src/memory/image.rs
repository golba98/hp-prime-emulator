//! All mapped guest RAM and allocator state, captured under one memory gate.
//! This component is used when reconstructing engines; it does not include
//! threads, OS resources or session state and cannot alone resume a calculator.
use super::{
    GuestMemory, PAGE_SIZE,
    component::{HeapComponent, HeapManifest},
};
use crate::sha256;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"HPRAM001";
const VERSION: u32 = 1;
const MAX_BLOCKS: usize = 128;
const MAX_BYTES: usize = 512 * 1024 * 1024;
const MAX_HEADER: usize = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub name: String,
    pub base: u32,
    pub size: u32,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub resource_identity: String,
    pub mappings: Vec<Mapping>,
    pub heap: HeapManifest,
    pub sha256: String,
}
pub struct MemoryImage {
    pub manifest: Manifest,
    bytes: Vec<Vec<u8>>,
}
impl Manifest {
    fn digest(&self) -> Result<String> {
        serde_json::to_vec(&(
            self.version,
            &self.resource_identity,
            &self.mappings,
            &self.heap,
        ))
        .map(|bytes| sha256::hex(&bytes))
        .map_err(|e| e.to_string())
    }
    fn validate(&self, identity: &str) -> Result<usize> {
        if self.version != VERSION
            || identity.is_empty()
            || identity.len() > 256
            || self.resource_identity != identity
            || self.sha256 != self.digest()?
        {
            return Err("Mapped memory image version/identity/checksum mismatch".into());
        }
        self.heap.validate(identity)?;
        if self.mappings.is_empty() || self.mappings.len() > MAX_BLOCKS {
            return Err("Mapped memory image block count exceeds bounds".into());
        }
        let mut previous_end = 0;
        let mut total = 0usize;
        for mapping in &self.mappings {
            let end = mapping
                .base
                .checked_add(mapping.size)
                .ok_or("Mapped memory image address overflow")?;
            if mapping.size == 0
                || mapping.size % PAGE_SIZE != 0
                || mapping.base % PAGE_SIZE != 0
                || mapping.base < previous_end
                || mapping.name.is_empty()
                || mapping.name.len() > 128
                || mapping.name.chars().any(char::is_control)
                || mapping.sha256.len() != 64
                || !mapping.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid or overlapping mapped memory image layout".into());
            }
            previous_end = end;
            total = total
                .checked_add(mapping.size as usize)
                .ok_or("Memory image size overflow")?;
            if total > MAX_BYTES {
                return Err("Mapped memory image exceeds byte budget".into());
            }
        }
        if !self
            .mappings
            .iter()
            .any(|m| m.base <= self.heap.base && m.base + m.size >= self.heap.base + self.heap.size)
        {
            return Err("Mapped memory image does not contain its complete heap block".into());
        }
        Ok(total)
    }
}
impl MemoryImage {
    /// Caller coordinates engine owners and other host writers. The gate
    /// excludes running engines; parked native continuations are refused.
    pub fn capture(memory: &GuestMemory, identity: &str) -> Result<Self> {
        let gate = memory
            .snapshot_gate
            .try_snapshot()
            .ok_or("Mapped memory capture has active engines")?;
        Self::capture_while_guarded(memory, &gate, identity)
    }
    pub fn capture_while_guarded(
        memory: &GuestMemory,
        gate: &super::snapshot::Snapshot<'_>,
        identity: &str,
    ) -> Result<Self> {
        if !gate.guards(&memory.snapshot_gate) {
            return Err("Memory image requires the matching memory-owner guard".into());
        }
        if memory.snapshot_gate.counts() != (0, 0) {
            return Err("Mapped memory capture has native continuations".into());
        }
        let mut heap = HeapComponent::capture_under_gate(memory, identity)?;
        let mut blocks = memory.blocks().iter().collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.vaddr);
        if blocks.len() > MAX_BLOCKS
            || blocks.iter().map(|block| block.size).sum::<usize>() > MAX_BYTES
        {
            return Err("Mapped memory capture exceeds size budget".into());
        }
        let mut mappings = Vec::with_capacity(blocks.len());
        let mut bytes = Vec::with_capacity(blocks.len());
        for block in blocks {
            let data = if block.vaddr == heap.manifest.base && block.size == heap.bytes.len() {
                // The heap and its allocator were read in this same gate.
                std::mem::take(&mut heap.bytes)
            } else {
                memory
                    .read_bytes_alloc(block.vaddr, block.size)
                    .map_err(|e| e.to_string())?
            };
            mappings.push(Mapping {
                name: block.name.clone(),
                base: block.vaddr,
                size: block.size as u32,
                sha256: sha256::hex(&data),
            });
            bytes.push(data);
        }
        let mut manifest = Manifest {
            version: VERSION,
            resource_identity: identity.into(),
            mappings,
            heap: heap.manifest,
            sha256: String::new(),
        };
        manifest.sha256 = manifest.digest()?;
        let image = Self { manifest, bytes };
        image.validate(identity)?;
        Ok(image)
    }
    fn validate(&self, identity: &str) -> Result<()> {
        self.manifest.validate(identity)?;
        if self.bytes.len() != self.manifest.mappings.len() {
            return Err("Mapped memory image payload count mismatch".into());
        }
        for (mapping, bytes) in self.manifest.mappings.iter().zip(&self.bytes) {
            if bytes.len() != mapping.size as usize || sha256::hex(bytes) != mapping.sha256 {
                return Err("Mapped memory image payload length/checksum mismatch".into());
            }
            let heap = &self.manifest.heap;
            if mapping.base <= heap.base && mapping.base + mapping.size >= heap.base + heap.size {
                let offset = (heap.base - mapping.base) as usize;
                if sha256::hex(&bytes[offset..offset + heap.size as usize]) != heap.payload_sha256 {
                    return Err("Mapped memory image heap checksum contradiction".into());
                }
            }
        }
        Ok(())
    }
    /// Construct a new memory world. Failure never changes an existing world.
    pub fn restore(&self, identity: &str) -> Result<GuestMemory> {
        self.validate(identity)?;
        let mut memory = GuestMemory::new();
        for (mapping, bytes) in self.manifest.mappings.iter().zip(&self.bytes) {
            memory
                .map(&mapping.name, mapping.base, mapping.size as usize)
                .map_err(|e| e.to_string())?;
            memory
                .write_bytes(mapping.base, bytes)
                .map_err(|e| e.to_string())?;
        }
        let heap = &self.manifest.heap;
        memory.set_heap(heap.base, heap.size as usize);
        let restored = super::Heap {
            free: heap
                .free
                .iter()
                .map(|i| (i.offset, i.length as usize))
                .collect(),
            allocs: heap
                .allocations
                .iter()
                .map(|i| (i.offset, i.length as usize))
                .collect(),
            total_allocated: heap.allocated_bytes as usize,
        };
        *memory
            .heap
            .get_mut()
            .map_err(|_| "Restored allocator mutex poisoned")? = restored;
        Ok(memory)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate(&self.manifest.resource_identity)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        self.write_to(&mut file)?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        // Match the existing platform-aware persistence durability policy.
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Stream form allows the whole-machine archive to compress immutable RAM
    /// after owners have resumed. Decoding remains bounded by the manifest.
    pub fn write_to(&self, mut output: impl Write) -> Result<()> {
        self.validate(&self.manifest.resource_identity)?;
        let header = serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?;
        if header.len() > MAX_HEADER {
            return Err("Mapped memory image manifest exceeds bounds".into());
        }
        output
            .write_all(MAGIC)
            .and_then(|_| output.write_all(&(header.len() as u32).to_le_bytes()))
            .and_then(|_| output.write_all(&header))
            .map_err(|e| e.to_string())?;
        for data in &self.bytes {
            output.write_all(data).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn load(path: &Path, identity: &str) -> Result<Self> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > (MAX_HEADER + MAX_BYTES + 12) as u64
        {
            return Err("Mapped memory image file exceeds bounds".into());
        }
        Self::read_from(file, identity)
    }
    pub fn read_from(mut file: impl Read, identity: &str) -> Result<Self> {
        let mut magic = [0; 8];
        let mut header_size = [0; 4];
        file.read_exact(&mut magic)
            .and_then(|_| file.read_exact(&mut header_size))
            .map_err(|e| e.to_string())?;
        let header_size = u32::from_le_bytes(header_size) as usize;
        if &magic != MAGIC || header_size == 0 || header_size > MAX_HEADER {
            return Err("Invalid mapped memory image header".into());
        }
        let mut header = vec![0; header_size];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let manifest: Manifest = serde_json::from_slice(&header).map_err(|e| e.to_string())?;
        manifest.validate(identity)?;
        let mut bytes = Vec::with_capacity(manifest.mappings.len());
        for mapping in &manifest.mappings {
            let mut data = Vec::new();
            data.try_reserve_exact(mapping.size as usize)
                .map_err(|e| e.to_string())?;
            data.resize(mapping.size as usize, 0);
            file.read_exact(&mut data).map_err(|e| e.to_string())?;
            bytes.push(data);
        }
        let mut trailing = [0];
        if file.read(&mut trailing).map_err(|e| e.to_string())? != 0 {
            return Err("Trailing mapped memory image payload".into());
        }
        let image = Self { manifest, bytes };
        image.validate(identity)?;
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory() -> GuestMemory {
        let mut memory = GuestMemory::new();
        memory.map("heap", 0x20000000, 8192).unwrap();
        memory.map("code", 0x10000000, 4096).unwrap();
        memory.map("peripheral", 0x40000000, 4096).unwrap();
        memory.set_heap(0x20000000, 8192);
        let first = memory.heap_alloc(31).unwrap();
        let hole = memory.heap_alloc(17).unwrap();
        memory.heap_alloc(45).unwrap();
        memory.heap_free(hole);
        memory.write_bytes(first, b"allocated").unwrap();
        memory.write_bytes(0x10000000, b"code marker").unwrap();
        memory.write_bytes(0x40000000, b"device marker").unwrap();
        memory
    }
    #[test]
    fn every_mapping_and_allocator_round_trip_then_allocate_identically() {
        let original = memory();
        let image = MemoryImage::capture(&original, "map-test").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ram.bin");
        image.save(&path).unwrap();
        let image = MemoryImage::load(&path, "map-test").unwrap();
        let restored = image.restore("map-test").unwrap();
        assert_eq!(
            restored
                .blocks()
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["code", "heap", "peripheral"]
        );
        for block in original.blocks() {
            assert_eq!(
                original.read_bytes_alloc(block.vaddr, block.size).unwrap(),
                restored.read_bytes_alloc(block.vaddr, block.size).unwrap()
            );
        }
        assert_eq!(original.heap_stats(), restored.heap_stats());
        for size in [16, 33, 127, 3000] {
            assert_eq!(
                original.heap_alloc(size).unwrap(),
                restored.heap_alloc(size).unwrap()
            );
        }
    }
    #[test]
    fn bad_payload_identity_layout_and_native_callbacks_are_refused() {
        let memory = memory();
        let mut image = MemoryImage::capture(&memory, "map-test").unwrap();
        assert!(image.restore("other").is_err());
        image.bytes[0][0] ^= 1;
        assert!(image.restore("map-test").is_err());
        image.bytes[0][0] ^= 1;
        image.manifest.mappings[1].base = image.manifest.mappings[0].base;
        image.manifest.sha256 = image.manifest.digest().unwrap();
        assert!(image.restore("map-test").is_err());
        let active = memory.snapshot_gate.enter();
        assert!(MemoryImage::capture(&memory, "map-test").is_err());
        let parked = memory.snapshot_gate.park();
        assert!(MemoryImage::capture(&memory, "map-test").is_err());
        drop(parked);
        drop(active);
        assert!(MemoryImage::capture(&memory, "map-test").is_ok());
    }
    #[test]
    fn truncated_and_trailing_data_fail_before_constructing_memory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ram.bin");
        MemoryImage::capture(&memory(), "map-test")
            .unwrap()
            .save(&path)
            .unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(1);
        std::fs::write(&path, &bytes).unwrap();
        assert!(MemoryImage::load(&path, "map-test").is_err());
        bytes.truncate(bytes.len() - 2);
        std::fs::write(&path, &bytes).unwrap();
        assert!(MemoryImage::load(&path, "map-test").is_err());
    }
    #[test]
    fn a_smaller_heap_budget_preserves_bytes_outside_the_allocator() {
        let mut memory = GuestMemory::new();
        memory.map("bounded-heap", 0x20000000, 8192).unwrap();
        memory.set_heap(0x20000000, 4096);
        memory.heap_alloc(16).unwrap();
        memory
            .write_bytes(0x20001000, b"outside allocator")
            .unwrap();
        let restored = MemoryImage::capture(&memory, "bounded")
            .unwrap()
            .restore("bounded")
            .unwrap();
        assert_eq!(restored.heap_size(), 4096);
        assert_eq!(restored.blocks()[0].size, 8192);
        assert_eq!(
            restored.read_bytes_alloc(0x20001000, 17).unwrap(),
            b"outside allocator"
        );
    }
}
