//! Deliberately narrow whitelist: no system, exception, coprocessor, SIMD,
//! saturation, exclusives, banked registers, self-modifying or hidden state.
pub(super) fn allowed(bytes: &[u8; 4], thumb: bool, size: u32) -> bool {
    if thumb {
        if size != 2 {
            return false;
        }
        let h = u16::from_le_bytes([bytes[0], bytes[1]]);
        return matches!(h & 0xf800, 0x2000 | 0x2800 | 0x3000 | 0x3800 // mov/cmp/add/sub immediate
            | 0x6000 | 0x6800 | 0x9000 | 0x9800 | 0xe000) // word ldr/str or branch
            || h & 0xff00 == 0xb000 // add/sub SP immediate
            || h & 0xff00 == 0xb400 && h & 0x00ff != 0 // push, no LR
            || h & 0xff00 == 0xbc00 && h & 0x00ff != 0 // pop, no PC
            || h & 0xff87 == 0x4700 // BX, no BLX
            || h & 0xf000 == 0xd000 && h & 0x0f00 < 0x0e00; // ordinary condition branch
    }
    if size != 4 {
        return false;
    }
    let word = u32::from_le_bytes(*bytes);
    if word >> 28 == 15 {
        return false;
    } // unconditional extension space
    if word & 0x0fff_fff0 == 0x012f_ff10 {
        return true;
    } // BX
    if word & 0x0e00_0000 == 0x0a00_0000 {
        return true;
    } // B/BL
    // Word immediate LDR/STR; reject PC destination, byte and unprivileged forms.
    if word & 0x0e40_0000 == 0x0400_0000 {
        return (word >> 12) & 15 != 15 && (word & (1 << 24) != 0 || word & (1 << 21) == 0);
    }
    // Ordinary data-processing immediate or immediate-shift register forms.
    // Reject test/status aliases, multiply/extra load forms and PC destinations.
    if word & 0x0c00_0000 != 0 || (word >> 12) & 15 == 15 {
        return false;
    }
    let opcode = (word >> 21) & 15;
    if !matches!(opcode, 0 | 1 | 2 | 4 | 8 | 10 | 12 | 13 | 14 | 15) {
        return false;
    }
    if matches!(opcode, 8 | 10) && word & (1 << 20) == 0 {
        return false;
    }
    word & (1 << 25) != 0 || word & (1 << 4) == 0
}
