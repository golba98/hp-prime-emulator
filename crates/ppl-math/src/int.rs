//! HP Prime "integer" objects: `#…h`, `#…b`, `#…o`, `#…d` literals.
//!
//! These are word-sized integers that carry their display base. The default
//! word size (32 bits) and default base (hexadecimal) follow the Home Settings
//! factory defaults in the User Guide. Mixed arithmetic rules are
//! **unverified** on the G1.

use std::fmt;

use crate::MathError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntBase {
    Bin,
    Oct,
    Dec,
    Hex,
}

impl IntBase {
    pub fn radix(self) -> u32 {
        match self {
            IntBase::Bin => 2,
            IntBase::Oct => 8,
            IntBase::Dec => 10,
            IntBase::Hex => 16,
        }
    }

    pub fn suffix(self) -> char {
        match self {
            IntBase::Bin => 'b',
            IntBase::Oct => 'o',
            IntBase::Dec => 'd',
            IntBase::Hex => 'h',
        }
    }

    pub fn from_suffix(c: char) -> Option<IntBase> {
        match c.to_ascii_lowercase() {
            'b' => Some(IntBase::Bin),
            'o' => Some(IntBase::Oct),
            'd' => Some(IntBase::Dec),
            'h' => Some(IntBase::Hex),
            _ => None,
        }
    }
}

pub const DEFAULT_BITS: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HpInt {
    value: u64,
    bits: u8,
    pub base: IntBase,
}

fn mask(bits: u8) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

impl HpInt {
    pub fn new(value: i128, bits: u8, base: IntBase) -> HpInt {
        let bits = bits.clamp(1, 64);
        HpInt {
            value: (value as u64) & mask(bits),
            bits,
            base,
        }
    }

    pub fn bits(self) -> u8 {
        self.bits
    }

    /// Value as an unsigned word.
    pub fn unsigned(self) -> u64 {
        self.value
    }

    /// Value read as a two's-complement signed word.
    pub fn signed(self) -> i64 {
        if self.bits >= 64 {
            return self.value as i64;
        }
        let sign = 1u64 << (self.bits - 1);
        if self.value & sign != 0 {
            (self.value as i128 - (1i128 << self.bits)) as i64
        } else {
            self.value as i64
        }
    }

    /// Parses the text after `#`: digits, optional `:bits`, optional base
    /// suffix (either order). `default_base` applies when no suffix is given.
    pub fn parse_literal(text: &str, default_base: IntBase) -> Option<HpInt> {
        let mut t = text.trim();
        let mut bits = DEFAULT_BITS;
        let mut base: Option<IntBase> = None;
        if let Some(i) = t.find(':') {
            let (head, tail) = t.split_at(i);
            let tail = &tail[1..];
            let (num, suf) = match tail.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
                Some((j, _)) => (&tail[..j], &tail[j..]),
                None => (tail, ""),
            };
            bits = num.parse().ok().filter(|b| (1..=64).contains(b))?;
            if !suf.is_empty() {
                let mut cs = suf.chars();
                base = Some(IntBase::from_suffix(cs.next()?)?);
                if cs.next().is_some() {
                    return None;
                }
            }
            t = head;
        }
        if base.is_none() {
            if let Some(last) = t.chars().last() {
                if let Some(b) = IntBase::from_suffix(last) {
                    let digits = &t[..t.len() - 1];
                    // `#1Bh` style: suffix only counts if what is left parses.
                    if !digits.is_empty() && u64::from_str_radix(digits, b.radix()).is_ok() {
                        base = Some(b);
                        t = digits;
                    }
                }
            }
        }
        let base = base.unwrap_or(default_base);
        let v = u64::from_str_radix(t, base.radix()).ok()?;
        Some(HpInt::new(v as i128, bits, base))
    }

    fn rewrap(self, v: i128) -> HpInt {
        HpInt::new(v, self.bits, self.base)
    }

    pub fn add(self, o: HpInt) -> HpInt {
        self.rewrap(self.value as i128 + o.value as i128)
    }

    pub fn sub(self, o: HpInt) -> HpInt {
        self.rewrap(self.value as i128 - o.value as i128)
    }

    pub fn mul(self, o: HpInt) -> HpInt {
        self.rewrap((self.value as i128).wrapping_mul(o.value as i128))
    }

    pub fn div(self, o: HpInt) -> Result<HpInt, MathError> {
        if o.value == 0 {
            return Err(MathError::DivisionByZero);
        }
        Ok(self.rewrap((self.value / o.value) as i128))
    }

    pub fn rem(self, o: HpInt) -> Result<HpInt, MathError> {
        if o.value == 0 {
            return Err(MathError::DivisionByZero);
        }
        Ok(self.rewrap((self.value % o.value) as i128))
    }

    pub fn neg(self) -> HpInt {
        self.rewrap(-(self.value as i128))
    }

    pub fn not(self) -> HpInt {
        self.rewrap(!self.value as i128)
    }

    pub fn and(self, o: HpInt) -> HpInt {
        self.rewrap((self.value & o.value) as i128)
    }

    pub fn or(self, o: HpInt) -> HpInt {
        self.rewrap((self.value | o.value) as i128)
    }

    pub fn xor(self, o: HpInt) -> HpInt {
        self.rewrap((self.value ^ o.value) as i128)
    }

    pub fn shift_left(self, n: u32) -> HpInt {
        if n >= 64 {
            self.rewrap(0)
        } else {
            self.rewrap(((self.value as u128) << n) as i128)
        }
    }

    pub fn shift_right(self, n: u32) -> HpInt {
        if n >= 64 {
            self.rewrap(0)
        } else {
            self.rewrap((self.value >> n) as i128)
        }
    }

    pub fn with_base(self, base: IntBase) -> HpInt {
        HpInt { base, ..self }
    }

    pub fn digits(self) -> String {
        match self.base {
            IntBase::Bin => format!("{:b}", self.value),
            IntBase::Oct => format!("{:o}", self.value),
            IntBase::Dec => format!("{}", self.value),
            IntBase::Hex => format!("{:X}", self.value),
        }
    }
}

impl fmt::Display for HpInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}{}", self.digits(), self.base.suffix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        let p = |s| HpInt::parse_literal(s, IntBase::Hex).unwrap();
        assert_eq!(p("FFh").unsigned(), 255);
        assert_eq!(p("1010b").unsigned(), 10);
        assert_eq!(p("777o").unsigned(), 511);
        assert_eq!(p("255d").unsigned(), 255);
        assert_eq!(p("1Bh").unsigned(), 27);
        assert_eq!(p("AF").unsigned(), 0xAF);
        assert_eq!(p("0").unsigned(), 0);
        assert_eq!(p("AF:16h").bits(), 16);
        assert_eq!(p("FFh").to_string(), "#FFh");
        assert!(HpInt::parse_literal("XYZ", IntBase::Hex).is_none());
    }

    #[test]
    fn wraps_to_word_size() {
        let a = HpInt::parse_literal("FFFFFFFFh", IntBase::Hex).unwrap();
        let one = HpInt::parse_literal("1h", IntBase::Hex).unwrap();
        assert_eq!(a.add(one).unsigned(), 0);
        assert_eq!(a.signed(), -1);
    }
}
