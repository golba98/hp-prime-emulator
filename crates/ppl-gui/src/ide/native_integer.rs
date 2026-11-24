//! Edit Integer keeps a copy of the selected word's actual carrier metadata.
use serde::{Deserialize, Deserializer, Serialize};
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct IntegerEdit {
    pub raw: u64,
    pub bits: u8,
    pub base: u8,
    pub signed: bool,
    pub negative: bool,
    pub system_base: bool,
}
impl<'de> Deserialize<'de> for IntegerEdit {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Saved {
            raw: u64,
            bits: u8,
            base: u8,
            signed: bool,
            #[serde(default)]
            negative: Option<bool>,
            #[serde(default)]
            system_base: bool,
        }
        let s = Saved::deserialize(d)?;
        if !(1..=64).contains(&s.bits) {
            return Err(serde::de::Error::custom("Invalid saved integer width"));
        }
        let legacy = s.negative.is_none();
        let negative = s
            .negative
            .unwrap_or(s.signed && s.raw & (1u64 << (s.bits - 1)) != 0);
        // Older editor copies interpreted the top payload bit as a sign. Keep
        // that displayed number, using the admitted extra-sign representation.
        let bits = if legacy && s.signed && s.bits == 64 {
            63
        } else {
            s.bits
        };
        let raw = if legacy && s.signed && s.bits == 64 {
            s.raw & mask(63)
        } else {
            s.raw
        };
        let result = Self {
            raw,
            bits,
            base: s.base,
            signed: s.signed,
            negative,
            system_base: s.system_base,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}
fn mask(bits: u8) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
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
                ppl_math::HpInt::new_with_width(
                    i128::from(n),
                    settings.integer_width_code(),
                    settings.int_base,
                )
                .ok_or("Invalid integer settings")?
                .with_system_base(true)
            }
        };
        let mut result = Self {
            raw: word.unsigned(),
            bits: word.bits(),
            base: word.base.radix() as u8,
            signed: word.is_signed(),
            negative: word.is_negative(),
            system_base: word.is_system_base(),
        };
        result.sync_system_base(settings.int_base);
        result.validate()?;
        Ok(result)
    }
    fn int_base(&self) -> Option<ppl_math::IntBase> {
        Some(match self.base {
            2 => ppl_math::IntBase::Bin,
            8 => ppl_math::IntBase::Oct,
            10 => ppl_math::IntBase::Dec,
            16 => ppl_math::IntBase::Hex,
            _ => return None,
        })
    }
    fn word(&self) -> Option<ppl_math::HpInt> {
        if !(1..=64).contains(&self.bits)
            || (self.signed && self.bits == 64)
            || (!self.signed && self.negative)
            || self.raw > mask(self.bits)
        {
            return None;
        }
        let width = if self.signed {
            -(self.bits as i64)
        } else {
            self.bits as i64
        };
        let carrier = self.raw | if self.negative { !mask(self.bits) } else { 0 };
        ppl_math::HpInt::from_raw(carrier, width, self.int_base()?)
            .map(|w| w.with_system_base(self.system_base))
    }
    fn accept(&mut self, w: ppl_math::HpInt) {
        self.raw = w.unsigned();
        self.bits = w.bits();
        self.signed = w.is_signed();
        self.negative = w.is_negative();
        self.system_base = w.is_system_base();
        self.base = w.base.radix() as u8;
    }
    pub fn sync_system_base(&mut self, base: ppl_math::IntBase) {
        if self.system_base {
            self.base = base.radix() as u8;
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.word()
            .map(|_| ())
            .ok_or_else(|| "Invalid saved integer editor".into())
    }
    pub fn shift(&mut self, left: bool) {
        if let Some(w) = self.word() {
            self.accept(if left {
                w.shift_left(1)
            } else {
                w.shift_right(1)
            });
        }
    }
    pub fn resize(&mut self, bits: u8) -> Result<(), String> {
        if !(1..=64).contains(&bits) || (self.signed && bits == 64) {
            return Err("Use1–64 unsigned or1–63 signed magnitude bits".into());
        }
        let w = self
            .word()
            .ok_or("Invalid integer editor")?
            .with_word_size(if self.signed {
                -(bits as i64)
            } else {
                bits as i64
            })
            .ok_or("Invalid integer width")?;
        self.accept(w);
        Ok(())
    }
    pub fn set_signed(&mut self, signed: bool) -> Result<(), String> {
        if signed && self.bits == 64 {
            return Err("Signed words support at most63 magnitude bits".into());
        }
        let w = self
            .word()
            .ok_or("Invalid integer editor")?
            .with_word_size(if signed {
                -(self.bits as i64)
            } else {
                self.bits as i64
            })
            .ok_or("Invalid integer width")?;
        self.accept(w);
        Ok(())
    }
    pub fn negate(&mut self) {
        if let Some(w) = self.word() {
            self.accept(w.neg());
        }
    }
    pub fn complement(&mut self) {
        if let Some(w) = self.word() {
            self.accept(w.not());
        }
    }
    pub fn cycle(&mut self, forward: bool) {
        let bases = [2, 8, 10, 16];
        let current = bases.iter().position(|b| *b == self.base).unwrap_or(3);
        self.base = bases[(current + if forward { 1 } else { 3 }) % 4];
        self.system_base = false;
    }
    pub fn decimal(&self) -> String {
        self.word()
            .map(|w| w.numeric_value().to_string())
            .unwrap_or_else(|| "Invalid word".into())
    }
    pub fn output(&self) -> String {
        self.word()
            .map(|w| {
                format!(
                    "{} :{} bits",
                    w.format_for(self.bits, self.signed, w.base),
                    self.bits
                )
            })
            .unwrap_or_else(|| "Invalid word".into())
    }
    pub fn set_decimal(&mut self, text: &str) -> Result<(), String> {
        let w = self.word().ok_or("Invalid integer editor")?;
        let value = text
            .parse::<i128>()
            .map_err(|_| "Enter an integer".to_string())?;
        let min = if self.signed {
            -(1i128 << self.bits)
        } else {
            0
        };
        let max = (1i128 << self.bits) - 1;
        if value < min || value > max {
            return Err("Integer does not fit the selected word size".into());
        }
        self.accept(w.with_value(value));
        Ok(())
    }
    pub fn set_hex(&mut self, text: &str) -> Result<(), String> {
        let value = u64::from_str_radix(text.trim_start_matches('#'), 16)
            .map_err(|_| "Enter hexadecimal digits".to_string())?;
        if value > mask(self.bits) {
            return Err("Integer does not fit the selected word size".into());
        }
        let mut next = self.clone();
        next.raw = value;
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn display_bits(&self) -> u8 {
        self.bits + u8::from(self.signed)
    }
    pub fn bit(&self, bit: u8) -> bool {
        if self.signed && bit == self.bits {
            self.negative
        } else {
            bit < self.bits && self.raw & (1u64 << bit) != 0
        }
    }
    pub fn toggle_bit(&mut self, bit: u8) {
        if self.signed && bit == self.bits {
            self.negative = !self.negative;
        } else if bit < self.bits {
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
            negative: false,
            system_base: false,
        }
    }
    #[test]
    fn extra_sign_is_independent_of_the_high_payload_bit() {
        let mut w = word();
        assert_eq!(w.decimal(), "128");
        w.negate();
        assert_eq!(w.decimal(), "-128");
        assert!(w.negative);
        w.complement();
        assert_eq!(w.decimal(), "127");
        w.shift(true);
        assert_eq!(w.decimal(), "254");
        w.shift(false);
        assert_eq!(w.decimal(), "127");
        w.resize(4).unwrap();
        assert_eq!(w.raw, 15);
        w.toggle_bit(0);
        assert_eq!(w.raw, 14);
        w.set_decimal("8").unwrap();
        assert_eq!(w.decimal(), "8");
        w.set_decimal("-16").unwrap();
        assert_eq!(w.raw, 0);
        assert_eq!(w.decimal(), "-16");
        assert_eq!(w.display_bits(), 5);
        w.toggle_bit(4);
        assert_eq!(w.decimal(), "0");
        w.set_decimal("-8").unwrap();
        w.set_hex("A").unwrap();
        assert_eq!(w.decimal(), "-6");
        assert!(w.set_hex("FF").is_err());
        let before = w.clone();
        assert!(w.resize(64).is_err());
        assert_eq!(w, before);
    }
    #[test]
    fn source_word_sign_and_system_base_survive_settings_and_persistence() {
        let settings = ppl_vm::Settings {
            int_signed: false,
            int_base: ppl_math::IntBase::Dec,
            ..Default::default()
        };
        for (value, width) in [(255, -8), (-5, -8), (255, 8)] {
            let w = ppl_math::HpInt::new_with_width(value, width, ppl_math::IntBase::Hex)
                .unwrap()
                .with_system_base(true);
            let edit =
                IntegerEdit::from_value(&ppl_vm::PortableValue::Integer(w), &settings).unwrap();
            assert_eq!(edit.decimal(), value.to_string());
            assert_eq!(edit.signed, width < 0);
            assert_eq!(edit.base, 10);
            assert!(edit.system_base);
            let saved = serde_json::to_string(&edit).unwrap();
            assert_eq!(serde_json::from_str::<IntegerEdit>(&saved).unwrap(), edit);
        }
    }
    #[test]
    fn full_unsigned_word_and_legacy_copies_remain_lossless() {
        let mut w = word();
        w.set_signed(false).unwrap();
        w.resize(64).unwrap();
        w.set_decimal("18446744073709551615").unwrap();
        assert_eq!(w.raw, u64::MAX);
        assert_eq!(w.decimal(), u64::MAX.to_string());
        assert!(w.set_signed(true).is_err());
        assert!(!w.signed);
        let saved = serde_json::to_string(&w).unwrap();
        let restored: IntegerEdit = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, w);
        let legacy: IntegerEdit =
            serde_json::from_str(r#"{"raw":255,"bits":8,"base":16,"signed":true}"#).unwrap();
        assert_eq!(legacy.decimal(), "-1");
        let legacy: IntegerEdit = serde_json::from_str(
            r#"{"raw":18446744073709551615,"bits":64,"base":16,"signed":true}"#,
        )
        .unwrap();
        assert_eq!(legacy.decimal(), "-1");
        assert_eq!(legacy.bits, 63);
        w.bits = 0;
        assert!(w.validate().is_err());
    }
}
