//! Self-authored PE32/ARM resource fixture, not vendor code or a heap capture.
pub fn pe() -> Vec<u8> {
    fn short(b: &mut [u8], at: usize, v: u16) {
        b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn word(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    word(&mut b, 0x3c, 0x80);
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    short(&mut b, 0x84, 0x1c0);
    short(&mut b, 0x86, 1);
    short(&mut b, 0x94, 0xe0);
    let opt = 0x98;
    short(&mut b, opt, 0x10b);
    word(&mut b, opt + 56, 0x2000);
    word(&mut b, opt + 60, 512);
    word(&mut b, opt + 64, 0x11223344);
    word(&mut b, opt + 92, 16);
    word(&mut b, opt + 112, 0x1000);
    word(&mut b, opt + 116, 384);
    let sec = 0x178;
    b[sec..sec + 8].copy_from_slice(b".rsrc\0\0\0");
    for (at, v) in [(8, 512), (12, 0x1000), (16, 512), (20, 512)] {
        word(&mut b, sec + at, v);
    }
    for (at, named, ids, key, target) in [
        (0, 1, 0, 0x800000b0, 0x80000030),
        (0x30, 1, 0, 0x800000c0, 0x80000060),
        (0x60, 0, 1, 0, 0x90),
    ] {
        short(&mut b, 512 + at + 12, named);
        short(&mut b, 512 + at + 14, ids);
        word(&mut b, 512 + at + 16, key);
        word(&mut b, 512 + at + 20, target);
    }
    for (at, name) in [(0xb0, "BIN"), (0xc0, "ROMSPC")] {
        short(&mut b, 512 + at, name.len() as u16);
        for (i, unit) in name.encode_utf16().enumerate() {
            short(&mut b, 514 + at + i * 2, unit);
        }
    }
    word(&mut b, 512 + 0x90, 0x1100);
    word(&mut b, 512 + 0x94, 128);
    b[768..776].copy_from_slice(&[0x4d, 0x46, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a]);
    b
}
