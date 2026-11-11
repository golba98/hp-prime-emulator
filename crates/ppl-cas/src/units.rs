//! HP measurement syntax at the Home/CAS boundary.
//!
//! HP Prime User Guide, chapter 24 (Units and constants), defines `8.175_cm/s`
//! and `5_kg*m^2/s^2` as compound measurements. Giac requires a grouped unit
//! suffix for these. This translator changes syntax only; real Giac performs
//! dimensional arithmetic, conversion, and errors. No unit factors are faked.
use crate::{CasError, MAX_REQUEST};

/// Translate supported HP measurement syntax for the external Giac service.
/// `None` means ordinary Home syntax: the caller should retain its native
/// evaluator. Quoted strings, comments and identifiers such as `speed_m` do not
/// activate this boundary. Unsupported attached units fail explicitly.
///
/// The four documented unit tools are also accepted with uppercase HP names.
/// User-defined unit names and affine temperature conversion are not supported.
pub fn translate_home(source: &str) -> Result<Option<String>, CasError> {
    if source.len() > MAX_REQUEST {
        return Err(CasError::InvalidRequest(
            "measurement expression too large".into(),
        ));
    }
    let mut output = String::with_capacity(source.len());
    let mut i = 0;
    let mut measurement = false;
    while i < source.len() {
        let c = source[i..].chars().next().unwrap();
        if c == '"' || source[i..].starts_with("//") || source[i..].starts_with("/*") {
            let end = opaque_end(source, i);
            output.push_str(&source[i..end]);
            i = end;
        } else if c.is_ascii_digit()
            || (c == '.' && source[i + 1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            let end = number_end(source, i);
            output.push_str(&source[i..end]);
            i = end;
        } else if c == '_' && attachment_boundary(source, i) {
            let (unit, end) = unit_expression(source, i + 1, false, 0)?;
            output.push_str("_(");
            output.push_str(&unit);
            output.push(')');
            measurement = true;
            i = end;
        } else if c.is_alphabetic() || c == '_' {
            let end = identifier_end(source, i);
            let word = &source[i..end];
            let following = skip_space(source, end);
            let replacement = match word {
                "CONVERT" | "convert" => Some("convert"),
                "MKSA" | "mksa" => Some("mksa"),
                "UFACTOR" | "ufactor" => Some("ufactor"),
                "USIMPLIFY" | "usimplify" => Some("usimplify"),
                _ => None,
            };
            if following < source.len() && source[following..].starts_with('(') {
                if let Some(replacement) = replacement {
                    output.push_str(replacement);
                    measurement = true;
                } else {
                    output.push_str(word);
                }
            } else {
                output.push_str(word);
            }
            i = end;
        } else {
            output.push(c);
            i += c.len_utf8();
        }
    }
    Ok(measurement.then_some(output))
}

fn opaque_end(s: &str, start: usize) -> usize {
    if s[start..].starts_with("//") {
        return s[start..].find('\n').map_or(s.len(), |n| start + n + 1);
    }
    if s[start..].starts_with("/*") {
        return s[start + 2..].find("*/").map_or(s.len(), |n| start + n + 4);
    }
    let mut escaped = false;
    for (offset, c) in s[start + 1..].char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return start + 1 + offset + 1;
        }
    }
    s.len()
}
fn skip_space(s: &str, mut i: usize) -> usize {
    while let Some(c) = s[i..].chars().next().filter(|c| c.is_whitespace()) {
        i += c.len_utf8();
    }
    i
}
fn identifier_end(s: &str, i: usize) -> usize {
    let mut end = i;
    for c in s[i..].chars() {
        if !(c.is_alphanumeric() || c == '_' || c == 'µ' || c == 'Ω' || c == '°') {
            break;
        }
        end += c.len_utf8();
    }
    end
}
fn attachment_boundary(s: &str, i: usize) -> bool {
    s[..i]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphabetic() && c != '_')
}
fn number_end(s: &str, mut end: usize) -> usize {
    while s
        .as_bytes()
        .get(end)
        .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
    {
        end += 1;
    }
    if s.as_bytes()
        .get(end)
        .is_some_and(|c| matches!(c, b'e' | b'E'))
    {
        let mut exponent = end + 1;
        if s.as_bytes()
            .get(exponent)
            .is_some_and(|c| matches!(c, b'+' | b'-'))
        {
            exponent += 1;
        }
        let digits = exponent;
        while s.as_bytes().get(exponent).is_some_and(u8::is_ascii_digit) {
            exponent += 1;
        }
        if exponent > digits {
            end = exponent;
        }
    }
    end
}
fn invalid(message: &str) -> CasError {
    CasError::InvalidRequest(format!("measurement: {message}"))
}
fn unit_expression(
    s: &str,
    mut i: usize,
    grouped: bool,
    depth: usize,
) -> Result<(String, usize), CasError> {
    if depth >= 32 {
        return Err(invalid("unit nesting exceeds limit"));
    }
    i = skip_space(s, i);
    let (mut result, mut end) = unit_factor(s, i, depth)?;
    loop {
        let operator = skip_space(s, end);
        if operator >= s.len() {
            break;
        }
        let c = s.as_bytes()[operator];
        if c != b'*' && c != b'/' {
            break;
        }
        if s[operator..].starts_with("//") || s[operator..].starts_with("/*") {
            break;
        }
        let next = skip_space(s, operator + 1);
        match unit_factor(s, next, depth) {
            Ok((factor, next_end)) => {
                result.push(c as char);
                result.push_str(&factor);
                end = next_end;
            }
            Err(_) if !grouped => break, // 2_m*3_m and 2_m*x remain arithmetic.
            Err(error) => return Err(error),
        }
    }
    Ok((result, end))
}
fn unit_factor(s: &str, i: usize, depth: usize) -> Result<(String, usize), CasError> {
    let i = skip_space(s, i);
    if i >= s.len() {
        return Err(invalid("missing unit after underscore"));
    }
    let (mut result, mut end) = if s[i..].starts_with('(') {
        let (inner, end) = unit_expression(s, i + 1, true, depth + 1)?;
        let end = skip_space(s, end);
        if !s[end..].starts_with(')') {
            return Err(invalid("invalid compound unit"));
        }
        (format!("({inner})"), end + 1)
    } else {
        let end = identifier_end(s, i);
        let name = &s[i..end];
        let normalized =
            normalize(name).ok_or_else(|| invalid(&format!("unsupported unit {name:?}")))?;
        (normalized, end)
    };
    let power = skip_space(s, end);
    if s[power..].starts_with('^') {
        let mut exponent_end = skip_space(s, power + 1);
        let start = exponent_end;
        if s[exponent_end..].starts_with(['+', '-']) {
            exponent_end += 1;
        }
        let digits = exponent_end;
        while s
            .as_bytes()
            .get(exponent_end)
            .is_some_and(u8::is_ascii_digit)
        {
            exponent_end += 1;
        }
        if exponent_end == digits {
            return Err(invalid("unit exponent must be numeric"));
        }
        if s[exponent_end..].starts_with('.') {
            exponent_end += 1;
            while s
                .as_bytes()
                .get(exponent_end)
                .is_some_and(u8::is_ascii_digit)
            {
                exponent_end += 1;
            }
        }
        result.push('^');
        result.push_str(&s[start..exponent_end]);
        end = exponent_end;
    }
    Ok((result, end))
}
fn normalize(name: &str) -> Option<String> {
    // Giac 2's parser reserves `in` and `min`; its documented equivalent unit
    // spellings are inch and mn. Ohm is case sensitive. These are syntax aliases.
    // `_fm` is Giac's fathom, so do not pretend it is HP's femto-metre.
    if name == "fm" {
        return None;
    }
    match name {
        "in" | "inch" => return Some("inch".into()),
        "min" | "mn" => return Some("mn".into()),
        "Ω" | "ohm" | "Ohm" => return Some("Ohm".into()),
        _ => (),
    }
    const BASE: &[&str] = &[
        "m", "g", "s", "A", "K", "mol", "Hz", "N", "Pa", "J", "W", "C", "V", "F", "Ohm", "S", "Wb",
        "T", "H", "Bq", "Gy", "Sv", "L", "l", "rad", "deg",
    ];
    const OTHER: &[&str] = &[
        "kg", "h", "ft", "yd", "mi", "lb", "oz", "ton", "atm", "bar", "psi",
    ];
    if BASE.contains(&name) || OTHER.contains(&name) {
        return Some(name.into());
    }
    const PREFIX: &[&str] = &[
        "Y", "Z", "E", "P", "T", "G", "M", "k", "h", "D", "d", "c", "m", "µ", "n", "p", "f", "a",
        "z", "y",
    ];
    PREFIX
        .iter()
        .any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|base| BASE.contains(&base))
        })
        .then(|| name.into())
}
