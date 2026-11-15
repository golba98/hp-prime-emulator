//! STRING numeric formatting. Explicit separator/DotZero contracts are covered
//! by genuine local firmware observations retained with the remediation report.
use super::fractions::{current_fraction, decimal_places, fraction};
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::{Machine, Settings, Value};
use ppl_math::{FormatOptions, NumFormat, Real};

#[derive(Clone)]
struct Format {
    options: FormatOptions,
    mode: i64,
    version: crate::StringFormatVersion,
    numeric_version: ppl_math::NumericVersion,
    precision: u8,
    grouping: Vec<u16>,
    repeating: Option<(usize, usize, u16)>,
    decimal: u16,
    exponent: u16,
    negative: u16,
    leading_zero: bool,
    delimiter: u16,
    collection_spaces: bool,
    fitting: Option<Fitting>,
}

/// STRING fits each numeric component, rather than truncating a completed
/// collection or replacing its text with an unrelated scientific rendering.
#[derive(Clone)]
struct Fitting {
    width: i64,
    pixels: f32,
    style: ppl_graphics::text::prime::FontStyle,
    fixed_cell: bool,
}
impl Fitting {
    fn parse(value: &Value, settings: &Settings) -> RtResult<Self> {
        use ppl_graphics::text::prime::{FontStyle, is_selected};
        let (width, pixels, style, fixed_cell) = match value {
            Value::List(values) if (1..=5).contains(&values.len()) => {
                let width = values[0].expect_i64("STRING size limit")?;
                // An omitted font uses the Home system size. Explicit zero
                // uses the small bitmap face; styled zero selects the 8px
                // installed face, unlike TEXTOUT's system-font zero.
                let mut zero_code = false;
                let pixels = if let Some(font) = values.get(1) {
                    let code = font.expect_i64("STRING font code")?;
                    if !(0..=7).contains(&code) {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgValue,
                            "STRING font code must be 0 through 7",
                        ));
                    }
                    zero_code = code == 0;
                    8. + 2. * code as f32
                } else {
                    settings.font_px
                };
                let mut flags = [false; 3];
                for (flag, value) in flags.iter_mut().zip(values.iter().skip(2)) {
                    *flag = !value.expect_real("STRING font style")?.is_zero();
                }
                // Original numeric advance controls establish that italic
                // changes ink only; mono takes precedence over bold.
                let style = if flags[2] {
                    FontStyle::Mono
                } else if flags[0] {
                    FontStyle::Bold
                } else {
                    FontStyle::Regular
                };
                (
                    width,
                    pixels,
                    style,
                    zero_code && style == FontStyle::Regular,
                )
            }
            Value::List(_) => {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "STRING size list needs one through five entries",
                ));
            }
            value => (
                value.expect_i64("STRING size limit")?,
                settings.font_px,
                FontStyle::Regular,
                false,
            ),
        };
        if width < 0 {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "STRING size limit must be nonnegative",
            ));
        }
        if style != FontStyle::Regular && !is_selected() {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                "STRING styled width fitting requires the installed Prime font resource",
            ));
        }
        Ok(Self {
            width,
            pixels,
            style,
            fixed_cell,
        })
    }
    fn fits(&self, text: &[u16]) -> bool {
        if self.fixed_cell {
            return text.len().saturating_mul(6) as u64 <= self.width as u64;
        }
        ppl_graphics::text::prime::with_style(self.style, || {
            ppl_graphics::text::measure_utf16_units(text, self.pixels) <= self.width
        })
    }
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
        let base = if mode % 7 == 0 {
            settings.hformat + 1
        } else {
            mode % 7
        };
        let mut options = settings.format();
        options.format = match base {
            2 => NumFormat::Fixed(precision),
            3 => NumFormat::Scientific(precision),
            4 => NumFormat::Engineering(precision),
            5 => NumFormat::Floating(precision),
            6 => NumFormat::Rounded(precision),
            _ => NumFormat::Standard,
        };
        let mut format = Self {
            options,
            mode,
            version: settings.string_format_version,
            numeric_version: settings.numeric_version,
            precision,
            grouping: Vec::new(),
            repeating: None,
            decimal: settings.digit_grouping.decimal() as u16,
            exponent: 'ᴇ' as u16,
            negative: '−' as u16,
            leading_zero: true,
            delimiter: if settings.digit_grouping.decimal() == ',' {
                b';' as u16
            } else {
                b',' as u16
            },
            collection_spaces: false,
            fitting: args
                .get(4)
                .map(|value| Fitting::parse(value, settings))
                .transpose()?,
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
                value if value.is_string() => {
                    let units = value.expect_string_units("STRING grouping pattern")?;
                    let units = units
                        .iter()
                        .copied()
                        .take_while(|unit| *unit != 0)
                        .collect::<Vec<_>>();
                    if units
                        .last()
                        .is_some_and(|unit| !(u16::from(b'0')..=u16::from(b'9')).contains(unit))
                    {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgValue,
                            "STRING grouping pattern must end in a digit",
                        ));
                    }
                    units
                }
                value => {
                    let code = value.expect_i64("STRING separator")?;
                    match code {
                        -1 => Vec::new(),
                        0..=10 => {
                            format.repeating = match code {
                                0 => Some((3, 3, '﹐' as u16)),
                                1 | 5 => Some((3, 3, '\'' as u16)),
                                2 | 6 => Some((3, 3, ' ' as u16)),
                                4 => Some((3, 3, '﹒' as u16)),
                                7 => Some((3, 3, '˙' as u16)),
                                9 => Some((4, 4, ',' as u16)),
                                10 => Some((3, 2, ',' as u16)),
                                _ => None,
                            };
                            Vec::new()
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
                let symbols =
                    symbols.expect_string_units("STRING decimal/exponent/sign symbols")?;
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
    fn numeric(&self, value: Real) -> Vec<u16> {
        if value.presentation() == ppl_math::RealPresentation::Sexagesimal {
            return value.format(self.options).encode_utf16().collect();
        }
        let legacy = self.version == crate::StringFormatVersion::Legacy15157;
        if self.mode >= 7
            && let Some((numerator, denominator)) = if legacy {
                fraction(value.decimal(), self.options.format)
            } else if self.numeric_version == ppl_math::NumericVersion::Current15751 {
                match value.origin() {
                    ppl_math::RealOrigin::Rational {
                        numerator,
                        denominator,
                    } => Some((u128::from(numerator), u128::from(denominator))),
                    _ => None,
                }
            } else {
                current_fraction(value, self.precision)
            }
        {
            let mut out = Vec::new();
            if value.is_negative() && numerator != 0 {
                out.push(self.negative);
            }
            let number = |n| {
                let number = Real::from_parts(false, n, 0).expect("bounded fraction coefficient");
                if !legacy {
                    return self.component(number);
                }
                let mut options = self.options;
                options.format = NumFormat::Standard;
                self.component_with_options(number, options)
            };
            if denominator == 1 {
                out.extend(number(numerator));
            } else {
                let numerator = if self.mode >= 14 && numerator > denominator {
                    out.extend(number(numerator / denominator));
                    out.push(if !legacy {
                        u16::from(b' ')
                    } else if value.is_negative() {
                        self.negative
                    } else {
                        u16::from(b'+')
                    });
                    numerator % denominator
                } else {
                    numerator
                };
                out.extend(number(numerator));
                out.push(u16::from(b'/'));
                out.extend(number(denominator));
            }
            return terminated(out);
        }
        if legacy && self.mode >= 7 && !value.is_zero() {
            let exponent = value.exp10();
            match self.options.format {
                NumFormat::Standard if !(-11..12).contains(&exponent) => {
                    let mut options = self.options;
                    options.format = NumFormat::Scientific(11);
                    return self.component_with_options(value, options);
                }
                NumFormat::Floating(p)
                    if exponent > i32::from(p) || exponent < -i32::from(p) - 1 =>
                {
                    // Floating's scientific fallback re-enters STRING's
                    // numeric conversion. Its new precision comes from the
                    // rounded coefficient, so it can select a fraction too.
                    let rounded = value.round_places(-i32::from(p) - 1);
                    let places = rounded.mantissa_digits().len().saturating_sub(1) as u8;
                    let mut nested = self.clone();
                    nested.options.format = NumFormat::Scientific(places);
                    return nested.numeric(value);
                }
                NumFormat::Fixed(p)
                | NumFormat::Scientific(p)
                | NumFormat::Engineering(p)
                | NumFormat::Floating(p)
                | NumFormat::Rounded(p)
                    if exponent < -11 && i32::from(p) >= -exponent =>
                {
                    // Outside the fraction routine's input exponent window it
                    // returns the scalar. Its integer-component formatter uses
                    // decimal places, including the twelfth fractional place.
                    return self.component_from_raw(decimal_places(value, p, true));
                }
                NumFormat::Engineering(p) if exponent < -11 => {
                    let mut options = self.options;
                    options.format = NumFormat::Scientific(p);
                    return self.component_with_options(value, options);
                }
                NumFormat::Floating(p) if exponent < 0 => {
                    let places = p.saturating_add(1).min(12);
                    let rounded = value.round_places(i32::from(places));
                    return self.component_from_raw(decimal_places(
                        value,
                        places,
                        !rounded.is_zero(),
                    ));
                }
                _ => {}
            }
        }
        self.component(value)
    }
    fn component(&self, value: Real) -> Vec<u16> {
        let original = self.component_with_options(value, self.options);
        let Some(fitting) = &self.fitting else {
            return original;
        };
        if fitting.fits(&original) {
            return original;
        }
        let standard = self.options.format == NumFormat::Standard;
        let precision = if standard { 11 } else { self.precision.min(11) };
        for precision in (0..=precision).rev() {
            let mut options = self.options;
            options.format = match options.format {
                NumFormat::Standard | NumFormat::Fixed(_) => NumFormat::Fixed(precision),
                NumFormat::Scientific(_) => NumFormat::Scientific(precision),
                NumFormat::Engineering(_) => NumFormat::Engineering(precision),
                NumFormat::Floating(_) => NumFormat::Floating(precision),
                NumFormat::Rounded(_) => NumFormat::Rounded(precision),
            };
            let candidate = self.component_with_options(value, options);
            if fitting.fits(&candidate) {
                return candidate;
            }
        }
        // A width cannot discard integral digits. Preserve the original
        // mantissa and append exponent-zero only in the observed base modes.
        if matches!(
            self.options.format,
            NumFormat::Standard | NumFormat::Fixed(_) | NumFormat::Rounded(_)
        ) && !original.contains(&self.exponent)
        {
            let mut result = original;
            result.extend([self.exponent, u16::from(b'0')]);
            terminated(result)
        } else {
            original
        }
    }
    fn component_with_options(&self, value: Real, options: FormatOptions) -> Vec<u16> {
        self.component_from_raw(value.format(options))
    }
    fn component_from_raw(&self, mut raw: String) -> Vec<u16> {
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
        let mut groups = std::collections::BTreeMap::<usize, Vec<u16>>::new();
        let mut right = 0;
        for unit in self.grouping.iter().rev().copied() {
            if (u16::from(b'0')..=u16::from(b'9')).contains(&unit) {
                right += 1;
            } else {
                groups.entry(right).or_default().insert(0, unit);
            }
        }
        let mut output = Vec::new();
        if sign {
            output.push(self.negative);
        }
        for (index, character) in digits.bytes().enumerate() {
            if index > 0 {
                let remaining = digits.len() - index;
                if let Some((first, repeat, separator)) = self.repeating {
                    if remaining >= first && (remaining - first) % repeat == 0 {
                        output.push(separator);
                    }
                } else if let Some(separator) = groups.get(&remaining) {
                    output.extend(separator);
                }
            }
            output.push(u16::from(character));
        }
        for character in mantissa[decimal..].encode_utf16() {
            output.push(if character == u16::from(b'.') {
                self.decimal
            } else {
                character
            });
        }
        if exponent < raw.len() {
            output.push(self.exponent);
            for character in raw[exponent + self.options.exp_char.len_utf8()..].encode_utf16() {
                output.push(if character == u16::from(b'-') {
                    self.negative
                } else {
                    character
                });
            }
        }
        terminated(output)
    }
    fn units(&self, value: &Value) -> Vec<u16> {
        match value {
            Value::Real(value) => self.numeric(*value),
            Value::Complex(value) => {
                if value.presentation() != ppl_math::ComplexPresentation::Rectangular {
                    return value
                        .format(self.options)
                        .replace('i', "\u{e003}")
                        .encode_utf16()
                        .collect();
                }
                if value.im.is_zero() {
                    return self.numeric(value.re);
                }
                let mut out = Vec::new();
                if self.options.complex_format == ppl_math::ComplexFormat::Pair {
                    out.push(u16::from(b'('));
                    out.extend(self.numeric(value.re));
                    out.push(self.delimiter);
                    out.extend(self.numeric(value.im));
                    out.push(u16::from(b')'));
                    return out;
                }
                if value.re.is_zero() {
                    if value.im.is_negative() {
                        out.push(self.negative);
                    }
                } else {
                    out.extend(self.numeric(value.re));
                    out.push(if value.im.is_negative() {
                        self.negative
                    } else {
                        u16::from(b'+')
                    });
                }
                if value.im.abs() != Real::ONE {
                    out.extend(self.numeric(value.im.abs()));
                    out.push(u16::from(b'*'));
                }
                out.push(0xe003);
                terminated(out)
            }
            Value::RawString(units) => {
                let mut out = vec![u16::from(b'"')];
                for unit in units.iter().copied() {
                    if unit == u16::from(b'"') {
                        out.push(u16::from(b'\\'));
                    }
                    out.push(unit);
                }
                out.push(u16::from(b'"'));
                out
            }
            Value::List(values) => {
                let mut out = vec![u16::from(b'{')];
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        out.push(self.delimiter);
                        if self.collection_spaces {
                            out.push(u16::from(b' '));
                        }
                    }
                    out.extend(self.units(value));
                }
                out.push(u16::from(b'}'));
                out
            }
            Value::Matrix(matrix) => {
                let mut out = Vec::new();
                if !matrix.vector {
                    out.push(u16::from(b'['));
                }
                for row in 0..matrix.rows {
                    if row != 0 {
                        out.push(self.delimiter);
                    }
                    out.push(u16::from(b'['));
                    for (column, value) in matrix.row(row).iter().enumerate() {
                        if column != 0 {
                            out.push(self.delimiter);
                            if self.collection_spaces {
                                out.push(u16::from(b' '));
                            }
                        }
                        out.extend(self.units(value));
                    }
                    out.push(u16::from(b']'));
                }
                if !matrix.vector {
                    out.push(u16::from(b']'));
                }
                out
            }
            other => other.string_form(self.options).encode_utf16().collect(),
        }
    }
}
fn terminated(mut units: Vec<u16>) -> Vec<u16> {
    if let Some(end) = units.iter().position(|unit| *unit == 0) {
        units.truncate(end);
    }
    units
}

pub(super) fn string(machine: &Machine, args: &[Value]) -> RtResult<Value> {
    let format = Format::from_args(&machine.settings, args)?;
    Ok(Value::from_utf16(format.units(&args[0])))
}

/// Menu and graphics value labels use calculator collection spacing. STRING's
/// returned data keeps its separate, compact collection contract.
pub(super) fn menu_label(machine: &Machine, value: &Value) -> RtResult<Value> {
    let mut format = Format::from_args(&machine.settings, std::slice::from_ref(value))?;
    format.collection_spaces = true;
    format.repeating = machine
        .settings
        .digit_grouping
        .grouping()
        .map(|(first, repeat, separator)| (first, repeat, separator as u16));
    Ok(Value::from_utf16(format.units(value)))
}
