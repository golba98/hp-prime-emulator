//! Synthetic ELF loader tests (no firmware required).

use prime_emulator::firmware::ElfImage;

/// Build a minimal valid ELF32 ARM executable with one PT_LOAD.
fn build_elf(entry: u32, segs: &[(u32, &[u8], usize)]) -> Vec<u8> {
    let ehdr_size = 52usize;
    let phdr_size = 32usize;
    let n = segs.len();
    let phoff = ehdr_size;
    let mut data_off = ehdr_size + phdr_size * n;
    let mut phdrs = Vec::new();
    let mut payload = Vec::new();
    let mut offs = Vec::new();
    for (vaddr, bytes, memsz) in segs {
        offs.push((data_off, *vaddr, bytes.len() as u32, *memsz as u32));
        payload.extend_from_slice(bytes);
        // pad to 4 for the next segment's offset
        while payload.len() % 4 != 0 {
            payload.push(0);
        }
        data_off = ehdr_size + phdr_size * n + payload.len();
        phdrs.extend_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        phdrs.extend_from_slice(&0u32.to_le_bytes()); // p_offset (patched below)
        phdrs.extend_from_slice(&(*vaddr).to_le_bytes()); // p_vaddr
        phdrs.extend_from_slice(&0u32.to_le_bytes()); // p_paddr
        phdrs.extend_from_slice(&(bytes.len() as u32).to_le_bytes()); // p_filesz
        phdrs.extend_from_slice(&(*memsz as u32).to_le_bytes()); // p_memsz
        phdrs.extend_from_slice(&7u32.to_le_bytes()); // p_flags RWX
        phdrs.extend_from_slice(&4u32.to_le_bytes()); // p_align
    }
    let mut out = Vec::new();
    // e_ident
    out.extend_from_slice(&[0x7F, b'E', b'L', b'F', 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    out.extend_from_slice(&40u16.to_le_bytes()); // EM_ARM
    out.extend_from_slice(&1u32.to_le_bytes()); // e_version
    out.extend_from_slice(&entry.to_le_bytes()); // e_entry
    out.extend_from_slice(&(phoff as u32).to_le_bytes()); // e_phoff
    out.extend_from_slice(&0u32.to_le_bytes()); // e_shoff
    out.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    out.extend_from_slice(&(ehdr_size as u16).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes()); // e_phentsize
    out.extend_from_slice(&(n as u16).to_le_bytes()); // e_phnum
    out.extend_from_slice(&40u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    // program headers with patched offsets
    let mut ph = Vec::new();
    for (i, (off, _, _, _)) in offs.iter().enumerate() {
        let base = i * phdr_size;
        let mut hdr = phdrs[base..base + phdr_size].to_vec();
        hdr[4..8].copy_from_slice(&(*off as u32).to_le_bytes());
        ph.extend_from_slice(&hdr);
    }
    out.extend_from_slice(&ph);
    out.extend_from_slice(&payload);
    out
}

#[test]
fn loads_valid_arm_elf() {
    let code = [0x00u8, 0x00, 0xa0, 0xe3]; // mov r0, #0
    let bytes = build_elf(0x1000, &[(0x1000, &code, 0x100)]);
    let elf = ElfImage::from_bytes(bytes).expect("parse");
    assert_eq!(elf.entry, 0x1000);
    assert_eq!(elf.segments.len(), 1);
    assert_eq!(elf.segments[0].vaddr, 0x1000);
    assert_eq!(elf.segments[0].file_size, 4);
    assert_eq!(elf.segments[0].mem_size, 0x100);
    assert!(elf.segments[0].readable && elf.segments[0].writable && elf.segments[0].executable);
    assert_eq!(elf.segment_bytes(&elf.segments[0]), &code);
}

#[test]
fn bss_larger_than_file_data_is_accepted() {
    let code = [1u8, 2, 3, 4];
    let bytes = build_elf(0x2000, &[(0x2000, &code, 0x2000)]);
    let elf = ElfImage::from_bytes(bytes).expect("parse");
    assert_eq!(elf.segments[0].mem_size, 0x2000);
    assert_eq!(elf.segments[0].file_size, 4);
}

#[test]
fn rejects_non_arm_machine() {
    let mut bytes = build_elf(0x1000, &[(0x1000, &[0; 16], 16)]);
    bytes[18..20].copy_from_slice(&3u16.to_le_bytes()); // EM_386
    assert!(ElfImage::from_bytes(bytes).is_err());
}

#[test]
fn rejects_non_exec_type() {
    let mut bytes = build_elf(0x1000, &[(0x1000, &[0; 16], 16)]);
    bytes[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
    assert!(ElfImage::from_bytes(bytes).is_err());
}

#[test]
fn rejects_truncated_file() {
    let bytes = build_elf(0x1000, &[(0x1000, &[0; 64], 64)]);
    let cut = &bytes[..bytes.len() - 10];
    assert!(ElfImage::from_bytes(cut.to_vec()).is_err());
}

#[test]
fn rejects_file_size_larger_than_mem_size() {
    let mut bytes = build_elf(0x1000, &[(0x1000, &[0; 64], 64)]);
    // Patch p_memsz < p_filesz in the first program header.
    let base = 52 + 20;
    bytes[base..base + 4].copy_from_slice(&2u32.to_le_bytes());
    assert!(ElfImage::from_bytes(bytes).is_err());
}

#[test]
fn rejects_segment_overflowing_32bit_space() {
    let bytes = build_elf(0x1000, &[(0xFFFF_0000u32, &[0; 16], 0x20_0000)]);
    assert!(ElfImage::from_bytes(bytes).is_err());
}
