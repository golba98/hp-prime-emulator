//! Edit Integer works on a copy of a selected result; it never changes Ans.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegerEdit {
    pub raw: u64,
    pub bits: u8,
    pub base: u8,
    pub signed: bool,
}
impl IntegerEdit {
    pub fn from_value(
        value: &ppl_vm::PortableValue,
        settings: &ppl_vm::Settings,
    ) -> Result<Self, String> {
        let word = match value {
            ppl_vm::PortableValue::Integer(word) => *word,
            _ => {
                let n = value
                    .to_value()
                    .expect_i64("Edit Integer")
                    .map_err(|_| "Select an integer result".to_string())?;
                ppl_math::HpInt::new(i128::from(n), settings.int_bits, settings.int_base)
            }
        };
        Ok(Self {
            raw: word.unsigned(),
            bits: word.bits(),
            base: word.base.radix() as u8,
            signed: settings.int_signed,
        })
    }
    fn word(&self) -> ppl_math::HpInt {
        ppl_math::HpInt::new(
            i128::from(self.raw),
            self.bits,
            match self.base {
                2 => ppl_math::IntBase::Bin,
                8 => ppl_math::IntBase::Oct,
                10 => ppl_math::IntBase::Dec,
                _ => ppl_math::IntBase::Hex,
            },
        )
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.bits)
            || !matches!(self.base, 2 | 8 | 10 | 16)
            || self.word().unsigned() != self.raw
        {
            Err("Invalid saved integer editor".into())
        } else {
            Ok(())
        }
    }
    pub fn shift(&mut self, left: bool) {
        self.raw = if left {
            self.word().shift_left(1)
        } else {
            self.word().shift_right(1)
        }
        .unsigned();
    }
    pub fn resize(&mut self, bits: u8) {
        self.bits = bits.clamp(1, 64);
        self.raw = self.word().unsigned();
    }
    pub fn negate(&mut self) {
        self.raw = self.word().neg().unsigned();
    }
    pub fn complement(&mut self) {
        self.raw = self.word().not().unsigned();
    }
    pub fn cycle(&mut self, forward: bool) {
        let bases = [2, 8, 10, 16];
        let current = bases
            .iter()
            .position(|base| *base == self.base)
            .unwrap_or(3);
        self.base = bases[(current + if forward { 1 } else { 3 }) % 4];
    }
    pub fn decimal(&self) -> String {
        if self.signed {
            self.word().signed().to_string()
        } else {
            self.raw.to_string()
        }
    }
    pub fn output(&self) -> String {
        format!("{} :{} bits", self.word(), self.bits)
    }
    pub fn set_decimal(&mut self, text: &str) -> Result<(), String> {
        let value = text
            .parse::<i128>()
            .map_err(|_| "Enter an integer".to_string())?;
        let min = if self.signed {
            -(1i128 << (self.bits - 1))
        } else {
            0
        };
        let max = if self.signed {
            (1i128 << (self.bits - 1)) - 1
        } else {
            (1i128 << self.bits) - 1
        };
        if value < min || value > max {
            return Err("Integer does not fit the selected word size".into());
        }
        self.raw = ppl_math::HpInt::new(value, self.bits, self.word().base).unsigned();
        Ok(())
    }
    pub fn set_hex(&mut self, text: &str) -> Result<(), String> {
        let value = u64::from_str_radix(text.trim_start_matches('#'), 16)
            .map_err(|_| "Enter hexadecimal digits".to_string())?;
        let word = ppl_math::HpInt::new(i128::from(value), self.bits, ppl_math::IntBase::Hex);
        if word.unsigned() != value {
            return Err("Integer does not fit the selected word size".into());
        }
        self.raw = value;
        Ok(())
    }
    pub fn toggle_bit(&mut self, bit: u8) {
        if bit < self.bits {
            self.raw ^= 1u64 << bit;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn word() -> IntegerEdit {
        IntegerEdit {
            raw: 128,
            bits: 8,
            base: 16,
            signed: true,
        }
    }
    #[test]
    fn edits_follow_fixed_width_words_and_signed_display() {
        let mut w = word();
        assert_eq!(w.decimal(), "-128");
        w.negate();
        assert_eq!(w.raw, 128);
        w.complement();
        assert_eq!(w.raw, 127);
        w.shift(true);
        assert_eq!(w.raw, 254);
        w.shift(false);
        assert_eq!(w.raw, 127);
        w.resize(4);
        assert_eq!(w.raw, 15);
        w.toggle_bit(0);
        assert_eq!(w.raw, 14);
        w.cycle(true);
        assert_eq!(w.base, 2);
        w.cycle(false);
        assert_eq!(w.base, 16);
        assert!(w.set_decimal("8").is_err());
        w.set_decimal("-8").unwrap();
        assert_eq!(w.raw, 8);
        assert!(w.set_hex("FF").is_err());
        w.set_hex("A").unwrap();
        assert_eq!(w.decimal(), "-6");
    }
    #[test]
    fn full_unsigned_word_is_lossless_and_validated() {
        let mut w = word();
        w.resize(64);
        w.signed = false;
        w.set_decimal("18446744073709551615").unwrap();
        assert_eq!(w.raw, u64::MAX);
        assert_eq!(w.decimal(), "18446744073709551615");
        let j = serde_json::to_string(&w).unwrap();
        let restored: IntegerEdit = serde_json::from_str(&j).unwrap();
        assert_eq!(restored, w);
        restored.validate().unwrap();
        w.bits = 0;
        assert!(w.validate().is_err());
    }
}
