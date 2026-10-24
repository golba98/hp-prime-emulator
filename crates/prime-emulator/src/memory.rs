//! Guest memory: host-backed blocks shared by all CPU engines, plus the
//! dynamic heap used by the OS memory services.
//!
//! Every mapped region is a host `mmap` that all Unicorn engines map with
//! `uc_mem_map_ptr`, so guest and host views are the same pages and host
//! service handlers can access guest memory directly and coherently. All
//! guest accesses go through the validated accessors here; handlers never
//! touch raw pointers themselves.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use memmap2::MmapMut;

pub const PAGE_SIZE: u32 = 0x1000;

#[derive(Debug)]
pub struct MemoryError {
    pub va: u32,
    pub msg: String,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "guest memory error at {:#010X}: {}", self.va, self.msg)
    }
}

impl std::error::Error for MemoryError {}

pub type MemResult<T> = Result<T, MemoryError>;

fn err<T>(va: u32, msg: impl Into<String>) -> MemResult<T> {
    Err(MemoryError {
        va,
        msg: msg.into(),
    })
}

/// Round up to the 4 KiB granularity Unicorn requires for mappings.
pub fn page_round(size: usize) -> usize {
    (size + PAGE_SIZE as usize - 1) & !(PAGE_SIZE as usize - 1)
}

pub struct MemBlock {
    pub name: &'static str,
    pub vaddr: u32,
    pub size: usize, // always page-aligned; host map has exactly this length
    map: MmapMut,
}

impl MemBlock {
    /// Host pointer of the block start. Valid for the lifetime of the
    /// owning `GuestMemory` (blocks are never unmapped).
    pub fn host_ptr(&self) -> *mut u8 {
        // The mapping is private anonymous memory; guest engines and host
        // accessors both mutate it, so the const-to-mut cast is sound.
        self.map.as_ptr() as *mut u8
    }

    pub fn end(&self) -> u32 {
        self.vaddr + self.size as u32
    }

    pub fn contains(&self, va: u32) -> bool {
        va >= self.vaddr && va < self.end()
    }
}

/// Free-list allocator over the pre-mapped heap block. The layout is private
/// to the host; the guest only ever observes returned pointers. 16-byte
/// alignment matches the reference emulator's allocator.
struct Heap {
    free: Vec<(u32, usize)>, // (start, size), sorted by start
    allocs: HashMap<u32, usize>,
    total_allocated: usize,
}

impl Heap {
    fn new(size: usize) -> Self {
        Heap {
            free: vec![(0, size)],
            allocs: HashMap::new(),
            total_allocated: 0,
        }
    }

    fn alloc(&mut self, size: usize) -> Option<u32> {
        if size == 0 {
            return None;
        }
        let aligned = (size + 15) & !15;
        let i = self.free.iter().position(|&(_, fsize)| fsize >= aligned)?;
        let (start, fsize) = self.free[i];
        if fsize > aligned {
            self.free[i] = (start + aligned as u32, fsize - aligned);
        } else {
            self.free.remove(i);
        }
        self.allocs.insert(start, aligned);
        self.total_allocated += aligned;
        Some(start)
    }

    /// First-fit with alignment: the padding before the aligned start stays
    /// in the free list.
    fn alloc_aligned(&mut self, size: usize, align: u32) -> Option<u32> {
        if size == 0 {
            return None;
        }
        let size = size as u32;
        let mut chosen: Option<(usize, u32, u32)> = None; // (index, start, padded size)
        for (i, &(start, fsize)) in self.free.iter().enumerate() {
            let fsize = fsize as u32;
            let aligned_start = start.div_ceil(align) * align;
            let pad = aligned_start - start;
            if fsize >= pad && fsize - pad >= size {
                chosen = Some((i, aligned_start, pad + size));
                break;
            }
        }
        let (i, start, total) = chosen?;
        let (block_start, fsize) = self.free[i];
        let total_us = total as usize;
        if block_start < start {
            // Keep the head padding as its own free block.
            self.free[i] = (block_start, (start - block_start) as usize);
            if fsize > total_us {
                self.free.insert(i + 1, (start + total, fsize - total_us));
            }
        } else if fsize > total_us {
            self.free[i] = (start + total, fsize - total_us);
        } else {
            self.free.remove(i);
        }
        self.allocs.insert(start, size as usize);
        self.total_allocated += size as usize;
        Some(start)
    }

    fn free(&mut self, offset: u32) -> bool {
        let Some(size) = self.allocs.remove(&offset) else {
            return false;
        };
        self.total_allocated -= size;
        let pos = self.free.partition_point(|&(s, _)| s < offset);
        self.free.insert(pos, (offset, size));
        // Coalesce with the neighbours so long-lived runs do not fragment.
        let mut pos = pos;
        if pos > 0 && self.free[pos - 1].0 + self.free[pos - 1].1 as u32 == offset {
            self.free[pos - 1].1 += size;
            self.free.remove(pos);
            pos -= 1;
        }
        if pos + 1 < self.free.len()
            && self.free[pos].0 + self.free[pos].1 as u32 == self.free[pos + 1].0
        {
            let next = self.free.remove(pos + 1);
            self.free[pos].1 += next.1;
        }
        true
    }
}

pub struct GuestMemory {
    blocks: Vec<MemBlock>,
    heap: Mutex<Heap>,
    /// Heap offsets are relative to this base (stored as the guest address).
    heap_base: u32,
    heap_size: usize,
}

impl Default for GuestMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestMemory {
    pub fn new() -> Self {
        GuestMemory {
            blocks: Vec::new(),
            heap: Mutex::new(Heap::new(0)),
            heap_base: 0,
            heap_size: 0,
        }
    }

    /// Map an anonymous zeroed region. `vaddr` must be page-aligned and must
    /// not overlap any existing block.
    pub fn map(&mut self, name: &'static str, vaddr: u32, size: usize) -> MemResult<()> {
        let rounded = page_round(size);
        if rounded == 0 {
            return err(vaddr, "zero-size mapping");
        }
        if vaddr % PAGE_SIZE != 0 {
            return err(vaddr, "mapping start not 4 KiB aligned");
        }
        let end = vaddr.checked_add(rounded as u32).ok_or(MemoryError {
            va: vaddr,
            msg: "mapping end overflows 32-bit space".into(),
        })?;
        for b in &self.blocks {
            if vaddr < b.end() && b.vaddr < end {
                return err(
                    vaddr,
                    format!("overlaps existing block '{}' at {:#X}", b.name, b.vaddr),
                );
            }
        }
        let map = MmapMut::map_anon(rounded).map_err(|e| MemoryError {
            va: vaddr,
            msg: format!("host mmap failed: {e}"),
        })?;
        self.blocks.push(MemBlock {
            name,
            vaddr,
            size: rounded,
            map,
        });
        Ok(())
    }

    /// Designate a mapped region as the dynamic heap used by the OS memory
    /// services. Must be called after mapping it.
    pub fn set_heap(&mut self, base: u32, size: usize) {
        self.heap_base = base;
        self.heap_size = page_round(size);
        *self.heap.get_mut().unwrap() = Heap::new(self.heap_size);
    }

    pub fn heap_base(&self) -> u32 {
        self.heap_base
    }

    pub fn heap_size(&self) -> usize {
        self.heap_size
    }

    pub fn heap_alloc(&self, size: usize) -> MemResult<u32> {
        let off = self.heap.lock().unwrap().alloc(size).ok_or(MemoryError {
            va: 0,
            msg: format!("heap exhausted (requested {size} bytes)"),
        })?;
        Ok(self.heap_base + off)
    }

    /// Allocate with a larger alignment (e.g. 64 KiB PE image alignment).
    pub fn heap_alloc_aligned(&self, size: usize, align: u32) -> MemResult<u32> {
        if align <= 16 {
            return self.heap_alloc(size);
        }
        let aligned = (size + 15) & !15;
        let off = self
            .heap
            .lock()
            .unwrap()
            .alloc_aligned(aligned, align)
            .ok_or(MemoryError {
                va: 0,
                msg: format!("heap exhausted (requested {size} bytes, aligned)"),
            })?;
        Ok(self.heap_base + off)
    }

    pub fn heap_free(&self, va: u32) -> bool {
        if va < self.heap_base {
            return false;
        }
        self.heap.lock().unwrap().free(va - self.heap_base)
    }

    pub fn heap_allocated(&self) -> usize {
        self.heap.lock().unwrap().total_allocated
    }

    /// `(live bytes, live block count, largest free block)` — diagnostics
    /// for allocation-failure analysis (leak vs fragmentation vs a single
    /// oversized request).
    pub fn heap_stats(&self) -> (usize, usize, usize) {
        let h = self.heap.lock().unwrap();
        let blocks = h.allocs.len();
        let largest = h.free.iter().map(|&(_, s)| s).max().unwrap_or(0);
        (h.total_allocated, blocks, largest)
    }

    /// Allocated (aligned) size of a live heap block, if it is one.
    pub fn heap_size_of(&self, va: u32) -> Option<usize> {
        if va < self.heap_base {
            return None;
        }
        self.heap
            .lock()
            .unwrap()
            .allocs
            .get(&(va - self.heap_base))
            .copied()
    }

    pub fn blocks(&self) -> &[MemBlock] {
        &self.blocks
    }

    /// If a mapped block fully covers [va, va+size), return its base.
    pub fn covering_block(&self, va: u32, size: usize) -> Option<u32> {
        let end = va.checked_add(size as u32)?;
        self.blocks
            .iter()
            .find(|b| va >= b.vaddr && end <= b.end())
            .map(|b| b.vaddr)
    }

    fn block_for(&self, va: u32, len: usize) -> MemResult<&MemBlock> {
        if len == 0 {
            return err(va, "zero-length access");
        }
        let end = va.checked_add(len as u32).ok_or(MemoryError {
            va,
            msg: "access end overflows 32-bit address space".into(),
        })?;
        self.blocks
            .iter()
            .find(|b| va >= b.vaddr && end <= b.end())
            .ok_or(MemoryError {
                va,
                msg: format!("unmapped access of {len} bytes"),
            })
    }

    /// Validated host pointer for a guest range. Blocks are never unmapped,
    /// so the pointer stays valid for the lifetime of `self`.
    fn host_ptr(&self, va: u32, len: usize) -> MemResult<*mut u8> {
        let b = self.block_for(va, len)?;
        let off = (va - b.vaddr) as usize;
        // SAFETY: `off + len <= b.size` was validated by block_for.
        Ok(unsafe { b.map.as_ptr().add(off) as *mut u8 })
    }

    pub fn read_bytes(&self, va: u32, buf: &mut [u8]) -> MemResult<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let p = self.host_ptr(va, buf.len())?;
        // SAFETY: pointer is validated for this exact range.
        unsafe { std::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }

    pub fn read_bytes_alloc(&self, va: u32, len: usize) -> MemResult<Vec<u8>> {
        let mut v = vec![0u8; len];
        self.read_bytes(va, &mut v)?;
        Ok(v)
    }

    pub fn write_bytes(&self, va: u32, bytes: &[u8]) -> MemResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let p = self.host_ptr(va, bytes.len())?;
        // SAFETY: pointer is validated for this exact range.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
        Ok(())
    }

    pub fn read_u32(&self, va: u32) -> MemResult<u32> {
        let mut b = [0u8; 4];
        self.read_bytes(va, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    pub fn write_u32(&self, va: u32, v: u32) -> MemResult<()> {
        self.write_bytes(va, &v.to_le_bytes())
    }

    pub fn write_u16(&self, va: u32, v: u16) -> MemResult<()> {
        self.write_bytes(va, &v.to_le_bytes())
    }

    pub fn write_u8(&self, va: u32, v: u8) -> MemResult<()> {
        self.write_bytes(va, &[v])
    }

    pub fn read_u16(&self, va: u32) -> MemResult<u16> {
        let mut b = [0u8; 2];
        self.read_bytes(va, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    /// Read a NUL-terminated guest string, bounded by the containing block
    /// and `max`. Unterminated strings are errors, never unbounded reads.
    pub fn read_cstr_bounded(&self, va: u32, max: usize) -> MemResult<Option<String>> {
        if va == 0 {
            return Ok(None);
        }
        let b = self.block_for(va, 1)?;
        let cap = ((b.end() - va) as usize).min(max);
        let p = self.host_ptr(va, 1)?;
        // SAFETY: slice stays inside [va, b.end()).
        let slice = unsafe { std::slice::from_raw_parts(p, cap) };
        match slice.iter().position(|&c| c == 0) {
            Some(n) => {
                let s = std::str::from_utf8(&slice[..n]).map_err(|_| MemoryError {
                    va,
                    msg: "guest string is not UTF-8".into(),
                })?;
                Ok(Some(s.to_owned()))
            }
            None => err(va, "unterminated guest string"),
        }
    }

    /// Read a NUL-terminated UTF-16LE guest string (Windows CE wide string).
    pub fn read_wstr_bounded(&self, va: u32, max_chars: usize) -> MemResult<Option<String>> {
        if va == 0 {
            return Ok(None);
        }
        let b = self.block_for(va, 2)?;
        let cap = (((b.end() - va) / 2) as usize).min(max_chars);
        let mut s = String::with_capacity(cap * 2);
        for i in 0..cap {
            let c = self.read_u16(va + (i as u32) * 2)?;
            if c == 0 {
                return Ok(Some(s));
            }
            s.push(char::from_u32(c as u32).unwrap_or('\u{FFFD}'));
        }
        err(va, "unterminated guest wide string")
    }

    /// Write a NUL-terminated string into guest memory. Fails if the write
    /// (including the terminator) leaves the destination block.
    pub fn write_cstr(&self, va: u32, s: &str) -> MemResult<()> {
        let bytes = s.as_bytes();
        self.write_bytes(va, bytes)?;
        self.write_u32(va + bytes.len() as u32, 0)
    }
}
