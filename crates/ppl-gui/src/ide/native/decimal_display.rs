//! Exact display conversion for finite decimal Home results. No rational
//! approximation is invented: values outside the bounded integer codec retain
//! their decimal display, while evaluation keeps the original typed value.
#[derive(Debug, PartialEq, Eq)]
pub struct FractionDisplay {
    pub common: String,
    pub mixed: String,
}
impl FractionDisplay {
    pub fn from_decimal(text: &str) -> Option<Self> {
        let (mantissa, exponent) = text
            .split_once(['e', 'E'])
            .map_or(Some((text, 0)), |(m, e)| {
                e.parse::<i32>().ok().map(|e| (m, e))
            })?;
        let negative = mantissa.starts_with('-');
        let digits = mantissa.trim_start_matches(['-', '+']);
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        if whole.is_empty()
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mut numerator = format!("{whole}{fraction}").parse::<u128>().ok()?;
        let scale = i32::try_from(fraction.len()).ok()?.checked_sub(exponent)?;
        let mut denominator = 1u128;
        if scale > 0 {
            denominator = 10u128.checked_pow(scale.try_into().ok()?)?;
        } else {
            numerator = numerator
                .checked_mul(10u128.checked_pow(scale.checked_neg()?.try_into().ok()?)?)?;
        }
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        numerator /= a;
        denominator /= a;
        let sign = if negative && numerator != 0 { "-" } else { "" };
        let common = if denominator == 1 {
            format!("{sign}{numerator}")
        } else {
            format!("{sign}{numerator}/{denominator}")
        };
        let whole = numerator / denominator;
        let remainder = numerator % denominator;
        let mixed = if whole == 0 || remainder == 0 {
            common.clone()
        } else {
            let join = if negative { "-" } else { "+" };
            format!("{sign}{whole}{join}{remainder}/{denominator}")
        };
        Some(Self { common, mixed })
    }
    pub fn next(&self, current: &str, decimal: &str) -> String {
        if current == self.common && self.mixed != self.common {
            self.mixed.clone()
        } else if current == self.mixed {
            decimal.to_owned()
        } else {
            self.common.clone()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_mixed_decimal_cycle_is_exact_and_sign_safe() {
        for (decimal, common, mixed) in [
            ("5.25", "21/4", "5+1/4"),
            ("-5.25", "-21/4", "-5-1/4"),
            ("1.25E-2", "1/80", "1/80"),
            ("0", "0", "0"),
            ("1.2E3", "1200", "1200"),
        ] {
            let value = FractionDisplay::from_decimal(decimal).unwrap();
            assert_eq!(value.common, common);
            assert_eq!(value.mixed, mixed);
            assert_eq!(value.next(decimal, decimal), common);
            if common != mixed {
                assert_eq!(value.next(common, decimal), mixed);
            }
            assert_eq!(value.next(mixed, decimal), decimal);
        }
    }
    #[test]
    fn decimal_limits_are_explicit_and_never_approximated() {
        for value in ["1E-499", "1E499", "NaN", "1/3", "", "5.2.3"] {
            assert!(FractionDisplay::from_decimal(value).is_none(), "{value}");
        }
        assert_eq!(
            FractionDisplay::from_decimal("0.333333333333")
                .unwrap()
                .common,
            "333333333333/1000000000000"
        );
    }
}
