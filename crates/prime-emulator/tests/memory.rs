//! Guest memory tests: mapping, access validation, strings, heap.

use prime_emulator::memory::{GuestMemory, PAGE_SIZE};

fn mem_with_heap() -> GuestMemory {
    let mut m = GuestMemory::new();
    m.map("test", 0x1000_0000, 0x10_000).unwrap();
    m.map("heap", 0x2000_0000, 0x40_000).unwrap();
    m.set_heap(0x2000_0000, 0x40_000);
    m
}

#[test]
fn map_read_write_roundtrip() {
    let m = mem_with_heap();
    m.write_u32(0x1000_0004, 0xDEAD_BEEF).unwrap();
    assert_eq!(m.read_u32(0x1000_0004).unwrap(), 0xDEAD_BEEF);
    m.write_bytes(0x1000_0010, &[1, 2, 3, 4, 5]).unwrap();
    let mut buf = [0u8; 5];
    m.read_bytes(0x1000_0010, &mut buf).unwrap();
    assert_eq!(buf, [1, 2, 3, 4, 5]);
}

#[test]
fn unmapped_access_is_rejected() {
    let m = mem_with_heap();
    assert!(m.read_u32(0x9000_0000).is_err());
    assert!(m.write_u32(0x9000_0000, 1).is_err());
}

#[test]
fn access_crossing_block_end_is_rejected() {
    let m = mem_with_heap();
    // Block ends at 0x10010000; a 4-byte read at the last 2 bytes escapes.
    assert!(m.read_bytes(0x1000_FFFE, &mut [0u8; 4]).is_err());
    assert!(m.read_bytes(0x1000_FFFC, &mut [0u8; 4]).is_ok());
}

#[test]
fn overlapping_mapping_is_rejected() {
    let mut m = GuestMemory::new();
    m.map("a", 0x1000_0000, 0x10_000).unwrap();
    assert!(m.map("b", 0x1000_8000, 0x1000).is_err()); // overlaps
    assert!(m.map("c", 0x1001_0000, 0x1000).is_ok()); // adjacent is fine
}

#[test]
fn unaligned_mapping_address_is_rejected() {
    let mut m = GuestMemory::new();
    assert!(m.map("x", 0x1000_0002, 0x1000).is_err());
}

#[test]
fn cstr_read_is_bounded() {
    let m = mem_with_heap();
    m.write_bytes(0x1000_0100, b"hello\0").unwrap();
    assert_eq!(
        m.read_cstr_bounded(0x1000_0100, 16).unwrap().as_deref(),
        Some("hello")
    );
    assert_eq!(m.read_cstr_bounded(0, 16).unwrap(), None); // NULL pointer
    // Unterminated within the cap → error, never an unbounded scan.
    m.write_bytes(0x1000_0200, b"0123456789ABCDEF").unwrap(); // no NUL
    assert!(m.read_cstr_bounded(0x1000_0200, 16).is_err());
}

#[test]
fn wstr_read_stops_at_nul() {
    let m = mem_with_heap();
    let mut w = Vec::new();
    for u in "AB".encode_utf16() {
        w.extend_from_slice(&u.to_le_bytes());
    }
    w.extend_from_slice(&0u16.to_le_bytes());
    m.write_bytes(0x1000_0300, &w).unwrap();
    assert_eq!(
        m.read_wstr_bounded(0x1000_0300, 16).unwrap().as_deref(),
        Some("AB")
    );
}

#[test]
fn heap_alloc_free_and_coalesce() {
    let m = mem_with_heap();
    let a = m.heap_alloc(0x100).unwrap();
    let b = m.heap_alloc(0x100).unwrap();
    assert_eq!(a, 0x2000_0000);
    assert_eq!(b, 0x2000_0100);
    assert_eq!(m.heap_size_of(a), Some(0x100));
    assert!(m.heap_free(a));
    assert!(m.heap_free(b));
    // Fully coalesced: the next 0x200 allocation reuses the base.
    let c = m.heap_alloc(0x200).unwrap();
    assert_eq!(c, 0x2000_0000);
}

#[test]
fn heap_free_of_unknown_pointer_is_rejected() {
    let m = mem_with_heap();
    assert!(!m.heap_free(0x1234_5678));
}

#[test]
fn aligned_allocation_honors_alignment() {
    let m = mem_with_heap();
    let a = m.heap_alloc(0x10).unwrap(); // 16-byte aligned base
    let b = m.heap_alloc_aligned(0x100, 0x1_0000).unwrap();
    assert_eq!(
        b % 0x1_0000,
        0,
        "PE-aligned allocation must be 64 KiB aligned"
    );
    assert!(b > a);
    let size = m.heap_size_of(b).unwrap();
    assert_eq!(size, 0x100);
}

#[test]
fn page_round_helper() {
    assert_eq!(prime_emulator::memory::page_round(1), PAGE_SIZE as usize);
    assert_eq!(
        prime_emulator::memory::page_round(PAGE_SIZE as usize),
        PAGE_SIZE as usize
    );
    assert_eq!(
        prime_emulator::memory::page_round(PAGE_SIZE as usize + 1),
        2 * PAGE_SIZE as usize
    );
}
