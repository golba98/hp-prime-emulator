//! STRING numeric formatting. Explicit separator/DotZero contracts are covered
//! by genuine local firmware observations retained with the remediation report.
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::{Machine, Settings, Value};
use ppl_math::{FormatOptions, NumFormat, Real};

struct Format {
    options: FormatOptions,
    mode: i64,
    precision: u8,
    grouping: String,
    repeating: Option<(usize, usize, char)>,
    decimal: char,
    exponent: char,
    negative: char,
    leading_zero: bool,
}
impl Format {
    fn from_args(settings: &Settings, args: &[Value]) -> RtResult<Self> {
        let mode = args
            .get(1)
            .map(|v| v.expect_i64("STRING mode"))
            .transpose()?
            .unwrap_or(0);
        if !(0..=20).contains(&mode) {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "STRING mode must be 0 through 20",
            ));
        }
        let precision = args
            .get(2)
            .map(|v| v.expect_i64("STRING precision"))
            .transpose()?
            .unwrap_or(-1);
        if !(-1..=12).contains(&precision) {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "STRING precision must be -1 through 12",
            ));
        }
        let precision = if precision == -1 {
            settings.hdigits as u8
        } else {
            precision as u8
        };
        let base = mode % 7;
        let mut options = settings.format();
        options.format = match base {
            0 => options.format,
            2 => NumFormat::Fixed(precision),
            3 => NumFormat::Scientific(precision),
            4 => NumFormat::Engineering(precision),
            _ => NumFormat::Standard,
        };
        if matches!(base, 3..=5) {
            options.exp_char = 'ᴇ';
        }
        let mut format = Self {
            options,
            mode,
            precision,
            grouping: String::new(),
            repeating: None,
            decimal: '.',
            exponent: options.exp_char,
            negative: if mode >= 7 { '−' } else { '-' },
            leading_zero: true,
        };
        if let Some(separator) = args.get(3) {
            let (separator, symbols, leading) = match separator {
                Value::List(values) if (1..=3).contains(&values.len()) => {
                    (values.first().unwrap(), values.get(1), values.get(2))
                }
                Value::List(_) => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "STRING separator list needs one through three entries",
                    ));
                }
                value => (value, None, None),
            };
            format.grouping = match separator {
                Value::Str(text) => text.to_string(),
                value => {
                    let code = value.expect_i64("STRING separator")?;
                    match code {
                        -1 => String::new(),
                        0..=10 => {
                            format.repeating = match code {
                                0 => Some((3, 3, '﹐')),
                                1 | 5 => Some((3, 3, '\'')),
                                2 | 6 => Some((3, 3, ' ')),
                                4 => Some((3, 3, '﹒')),
                                7 => Some((3, 3, '˙')),
                                9 => Some((4, 4, ',')),
                                10 => Some((3, 2, ',')),
                                _ => None,
                            };
                            String::new()
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                ErrorKind::BadArgValue,
                                "STRING separator must be -1 through 10",
                            ));
                        }
                    }
                }
            };
            if let Some(symbols) = symbols {
                let symbols = symbols.expect_str("STRING decimal/exponent/sign symbols")?;
                let symbols: Vec<_> = symbols.chars().collect();
                if symbols.len() > 3 {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "STRING uses at most three decimal/exponent/sign symbols",
                    ));
                }
                if let Some(symbol) = symbols.first() {
                    format.decimal = *symbol;
                }
                if let Some(symbol) = symbols.get(1) {
                    format.exponent = *symbol;
                }
                if let Some(symbol) = symbols.get(2) {
                    format.negative = *symbol;
                }
            }
            if let Some(leading) = leading {
                format.leading_zero = !leading.expect_real("STRING DotZero")?.is_zero();
            }
        }
        Ok(format)
    }
    fn numeric(&self, value: Real) -> String {
        let mut raw = if self.mode >= 7 {
            fraction(value, self.mode >= 14, self.precision)
                .unwrap_or_else(|| value.format(self.options))
        } else if matches!(self.mode % 7, 5 | 6) {
            let floating = self.mode % 7 == 5;
            let exponent = value.exp10();
            let mut options = self.options;
            options.format = if floating
                && (exponent > self.precision as i32 || exponent < -(self.precision as i32))
            {
                NumFormat::Scientific(self.precision)
            } else {
                NumFormat::Fixed(self.precision + u8::from(floating && exponent < 0))
            };
            let text = value.format(options);
            let (mantissa, exponent) = text
                .split_once(options.exp_char)
                .map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
            let trimmed = if mantissa.contains('.') {
                mantissa.trim_end_matches('0').trim_end_matches('.')
            } else {
                mantissa
            };
            match exponent {
                Some(exponent) => format!("{trimmed}{}{exponent}", options.exp_char),
                None => trimmed.to_owned(),
            }
        } else {
            value.format(self.options)
        };
        if !self.leading_zero {
            if raw.starts_with("0.") {
                raw.remove(0);
            } else if raw.starts_with("-0.") {
                raw.remove(1);
            }
        }
        let exponent = raw.find(self.options.exp_char).unwrap_or(raw.len());
        let mantissa = &raw[..exponent];
        let sign = mantissa.starts_with('-');
        let decimal = mantissa.find('.').unwrap_or(mantissa.len());
        let digits = &mantissa[usize::from(sign)..decimal];
        let mut groups = std::collections::BTreeMap::<usize, String>::new();
        let mut right = 0;
        for character in self.grouping.chars().rev() {
            if character.is_ascii_digit() {
                right += 1;
            } else {
                groups.entry(right).or_default().insert(0, character);
            }
        }
        let mut output = String::new();
        if sign {
            output.push(self.negative);
        }
        for (index, character) in digits.chars().enumerate() {
            if index > 0 {
                let remaining = digits.len() - index;
                if let Some((first, repeat, separator)) = self.repeating {
                    if remaining >= first && (remaining - first) % repeat == 0 {
                        output.push(separator);
                    }
                } else if let Some(separator) = groups.get(&remaining) {
                    output.push_str(separator);
                }
            }
            output.push(character);
        }
        for character in mantissa[decimal..].chars() {
            output.push(if character == '.' {
                self.decimal
            } else {
                character
            });
        }
        if exponent < raw.len() {
            output.push(self.exponent);
            for character in raw[exponent + self.options.exp_char.len_utf8()..].chars() {
                output.push(if character == '-' {
                    self.negative
                } else {
                    character
                });
            }
        }
        output
    }
    fn value(&self, value: &Value) -> String {
        match value {
            Value::Real(value) => self.numeric(*value),
            Value::Complex(value) => {
                if value.im.is_zero() {
                    return self.numeric(value.re);
                }
                let imaginary = if value.im.abs() == Real::ONE {
                    "i".to_owned()
                } else {
                    format!("{}*i", self.numeric(value.im.abs()))
                };
                if value.re.is_zero() {
                    if value.im.is_negative() {
                        format!("{}{imaginary}", self.negative)
                    } else {
                        imaginary
                    }
                } else {
                    format!(
                        "{}{}{imaginary}",
                        self.numeric(value.re),
                        if value.im.is_negative() {
                            self.negative
                        } else {
                            '+'
                        }
                    )
                }
            }
            Value::List(values) => format!(
                "{{{}}}",
                values
                    .iter()
                    .map(|v| self.value(v))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Matrix(matrix) => {
                let rows = (0..matrix.rows)
                    .map(|row| {
                        format!(
                            "[{}]",
                            matrix
                                .row(row)
                                .iter()
                                .map(|v| self.value(v))
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    })
                    .collect::<Vec<_>>();
                if matrix.vector {
                    rows.into_iter().next().unwrap_or_else(|| "[]".into())
                } else {
                    format!("[{}]", rows.join(","))
                }
            }
            other => other.string_form(self.options),
        }
    }
}

/// Continued fractions reproduce common calculator fraction entries without
/// overflowing at the outer decimal exponent range.
fn fraction(value: Real, mixed: bool, precision: u8) -> Option<String> {
    let x = value.abs().to_f64();
    if !x.is_finite() || x > 1e15 {
        return None;
    }
    let tolerance = 10f64.powi(-(precision.max(1) as i32));
    let mut y = x;
    let (mut h0, mut h1, mut k0, mut k1) = (0u64, 1u64, 1u64, 0u64);
    let mut result = None;
    for _ in 0..48 {
        let a = y.floor() as u64;
        let h = a.checked_mul(h1)?.checked_add(h0)?;
        let k = a.checked_mul(k1)?.checked_add(k0)?;
        if k > 1_000_000_000_000 {
            break;
        }
        result = Some((h, k));
        if (h as f64 / k as f64 - x).abs() <= tolerance {
            break;
        }
        let remainder = y - a as f64;
        if remainder == 0.0 {
            break;
        }
        (h0, h1, k0, k1) = (h1, h, k1, k);
        y = 1.0 / remainder;
    }
    let (numerator, denominator) = result?;
    let sign = if value.is_negative() { "-" } else { "" };
    Some(if denominator == 1 {
        format!("{sign}{numerator}")
    } else if mixed && numerator > denominator {
        format!(
            "{sign}{} {}/{}",
            numerator / denominator,
            numerator % denominator,
            denominator
        )
    } else {
        format!("{sign}{numerator}/{denominator}")
    })
}

pub(super) fn string(machine: &Machine, args: &[Value]) -> RtResult<Value> {
    let mut format = Format::from_args(&machine.settings, args)?;
    let mut result = format.value(&args[0]);
    if let Some(size) = args.get(4) {
        let (limit, font) = match size {
            Value::List(values) if (1..=5).contains(&values.len()) => {
                let limit = values[0].expect_i64("STRING size limit")?;
                let font = values
                    .get(1)
                    .map(|v| v.expect_i64("STRING font size"))
                    .transpose()?
                    .unwrap_or(12);
                if !(10..=22).contains(&font) {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "STRING font size must be 10 through 22",
                    ));
                }
                for style in values.iter().skip(2) {
                    if !style.expect_real("STRING font style")?.is_zero() {
                        return Err(RuntimeError::new(
                            ErrorKind::Unsupported,
                            "STRING bold, italic and monospaced width fitting requires matching font metrics",
                        ));
                    }
                }
                (limit, font as f32)
            }
            Value::List(_) => {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "STRING size list needs one through five entries",
                ));
            }
            value => (value.expect_i64("STRING size limit")?, 12.0),
        };
        if limit < 0 {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "STRING size limit must be nonnegative",
            ));
        }
        if let Value::Real(value) = args[0] {
            for digits in (0..=format.precision).rev() {
                if ppl_graphics::text::measure(&result, font) <= limit {
                    break;
                }
                format.options.format = NumFormat::Scientific(digits);
                let candidate = format.numeric(value);
                if ppl_graphics::text::measure(&candidate, font)
                    < ppl_graphics::text::measure(&result, font)
                {
                    result = candidate;
                }
            }
        }
    }
    Ok(Value::str(result))
}
