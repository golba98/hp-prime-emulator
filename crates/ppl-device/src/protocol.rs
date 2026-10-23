//! Wire-format helpers shared by the hplp v1 framing.

/// Extracts runs of printable UTF-16LE text of at least `min` characters.
pub fn utf16_runs(bytes: &[u8], min: usize) -> Vec<String> {
    let mut out = Vec::new();
    for start in 0..2 {
        let units: Vec<u16> = bytes[start..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut cur = String::new();
        for u in units {
            match char::from_u32(u as u32) {
                Some(c)
                    if !c.is_control()
                        && u != 0
                        && !(0xD800..=0xDFFF).contains(&u)
                        && u < 0x3000 =>
                {
                    cur.push(c)
                }
                _ => {
                    if cur.chars().count() >= min && cur.chars().any(|c| c.is_ascii_alphanumeric())
                    {
                        out.push(std::mem::take(&mut cur));
                    } else {
                        cur.clear();
                    }
                }
            }
        }
        if cur.chars().count() >= min && cur.chars().any(|c| c.is_ascii_alphanumeric()) {
            out.push(cur);
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// CRC-16/XMODEM (poly 0x1021, init 0), as used by hplp `crc16_block`.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}
