//! Calculator display preferences are separate from canonical numeric transport.
//! Separator code points follow original G1 ASC(STRING(...)) controls; the Home
//! menus combine that grouping choice with the matching decimal convention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DigitGrouping(u8);
impl DigitGrouping {
    pub fn from_code(code: i64) -> Option<Self> {
        (0..=10).contains(&code).then_some(Self(code as u8))
    }
    pub fn code(self) -> i64 {
        i64::from(self.0)
    }
    pub fn decimal(self) -> char {
        if (4..=8).contains(&self.0) { ',' } else { '.' }
    }
    pub fn grouping(self) -> Option<(usize, usize, char)> {
        match self.0 {
            0 => Some((3, 3, '﹐')),
            1 | 5 => Some((3, 3, '\'')),
            2 | 6 => Some((3, 3, ' ')),
            4 => Some((3, 3, '﹒')),
            7 => Some((3, 3, '˙')),
            9 => Some((4, 4, ',')),
            10 => Some((3, 2, ',')),
            _ => None,
        }
    }
    /// Group an unsigned, plain integer literal for painting without a
    /// numeric conversion. Short fields and non-integer syntax keep their text.
    pub fn grouped_integer_literal(self, digits: &str) -> Option<String> {
        let (first, _, _) = self.grouping()?;
        (digits.len() > first && digits.bytes().all(|digit| digit.is_ascii_digit()))
            .then(|| self.display(digits, 'E'))
    }

    pub(crate) fn display(self, raw: &str, exponent: char) -> String {
        let end = raw.find(exponent).unwrap_or(raw.len());
        let (mantissa, suffix) = raw.split_at(end);
        let (integer, fraction) = mantissa
            .split_once('.')
            .map_or((mantissa, None), |(i, f)| (i, Some(f)));
        let (sign, integer) = integer
            .strip_prefix('-')
            .map_or(("", integer), |i| ("-", i));
        let mut result = String::from(sign);
        for (index, ch) in integer.chars().enumerate() {
            result.push(ch);
            let remaining = integer.len() - index - 1;
            if let Some((first, repeat, separator)) = self.grouping() {
                if remaining >= first && (remaining - first) % repeat == 0 {
                    result.push(separator);
                }
            }
        }
        if let Some(fraction) = fraction {
            result.push(self.decimal());
            result.push_str(fraction);
        }
        result.push_str(suffix);
        result
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComplexFormat {
    #[default]
    Algebraic,
    Pair,
}
impl ComplexFormat {
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Algebraic),
            1 => Some(Self::Pair),
            _ => None,
        }
    }
    pub fn code(self) -> i64 {
        match self {
            Self::Algebraic => 0,
            Self::Pair => 1,
        }
    }
}
