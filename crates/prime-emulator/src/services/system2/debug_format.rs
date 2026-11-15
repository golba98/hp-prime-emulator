//! Bounded C debug formatting with AAPCS32 argument selection. Never forward
//! a guest format or pointer to libc: only a parsed, canonical single conversion
//! and a host-owned typed value cross that boundary.
use std::ffi::{CString, c_char, c_int};

pub(super) const MAX_OUTPUT: usize = 16_384;
const MAX_FIELD: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Length {
    Char,
    Short,
    #[default]
    Normal,
    Long,
    LongLong,
    Size,
    PtrDiff,
    IntMax,
    Wide,
    LongDouble,
}
impl Length {
    fn bytes(self) -> u8 {
        match self {
            Self::Char => 1,
            Self::Short => 2,
            Self::LongLong | Self::IntMax => 8,
            _ => 4,
        }
    }
}
#[derive(Clone, Copy)]
enum Size {
    Fixed(usize),
    Argument,
}
#[derive(Default)]
struct Flags {
    left: bool,
    zero: bool,
    plus: bool,
    space: bool,
    alternate: bool,
}
struct Conversion {
    flags: Flags,
    width: Size,
    precision: Option<Size>,
    length: Length,
    code: char,
}
enum Piece {
    Text(String),
    Conversion(Conversion),
}

fn number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<usize> {
    let mut value = 0usize;
    while let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(10)) {
        chars.next();
        value = value.checked_mul(10)?.checked_add(digit as usize)?;
        if value > MAX_FIELD {
            return None;
        }
    }
    Some(value)
}
fn size(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<Size> {
    if chars.peek() == Some(&'*') {
        chars.next();
        Some(Size::Argument)
    } else {
        number(chars).map(Size::Fixed)
    }
}

/// Validate the whole format before reading arguments or handling `%n`.
fn parse(format: &str) -> Option<Vec<Piece>> {
    if format.len() > MAX_OUTPUT {
        return None;
    }
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            literal.push(ch);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            literal.push('%');
            continue;
        }
        if !literal.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut literal)));
        }
        let mut flags = Flags::default();
        loop {
            match chars.peek() {
                Some('-') => flags.left = true,
                Some('0') => flags.zero = true,
                Some('+') => flags.plus = true,
                Some(' ') => flags.space = true,
                Some('#') => flags.alternate = true,
                _ => break,
            }
            chars.next();
        }
        let width = size(&mut chars)?;
        let precision = if chars.peek() == Some(&'.') {
            chars.next();
            Some(size(&mut chars)?)
        } else {
            None
        };
        let length = match chars.peek() {
            Some('h') => {
                chars.next();
                if chars.peek() == Some(&'h') {
                    chars.next();
                    Length::Char
                } else {
                    Length::Short
                }
            }
            Some('l') => {
                chars.next();
                if chars.peek() == Some(&'l') {
                    chars.next();
                    Length::LongLong
                } else {
                    Length::Long
                }
            }
            Some('I') => {
                chars.next();
                match (chars.next(), chars.next()) {
                    (Some('6'), Some('4')) => Length::LongLong,
                    (Some('3'), Some('2')) => Length::Normal,
                    _ => return None,
                }
            }
            Some('j') => {
                chars.next();
                Length::IntMax
            }
            Some('z') => {
                chars.next();
                Length::Size
            }
            Some('t') => {
                chars.next();
                Length::PtrDiff
            }
            Some('w') => {
                chars.next();
                Length::Wide
            }
            Some('L') => {
                chars.next();
                Length::LongDouble
            }
            _ => Length::Normal,
        };
        let code = chars.next()?;
        let valid = match code {
            'd' | 'i' | 'u' | 'o' | 'x' | 'X' | 'n' => {
                !matches!(length, Length::Wide | Length::LongDouble)
            }
            's' | 'S' | 'c' | 'C' => matches!(
                length,
                Length::Normal | Length::Char | Length::Short | Length::Long | Length::Wide
            ),
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'a' | 'A' => {
                matches!(length, Length::Normal | Length::Long | Length::LongDouble)
            }
            'p' => length == Length::Normal,
            _ => false,
        };
        if !valid {
            return None;
        }
        pieces.push(Piece::Conversion(Conversion {
            flags,
            width,
            precision,
            length,
            code,
        }));
    }
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    Some(pieces)
}

struct Arguments<F> {
    read: F,
    next: usize,
}
impl<F> Arguments<F> {
    fn word<E>(&mut self) -> Result<u32, E>
    where
        F: FnMut(usize) -> Result<u32, E>,
    {
        let word = (self.read)(self.next)?;
        self.next += 1;
        Ok(word)
    }
    fn double_word<E>(&mut self) -> Result<u64, E>
    where
        F: FnMut(usize) -> Result<u32, E>,
    {
        // AAPCS32 C.3: eight-byte values start at an even core-register
        // index or an eight-byte-aligned caller-stack address. r0 is fmt.
        self.next = (self.next + 1) & !1;
        let low = self.word()? as u64;
        Ok(low | (self.word()? as u64) << 32)
    }
}
enum Value {
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Text(CString),
    Char(c_int),
}

fn render(format: &str, value: &Value) -> Option<String> {
    let format = CString::new(format).ok()?;
    let mut buffer = vec![0u8; MAX_OUTPUT + 1];
    // SAFETY: format is a generated single conversion with a type matching
    // this branch. Text pointers own NUL-terminated storage for the call;
    // buffer is writable for exactly its supplied size. No guest pointer,
    // `%n`, dynamic precision or unvalidated format reaches libc.
    let written = unsafe {
        let output = buffer.as_mut_ptr().cast::<c_char>();
        let size = buffer.len();
        match value {
            Value::Signed(value) => {
                libc::snprintf(output, size, format.as_ptr(), *value as libc::c_longlong)
            }
            Value::Unsigned(value) => {
                libc::snprintf(output, size, format.as_ptr(), *value as libc::c_ulonglong)
            }
            Value::Float(value) => libc::snprintf(output, size, format.as_ptr(), *value),
            Value::Text(value) => libc::snprintf(output, size, format.as_ptr(), value.as_ptr()),
            Value::Char(value) => libc::snprintf(output, size, format.as_ptr(), *value),
        }
    };
    if written < 0 || written as usize > MAX_OUTPUT {
        return None;
    }
    buffer.truncate(written as usize);
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

fn canonical(flags: &Flags, width: usize, precision: Option<usize>, kind: &str) -> String {
    let mut format = String::from("%");
    for (enabled, ch) in [
        (flags.left, '-'),
        (flags.zero, '0'),
        (flags.plus, '+'),
        (flags.space, ' '),
        (flags.alternate, '#'),
    ] {
        if enabled {
            format.push(ch);
        }
    }
    if width != 0 {
        format.push_str(&width.to_string());
    }
    if let Some(precision) = precision {
        format.push('.');
        format.push_str(&precision.to_string());
    }
    format.push_str(kind);
    format
}

pub(super) fn format<E>(
    format: &str,
    argument: impl FnMut(usize) -> Result<u32, E>,
    mut text: impl FnMut(u32, bool, Option<usize>) -> Result<String, E>,
    mut count: impl FnMut(u32, u8, usize) -> Result<(), E>,
) -> Result<Option<String>, E> {
    let Some(pieces) = parse(format) else {
        return Ok(None);
    };
    let mut args = Arguments {
        read: argument,
        next: 1,
    };
    let mut output = String::new();
    for piece in pieces {
        let Piece::Conversion(mut spec) = piece else {
            let Piece::Text(literal) = piece else {
                unreachable!()
            };
            if literal.len() > MAX_OUTPUT - output.len() {
                return Ok(None);
            }
            output.push_str(&literal);
            continue;
        };
        let width = match spec.width {
            Size::Fixed(width) => width,
            Size::Argument => {
                let width = args.word()? as i32;
                if width < 0 {
                    spec.flags.left = true;
                }
                width.unsigned_abs() as usize
            }
        };
        let precision = match spec.precision {
            Some(Size::Fixed(value)) => Some(value),
            Some(Size::Argument) => {
                let value = args.word()? as i32;
                (value >= 0).then_some(value as usize)
            }
            None => None,
        };
        if width > MAX_FIELD || precision.is_some_and(|value| value > MAX_FIELD) {
            return Ok(None);
        }
        if spec.code == 'n' {
            count(args.word()?, spec.length.bytes(), output.len())?;
            continue;
        }
        let (kind, value) = match spec.code {
            'd' | 'i' => {
                let value = if spec.length.bytes() == 8 {
                    args.double_word()? as i64
                } else {
                    let word = args.word()?;
                    match spec.length {
                        Length::Char => word as i8 as i64,
                        Length::Short => word as i16 as i64,
                        _ => word as i32 as i64,
                    }
                };
                ("lld".to_owned(), Value::Signed(value))
            }
            'u' | 'o' | 'x' | 'X' => {
                let value = if spec.length.bytes() == 8 {
                    args.double_word()?
                } else {
                    let word = args.word()?;
                    match spec.length {
                        Length::Char => word as u8 as u64,
                        Length::Short => word as u16 as u64,
                        _ => word as u64,
                    }
                };
                (format!("ll{}", spec.code), Value::Unsigned(value))
            }
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'a' | 'A' => (
                spec.code.to_string(),
                Value::Float(f64::from_bits(args.double_word()?)),
            ),
            's' | 'S' => {
                let address = args.word()?;
                let wide = matches!(spec.length, Length::Long | Length::Wide)
                    || spec.code == 'S' && !matches!(spec.length, Length::Char | Length::Short);
                let mut value = if precision == Some(0) {
                    String::new()
                } else {
                    text(address, wide, precision)?
                };
                if wide && let Some(limit) = precision {
                    let mut end = value.len().min(limit);
                    while !value.is_char_boundary(end) {
                        end -= 1;
                    }
                    value.truncate(end);
                }
                let Ok(value) = CString::new(value) else {
                    return Ok(None);
                };
                ("s".into(), Value::Text(value))
            }
            'c' | 'C' => {
                let value = args.word()?;
                let wide = matches!(spec.length, Length::Long | Length::Wide)
                    || spec.code == 'C' && !matches!(spec.length, Length::Char | Length::Short);
                if wide && value != 0 {
                    let Some(ch) = char::from_u32(value & 0xffff) else {
                        return Ok(None);
                    };
                    let Ok(value) = CString::new(ch.to_string()) else {
                        return Ok(None);
                    };
                    ("s".into(), Value::Text(value))
                } else {
                    ("c".into(), Value::Char((value & 255) as c_int))
                }
            }
            'p' => {
                // Preserve the emulator's existing guest-address convention;
                // passing it as a host pointer would use host pointer width.
                let mut value = format!("0x{:08x}", args.word()?);
                let padding = width.saturating_sub(value.len());
                if spec.flags.left {
                    value.push_str(&" ".repeat(padding));
                } else if spec.flags.zero {
                    value.insert_str(2, &"0".repeat(padding));
                } else {
                    value.insert_str(0, &" ".repeat(padding));
                }
                output.push_str(&value);
                if output.len() > MAX_OUTPUT {
                    return Ok(None);
                }
                continue;
            }
            _ => unreachable!(),
        };
        let Some(formatted) = render(&canonical(&spec.flags, width, precision, &kind), &value)
        else {
            return Ok(None);
        };
        if formatted.len() > MAX_OUTPUT - output.len() {
            return Ok(None);
        }
        output.push_str(&formatted);
    }
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, convert::Infallible};
    fn formatted(format: &str, words: &[u32]) -> Option<String> {
        self::format::<Infallible>(
            format,
            |index| Ok(words[index - 1]),
            |_, wide, _| Ok(if wide { "wide" } else { "abcdef" }.into()),
            |_, _, _| Ok(()),
        )
        .unwrap()
    }
    #[test]
    fn reads_aligned_double_and_signed64_across_the_register_stack_boundary() {
        let double = 2.5f64.to_bits();
        let integer = (-1_234_567_890_123i64) as u64;
        let words = [
            0xbad,
            double as u32,
            (double >> 32) as u32,
            integer as u32,
            (integer >> 32) as u32,
        ];
        let reads = RefCell::new(Vec::new());
        let value = self::format::<Infallible>(
            "%.2f %lld",
            |index| {
                reads.borrow_mut().push(index);
                Ok(words[index - 1])
            },
            |_, _, _| unreachable!(),
            |_, _, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(value.as_deref(), Some("2.50 -1234567890123"));
        assert_eq!(*reads.borrow(), [2, 3, 4, 5]);
    }
    #[test]
    fn flags_integer_lengths_precision_and_dynamic_fields_are_real_conversions() {
        assert_eq!(
            formatted("%hhd %hu %I64u", &[255, 0x10001, 0xbad, u32::MAX, u32::MAX]).as_deref(),
            Some("-1 1 18446744073709551615")
        );
        assert_eq!(
            formatted("%#08x %.0d %#.0o", &[0xabcd, 0, 0]).as_deref(),
            Some("0x00abcd  0")
        );
        assert_eq!(
            formatted("%*.*s", &[(-8i32) as u32, 3, 0x100]).as_deref(),
            Some("abc     ")
        );
        assert_eq!(
            formatted("%ls %hs", &[1, 2]).as_deref(),
            Some("wide abcdef")
        );
    }
    #[test]
    fn scientific_general_hexadecimal_and_nonfinite_float_formats() {
        for (format, value, expected) in [
            ("%.2e", 125.0, "1.25e+02"),
            ("%.3g", 0.000123, "0.000123"),
            ("%.3g", 1_230_000.0, "1.23e+06"),
            ("%a", 1.5, "0x1.8p+0"),
            ("%+.1f", -0.0, "-0.0"),
            ("%F", f64::INFINITY, "INF"),
        ] {
            let bits = value.to_bits();
            assert_eq!(
                formatted(format, &[0xbad, bits as u32, (bits >> 32) as u32]).as_deref(),
                Some(expected),
                "{format}"
            );
        }
    }
    #[test]
    fn output_budget_includes_literals_and_wide_precision_keeps_scalars_whole() {
        let format = format!("%4096s%4096s%4096s%4096s{}", "x".repeat(100));
        assert!(formatted(&format, &[1, 2, 3, 4]).is_none());
        assert_eq!(
            self::format::<Infallible>(
                "%.3ls",
                |_| Ok(1),
                |_, _, _| Ok("ππ".into()),
                |_, _, _| unreachable!()
            )
            .unwrap()
            .as_deref(),
            Some("π")
        );
        assert_eq!(formatted("%*d", &[i32::MIN as u32, 1]), None);
    }
    #[test]
    fn count_writes_are_explicit_guest_operations_and_invalid_formats_do_nothing() {
        let writes = RefCell::new(Vec::new());
        let result = self::format::<Infallible>(
            "ab%hhn!%lln",
            |index| Ok(index as u32),
            |_, _, _| unreachable!(),
            |pointer, bytes, count| {
                writes.borrow_mut().push((pointer, bytes, count));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result.as_deref(), Some("ab!"));
        assert_eq!(*writes.borrow(), [(1, 1, 2), (2, 8, 3)]);
        for invalid in ["%q", "%", "%999999999d", "%ls %Lf %Q"] {
            assert!(
                self::format::<Infallible>(
                    invalid,
                    |_| panic!("invalid format read arguments"),
                    |_, _, _| unreachable!(),
                    |_, _, _| unreachable!()
                )
                .unwrap()
                .is_none()
            );
        }
        assert_eq!(
            self::format::<Infallible>(
                "%.0s",
                |_| Ok(0xdead),
                |_, _, _| panic!("zero precision read a pointer"),
                |_, _, _| unreachable!()
            )
            .unwrap(),
            Some(String::new())
        );
    }
}
