//! HP Prime "integer" objects: `#…h`, `#…b`, `#…o`, `#…d` literals.
//!
//! These are word-sized integers that carry their display base. The default
//! word size (32 bits) and default base (hexadecimal) follow the Home Settings
//! factory defaults in the User Guide. Signed-width and arithmetic rules are
//! covered by self-authored original virtual-firmware controls.

use std::fmt;

use crate::{MathError, Real};

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
    signed_word: bool,
    negative: bool,
    system_base: bool,
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
            signed_word: false,
            negative: false,
            system_base: false,
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

    /// A signed width reserves an additional sign bit; its magnitude width
    /// is1..63. Unsigned widths are1..64. Positive and negative operands
    /// retain their mathematical sign when arithmetic wraps the magnitude.
    pub fn new_with_width(value: i128, width: i64, base: IntBase) -> Option<Self> {
        let (bits, signed_word) = match width {
            -63..=-1 => ((-width) as u8, true),
            1..=64 => (width as u8, false),
            _ => return None,
        };
        Some(Self {
            value: value as u64 & mask(bits),
            bits,
            base,
            signed_word,
            negative: signed_word && value < 0,
            system_base: false,
        })
    }
    /// Admit an original firmware carrier only when all bits outside its
    /// width agree with its actual mathematical sign. Reserved padding is
    /// never interpreted as value data.
    pub fn from_raw(raw: u64, width: i64, base: IntBase) -> Option<Self> {
        let mut result = Self::new_with_width(0, width, base)?;
        let outside = raw & !mask(result.bits);
        if outside != 0 {
            if !result.signed_word || outside != !mask(result.bits) {
                return None;
            }
            result.negative = true;
        }
        result.value = raw & mask(result.bits);
        Some(result)
    }
    pub fn is_signed(self) -> bool {
        self.signed_word
    }
    pub fn is_negative(self) -> bool {
        self.negative
    }
    pub fn is_system_base(self) -> bool {
        self.system_base
    }
    pub fn width_code(self) -> i64 {
        if self.signed_word {
            -(self.bits as i64)
        } else {
            self.bits as i64
        }
    }
    pub fn with_system_base(self, system_base: bool) -> Self {
        Self {
            system_base,
            ..self
        }
    }
    pub fn numeric_value(self) -> i128 {
        self.value as i128 - if self.negative { 1i128 << self.bits } else { 0 }
    }
    pub fn raw_value(self) -> u64 {
        self.value | if self.negative { !mask(self.bits) } else { 0 }
    }
    pub fn to_real(self) -> Real {
        if self.negative {
            Real::from_i64(self.numeric_value() as i64)
        } else {
            Real::from_u64(self.value)
        }
    }
    pub fn with_value(self, value: i128) -> Self {
        Self::new_with_width(value, self.width_code(), self.base)
            .expect("admitted word width")
            .with_system_base(self.system_base)
    }
    pub fn with_word_size(self, width: i64) -> Option<Self> {
        Self::new_with_width(self.numeric_value(), width, self.base)
            .map(|word| word.with_system_base(self.system_base))
    }

    /// Parses digits, an optional signed width and base suffix. Unsuffixed
    /// words keep their system-base affinity independently of their value.
    pub fn parse_literal(text: &str, default_base: IntBase) -> Option<Self> {
        Self::parse_literal_with_defaults(text, default_base, DEFAULT_BITS as i64)
    }
    pub fn parse_literal_with_defaults(
        text: &str,
        default_base: IntBase,
        default_width: i64,
    ) -> Option<Self> {
        let normalized = text.trim().replace('−', "-");
        let mut t = normalized.as_str();
        let mut width = default_width;
        let mut base = None;
        if let Some((head, tail)) = t.split_once(':') {
            let digits = tail.strip_prefix('-').unwrap_or(tail);
            let end = digits
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(digits.len());
            if end == 0 {
                return None;
            }
            let sign = if tail.starts_with('-') { -1 } else { 1 };
            width = digits[..end].parse::<i64>().ok()?.checked_mul(sign)?;
            let suffix = &digits[end..];
            if !suffix.is_empty() {
                let mut characters = suffix.chars();
                base = Some(IntBase::from_suffix(characters.next()?)?);
                if characters.next().is_some() {
                    return None;
                }
            }
            t = head;
        }
        let negative = t.starts_with('-');
        t = t.strip_prefix('-').unwrap_or(t);
        if base.is_none()
            && let Some(last) = t.chars().last()
            && let Some(candidate) = IntBase::from_suffix(last)
        {
            let digits = &t[..t.len() - last.len_utf8()];
            if !digits.is_empty() && u64::from_str_radix(digits, candidate.radix()).is_ok() {
                base = Some(candidate);
                t = digits;
            }
        }
        let system_base = base.is_none();
        let base = base.unwrap_or(default_base);
        let value = u64::from_str_radix(t, base.radix()).ok()? as i128;
        Self::new_with_width(if negative { -value } else { value }, width, base)
            .map(|word| word.with_system_base(system_base))
    }

    fn rewrap(self, value: i128) -> Self {
        self.with_value(value)
    }

    pub fn add(self, o: HpInt) -> HpInt {
        self.rewrap(self.numeric_value() + o.numeric_value())
    }

    pub fn sub(self, o: HpInt) -> HpInt {
        self.rewrap(self.numeric_value() - o.numeric_value())
    }

    pub fn mul(self, o: HpInt) -> HpInt {
        self.rewrap(self.numeric_value().wrapping_mul(o.numeric_value()))
    }

    pub fn div(self, o: HpInt) -> Result<HpInt, MathError> {
        if o.numeric_value() == 0 {
            return Err(MathError::DivisionByZero);
        }
        Ok(self.rewrap(self.numeric_value() / o.numeric_value()))
    }

    pub fn rem(self, o: HpInt) -> Result<HpInt, MathError> {
        if o.numeric_value() == 0 {
            return Err(MathError::DivisionByZero);
        }
        Ok(self.rewrap(self.numeric_value() % o.numeric_value()))
    }

    pub fn neg(self) -> HpInt {
        self.rewrap(-self.numeric_value())
    }

    fn bit_result(self, raw: u64) -> Self {
        self.rewrap(if self.signed_word {
            (raw as i64) as i128
        } else {
            raw as i128
        })
    }
    pub fn not(self) -> Self {
        self.bit_result(!self.raw_value())
    }
    pub fn and(self, other: Self) -> Self {
        self.bit_result(self.raw_value() & other.raw_value())
    }
    pub fn or(self, other: Self) -> Self {
        self.bit_result(self.raw_value() | other.raw_value())
    }
    pub fn xor(self, other: Self) -> Self {
        self.bit_result(self.raw_value() ^ other.raw_value())
    }
    pub fn shift_left(self, count: u32) -> Self {
        if count >= 64 {
            self.rewrap(0)
        } else {
            self.rewrap(self.numeric_value() << count)
        }
    }
    pub fn shift_right(self, count: u32) -> Self {
        if count >= 64 {
            self.rewrap(if self.negative { -1 } else { 0 })
        } else {
            self.rewrap(self.numeric_value() >> count)
        }
    }
    pub fn pow(self, mut exponent: u64) -> Self {
        let mut result = self.with_value(1);
        let mut base = self;
        while exponent != 0 {
            if exponent & 1 != 0 {
                result = result.mul(base);
            }
            exponent >>= 1;
            if exponent != 0 {
                base = base.mul(base);
            }
        }
        result
    }

    pub fn with_base(self, base: IntBase) -> HpInt {
        HpInt {
            base,
            system_base: false,
            ..self
        }
    }

    /// Render with the current display context while retaining explicit
    /// signed-width and system-base metadata in the value itself.
    pub fn format(self, default_bits: u8) -> String {
        self.format_for(default_bits, false, self.base)
    }
    pub fn format_for(
        self,
        default_bits: u8,
        default_signed: bool,
        system_base: IntBase,
    ) -> String {
        let base = if self.system_base {
            system_base
        } else {
            self.base
        };
        let width = if self.bits == default_bits && self.signed_word == default_signed {
            String::new()
        } else {
            format!(":{}", self.width_code())
        };
        format!(
            "#{}{}{}{}",
            if self.negative { "−" } else { "" },
            self.magnitude_digits(base),
            width,
            base.suffix()
        )
    }
    fn magnitude_digits(self, base: IntBase) -> String {
        let value = self.numeric_value().unsigned_abs();
        match base {
            IntBase::Bin => format!("{value:b}"),
            IntBase::Oct => format!("{value:o}"),
            IntBase::Dec => format!("{value}"),
            IntBase::Hex => format!("{value:X}"),
        }
    }
    pub fn digits(self) -> String {
        self.magnitude_digits(self.base)
    }
}
impl fmt::Display for HpInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{}{}{}{}",
            if self.negative { "-" } else { "" },
            self.digits(),
            if self.signed_word {
                format!(":{}", self.width_code())
            } else {
                String::new()
            },
            self.base.suffix()
        )
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

#[cfg(test)]
mod g1_signed_controls {
    use super::*;
    #[test]
    fn signed_width_and_actual_negative_carrier_are_independent() {
        let positive = HpInt::parse_literal("FF:-8h", IntBase::Hex).unwrap();
        let negative = HpInt::parse_literal("−1:-8h", IntBase::Hex).unwrap();
        let minimum = HpInt::parse_literal("-100:-8h", IntBase::Hex).unwrap();
        assert_eq!(
            (
                positive.width_code(),
                positive.numeric_value(),
                positive.raw_value()
            ),
            (-8, 255, 255)
        );
        assert_eq!(
            (
                negative.width_code(),
                negative.numeric_value(),
                negative.unsigned(),
                negative.raw_value()
            ),
            (-8, -1, 255, u64::MAX)
        );
        assert_eq!(
            (
                minimum.numeric_value(),
                minimum.unsigned(),
                minimum.raw_value()
            ),
            (-256, 0, u64::MAX - 255)
        );
        assert!(HpInt::parse_literal("FF:-64h", IntBase::Hex).is_none());
        assert!(HpInt::from_raw(0xffff_ffff_ffff_fe00, -8, IntBase::Hex).is_none());
        assert_eq!(
            HpInt::from_raw(negative.raw_value(), -8, IntBase::Hex),
            Some(negative)
        );
        assert_eq!(
            HpInt::from_raw(minimum.raw_value(), -8, IntBase::Hex),
            Some(minimum)
        );
    }
    #[test]
    fn signed_arithmetic_wraps_without_changing_metadata_or_mistaking_positive_ff_for_negative() {
        let word = |value| HpInt::new_with_width(value, -8, IntBase::Hex).unwrap();
        assert_eq!(word(255).add(word(1)).numeric_value(), 0);
        assert_eq!(word(0).sub(word(1)).numeric_value(), -1);
        assert_eq!(word(-5).div(word(2)).unwrap().numeric_value(), -2);
        assert_eq!(word(-128).mul(word(4)).numeric_value(), -256);
        assert_eq!(word(-5).shift_right(1).numeric_value(), -3);
        assert_eq!(word(-5).shift_right(64).numeric_value(), -1);
        assert_eq!(word(-5).shift_left(8).numeric_value(), -256);
        assert_eq!(
            HpInt::new(3, 32, IntBase::Hex).pow(0x65).unsigned(),
            0x6da83b73
        );
    }
    #[test]
    fn display_context_respects_sign_width_and_system_base_affinity() {
        let word = HpInt::new_with_width(-1, -8, IntBase::Hex).unwrap();
        assert_eq!(word.format_for(8, true, IntBase::Hex), "#−1h");
        assert_eq!(word.format_for(8, false, IntBase::Hex), "#−1:-8h");
        let implicit = HpInt::parse_literal("FF", IntBase::Hex).unwrap();
        assert!(implicit.is_system_base());
        assert_eq!(implicit.format_for(32, false, IntBase::Dec), "#255d");
        assert_eq!(
            implicit
                .with_base(IntBase::Hex)
                .format_for(32, false, IntBase::Dec),
            "#FFh"
        );
        assert_eq!(
            HpInt::new(255, 32, IntBase::Hex).format_for(32, true, IntBase::Hex),
            "#FF:32h"
        );
        assert_eq!(
            HpInt::parse_literal_with_defaults("FFh", IntBase::Hex, -8)
                .unwrap()
                .width_code(),
            -8
        );
        assert_eq!(
            HpInt::parse_literal_with_defaults("FF:8h", IntBase::Hex, -8)
                .unwrap()
                .width_code(),
            8
        );
    }
}
